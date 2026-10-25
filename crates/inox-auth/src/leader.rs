//! The session leader: one process per login, and the parent that starts it.
//!
//! The boot login chain replaces "the helper opens the session in its own long-lived
//! process" with "the helper starts a one-shot child that becomes the login". The
//! reason is PAM's: `pam_open_session` registers the session for the process that
//! calls it (`pam_systemd` asks logind to create a session for that pid, and logind
//! puts *that process* in the session's scope), and the session's VT comes from the
//! caller's `XDG_VTNR`. Only the process that is about to `exec` the session
//! program can do that, and only a fresh process can carry a fresh session
//! environment — the long-lived helper cannot rewrite its own environment under
//! edition 2024 (`std::env::set_var` is `unsafe` there, which this crate forbids).
//!
//! This module is both halves of that arrangement:
//!
//! - the parent's side — [`SessionLauncher`], [`ShippedLauncher`] and
//!   [`LeaderPlan`]: start `inox-auth --session-leader`, give it the account and the
//!   VT, and wait for the one line that says `pam_open_session` succeeded;
//! - the child's side — [`run`]: check the arguments the root parent passed,
//!   open the PAM session as root, claim the VT through logind, announce the
//!   session and `exec` the configured program as the account.
//!
//! The two halves talk over a pipe (the child's stdout), and the protocol is one
//! line: [`LEADER_STARTED`] on success, anything else — [`LEADER_FAILED`], end of
//! file, or silence until [`DEFAULT_LEADER_DEADLINE`] — on failure. The child never
//! writes anything else to stdout, and the session program is started with stdout
//! on `/dev/null`, so the pipe is released by the `exec` rather than held open by
//! the login.
//!
//! What the leader is *not* given: a credential. It is told the account name the
//! socket service already authenticated and the VT the parent picked, and nothing
//! else. Its command line comes from the root parent, never from the wire (ADR
//! 0037 §失败、恢复与安全).

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};
use inox_common::i18n::{Arg, Localizer};
use rustix::process::geteuid;
use tracing::{debug, info, warn};

use crate::logind::{Logind, SystemLogind, VtRange};
use crate::pam;
use crate::server::Config;
use crate::session::{self, Account, SessionError};

/// The line a session leader writes once `pam_open_session` has succeeded.
///
/// It is written *before* the `exec`, because an `exec` that succeeds never
/// returns. An `exec` that fails is observed by the parent as the session ending
/// immediately, which hands the display back rather than leaving it black.
pub const LEADER_STARTED: &str = "started";

/// The line a session leader writes when it will not become the session.
///
/// The parent treats *anything* other than [`LEADER_STARTED`] as this, including an
/// empty pipe: a leader that died before it could say anything has failed too.
pub const LEADER_FAILED: &str = "failed";

/// How long the parent waits for the leader's confirmation.
///
/// Long enough for a PAM session stack that has to ask logind or read a file, short
/// enough that a login screen answers "the session did not start" rather than
/// hanging on a leader that is stuck. On timeout the leader is killed: a leader
/// still inside `pam_open_session` would otherwise own a VT and a logind session
/// that nothing will hand back.
///
/// It stays below [`crate::server::DEFAULT_ATTEMPT_DEADLINE`] (30 s) on purpose: the
/// client already bounds `StartSession` by that deadline, so a leader deadline at or
/// above it would turn the leader's failure into the client's timeout — the answer
/// would be "not started" either way, but the reason would be lost.
pub const DEFAULT_LEADER_DEADLINE: Duration = Duration::from_secs(25);

