//! The authentication boundary over a real socket.
//!
//! These tests start the real [`inox_auth::server`] on a real Unix socket and drive
//! it with the real `auth-v1` framing. Two things are not real, and they are the
//! two system boundaries ADR 0037 draws:
//!
//! - the authenticator itself — [`ScriptedBoundary`] answers with a script instead
//!   of calling PAM;
//! - the session placement — a scripted [`Logind`] and a [`TestLauncher`] that
//!   starts the configured program directly instead of re-execing the binary in
//!   `--session-leader` mode (which needs root, a PAM stack and a real VT).
//!
//! Everything else — the peer credential checks, the rolling allowance, one request
//! per connection, an answer only for the attempt in hand, one session at a time,
//! the VT the service picks, the session it hands the display back to — is the real
//! server's own logic, which is exactly the split root AGENTS.md §6 asks for: stub
//! the system boundary, never the rules.
//!
//! They run unprivileged, which is why the test configuration admits the current
//! uid, sets `minimum_uid = 0`, and points `session_program` at a program that
//! exits immediately. The tests that need a *running* session use `sleep`, and the
//! one that needs a session to end uses `exit_after_session`.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use inox_auth::boundary::{Boundary, BoundaryFactory, Verdict};
use inox_auth::leader::{LeaderPlan, RunningSession, SessionLauncher};
use inox_auth::logind::{KnownSession, Logind, LogindError, SeatedSession, VtRange};
use inox_auth::server::{self, Config, Deps};
use inox_auth::session::{self, SessionError};
use inox_auth::wire::{self, ErrorCode, Reply, Request, UnavailableReason};
use inox_common::secret::Secret;
use rustix::process::geteuid;

/// A boundary that answers whatever the test told it to.
struct ScriptedBoundary {
    verifies: Arc<AtomicUsize>,
    verdict: Verdict,
}

impl Boundary for ScriptedBoundary {
    fn authenticate(&mut self, _username: &str, _credential: &Secret) -> Verdict {
        self.verifies.fetch_add(1, Ordering::SeqCst);
        self.verdict
    }
}

struct ScriptedFactory {
    verifies: Arc<AtomicUsize>,
    verdict: Verdict,
}

impl BoundaryFactory for ScriptedFactory {
    fn create(&self) -> Box<dyn Boundary> {
        Box::new(ScriptedBoundary {
            verifies: Arc::clone(&self.verifies),
            verdict: self.verdict,
        })
    }
}

/// The launcher seam: starts the configured program instead of a session leader.
///
/// The real launcher re-execs this binary as `inox-auth --session-leader …`, which
/// needs root and a session-only PAM stack. What these tests are about is the
/// daemon's sequence — which VT it asks for, when it answers `started`, when it
/// hands the display back — so the leader is replaced here, and only here.
#[derive(Default)]
struct TestLauncher {
    launches: Mutex<Vec<LeaderPlan>>,
}

impl TestLauncher {
    fn launches(&self) -> Vec<LeaderPlan> {
        self.launches.lock().expect("the test's own mutex").clone()
    }
}

impl SessionLauncher for TestLauncher {
    fn launch(&self, plan: &LeaderPlan) -> Result<RunningSession, SessionError> {
        self.launches
            .lock()
            .expect("the test's own mutex")
            .push(plan.clone());

        let session_plan = session::plan(
            plan.account.clone(),
            plan.program.clone(),
            plan.arguments.clone(),
        );

        session::start(&session_plan).map(RunningSession::new)
    }
}

/// A logind whose answers the test writes down.
#[derive(Default)]
struct ScriptedLogind {
    active: Mutex<Option<KnownSession>>,
    taken: Mutex<Vec<u32>>,
    seat_sessions: Mutex<Vec<SeatedSession>>,
    activated: Mutex<Vec<String>>,
    reachable: AtomicBool,
}

impl ScriptedLogind {
    fn reachable() -> Self {
        let logind = Self::default();
        logind.reachable.store(true, Ordering::SeqCst);

        logind
    }

    fn set_reachable(&self, reachable: bool) {
        self.reachable.store(reachable, Ordering::SeqCst);
    }

    /// A session is active on the seat, on `vt`.
    fn say_active(&self, id: &str, vt: Option<u32>) {
        *self.active.lock().expect("the test's own mutex") = Some(KnownSession {
            id: id.to_owned(),
            vt,
        });
    }

