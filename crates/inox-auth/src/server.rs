//! The socket service the login screen talks to.
//!
//! The service is small on purpose, and every rule in it is one the session-lock boundary asked
//! for:
//!
//! - **The peer is checked before anything else happens.** `SO_PEERCRED` — the
//!   kernel's answer, not the client's — must name a process this boundary serves
//!   (the greeter's uid, or root). A socket is only as private as its peers.
//! - **One attempt at a time.** `libpam`'s conversation is per-process state and
//!   this service keeps it on a single thread, which is also what makes "the
//!   session this boundary starts belongs to the attempt that was accepted" a
//!   property rather than a lock discipline to get right in two places.
//! - **A connection is one attempt.** After a refusal, a second attempt needs a
//!   second connection: there is then exactly one place where an attempt starts
//!   and exactly one where it ends, and no way to leave half of one behind.
//! - **Nothing about an attempt is decided by the client.** The command that is
//!   started comes from configuration; the uid, gid, groups and home come from the
//!   account database; the only things the wire carries are an account name, a
//!   credential and an identifier to correlate the answer with.
//!
//! Since the boot login chain this service also owns the *placement* of a session, which is
//! what the rest of this file is about when it is not about a socket:
//!
//! - `StartSession` records which logind session is active on the seat, asks logind
//!   for the first free VT in `config.session_vt`, and starts a session leader
//!   (`inox-auth --session-leader --user … --vt …`) for that account on that VT.
//!   It answers `SessionStarted { started: true }` only once the leader has
//!   confirmed that `pam_open_session` succeeded.
//! - Exactly one session at a time: a second `StartSession` while one is running is
//!   `Unavailable(Busy)`, and a second `Authenticate` too.
//! - When the session process exits the service reaps it, then activates the
//!   session it recorded before the switch — the greeter's, in the shipped
//!   deployment — so the display comes back without the login screen doing anything
//!   cross-VT.
//!
//! Failures are answered rather than shouted: a malformed frame gets
//! `Reply::Error`, a rate-limited or already-busy attempt gets
//! `Reply::Unavailable`, and anything the service does not understand ends the
//! connection with a log line. Nothing in this file panics on client input — the
//! fuzz-shaped inputs are in `tests/boundary.rs`, which drives the real server over
//! a real socket.
//!
//! When logind cannot be reached — a CI runner, a development instance — the
//! session is still started: the VT is chosen without arbitration and the display
//! is not handed back, both with a warning. A login chain that refuses to work
//! without a system service it can live without would be untestable.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use inox_common::secret::Secret;
use rustix::fs::chown;
use rustix::net::Shutdown;
use rustix::net::sockopt::socket_peercred;
use rustix::process::Gid;
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::boundary::{BoundaryFactory, Verdict};
use crate::leader::{DEFAULT_LEADER_DEADLINE, LeaderPlan, SessionLauncher, ShippedLauncher};
use crate::limit::{AttemptLog, RatePolicy};
use crate::logind::{self, Logind, SystemLogind, VtRange};
use crate::session::{self, Account, SessionError};
use crate::wire::{self, Attempt, ErrorCode, Reply, Request, UnavailableReason};

/// The socket the shipped binary listens on.
///
/// Defined once, in the contract (`wire::DEFAULT_SOCKET`): the login screen and the
/// boundary are two halves of one endpoint, and two constants are two chances to
/// disagree about it.
pub use crate::wire::DEFAULT_SOCKET;

/// The PAM service the shipped binary authenticates through.
///
/// It is the name of `packaging/pam.d/inox-auth`, which the boot login chain
/// as `/etc/pam.d/inox-auth` and whose `auth`/`account` lines walk `system-login`.
/// The deployment unit passes `--service` for the same value; the default and the
/// file that ships are one decision, not two.
pub const DEFAULT_PAM_SERVICE: &str = "inox-auth";

/// The PAM service the session leader opens its session through.
///
/// **A different stack from [`DEFAULT_PAM_SERVICE`], deliberately.** The leader
/// holds no credential, and the safe `pam` wrapper only
/// opens a session on a client that has authenticated, so the leader's transaction
/// authenticates against a stack whose `auth` line permits unconditionally
/// (`auth required pam_permit.so`) while its `account`/`session` lines are the real
/// login stack. That cannot be the same file the password is checked against: a
/// permit there would accept anybody.
///
/// See `pam::open_session` for the required contents, and this crate's `AGENTS.md`
/// for the packaging requirement.
pub const DEFAULT_SESSION_SERVICE: &str = "inox-auth-session";

/// The session the shipped binary starts after a successful login.
pub const DEFAULT_SESSION_PROGRAM: &str = "inox-session";

/// The floor below which an account is a system account rather than a person's.
///
/// Debian and every distribution that follows it allocate interactive accounts
/// from 1000; root is refused separately, whatever this is set to.
pub const DEFAULT_MINIMUM_UID: u32 = 1000;