/// What the parent tells a session leader.
#[derive(Debug, Clone)]
pub struct LeaderPlan {
    /// The account the socket service authenticated.
    pub account: Account,
    /// The VT the parent picked, and the leader re-checks.
    pub vt: u32,
    /// The program the leader becomes, from this boundary's configuration.
    pub program: PathBuf,
    /// Its literal arguments, likewise from configuration.
    pub arguments: Vec<OsString>,
    /// The PAM service the leader opens the session through.
    pub service: String,
    /// The uid floor the leader re-checks the account against.
    pub minimum_uid: u32,
    /// The VT range the leader re-checks [`LeaderPlan::vt`] against.
    pub vt_range: VtRange,
    /// The logind seat, passed on to the session as `XDG_SEAT`.
    pub seat: String,
}

/// A session leader that has confirmed `pam_open_session`.
///
/// Holding it is what keeps the login alive: dropping it without waiting leaves the
/// child running (it is not killed on drop), which is why the server waits for it
/// before handing the display back.
pub struct RunningSession {
    child: Child,
}

impl RunningSession {
    /// The handle for an already-started session process.
    ///
    /// Used by the seam's test implementation, which starts the configured program
    /// directly instead of going through PAM.
    pub fn new(child: Child) -> Self {
        Self { child }
    }

    /// Waits for the session process to exit.
    ///
    /// # Errors
    ///
    /// Propagates the wait error (an interrupted wait, a child that was reaped by
    /// somebody else).
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait()
    }
}

/// How the daemon starts the session leader.
///
/// A trait rather than a function so that the service's own rules — one session at
/// a time, the VT it chose, the display it hands back — can be tested without root
/// and without PAM (root AGENTS.md §6: PAM and logind are system boundaries and may
/// be stubbed; the rules of the service may not).
pub trait SessionLauncher: Send + Sync {
    /// Starts the leader for `plan` and waits for its confirmation.
    ///
    /// # Errors
    ///
    /// [`SessionError::Leader`] when the process cannot be started,
    /// [`SessionError::LeaderFailed`] when it reports (or dies without reporting)
    /// that the session did not open, and [`SessionError::LeaderTimeout`] when it
    /// does not report at all in time. A leader that did not confirm is killed and
    /// reaped before the error is returned.
    fn launch(&self, plan: &LeaderPlan) -> Result<RunningSession, SessionError>;
}

/// The launcher the shipped binary uses: re-exec itself in `--session-leader` mode.
pub struct ShippedLauncher {
    deadline: Duration,
}

impl ShippedLauncher {
    /// A launcher that waits `deadline` for each leader's confirmation.
    pub fn new(deadline: Duration) -> Self {
        Self { deadline }
    }

    /// The command that starts the leader, fully configured but not spawned.
    ///
    /// Split out so the arguments can be asserted in a test without starting a
    /// process: every one of them is a fact this crate is responsible for, and a
    /// missing `--vt` would be a leader that refuses to run.
    fn command(&self, executable: &Path, plan: &LeaderPlan) -> Command {
        let mut command = Command::new(executable);

        command
            .arg("--session-leader")
            .arg("--user")
            .arg(&plan.account.username)
            .arg("--vt")
            .arg(plan.vt.to_string())
            .arg("--session-service")
            .arg(&plan.service)
            .arg("--minimum-uid")
            .arg(plan.minimum_uid.to_string())
            .arg("--session-program")
            .arg(&plan.program)
            .arg("--seat")
            .arg(&plan.seat)
            .arg("--session-vt-min")
            .arg(plan.vt_range.min().to_string())
            .arg("--session-vt-max")
            .arg(plan.vt_range.max().to_string());

        for argument in &plan.arguments {
            command.arg("--session-arg").arg(argument);
        }

        // The VT is how `pam_systemd` learns which VT to put the session on. It
        // reads `XDG_VTNR` from the environment, and the child's environment is
        // set by the parent through `Command` — the safe API the ADR asks for —
        // rather than by the child writing its own (which edition 2024 forbids and
        // which this crate could not do without `unsafe`).
        command
            .env("XDG_VTNR", plan.vt.to_string())
            .stdin(Stdio::null())
            // The confirmation pipe. The leader writes one line into it and then
            // `exec`s with stdout on `/dev/null`, so the session never inherits it.
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        command
    }
}