    /// A session already occupies `vt`.
    fn occupy(&self, vt: u32) {
        self.taken.lock().expect("the test's own mutex").push(vt);
    }

    fn on_seat(&self, id: &str, uid: u32) {
        self.seat_sessions
            .lock()
            .expect("the test's own mutex")
            .push(SeatedSession {
                id: id.to_owned(),
                uid,
            });
    }

    fn activated(&self) -> Vec<String> {
        self.activated.lock().expect("the test's own mutex").clone()
    }

    fn answer<T>(&self, value: T) -> Result<T, LogindError> {
        if self.reachable.load(Ordering::SeqCst) {
            Ok(value)
        } else {
            Err(LogindError::NotAvailable)
        }
    }
}

impl Logind for ScriptedLogind {
    fn active_session(&self) -> Result<Option<KnownSession>, LogindError> {
        let active = self.active.lock().expect("the test's own mutex").clone();

        self.answer(active)
    }

    fn session_on_vt(&self, vt: u32) -> Result<Option<KnownSession>, LogindError> {
        let taken = self
            .taken
            .lock()
            .expect("the test's own mutex")
            .contains(&vt);

        self.answer(taken.then(|| KnownSession {
            id: format!("session-on-{vt}"),
            vt: Some(vt),
        }))
    }

    fn session_of_pid(&self, _pid: u32) -> Result<Option<KnownSession>, LogindError> {
        let active = self.active.lock().expect("the test's own mutex").clone();

        self.answer(active)
    }

    fn sessions_on_seat(&self) -> Result<Vec<SeatedSession>, LogindError> {
        let sessions = self
            .seat_sessions
            .lock()
            .expect("the test's own mutex")
            .clone();

        self.answer(sessions)
    }

    fn activate(&self, id: &str) -> Result<(), LogindError> {
        if !self.reachable.load(Ordering::SeqCst) {
            return Err(LogindError::NotAvailable);
        }
        self.activated
            .lock()
            .expect("the test's own mutex")
            .push(id.to_owned());

        Ok(())
    }
}

/// A server running in its own thread on its own socket.
struct Harness {
    /// Held, never read: dropping it would remove the directory the socket is in.
    _directory: tempfile::TempDir,
    socket: PathBuf,
    worker: Option<JoinHandle<()>>,
    verifies: Arc<AtomicUsize>,
    launcher: Arc<TestLauncher>,
    logind: Arc<ScriptedLogind>,
}

impl Harness {
    /// Starts a server with `configure` applied to the defaults.
    fn start(
        verdict: Verdict,
        program: &str,
        arguments: &[&str],
        configure: impl FnOnce(&mut Config),
    ) -> Self {
        Self::start_with(
            verdict,
            program,
            arguments,
            Arc::new(ScriptedLogind::reachable()),
            configure,
        )
    }

    /// Starts a server with the logind answers already written down.
    ///
    /// Needed by the start-up hand-back, which is the one rule that runs before the
    /// socket accepts anything.
    fn start_with(
        verdict: Verdict,
        program: &str,
        arguments: &[&str],
        logind: Arc<ScriptedLogind>,
        configure: impl FnOnce(&mut Config),
    ) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("auth.sock");
        let mut config = Config::new(socket.clone(), program);
        config.allowed_uids = vec![geteuid().as_raw()];
        config.minimum_uid = 0;
        config.session_arguments = arguments.iter().map(Into::into).collect();
        // Short deadlines: a test that hangs should fail, not stall CI.
        config.handshake_deadline = Duration::from_secs(5);
        config.attempt_deadline = Duration::from_secs(5);
        configure(&mut config);

        let verifies = Arc::new(AtomicUsize::new(0));
        let factory = ScriptedFactory {
            verifies: Arc::clone(&verifies),
            verdict,
        };

        let launcher = Arc::new(TestLauncher::default());
        let deps = Deps {
            logind: logind.clone(),
            launcher: launcher.clone(),
        };

        let worker = thread::spawn(move || {
            if let Err(error) = server::run_with(config, factory, deps) {
                eprintln!("the test server stopped: {error}");
            }
        });

        wait_for_socket(&socket);