/// How long a client has to finish the handshake.
///
/// Short, because the handshake is two frames with nothing to think about: a
/// connection that has not spoken by then is holding the one attempt this service
/// can serve.
pub const DEFAULT_HANDSHAKE_DEADLINE: Duration = Duration::from_secs(5);

/// How long a client has to finish an attempt, and how long the authenticator has
/// to answer one.
///
/// The same 30 s the login screen's model holds an attempt for, so a
/// client that gives up and this service that gives up do so at the same moment
/// rather than disagreeing about which attempt is in flight.
pub const DEFAULT_ATTEMPT_DEADLINE: Duration = Duration::from_secs(30);

/// How this service is configured.
///
/// A plain value with public fields rather than a builder: it is assembled once by
/// the binary from its arguments, and it is part of the tests' surface, which is
/// how the policy rules below are checked against a real socket.
#[derive(Debug, Clone)]
pub struct Config {
    /// The socket to create.
    pub socket: PathBuf,
    /// The group the socket is chowned to, so a greeter running as its own user
    /// can connect. `None` leaves a root-only socket.
    pub socket_group: Option<u32>,
    /// The uids that may connect.
    pub allowed_uids: Vec<u32>,
    /// The gids that may connect, for a greeter that is admitted by group rather
    /// than by uid.
    pub allowed_gids: Vec<u32>,
    /// The PAM stack a credential is authenticated through.
    pub service: String,
    /// The PAM stack the session leader opens its session through.
    ///
    /// See [`DEFAULT_SESSION_SERVICE`]: it must not be the authentication stack,
    /// because the leader has no credential and its `auth` line has to permit.
    pub session_service: String,
    /// The lowest uid a login is started for.
    pub minimum_uid: u32,
    /// The program a session is started with. Never from the wire.
    pub session_program: PathBuf,
    /// Its literal arguments.
    pub session_arguments: Vec<OsString>,
    /// The VTs a user session may be placed on.
    pub session_vt: VtRange,
    /// The logind seat a session is activated on.
    pub seat: String,
    /// The greeter's uid, when the greeter account exists on this machine.
    ///
    /// Used by exactly one rule, the start-up hand-back: after a helper restart, a
    /// seat with no active session gets the greeter's own session back
    /// (`logind::hand_back_at_startup`). `None` simply means that repair is not
    /// attempted.
    pub greeter_uid: Option<u32>,
    /// How long the handshake may take.
    pub handshake_deadline: Duration,
    /// How long an attempt may take.
    pub attempt_deadline: Duration,
    /// How often an attempt may be made.
    pub rate: RatePolicy,
    /// Whether the service exits once the session it started has ended.
    ///
    /// Before the boot login chain this was the mitigation for PAM's process-global
    /// environment: `pam_open_session` wrote into the long-lived helper's
    /// environment, so a service that survived several logins carried a previous
    /// account's session variables into the next one. That is no longer the case —
    /// the session is opened in a one-shot session leader  — so
    /// the switch is kept as a state reset for the deployment (a fresh boundary
    /// process after every session), not as a correctness requirement.
    pub exit_after_session: bool,
}

impl Config {
    /// The the authentication boundary defaults, for one socket and one session program.
    pub fn new(socket: impl Into<PathBuf>, session_program: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            socket_group: None,
            allowed_uids: vec![0],
            allowed_gids: Vec::new(),
            service: DEFAULT_PAM_SERVICE.to_owned(),
            session_service: DEFAULT_SESSION_SERVICE.to_owned(),
            minimum_uid: DEFAULT_MINIMUM_UID,
            session_program: session_program.into(),
            session_arguments: Vec::new(),
            session_vt: VtRange::default(),
            seat: logind::DEFAULT_SEAT.to_owned(),
            greeter_uid: None,
            handshake_deadline: DEFAULT_HANDSHAKE_DEADLINE,
            attempt_deadline: DEFAULT_ATTEMPT_DEADLINE,
            rate: RatePolicy::default(),
            exit_after_session: false,
        }
    }

    /// Whether `uid`/`gid` identify a peer this service serves.
    pub fn admits(&self, uid: u32, gid: u32) -> bool {
        self.allowed_uids.contains(&uid) || self.allowed_gids.contains(&gid)
    }
}