impl SessionLauncher for ShippedLauncher {
    fn launch(&self, plan: &LeaderPlan) -> Result<RunningSession, SessionError> {
        // An unprivileged instance cannot be a session leader: the leader mode
        // refuses to run as non-root, and `pam_open_session` would register no
        // session for a process that cannot create one. A development instance is
        // exactly that — `inox-auth --socket /tmp/…` — and it still has to be able
        // to log in, so it starts the session program directly, as this crate did
        // before the boot login chain, and says what is missing.
        //
        // The shipped deployment is root and always goes through the leader. This
        // branch cannot start a session as anybody but the invoking user:
        // `session::start` only changes credentials for a root process.
        if geteuid().as_raw() != 0 {
            warn!(
                uid = plan.account.uid,
                vt = plan.vt,
                "running unprivileged: the session is started directly, without a logind session or \
                 a VT switch"
            );
            let session_plan = session::plan(
                plan.account.clone(),
                plan.program.clone(),
                plan.arguments.clone(),
            );

            return session::start(&session_plan).map(RunningSession::new);
        }

        let executable = std::env::current_exe().map_err(SessionError::Leader)?;
        let mut child = self
            .command(&executable, plan)
            .spawn()
            .map_err(SessionError::Leader)?;

        match confirmation(&mut child, self.deadline) {
            Ok(true) => Ok(RunningSession { child }),
            Ok(false) => {
                abandon(&mut child);
                Err(SessionError::LeaderFailed)
            }
            Err(error) => {
                abandon(&mut child);
                Err(error)
            }
        }
    }
}

/// Reads the leader's one line, with a deadline, on a thread that cannot outlive
/// the wait for long.
fn confirmation(child: &mut Child, deadline: Duration) -> Result<bool, SessionError> {
    let Some(stdout) = child.stdout.take() else {
        return Err(SessionError::Leader(io::Error::other(
            "the session leader was started without a confirmation pipe",
        )));
    };

    let (sender, receiver) = mpsc::channel();
    thread::Builder::new()
        .name("inox-auth-leader-report".to_owned())
        .spawn(move || {
            let mut line = String::new();
            let outcome = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            // A receiver that timed out and killed the child is the expected case
            // here; the send fails then and there is nothing left to do about it.
            let _ = sender.send(outcome);
        })
        .map_err(SessionError::Leader)?;

    match receiver.recv_timeout(deadline) {
        Ok(Ok(line)) => Ok(line.trim() == LEADER_STARTED),
        Ok(Err(error)) => Err(SessionError::Leader(error)),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(SessionError::LeaderTimeout),
        // The reporting thread ended without sending, which can only mean it was
        // killed by the process ending: the leader produced nothing.
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(false),
    }
}

/// Kills and reaps a leader that did not confirm.
fn abandon(child: &mut Child) {
    if let Err(error) = child.kill() {
        debug!(%error, "the session leader had already exited");
    }
    if let Err(error) = child.wait() {
        warn!(%error, "the session leader could not be reaped");
    }
}

/// What `--session-leader` was started with.
#[derive(Debug, Clone)]
pub struct LeaderRequest {
    /// The account the parent authenticated.
    pub user: String,
    /// The VT the parent picked.
    pub vt: u32,
}