        Self {
            _directory: directory,
            socket,
            worker: Some(worker),
            verifies,
            launcher,
            logind,
        }
    }

    fn connect(&self) -> Client {
        Client::connect(&self.socket)
    }

    /// How many times the scripted authenticator was asked.
    fn verifications(&self) -> usize {
        self.verifies.load(Ordering::SeqCst)
    }

    /// The leaders the service asked to start, in order.
    fn launches(&self) -> Vec<LeaderPlan> {
        self.launcher.launches()
    }

    /// The sessions the service asked logind to activate, in order.
    fn activated(&self) -> Vec<String> {
        self.logind.activated()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // The server thread is detached rather than joined: `server::run` returns
        // when its accept loop sees the shutdown flag, which only the
        // `exit_after_session` case sets. Every other test's server is still waiting
        // for a connection when the test ends, and the process ending is what stops
        // it. `self.directory` is dropped normally right after.
        drop(self.worker.take());
    }
}

/// Waits until the server's socket exists and accepts connections.
fn wait_for_socket(socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);

    while Instant::now() < deadline {
        if UnixStream::connect(socket).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }

    panic!("the server did not start listening on {}", socket.display());
}

/// One client connection, speaking the contract.
struct Client {
    stream: UnixStream,
}

impl Client {
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("the server accepts connections");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("a read timeout");
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .expect("a write timeout");

        Self { stream }
    }

    /// The handshake every connection starts with.
    fn handshake(&mut self) {
        wire::write_request(
            &mut self.stream,
            &Request::Hello {
                version: wire::VERSION,
            },
        )
        .expect("the hello is written");
        assert_eq!(
            wire::read_reply(&mut self.stream).expect("a reply"),
            Some(Reply::Welcome {
                version: wire::VERSION
            })
        );
    }

    /// One whole connection: handshake, request, answer.
    fn request(&mut self, request: Request) -> Option<Reply> {
        self.handshake();
        wire::write_request(&mut self.stream, &request).expect("the request is written");

        wire::read_reply(&mut self.stream).expect("a reply")
    }

    fn authenticate(&mut self, attempt: u64, username: &str, credential: &str) -> Option<Reply> {
        self.request(Request::Authenticate {
            attempt,
            username: username.to_owned(),
            credential: Secret::from_bytes(credential.as_bytes()),
        })
    }

    fn start_session(&mut self, attempt: u64) -> Option<Reply> {
        self.request(Request::StartSession { attempt })
    }

    fn cancel(&mut self, attempt: u64) {
        self.handshake();
        wire::write_request(&mut self.stream, &Request::Cancel { attempt })
            .expect("the cancel is written");
    }
}

/// The name of an account the tests may use: the one running them.
///
/// `root` is refused by policy whatever is configured, so a test running as root
/// falls back to an account that is not root.
fn usable_account() -> String {
    let from_env = std::env::var("USER").unwrap_or_default();

    if !from_env.is_empty() && from_env != "root" {
        return from_env;
    }

    "nobody".to_owned()
}

/// A program that exits successfully right away.
const TRUE: &str = "/bin/true";

/// A program that keeps a session alive for a moment.
const SLEEP: &str = "/bin/sleep";

#[test]
fn an_accepted_attempt_is_continued_into_a_session() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let account = usable_account();

    let mut attempt = harness.connect();
    assert_eq!(
        attempt.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );

    // The continuation is its own connection, which is what lets the interface
    // decide in between whether it still wants the session.
    let mut continuation = harness.connect();
    assert_eq!(
        continuation.start_session(1),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        })
    );
    assert_eq!(harness.verifications(), 1);
    assert_eq!(harness.launches().len(), 1);
}

#[test]
fn a_refused_attempt_starts_nothing() {
    let harness = Harness::start(Verdict::Rejected, TRUE, &[], |_| {});
    let mut client = harness.connect();

    assert_eq!(
        client.authenticate(1, &usable_account(), "hunter2"),
        Some(Reply::Rejected { attempt: 1 })
    );
    assert!(harness.launches().is_empty());
}

#[test]
fn a_broken_authenticator_is_not_reported_as_a_wrong_password() {
    let harness = Harness::start(
        Verdict::Unavailable(UnavailableReason::Failed),
        TRUE,
        &[],
        |_| {},
    );
    let mut client = harness.connect();

    assert_eq!(
        client.authenticate(1, &usable_account(), "hunter2"),
        Some(Reply::Unavailable {
            attempt: 1,
            reason: UnavailableReason::Failed
        })
    );
    assert!(harness.launches().is_empty());
}