/// Why the service could not run.
#[derive(Debug, Error)]
pub enum ServerError {
    /// The socket could not be created.
    #[error("the socket could not be created: {0}")]
    Socket(#[source] io::Error),
    /// Something is already listening on the socket.
    #[error("another boundary is already listening on {0}")]
    AlreadyRunning(PathBuf),
    /// The socket path exists and is not a socket this service may replace.
    #[error("the socket path is not a socket: {0}")]
    UnusableSocketPath(PathBuf),
    /// The thread that runs the authenticator could not be started.
    #[error("the authenticator thread could not be started: {0}")]
    Thread(#[source] io::Error),
}

/// One unit of work for the thread that owns the authenticator and the session.
enum Job {
    /// Check a credential for an account.
    Authenticate {
        account: Account,
        credential: Secret,
        reply: Sender<Verdict>,
    },
    /// Place the session on a VT and start it.
    Start {
        account: Account,
        reply: Sender<Result<(), SessionError>>,
    },
    /// Stop serving.
    Stop,
}

/// The services this boundary talks to besides its socket and PAM.
///
/// Two seams, both of them system boundaries (root AGENTS.md §6): logind, which
/// owns the VT and the seat, and the launcher, which starts the session leader.
/// They are separate values rather than hidden globals so that the service's own
/// rules — the VT it chooses, the session it hands the display back to, the
/// refusal of a second session — are tests against this server and not against a
/// machine with a system bus.
#[derive(Clone)]
pub struct Deps {
    /// logind, for VT selection and the hand-back.
    pub logind: Arc<dyn Logind>,
    /// How a session leader is started and confirmed.
    pub launcher: Arc<dyn SessionLauncher>,
}

impl Deps {
    /// The shipped implementations, configured from `config`.
    pub fn system(config: &Config) -> Self {
        Self {
            logind: Arc::new(SystemLogind::new(config.seat.clone())),
            launcher: Arc::new(ShippedLauncher::new(DEFAULT_LEADER_DEADLINE)),
        }
    }
}

/// Runs the service until it is asked to stop or the socket stops accepting.
///
/// # Errors
///
/// Returns [`ServerError`] when the socket cannot be created or the authenticator
/// thread cannot be started. Failures *inside* one connection are logged and the
/// connection is closed; they never end the service, because a display manager's
/// authentication boundary that exits on a malformed frame is a boundary anyone
/// can turn off.
pub fn run<F: BoundaryFactory + 'static>(config: Config, factory: F) -> Result<(), ServerError> {
    let deps = Deps::system(&config);

    run_with(config, factory, deps)
}

/// Runs the service against injected system dependencies.
///
/// [`run`] is this with [`Deps::system`]; the tests pass a scripted logind and a
/// launcher that starts the configured program directly, because PAM and logind
/// are the boundaries that may be stubbed and the rules being tested are this
/// file's.
///
/// # Errors
///
/// The same as [`run`].
pub fn run_with<F: BoundaryFactory + 'static>(
    config: Config,
    factory: F,
    deps: Deps,
) -> Result<(), ServerError> {
    let listener = bind(&config)?;

    let (jobs, work) = mpsc::channel::<Job>();
    let session_running = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::new(AtomicBool::new(false));

    // Before anything is served: a helper that died between "the session ended"
    // and "the display was handed back" leaves a seat with nothing active on it.
    // This is a repair, so it never stops the service — a machine without logind
    // has nothing to repair.
    hand_back_at_startup(&config, &deps);

    let worker = {
        let config = config.clone();
        let deps = deps.clone();
        let session_running = Arc::clone(&session_running);
        let shutdown = Arc::clone(&shutdown);
        let listener = listener.try_clone().map_err(ServerError::Socket)?;

        thread::Builder::new()
            .name("inox-auth-pam".to_owned())
            .spawn(move || {
                authenticate_loop(
                    &config,
                    &deps,
                    factory,
                    &work,
                    &session_running,
                    &shutdown,
                    &listener,
                );
            })
            .map_err(ServerError::Thread)?
    };

    let attempts = Mutex::new(AttemptLog::new(config.rate));
    let accepted: Mutex<Option<Accepted>> = Mutex::new(None);
    info!(
        socket = %config.socket.display(),
        service = %config.service,
        session_service = %config.session_service,
        vt_range = %config.session_vt,
        "the authentication boundary is listening"
    );

    let outcome = accept_loop(
        &config,
        &listener,
        &jobs,
        &attempts,
        &session_running,
        &accepted,
        &shutdown,
    );

    let _ = jobs.send(Job::Stop);
    if worker.join().is_err() {
        warn!("the authenticator thread ended unexpectedly");
    }

    // The socket file is this process's to remove: it only exists because this
    // process created it, and a TOCTOU replacement by someone else is not
    // something an unlink of the same path can be trusted to avoid. A failure here
    // is reported rather than ignored, because a stale root-owned socket is what
    // the next start will refuse to replace.
    if let Err(error) = fs::remove_file(&config.socket)
        && error.kind() != io::ErrorKind::NotFound
    {
        warn!(socket = %config.socket.display(), %error, "the socket could not be removed");
    }

    outcome
}