/// Runs the `--session-leader` mode: open the session, claim the VT, become it.
///
/// # Errors
///
/// Returns an error — after writing [`LEADER_FAILED`] where it still can — when
/// this process is not root, the account or the VT is not one this boundary will
/// log in, the PAM session cannot be opened, or the session program cannot be
/// executed. The parent turns any of those into `SessionStarted { started: false }`.
pub fn run(config: &Config, request: &LeaderRequest, localizer: &Localizer) -> Result<()> {
    if geteuid().as_raw() != 0 {
        let message = localizer.message("auth-leader-root");
        fail(&message);

        bail!("{message}");
    }

    let account = match validate(config, request) {
        Ok(account) => account,
        Err(refusal) => {
            let message = refusal.message(localizer);
            warn!(%message, "the session leader refused to start the session");
            fail(&message);

            bail!("{message}");
        }
    };

    let logind = SystemLogind::new(config.seat.clone());

    // PAM's session stack runs here, in the process that is about to become the
    // login: `pam_systemd` registers this pid with logind, which is what gives the
    // session its cgroup, its runtime directory and its VT. No credential reaches
    // this process.
    let opened = match pam::open_session(&config.session_service, &account) {
        Ok(opened) => opened,
        Err(error) => {
            fail(LEADER_FAILED);

            return Err(error.into());
        }
    };

    let session_id = match logind.session_of_pid(std::process::id()) {
        Ok(session) => session.map(|session| session.id),
        Err(error) => {
            warn!(
                %error,
                "logind could not be asked which session this process belongs to; the VT is not \
                 switched"
            );
            None
        }
    };

    match session_id.as_deref() {
        Some(id) => match logind.activate(id) {
            Ok(()) => info!(
                session = id,
                vt = request.vt,
                "the session was activated on its VT"
            ),
            Err(error) => warn!(
                %error,
                session = id,
                "the session could not be activated; the display is not switched"
            ),
        },
        None => warn!(
            "this process has no logind session; the VT is not switched (is pam_systemd in the \
             session stack?)"
        ),
    }

    let mut environment = vec![
        ("XDG_VTNR".to_owned(), request.vt.to_string()),
        ("XDG_SEAT".to_owned(), config.seat.clone()),
    ];
    if let Some(id) = &session_id {
        environment.push(("XDG_SESSION_ID".to_owned(), id.clone()));
    }

    let plan = session::plan(
        account,
        config.session_program.clone(),
        config.session_arguments.clone(),
    )
    .with_environment(environment);

    announce(LEADER_STARTED);

    // The session program is execed, not spawned: this process becomes it, which
    // is what keeps the logind session, the cgroup and the PAM session attached to
    // the login. `exec` only returns on failure.
    let error = session::exec(&plan);

    // The `exec` failed, so this process still owns the PAM session; dropping the
    // handle closes it rather than leaving a session nobody will ever use.
    drop(opened);

    warn!(error = %error, "the session program could not be executed");
    fail(LEADER_FAILED);

    Err(error.into())
}

/// Why a session leader will not become the session.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LeaderRefusal {
    /// The VT is outside the range this boundary places sessions on.
    Vt {
        /// The VT the parent asked for.
        vt: u32,
        /// The range the boundary allows.
        range: VtRange,
    },
    /// The program the parent named cannot be executed.
    Program(PathBuf),
    /// The account is not one this boundary starts a session for.
    Account(String),
}

impl LeaderRefusal {
    /// The sentence for whoever runs the machine.
    fn message(&self, localizer: &Localizer) -> String {
        match self {
            Self::Vt { vt, range } => localizer.message_with(
                "auth-leader-vt",
                &[
                    ("vt", Arg::from(vt.to_string())),
                    ("range", Arg::from(range.to_string())),
                ],
            ),
            Self::Program(path) => localizer.message_with(
                "auth-session-program",
                &[("path", Arg::from(path.to_string_lossy().to_string()))],
            ),
            Self::Account(user) => {
                localizer.message_with("auth-leader-account", &[("user", Arg::from(user.clone()))])
            }
        }
    }
}