#[test]
fn an_unknown_account_is_answered_like_a_wrong_password() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let mut client = harness.connect();

    // The account does not exist, so the boundary never reaches the authenticator.
    assert_eq!(
        client.authenticate(1, "no-such-account-9f2c", "hunter2"),
        Some(Reply::Rejected { attempt: 1 })
    );
    assert_eq!(
        harness.verifications(),
        0,
        "an unknown account must not spend the authenticator's time"
    );
    assert!(harness.launches().is_empty());
}

#[test]
fn root_is_never_logged_in_whatever_the_configuration_says() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        config.minimum_uid = 0;
    });
    let mut client = harness.connect();

    assert_eq!(
        client.authenticate(1, "root", "hunter2"),
        Some(Reply::Rejected { attempt: 1 })
    );
    assert_eq!(harness.verifications(), 0);
    assert!(harness.launches().is_empty());
}

#[test]
fn the_rolling_allowance_bounds_attempts_across_connections() {
    let harness = Harness::start(Verdict::Rejected, TRUE, &[], |_| {});
    let account = usable_account();

    for attempt in 1..=3 {
        let mut client = harness.connect();
        assert_eq!(
            client.authenticate(attempt, &account, "hunter2"),
            Some(Reply::Rejected { attempt })
        );
    }

    // The fourth is refused without reaching the authenticator at all: that is what
    // the allowance is for.
    let mut client = harness.connect();
    assert_eq!(
        client.authenticate(4, &account, "hunter2"),
        Some(Reply::Unavailable {
            attempt: 4,
            reason: UnavailableReason::RateLimited
        })
    );
    assert_eq!(harness.verifications(), 3);
}

#[test]
fn a_peer_this_boundary_does_not_serve_is_refused_before_anything_else() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        // The current uid is deliberately not admitted.
        config.allowed_uids = vec![];
        config.allowed_gids = vec![];
    });
    let mut client = harness.connect();

    // Nothing is sent: the refusal is the service's first act, before it reads a
    // single byte, which is what "checked before anything else happens" means.
    assert_eq!(
        wire::read_reply(&mut client.stream).expect("a reply"),
        Some(Reply::Error {
            code: ErrorCode::UnauthorizedPeer
        })
    );
    assert_eq!(harness.verifications(), 0);
}

#[test]
fn a_client_that_leads_with_an_attempt_is_refused() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let mut client = harness.connect();

    // No `Hello` first.
    wire::write_request(
        &mut client.stream,
        &Request::Authenticate {
            attempt: 1,
            username: usable_account(),
            credential: Secret::from_bytes(b"hunter2"),
        },
    )
    .expect("the request is written");

    assert_eq!(
        wire::read_reply(&mut client.stream).expect("a reply"),
        Some(Reply::Error {
            code: ErrorCode::Malformed
        })
    );
    assert_eq!(harness.verifications(), 0);
}

#[test]
fn an_unknown_frame_tag_is_refused() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let mut client = harness.connect();
    client.handshake();

    // Tag 99 is not in this version.
    wire::write_frame(&mut client.stream, &[99]).expect("the frame is written");

    assert_eq!(
        wire::read_reply(&mut client.stream).expect("a reply"),
        Some(Reply::Error {
            code: ErrorCode::Malformed
        })
    );
}

#[test]
fn an_oversized_frame_is_refused_without_being_allocated() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let mut client = harness.connect();

    // A length prefix far above the bound. The reply is a closed connection: the
    // service cannot know where the next frame would start, so guessing is worse
    // than hanging up.
    let announced = u32::try_from(wire::MAX_FRAME_BYTES + 1024).expect("fits in u32");
    client
        .stream
        .write_all(&announced.to_be_bytes())
        .expect("the header is written");

    assert!(
        wire::read_reply(&mut client.stream)
            .expect("reading a closed connection is not an error")
            .is_none(),
        "the service closed instead of answering"
    );
}

#[test]
fn another_version_is_refused_by_name() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let mut client = harness.connect();

    // Hand-built `Hello { version: 2 }`: the encoder refuses to write a version this
    // build does not speak, so the frame is written by hand.
    wire::write_frame(&mut client.stream, &[1, 0, 0, 0, 2]).expect("the frame is written");

    assert_eq!(
        wire::read_reply(&mut client.stream).expect("a reply"),
        Some(Reply::Error {
            code: ErrorCode::UnsupportedVersion
        })
    );
}