/// Creates the socket, refusing to take over a path it does not own.
fn bind(config: &Config) -> Result<UnixListener, ServerError> {
    if let Some(parent) = config.socket.parent()
        && !parent.as_os_str().is_empty()
    {
        {
            match fs::symlink_metadata(parent) {
                // An existing directory is left exactly as it is: its permissions
                // and owner belong to whoever made it, and re-applying ours would
                // mean a mistyped `--socket` could rewrite `/tmp`'s mode.
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => {
                    return Err(ServerError::UnusableSocketPath(config.socket.clone()));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir_all(parent).map_err(ServerError::Socket)?;
                    // The directory is traversable but not writable: only this
                    // process puts anything in it, and the greeter only has to reach
                    // the socket file.
                    fs::set_permissions(parent, fs::Permissions::from_mode(0o755))
                        .map_err(ServerError::Socket)?;
                }
                Err(error) => return Err(ServerError::Socket(error)),
            }
        }
    }

    match fs::symlink_metadata(&config.socket) {
        Ok(metadata) => {
            if !metadata.file_type().is_socket() {
                return Err(ServerError::UnusableSocketPath(config.socket.clone()));
            }
            // A socket nobody is listening on is a leftover from a killed process.
            if UnixStream::connect(&config.socket).is_ok() {
                return Err(ServerError::AlreadyRunning(config.socket.clone()));
            }
            fs::remove_file(&config.socket).map_err(ServerError::Socket)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(ServerError::Socket(error)),
    }

    let listener = UnixListener::bind(&config.socket).map_err(ServerError::Socket)?;

    // Mode and ownership are set explicitly rather than left to the umask: the
    // socket is the thing that decides who may even reach the peer check.
    let mode = if config.socket_group.is_some() {
        0o660
    } else {
        0o600
    };
    fs::set_permissions(&config.socket, fs::Permissions::from_mode(mode))
        .map_err(ServerError::Socket)?;
    if let Some(gid) = config.socket_group {
        chown(&config.socket, None, Some(Gid::from_raw(gid)))
            .map_err(|error| ServerError::Socket(io::Error::from(error)))?;
    }

    Ok(listener)
}

/// Accepts connections until the service is told to stop.
fn accept_loop(
    config: &Config,
    listener: &UnixListener,
    jobs: &Sender<Job>,
    attempts: &Mutex<AttemptLog>,
    session_running: &AtomicBool,
    accepted: &Mutex<Option<Accepted>>,
    shutdown: &AtomicBool,
) -> Result<(), ServerError> {
    for stream in listener.incoming() {
        if shutdown.load(Ordering::SeqCst) {
            info!("the session this boundary started has ended; stopping");
            return Ok(());
        }

        match stream {
            Ok(stream) => {
                if let Err(error) = serve(config, stream, jobs, attempts, session_running, accepted)
                {
                    debug!(%error, "a connection ended with an error");
                }
            }
            Err(error) => {
                if shutdown.load(Ordering::SeqCst) {
                    info!("the session this boundary started has ended; stopping");
                    return Ok(());
                }
                warn!(%error, "the connection could not be accepted");
            }
        }
    }

    Ok(())
}

/// An attempt the boundary accepted and has not yet been asked to start.
///
/// It is bound to the peer that made it, the identifier it was made under, and a
/// deadline — not to the connection it arrived on. That is what lets the login
/// screen decide *after* a successful check whether the person actually asked for
/// the session (they can still cancel, or close the window), without the boundary
/// holding a socket open across a UI decision or committing to a session nobody
/// confirmed. The alternative — a connection-scoped attempt — would make either the
/// client or the service lie about one of those two.
struct Accepted {
    attempt: Attempt,
    /// The uid that authenticated, from `SO_PEERCRED`.
    uid: u32,
    /// The account, resolved before the credential was checked.
    account: Account,
    /// When this stops being continuable.
    expires: Instant,
}

/// Serves one connection.
///
/// A connection carries exactly one request and one answer, and the request may be:
///
/// - `Hello` + `Authenticate`: the authentication itself. A success is remembered as
///   an [`Accepted`] attempt and the connection ends there.
/// - `Hello` + `StartSession`: continue an accepted attempt from the same peer.
/// - `Hello` + `Cancel`: give up an accepted attempt.
///
/// Anything else is a protocol error, and the connection is closed after the answer
/// rather than being reused: one connection is one request, so "which attempt is
/// this frame about" never has two answers.
fn serve(
    config: &Config,
    stream: UnixStream,
    jobs: &Sender<Job>,
    attempts: &Mutex<AttemptLog>,
    session_running: &AtomicBool,
    accepted: &Mutex<Option<Accepted>>,
) -> Result<(), wire::WireError> {
    // `SO_PEERCRED` is the kernel's answer about the process at the other end: the
    // uid, gid and pid it had when it connected. Nothing the client *sends* is
    // consulted, which is the whole point of asking.
    let peer = socket_peercred(&stream).map_err(io::Error::from)?;
    let uid = peer.uid.as_raw();

    if !config.admits(uid, peer.gid.as_raw()) {
        warn!(
            uid,
            gid = peer.gid.as_raw(),
            pid = peer.pid.as_raw_pid(),
            "refusing a peer this boundary does not serve"
        );
        let mut stream = stream;
        let _ = wire::write_reply(
            &mut stream,
            &Reply::Error {
                code: ErrorCode::UnauthorizedPeer,
            },
        );
        return Ok(());
    }

    // An attempt nobody continued is dropped the next time the service wakes, which
    // is here. Its PAM transaction ended when the verdict was returned, so all that
    // is left to drop is its claim on the one session slot.
    expire_accepted(accepted);

    let mut stream = stream;
    stream.set_read_timeout(Some(config.handshake_deadline))?;
    stream.set_write_timeout(Some(config.attempt_deadline))?;

    // 1. The handshake. Nothing else is accepted as the first frame: a client that
    //    leads with a request is a client that has not agreed on what version it
    //    means.
    match next_request(&mut stream)? {
        Read::Request(Request::Hello { .. }) => {}
        Read::Request(_) => return answer(&mut stream, ErrorCode::Malformed),
        Read::Answered | Read::Closed => return Ok(()),
    }
    wire::write_reply(
        &mut stream,
        &Reply::Welcome {
            version: wire::VERSION,
        },
    )?;

    // 2. The request.
    stream.set_read_timeout(Some(config.attempt_deadline))?;

    match next_request(&mut stream)? {
        Read::Request(Request::Authenticate {
            attempt,
            username,
            credential,
        }) => authenticate(
            config,
            &mut stream,
            jobs,
            attempts,
            session_running,
            accepted,
            uid,
            attempt,
            username,
            credential,
        ),
        Read::Request(Request::StartSession { attempt }) => {
            start_session(config, &mut stream, jobs, accepted, uid, attempt)
        }
        Read::Request(Request::Cancel { attempt }) => {
            // The contract still has `Cancel` (the login screen uses it when the
            // person walks away), but there is no PAM transaction left to abandon:
            // the attempt's only state is its claim on the session slot.
            let mut guard = lock(accepted);
            if guard
                .as_ref()
                .is_some_and(|current| current.attempt == attempt && current.uid == uid)
            {
                *guard = None;
            }
            Ok(())
        }
        Read::Request(_) => answer(&mut stream, ErrorCode::Malformed),
        Read::Answered | Read::Closed => Ok(()),
    }
}

/// Checks one credential and remembers the attempt if it is accepted.
#[allow(clippy::too_many_arguments)]
fn authenticate(
    config: &Config,
    stream: &mut UnixStream,
    jobs: &Sender<Job>,
    attempts: &Mutex<AttemptLog>,
    session_running: &AtomicBool,
    accepted: &Mutex<Option<Accepted>>,
    uid: u32,
    attempt: Attempt,
    username: String,
    credential: Secret,
) -> Result<(), wire::WireError> {
    // 3. The allowance, before the account is even looked up: what it bounds is how
    //    often this service does privileged work, not how often a name is spelled.
    let now = Instant::now();
    {
        let mut log = lock(attempts);
        if !log.allows(now) {
            info!(%username, "an attempt was refused by the rolling allowance");
            return unavailable(stream, attempt, UnavailableReason::RateLimited);
        }
        log.record(now);
    }

    // 4. One attempt at a time, whether it is running a session or waiting to be
    //    continued.
    if session_running.load(Ordering::SeqCst) || lock(accepted).is_some() {
        return unavailable(stream, attempt, UnavailableReason::Busy);
    }

    // 5. Account policy. An account that does not exist, root, and a system account
    //    are all answered exactly like a wrong password: the wire has no way to say
    //    "no such user", and that is deliberate (see `wire::UnavailableReason`).
    let account = match session::account(&username, config.minimum_uid) {
        Ok(account) => account,
        Err(error) => {
            info!(%username, error = %error, "the attempt names an account this boundary will not log in");
            return answer_attempt(stream, Reply::Rejected { attempt });
        }
    };

    // 6. The authenticator, on the thread that owns it. The credential moves into
    //    the job: after this line the only copy is the one PAM is looking at.
    let (verdict_tx, verdict_rx) = mpsc::channel();
    let job = Job::Authenticate {
        account: account.clone(),
        credential,
        reply: verdict_tx,
    };
    if jobs.send(job).is_err() {
        warn!("the authenticator thread is gone");
        return unavailable(stream, attempt, UnavailableReason::Failed);
    }

    let verdict = verdict_rx
        .recv_timeout(config.attempt_deadline)
        .unwrap_or(Verdict::Unavailable(UnavailableReason::Failed));

    match verdict {
        Verdict::Accepted => {
            *lock(accepted) = Some(Accepted {
                attempt,
                uid,
                account,
                expires: Instant::now() + config.attempt_deadline,
            });
            answer_attempt(stream, Reply::Accepted { attempt })
        }
        Verdict::Rejected => answer_attempt(stream, Reply::Rejected { attempt }),
        Verdict::Unavailable(reason) => unavailable(stream, attempt, reason),
    }
}

/// Continues an accepted attempt on behalf of the peer that made it.
fn start_session(
    config: &Config,
    stream: &mut UnixStream,
    jobs: &Sender<Job>,
    accepted: &Mutex<Option<Accepted>>,
    uid: u32,
    attempt: Attempt,
) -> Result<(), wire::WireError> {
    let taken = {
        let mut guard = lock(accepted);
        match guard.as_ref() {
            Some(current) if current.attempt != attempt => {
                // A continuation of an attempt this boundary never accepted.
                drop(guard);
                return answer(stream, ErrorCode::Malformed);
            }
            Some(current) if current.uid != uid => {
                let owner = current.uid;
                drop(guard);
                warn!(
                    uid,
                    owner, "a process asked to continue an attempt another uid authenticated"
                );
                return answer(stream, ErrorCode::Malformed);
            }
            Some(_) => guard.take(),
            None => None,
        }
    };

    let Some(current) = taken else {
        // Expired, already continued, or never accepted. The boundary is not in a
        // state to start it, and no session is started.
        return unavailable(stream, attempt, UnavailableReason::Busy);
    };

    let (started_tx, started_rx) = mpsc::channel();
    let job = Job::Start {
        account: current.account,
        reply: started_tx,
    };
    if jobs.send(job).is_err() {
        warn!("the authenticator thread is gone");
        return answer_attempt(
            stream,
            Reply::SessionStarted {
                attempt,
                started: false,
            },
        );
    }

    let started = started_rx
        .recv_timeout(config.attempt_deadline)
        .map(|result| result.is_ok())
        .unwrap_or(false);

    answer_attempt(stream, Reply::SessionStarted { attempt, started })
}

/// Drops an accepted attempt whose deadline has passed.
fn expire_accepted(accepted: &Mutex<Option<Accepted>>) {
    let mut guard = lock(accepted);
    if guard
        .as_ref()
        .is_some_and(|current| current.expires <= Instant::now())
    {
        debug!("an accepted attempt was never continued and has expired");
        *guard = None;
    }
}

enum Read {
    /// A request this service understands.
    Request(Request),
    /// Nothing the service understands. If it was answerable, the answer has
    /// already been written; either way the connection is over.
    Answered,
    /// The client closed, or the frame was one no answer can be given about.
    Closed,
}

/// Reads one request, answering a protocol error the client can be told about.
///
/// A frame the service cannot parse *and* cannot answer about — a length prefix
/// beyond the bound, a truncated frame, a transport error — ends the connection
/// instead of being guessed at: there is no resynchronisation point in a
/// length-prefixed stream that a peer which lied about a length can be trusted to
/// agree on.
fn next_request(stream: &mut UnixStream) -> Result<Read, wire::WireError> {
    match wire::read_request(stream) {
        Ok(Some(request)) => Ok(Read::Request(request)),
        Ok(None) => Ok(Read::Closed),
        Err(error) => match error.code() {
            Some(code) => {
                wire::write_reply(stream, &Reply::Error { code })?;
                Ok(Read::Answered)
            }
            None => Err(error),
        },
    }
}

/// The one thread that owns PAM and the running session.
///
/// It is separate from the socket loop for two reasons: a `pam` handle is not
/// `Send`, and the session's lifetime has to be watched — starting it, waiting for
/// it and handing the display back are one sequence, and doing all of it on one
/// thread keeps that sequence in one place instead of inventing a lock discipline
/// for process-global state.
fn authenticate_loop<F: BoundaryFactory>(
    config: &Config,
    deps: &Deps,
    factory: F,
    work: &Receiver<Job>,
    session_running: &AtomicBool,
    shutdown: &AtomicBool,
    listener: &UnixListener,
) {
    while let Ok(job) = work.recv() {
        match job {
            Job::Authenticate {
                account,
                credential,
                reply,
            } => {
                let mut fresh = factory.create();
                let verdict = fresh.authenticate(&account.username, &credential);
                // The job's copy goes out of scope here; `Secret` zeroes it. The
                // boundary is dropped too: since the boot login chain its PAM transaction ends
                // with the verdict, and the session is opened by the leader.
                drop(credential);
                drop(fresh);
                let _ = reply.send(verdict);
            }
            Job::Start { account, reply } => {
                let ran = start_one_session(config, deps, account, &reply, session_running);

                if config.exit_after_session && ran {
                    info!("exiting now that the session has ended");
                    shutdown.store(true, Ordering::SeqCst);
                    // Wakes the accept loop, which is blocked on `accept`: `std` has
                    // no way to interrupt it, and shutting the listener down is the
                    // portable way to make it return.
                    if let Err(error) = rustix::net::shutdown(listener, Shutdown::Both) {
                        warn!(%error, "the listener could not be shut down");
                    }
                    return;
                }
            }
            Job::Stop => return,
        }
    }
}

/// Runs one login from "place the session" to "hand the display back".
///
/// The sequence is the boot login chain
/// active *before* anything switches away from it, pick a VT, start a leader for
/// that account on that VT, answer the client once the leader has confirmed
/// `pam_open_session`, wait for the session to end, and put the recorded session
/// back on the display.
/// Returns whether a session was actually started, which is what
/// `--exit-after-session` waits for: a login that never started is not a session
/// that ended.
fn start_one_session(
    config: &Config,
    deps: &Deps,
    account: Account,
    reply: &Sender<Result<(), SessionError>>,
    session_running: &AtomicBool,
) -> bool {
    let uid = account.uid;

    // 1. The session to hand the display back to. Sampled before the switch,
    //    because after it the seat's active session is the user's.
    let recorded = record_active_session(&*deps.logind);

    // 2. A free VT, never the one the recorded session is on.
    let skip: Vec<u32> = recorded
        .as_ref()
        .and_then(|session| session.vt)
        .into_iter()
        .collect();
    let vt = match choose_vt(config, deps, &skip) {
        Ok(vt) => vt,
        Err(error) => {
            warn!(error = %error, "no VT is available for the session");
            let _ = reply.send(Err(error));
            return false;
        }
    };

    // 3. The leader. It opens the PAM session as root, claims the VT and becomes
    //    the session; this process only watches it.
    session_running.store(true, Ordering::SeqCst);

    let plan = LeaderPlan {
        account,
        vt,
        program: config.session_program.clone(),
        arguments: config.session_arguments.clone(),
        service: config.session_service.clone(),
        minimum_uid: config.minimum_uid,
        vt_range: config.session_vt,
        seat: config.seat.clone(),
    };

    let mut session = match deps.launcher.launch(&plan) {
        Ok(session) => session,
        Err(error) => {
            session_running.store(false, Ordering::SeqCst);
            warn!(uid, vt, error = %error, "the session was not started");
            let _ = reply.send(Err(error));
            return false;
        }
    };

    // 4. The answer goes out only now: the login screen is told a session runs when
    //    one does, not when one was asked for.
    let _ = reply.send(Ok(()));
    info!(
        uid,
        vt, "a session was started for the authenticated account"
    );

    match session.wait() {
        Ok(status) => info!(uid, ?status, "the session ended"),
        Err(error) => warn!(uid, %error, "the session could not be waited for"),
    }

    // 5. The display goes back to the session that had it before the switch.
    hand_back(deps, recorded);
    session_running.store(false, Ordering::SeqCst);

    true
}

/// The session that is active on the seat now, if logind can say.
fn record_active_session(logind: &dyn Logind) -> Option<logind::KnownSession> {
    match logind.active_session() {
        Ok(Some(session)) => {
            info!(
                session = %session.id,
                vt = ?session.vt,
                "recording the active session to hand the display back to"
            );
            Some(session)
        }
        Ok(None) => {
            debug!(
                "no session is active on the seat; the display will not be handed back after this \
                 one"
            );
            None
        }
        Err(error) => {
            warn!(
                %error,
                "logind could not be asked which session is active; the display will not be handed \
                 back"
            );
            None
        }
    }
}

/// Picks the VT a session goes on, degrading when logind cannot arbitrate.
fn choose_vt(config: &Config, deps: &Deps, skip: &[u32]) -> Result<u32, SessionError> {
    match logind::first_free_vt(config.session_vt, skip, &*deps.logind) {
        Ok(Some(vt)) => Ok(vt),
        Ok(None) => {
            warn!(
                range = %config.session_vt,
                "every VT in the configured range already has a session"
            );
            Err(SessionError::NoFreeVt)
        }
        Err(error) => {
            // No logind: CI, a development instance. There is no VT arbitration to
            // lose on such a machine, and refusing to log in because a system
            // service is absent would be a worse answer than the first VT in the
            // configured range.
            warn!(
                %error,
                range = %config.session_vt,
                "logind could not be asked which VT is free; the session is placed without it"
            );
            logind::first_free_vt_without_logind(config.session_vt, skip)
                .ok_or(SessionError::NoFreeVt)
        }
    }
}

/// Puts the display back on the session that had it before this login.
fn hand_back(deps: &Deps, recorded: Option<logind::KnownSession>) {
    let Some(recorded) = recorded else {
        debug!("there was no recorded session to hand the display back to");
        return;
    };

    match deps.logind.activate(&recorded.id) {
        Ok(()) => info!(session = %recorded.id, "the display was handed back"),
        Err(error) => warn!(
            %error,
            session = %recorded.id,
            "the display could not be handed back"
        ),
    }
}

/// Hands the display back once, at start-up, after a helper restart.
fn hand_back_at_startup(config: &Config, deps: &Deps) {
    match logind::hand_back_at_startup(&*deps.logind, config.greeter_uid) {
        Ok(logind::StartupHandBack::Active) => {
            debug!("a session is already active on the seat; nothing to hand back at start-up")
        }
        Ok(logind::StartupHandBack::Activated) => {
            info!("the display was handed back to the greeter's session after a restart")
        }
        Ok(outcome) => debug!(
            ?outcome,
            "no greeter session to hand the display back to at start-up"
        ),
        Err(error) => debug!(
            %error,
            "logind could not be asked about the display at start-up; the hand-back is skipped"
        ),
    }
}

/// Answers a protocol error and ends the connection.
fn answer(stream: &mut UnixStream, code: ErrorCode) -> Result<(), wire::WireError> {
    wire::write_reply(stream, &Reply::Error { code })
}

/// Writes one reply and closes.
fn answer_attempt(stream: &mut UnixStream, reply: Reply) -> Result<(), wire::WireError> {
    wire::write_reply(stream, &reply)
}

/// Writes an unavailable reply and closes.
fn unavailable(
    stream: &mut UnixStream,
    attempt: Attempt,
    reason: UnavailableReason,
) -> Result<(), wire::WireError> {
    wire::write_reply(stream, &Reply::Unavailable { attempt, reason })
}

/// Locks one of this service's mutexes, surviving a panic in another thread.
///
/// A poisoned mutex would otherwise turn "one connection panicked" into "the
/// boundary refuses every connection for the rest of the process's life", which is
/// the wrong trade for a counter whose worst case is a slightly generous window and
/// for a slot whose worst case is a login that has to be retyped.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The mode and ownership rules, as data, so a test can check them without a root
/// account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketPolicy {
    /// Whether a socket path belongs to this service.
    pub mode: u32,
    /// Whether the group is set at all.
    pub restricted_to_group: bool,
}