/// Checks the arguments the root parent passed, before PAM is touched.
///
/// The leader re-checks everything the daemon decided, because its command line is
/// the one place privileges enter this mode: a VT outside the configured range, a
/// program that cannot be executed, or an account the policy refuses are all
/// refusals here even if the daemon would have refused them first. The only check
/// not in here is root, which [`run`] makes first and which no test can make
/// (CI is unprivileged) — `tests/cli.rs` checks that refusal as a process instead.
///
/// # Errors
///
/// [`LeaderRefusal`] naming the first argument that is not acceptable.
fn validate(config: &Config, request: &LeaderRequest) -> Result<Account, LeaderRefusal> {
    if !config.session_vt.contains(request.vt) {
        return Err(LeaderRefusal::Vt {
            vt: request.vt,
            range: config.session_vt,
        });
    }

    if !session::is_executable(&config.session_program) {
        return Err(LeaderRefusal::Program(config.session_program.clone()));
    }

    session::account(&request.user, config.minimum_uid)
        .map_err(|_| LeaderRefusal::Account(request.user.clone()))
}

/// Writes one line to the parent, if there still is a parent to write to.
fn announce(line: &str) {
    let mut stdout = io::stdout();
    if let Err(error) = stdout
        .write_all(line.as_bytes())
        .and_then(|()| stdout.write_all(b"\n"))
        .and_then(|()| stdout.flush())
    {
        // The parent may be gone (it was killed). The person in front of the
        // machine still asked for a session, so it is started rather than
        // abandoned because a report could not be delivered.
        warn!(%error, "the session leader could not report to its parent");
    }
}