#[test]
fn an_accepted_attempt_that_is_cancelled_starts_nothing() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let account = usable_account();

    let mut attempt = harness.connect();
    assert_eq!(
        attempt.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );

    let mut cancellation = harness.connect();
    cancellation.cancel(1);

    // The slot is free again and no session was started.
    let mut second = harness.connect();
    assert_eq!(
        second.authenticate(2, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 2 })
    );
    assert!(harness.launches().is_empty());
}

#[test]
fn an_attempt_that_was_never_accepted_cannot_be_continued() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let mut client = harness.connect();

    assert_eq!(
        client.start_session(7),
        Some(Reply::Unavailable {
            attempt: 7,
            reason: UnavailableReason::Busy
        }),
        "there is no accepted attempt to start"
    );
    assert!(harness.launches().is_empty());
}

#[test]
fn an_accepted_attempt_is_continued_exactly_once() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let account = usable_account();

    let mut attempt = harness.connect();
    assert_eq!(
        attempt.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );

    let mut first = harness.connect();
    assert_eq!(
        first.start_session(1),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        })
    );

    let mut second = harness.connect();
    assert_eq!(
        second.start_session(1),
        Some(Reply::Unavailable {
            attempt: 1,
            reason: UnavailableReason::Busy
        }),
        "a continuation is consumed by the session it started"
    );
    assert_eq!(harness.launches().len(), 1);
}

#[test]
fn an_accepted_attempt_that_is_never_continued_expires() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        config.attempt_deadline = Duration::from_millis(300);
        config.handshake_deadline = Duration::from_millis(300);
    });
    let account = usable_account();

    let mut attempt = harness.connect();
    assert_eq!(
        attempt.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );

    thread::sleep(Duration::from_millis(600));

    let mut late = harness.connect();
    assert_eq!(
        late.start_session(1),
        Some(Reply::Unavailable {
            attempt: 1,
            reason: UnavailableReason::Busy
        })
    );
    assert!(
        harness.launches().is_empty(),
        "an attempt nobody continued must not start a session later"
    );
}

#[test]
fn a_session_that_is_still_running_makes_the_boundary_busy() {
    let harness = Harness::start(Verdict::Accepted, SLEEP, &["1"], |_| {});
    let account = usable_account();

    let mut attempt = harness.connect();
    assert_eq!(
        attempt.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );

    let mut start = harness.connect();
    assert_eq!(
        start.start_session(1),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        })
    );

    // The session is still running, so a second login is refused rather than
    // interleaved with it.
    let mut second = harness.connect();
    assert_eq!(
        second.authenticate(2, &account, "hunter2"),
        Some(Reply::Unavailable {
            attempt: 2,
            reason: UnavailableReason::Busy
        })
    );
    assert_eq!(
        harness.launches().len(),
        1,
        "the second login never reaches the launcher"
    );
}

#[test]
fn a_session_that_will_not_start_is_reported_rather_than_claimed() {
    // A program that does not exist cannot be executed.
    let harness = Harness::start(Verdict::Accepted, "/nonexistent/inox-session", &[], |_| {});
    let account = usable_account();

    let mut attempt = harness.connect();
    assert_eq!(
        attempt.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );

    let mut start = harness.connect();
    assert_eq!(
        start.start_session(1),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: false
        }),
        "the boundary must not claim a session it did not start"
    );
}

#[test]
fn the_service_exits_once_the_session_it_started_has_ended() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        config.exit_after_session = true;
    });
    let account = usable_account();

    let mut attempt = harness.connect();
    assert_eq!(
        attempt.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );

    let mut start = harness.connect();
    assert_eq!(
        start.start_session(1),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        })
    );

    // `/bin/true` has exited by now (or will in a moment), so the service shuts
    // down and removes its socket.
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.socket.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }

    assert!(
        !harness.socket.exists(),
        "the socket is removed when the service stops"
    );
}

#[test]
fn the_answer_is_always_for_the_attempt_the_client_named() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});

    let mut client = harness.connect();
    assert_eq!(
        client.authenticate(4242, &usable_account(), "hunter2"),
        Some(Reply::Accepted { attempt: 4242 }),
        "the identifier is echoed rather than replaced"
    );
}

#[test]
fn nothing_the_boundary_answers_echoes_what_the_client_sent() {
    let harness = Harness::start(Verdict::Rejected, TRUE, &[], |_| {});
    let mut client = harness.connect();
    let account = usable_account();

    let reply = client.authenticate(1, &account, "hunter2");
    let rendered = format!("{reply:?}");

    assert!(!rendered.contains(&account), "{rendered}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
}

#[test]
fn a_client_that_disappears_does_not_wedge_the_boundary() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    let account = usable_account();

    {
        let mut abandoned = harness.connect();
        abandoned.handshake();
        // Dropped with no request in flight.
    }

    let mut client = harness.connect();
    assert_eq!(
        client.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );
}