/// The mode and ownership `bind` applies for `config`.
pub fn socket_policy(config: &Config) -> SocketPolicy {
    SocketPolicy {
        mode: if config.socket_group.is_some() {
            0o660
        } else {
            0o600
        },
        restricted_to_group: config.socket_group.is_some(),
    }
}

/// Whether `path` is a path this service may create its socket at.
///
/// Used by the binary before it drops privileges, so a mistyped `--socket` is an
/// argument error rather than a half-created service.
pub fn socket_parent_exists(path: &Path) -> bool {
    path.parent()
        .is_none_or(|parent| parent.as_os_str().is_empty() || parent.is_dir() || !parent.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::new("/tmp/inox-auth-test.sock", "/bin/true")
    }

    #[test]
    fn the_defaults_are_the_ones_adr_0029_and_adr_0037_record() {
        let config = config();

        assert_eq!(config.service, "inox-auth");
        assert_eq!(
            config.session_service, "inox-auth-session",
            "the leader has no credential, so its stack cannot be the one a password is \
             authenticated against"
        );
        assert_eq!(config.minimum_uid, 1000);
        assert_eq!(config.rate, RatePolicy::default());
        assert_eq!(config.attempt_deadline, Duration::from_secs(30));
        assert_eq!(config.session_vt, VtRange::default());
        assert_eq!(config.session_vt.to_string(), "2..=6");
        assert_eq!(config.seat, "seat0");
        assert_eq!(config.greeter_uid, None);
        assert!(config.allowed_uids.contains(&0));
        assert!(
            !config.exit_after_session,
            "staying up is the default; exit-after-session is the opt-in reset"
        );
    }

    #[test]
    fn a_peer_is_admitted_by_uid_or_by_group_and_nothing_else() {
        let mut config = config();
        config.allowed_uids = vec![0, 967];
        config.allowed_gids = vec![967];

        assert!(config.admits(0, 0), "root is always served");
        assert!(config.admits(967, 967), "the greeter's own uid");
        assert!(config.admits(1234, 967), "a process in the greeter's group");
        assert!(!config.admits(1000, 1000), "an ordinary user's session");
    }

    #[test]
    fn a_group_owned_socket_is_wider_than_a_private_one() {
        let mut config = config();
        assert_eq!(
            socket_policy(&config),
            SocketPolicy {
                mode: 0o600,
                restricted_to_group: false
            }
        );

        config.socket_group = Some(967);
        assert_eq!(
            socket_policy(&config),
            SocketPolicy {
                mode: 0o660,
                restricted_to_group: true
            }
        );
    }

    #[test]
    fn the_shipped_names_are_the_ones_the_contract_and_the_packaging_use() {
        assert_eq!(DEFAULT_SOCKET, "/run/inox/auth.sock");
        assert_eq!(DEFAULT_PAM_SERVICE, "inox-auth");
        assert_eq!(DEFAULT_SESSION_SERVICE, "inox-auth-session");
        assert_eq!(DEFAULT_SESSION_PROGRAM, "inox-session");
        assert_eq!(logind::DEFAULT_SEAT, "seat0");
    }

    #[test]
    fn a_path_whose_parent_cannot_exist_is_refused_before_serving() {
        assert!(
            socket_parent_exists(Path::new("/run/inox/auth.sock")),
            "an absent parent is created by `bind`"
        );
        assert!(
            socket_parent_exists(Path::new("auth.sock")),
            "no parent at all"
        );
        assert!(!socket_parent_exists(Path::new("/proc/self/mem/auth.sock")));
    }

    #[test]
    fn mode_helper_agrees_with_the_permission_bits_the_kernel_uses() {
        let mut config = config();
        config.socket_group = Some(967);

        let policies = socket_policy(&config);
        assert_eq!(policies.mode & 0o007, 0, "nothing for other");
        assert_eq!(policies.mode & 0o070, 0o060, "read and write for the group");
        assert_eq!(policies.mode & 0o700, 0o600, "read and write for the owner");
    }
}