/// Announces a failure the parent can still read, unless it already said why.
fn fail(message: &str) {
    let mut stdout = io::stdout();
    if let Err(error) = stdout
        .write_all(LEADER_FAILED.as_bytes())
        .and_then(|()| stdout.write_all(b"\n"))
        .and_then(|()| stdout.flush())
    {
        debug!(%error, %message, "the session leader could not report its refusal");
        return;
    }

    info!(
        refusal = message,
        "the session leader refused to start the session"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Instant;

    fn account_fixture() -> Account {
        Account {
            username: "alice".to_owned(),
            uid: 1000,
            gid: 1000,
            groups: vec![1000],
            home: PathBuf::from("/home/alice"),
            shell: PathBuf::from("/bin/sh"),
        }
    }

    /// The account the tests run as, so that `uid`/`gid` need no privilege.
    fn current_account() -> Account {
        let gid = rustix::process::getegid().as_raw();

        Account {
            username: "fixture".to_owned(),
            uid: geteuid().as_raw(),
            gid,
            groups: vec![gid],
            home: PathBuf::from("/"),
            shell: PathBuf::from("/bin/sh"),
        }
    }

    fn plan() -> LeaderPlan {
        LeaderPlan {
            account: account_fixture(),
            vt: 3,
            program: PathBuf::from("/usr/bin/inox-session"),
            arguments: vec![OsString::from("--headless")],
            service: "inox-auth-session".to_owned(),
            minimum_uid: 1000,
            vt_range: VtRange::new(2, 6).expect("valid"),
            seat: "seat0".to_owned(),
        }
    }

    #[test]
    fn the_leader_is_told_the_account_and_the_vt_and_nothing_else_authoritative() {
        let command = ShippedLauncher::new(DEFAULT_LEADER_DEADLINE)
            .command(Path::new("/usr/bin/inox-auth"), &plan());
        let rendered = format!("{command:?}");

        assert!(rendered.contains("--session-leader"), "{rendered}");
        assert!(rendered.contains("--user"), "{rendered}");
        assert!(rendered.contains("alice"), "{rendered}");
        assert!(rendered.contains("--vt"), "{rendered}");
        assert!(rendered.contains("3"), "{rendered}");
        assert!(
            rendered.contains("--session-vt-min") && rendered.contains("--session-vt-max"),
            "the leader re-checks the VT against the same range the parent used: {rendered}"
        );
        assert!(
            rendered.contains("--session-program") && rendered.contains("/usr/bin/inox-session"),
            "the program comes from configuration, never from the wire: {rendered}"
        );
        assert!(
            rendered.contains("--session-arg") && rendered.contains("--headless"),
            "{rendered}"
        );
    }

    fn config() -> Config {
        Config::new("/tmp/inox-auth-leader-test.sock", "/bin/true")
    }

    #[test]
    fn the_leader_refuses_a_vt_outside_the_configured_range() {
        let mut config = config();
        config.session_vt = VtRange::new(2, 4).expect("a valid range");
        let request = LeaderRequest {
            user: "root".to_owned(),
            vt: 9,
        };

        assert_eq!(
            validate(&config, &request),
            Err(LeaderRefusal::Vt {
                vt: 9,
                range: VtRange::new(2, 4).expect("a valid range"),
            }),
            "the parent's VT is re-checked here, not trusted"
        );
    }

    #[test]
    fn the_leader_refuses_an_account_this_boundary_does_not_log_in() {
        let request = LeaderRequest {
            user: "root".to_owned(),
            vt: 2,
        };

        assert_eq!(
            validate(&config(), &request),
            Err(LeaderRefusal::Account("root".to_owned())),
            "root is refused by the account policy before PAM is reached"
        );
    }

    #[test]
    fn the_leader_refuses_a_program_that_cannot_be_executed() {
        let mut config = config();
        config.session_program = PathBuf::from("/nonexistent/inox-session");
        let request = LeaderRequest {
            user: "root".to_owned(),
            vt: 2,
        };

        assert_eq!(
            validate(&config, &request),
            Err(LeaderRefusal::Program(PathBuf::from(
                "/nonexistent/inox-session"
            )))
        );
    }

    #[test]
    fn an_unprivileged_instance_starts_the_session_without_a_leader() {
        if geteuid().as_raw() == 0 {
            // As root the launcher really does re-exec this binary as a leader,
            // which is not something a test binary can be; the fallback is only for
            // the unprivileged case anyway.
            eprintln!("running as root: the unprivileged fallback does not apply");
            return;
        }

        let launcher = ShippedLauncher::new(Duration::from_secs(5));
        let mut plan = plan();
        plan.program = PathBuf::from("/bin/true");
        plan.arguments.clear();
        plan.account = current_account();

        let mut session = launcher
            .launch(&plan)
            .expect("an unprivileged instance starts the program directly");
        assert!(
            session.wait().expect("the session exits").success(),
            "the configured program ran"
        );
    }

    #[test]
    fn a_leader_that_confirms_is_a_running_session() {
        // A stand-in for the leader: the real one opens a PAM session first, this
        // one says the one line the protocol is made of.
        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf 'started\\n'"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("a shell to stand in for the leader");

        assert!(confirmation(&mut child, Duration::from_secs(10)).expect("a report"));
        assert!(child.wait().expect("the shell exits").success());
    }

    #[test]
    fn a_leader_that_refuses_is_not_a_started_session() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf 'failed\\n'"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("a shell to stand in for the leader");

        assert!(!confirmation(&mut child, Duration::from_secs(10)).expect("a report"));
        assert!(child.wait().expect("the shell exits").success());
    }

    #[test]
    fn a_leader_that_dies_without_a_word_is_a_failure_not_a_hang() {
        let mut child = Command::new("/bin/true")
            .stdout(Stdio::piped())
            .spawn()
            .expect("a child that says nothing");

        assert!(!confirmation(&mut child, Duration::from_secs(10)).expect("a report"));
        assert!(child.wait().expect("the child exits").success());
    }

    #[test]
    fn a_leader_that_never_reports_is_given_a_deadline() {
        let mut child = Command::new("/bin/sleep")
            .arg("30")
            .stdout(Stdio::piped())
            .spawn()
            .expect("a child that stays silent");

        let started = Instant::now();
        let outcome = confirmation(&mut child, Duration::from_millis(100));

        assert!(
            matches!(outcome, Err(SessionError::LeaderTimeout)),
            "a leader that never reports is a timeout, not a started session"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the wait is bounded by the deadline"
        );

        abandon(&mut child);
        assert!(
            child.try_wait().expect("waitable").is_some(),
            "a leader that timed out is killed and reaped, not left behind"
        );
    }
}