/// Authenticates and continues the attempt, answering what `StartSession` said.
fn start_one(harness: &Harness) -> Option<Reply> {
    let account = usable_account();

    let mut attempt = harness.connect();
    assert_eq!(
        attempt.authenticate(1, &account, "hunter2"),
        Some(Reply::Accepted { attempt: 1 })
    );

    harness.connect().start_session(1)
}

#[test]
fn the_session_goes_on_the_first_free_vt_in_the_configured_range() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        config.session_vt = VtRange::new(2, 4).expect("a valid range");
    });
    harness.logind.occupy(2);
    harness.logind.occupy(3);

    assert_eq!(
        start_one(&harness),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        })
    );

    let launches = harness.launches();
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0].vt, 4,
        "the first VT in the range with no session on it"
    );
}

#[test]
fn the_session_is_not_placed_on_the_active_sessions_vt() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        config.session_vt = VtRange::new(2, 4).expect("a valid range");
    });
    // The greeter, on VT 2. `GetSessionByVT(2)` would answer for it anyway; the
    // recorded session's VT is skipped so that a moment's inconsistency cannot put
    // the user session on top of the display.
    harness.logind.say_active("greeter", Some(2));

    assert_eq!(
        start_one(&harness),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        })
    );

    let launches = harness.launches();
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].vt, 3);
}

#[test]
fn the_display_is_handed_back_to_the_session_that_was_active() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |_| {});
    harness.logind.say_active("greeter", Some(2));

    assert_eq!(
        start_one(&harness),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        })
    );

    // `/bin/true` ends immediately, so the hand-back happens in a moment. Polling is
    // honest here: the answer to `StartSession` is not the hand-back, the session
    // ending is.
    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.activated().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(
        harness.activated(),
        vec!["greeter".to_owned()],
        "the session that had the display before the switch gets it back"
    );
}

#[test]
fn a_seat_with_no_active_session_starts_the_session_and_hands_nothing_back() {
    // `exit_after_session` is the synchronisation point: the service stops only
    // after the session has been reaped and the hand-back (which has nothing to do)
    // has run.
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        config.exit_after_session = true;
    });

    assert_eq!(
        start_one(&harness),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        })
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.socket.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }

    assert!(!harness.socket.exists(), "the service stopped");
    assert!(
        harness.activated().is_empty(),
        "with no recorded session there is nothing to hand back to"
    );
}

#[test]
fn a_range_with_no_free_vt_is_reported_rather_than_started() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        config.session_vt = VtRange::new(2, 2).expect("a valid range");
    });
    harness.logind.occupy(2);

    assert_eq!(
        start_one(&harness),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: false
        }),
        "an exhausted range is not a session on a busy VT"
    );
    assert!(harness.launches().is_empty());
}

#[test]
fn a_restarted_boundary_hands_the_display_back_to_the_greeter() {
    // The failure this repairs: the helper died while a user session ran, the
    // session ended, and the seat was left with no active session. The greeter's
    // own session is still on the seat, so a fresh helper puts it back — before it
    // serves anything.
    let logind = Arc::new(ScriptedLogind::reachable());
    logind.on_seat("greeter", 967);

    let harness = Harness::start_with(Verdict::Accepted, TRUE, &[], logind, |config| {
        config.greeter_uid = Some(967);
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while harness.activated().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(harness.activated(), vec!["greeter".to_owned()]);
}

#[test]
fn a_logind_that_cannot_answer_degrades_instead_of_blocking_the_login() {
    let harness = Harness::start(Verdict::Accepted, TRUE, &[], |config| {
        config.session_vt = VtRange::new(2, 4).expect("a valid range");
    });
    harness.logind.set_reachable(false);

    assert_eq!(
        start_one(&harness),
        Some(Reply::SessionStarted {
            attempt: 1,
            started: true
        }),
        "a machine without logind still logs people in"
    );

    let launches = harness.launches();
    assert_eq!(launches.len(), 1);
    assert_eq!(
        launches[0].vt, 2,
        "without arbitration the first VT in the range is used"
    );
    assert!(
        harness.activated().is_empty(),
        "nothing was recorded and no activation was attempted"
    );
}
