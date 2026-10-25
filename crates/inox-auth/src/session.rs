//! Which account a login is for, and how its session is started.
//!
//! Two things live here and they are kept apart on purpose:
//!
//! - [`Account`] — the answer to "is this a login this boundary will perform at
//!   all?", resolved through the system's own account database (`getpwnam_r` via
//!   the `users` crate, so NSS/LDAP/SSSD accounts work). The policy is the
//!   boundary's, not the client's: the name comes off the wire and the uid, gid,
//!   groups and home never do.
//! - [`SessionPlan`] — the *data* a session start consists of, so the rules about
//!   it ("the program is fixed by configuration", "the session runs as the
//!   authenticated uid with that user's groups", "the working directory is the
//!   account's home") are unit-testable without a child process.
//!
//! Since the boot login chain there are two ways a plan is turned into a running login, and
//! they are deliberately different primitives:
//!
//! - [`start`] spawns the program as a child. It is used by the tests' session
//!   launcher, and it is the shape this crate used before the boot login chain.
//! - [`exec`] replaces the calling process with the session program. That is what
//!   the session leader does: the process that ran `pam_open_session` *is* the
//!   login, so the logind session, the cgroup and the VT stay attached to it.
//!
//! There is no `unsafe` and no hand-written `setuid`: the privilege drop is
//! `std::os::unix::process::CommandExt::{uid, gid}` (and `process_group`), which
//! the standard library performs in the child between `fork` and `exec`; the
//! supplementary groups are applied to the calling thread with
//! `rustix::thread::set_thread_groups` immediately before either. Both are
//! required: `setgid` alone leaves the child with the helper's supplementary
//! groups, which for a root helper are root's.
//!
//! The VT/seat side lives in [`crate::logind`] and [`crate::leader`]: this module
//! never asks logind anything, and it never opens a PAM session.

use std::ffi::{CString, OsString};
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use rustix::process::Gid;
use thiserror::Error;
use tracing::{debug, warn};

/// Why an account cannot be logged into through this boundary.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AccountError {
    /// The account database has no such name.
    #[error("no such account")]
    Unknown,
    /// The account exists but is not one a login screen will start a session for
    /// (root, or a system account below the configured floor).
    #[error("the account is not one this boundary starts a session for")]
    NotALoginAccount,
    /// The account's own fields cannot be handed to PAM or to `exec`.
    ///
    /// This is a refusal rather than a warning because `pam`'s session setup
    /// panics on a non-UTF-8 home or shell — in a root process, that is a
    /// crash rather than an error, and a login screen that crashes the process
    /// holding every other login is worse than one that says no.
    #[error("the account's home or shell is not usable")]
    UnusableAccountData,
}

/// An account a session may be started for.
///
/// Not `Serialize`, not printable into a log in full: it is a fact about a person
/// (their uid, their home), and the only thing that needs it is the process that
/// is about to become them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The name, as the account database spells it.
    pub username: String,
    /// The account's numeric id.
    pub uid: u32,
    /// The account's primary group.
    pub gid: u32,
    /// Every group the account belongs to, primary included.
    ///
    /// Applied with `rustix::thread::set_thread_groups` immediately before the
    /// session is spawned, which is not optional: `setgid` does not touch the
    /// supplementary groups a root process already has, so a session started
    /// without this list would run with root's supplementary groups.
    pub groups: Vec<u32>,
    /// The account's home directory.
    pub home: PathBuf,
    /// The account's login shell.
    pub shell: PathBuf,
}

/// Resolves `username` into an account this boundary will start a session for.
///
/// `minimum_uid` is the floor below which an account is a system account: the
/// shipped default is 1000, and root is refused whatever the floor is.
///
/// # Errors
///
/// Returns [`AccountError`] when the account does not exist, is not a login
/// account, or carries data that cannot be handed to PAM.
pub fn account(username: &str, minimum_uid: u32) -> Result<Account, AccountError> {
    use users::os::unix::UserExt;

    let user = users::get_user_by_name(username).ok_or(AccountError::Unknown)?;
    let uid = user.uid();

    eligible(uid, user.primary_group_id(), minimum_uid)?;

    // PAM's `open_session` looks the account up again and calls `expect()` on the
    // user and on the UTF-8-ness of its home, shell and name. Checking it here
    // turns a panic in a privileged process into a refusal it can report.
    let name = user
        .name()
        .to_str()
        .ok_or(AccountError::UnusableAccountData)?;
    let home = user
        .home_dir()
        .to_str()
        .ok_or(AccountError::UnusableAccountData)?;
    let shell = user
        .shell()
        .to_str()
        .ok_or(AccountError::UnusableAccountData)?;

    let groups = match supplementary_groups(name, user.primary_group_id()) {
        Some(groups) => groups,
        None => {
            warn!(
                %username,
                "the account's group list could not be read; the session starts with its primary \
                 group only"
            );
            vec![user.primary_group_id()]
        }
    };

    Ok(Account {
        username: name.to_owned(),
        uid,
        gid: user.primary_group_id(),
        groups,
        home: PathBuf::from(home),
        shell: PathBuf::from(shell),
    })
}

/// Whether this account is one a login screen may start a session for.
///
/// Root is refused whatever the floor says, system accounts are refused by the
/// floor, and an account whose *primary* group is root's is refused too: it would
/// run with the root group whatever the supplementary list says, and no display
/// manager should be able to hand that out by accident.
///
/// # Errors
///
/// Returns [`AccountError::NotALoginAccount`] for any of those.
pub fn eligible(uid: u32, primary_gid: u32, minimum_uid: u32) -> Result<(), AccountError> {
    if uid == 0 || uid < minimum_uid || primary_gid == 0 {
        return Err(AccountError::NotALoginAccount);
    }

    Ok(())
}

/// The supplementary groups a session runs with, from the account database's list.
///
/// Pure, and separate from the lookup below so that the three properties it exists
/// for — the primary group is present, nothing is duplicated, and **root's group is
/// never in the list** — are unit tests rather than a consequence of what one
/// library happened to return.
pub fn session_groups(primary: u32, listed: impl IntoIterator<Item = u32>) -> Vec<u32> {
    let mut groups = vec![primary];

    for gid in listed {
        if gid == 0 {
            continue;
        }
        if !groups.contains(&gid) {
            groups.push(gid);
        }
    }

    groups
}

/// The account's supplementary groups, as the kernel's own `getgrouplist` returns
/// them.
///
/// **Not `users::User::groups`.** That function allocates a fixed 1024-entry
/// buffer, ignores the count `getgrouplist` writes back, and then maps the whole
/// buffer — so every entry past the real end reads as gid 0, root's group
/// ([RUSTSEC-2025-0040]). The same crate also has an unsound unaligned read in
/// its `gr_mem` handling ([RUSTSEC-2023-0059]). `nix`'s wrapper grows the buffer
/// until the call succeeds and takes the returned count, and the result is
/// filtered here as well: root's group never reaches a session, whatever the
/// library says.
///
/// # Returns
///
/// `None` when the group database could not be read at all; the caller then
/// starts the session with its primary group and says so in the log.
fn supplementary_groups(username: &str, primary: u32) -> Option<Vec<u32>> {
    let name = CString::new(username).ok()?;
    let listed = nix::unistd::getgrouplist(&name, nix::unistd::Gid::from_raw(primary)).ok()?;
    let groups = session_groups(primary, listed.into_iter().map(|group| group.as_raw()));

    if groups.len() < 2 {
        warn!(%username, "the account belongs to no supplementary group of its own");
    }

    Some(groups)
}

/// The `XDG_RUNTIME_DIR` a session runs with.
///
/// `/run/user/<uid>`, which is what `pam_systemd` creates and what every Wayland
/// client expects. It is *named* here and not created: creating a directory owned
/// by another user is exactly the kind of privileged side effect that belongs to
/// logind, and a boundary that made one itself would be a second implementation of
/// a rule it does not own.
pub fn runtime_dir(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/user/{uid}"))
}

/// Everything a session start consists of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlan {
    /// The account the session becomes.
    pub account: Account,
    /// The fixed program from this boundary's configuration. Never from the wire.
    pub program: PathBuf,
    /// Its literal arguments.
    pub arguments: Vec<OsString>,
    /// The session's runtime directory.
    pub runtime_dir: PathBuf,
    /// Variables the session phase answered with — logind's session id, the seat,
    /// the VT.
    ///
    /// They are applied before the account's own values, so a variable this crate
    /// is responsible for (`USER`, `HOME`, `XDG_RUNTIME_DIR`, …) cannot be
    /// overridden by something a PAM module or a wire-adjacent path supplied.
    pub environment: Vec<(String, String)>,
}

/// Builds the plan for `account` under this boundary's fixed session program.
pub fn plan(account: Account, program: PathBuf, arguments: Vec<OsString>) -> SessionPlan {
    let runtime_dir = runtime_dir(account.uid);

    SessionPlan {
        account,
        program,
        arguments,
        runtime_dir,
        environment: Vec::new(),
    }
}

impl SessionPlan {
    /// Adds the variables the session phase answered with.
    pub fn with_environment(mut self, environment: Vec<(String, String)>) -> Self {
        self.environment = environment;

        self
    }

    /// The command that starts the session, fully configured but not spawned.
    ///
    /// Split out from [`start`] so that the arguments and the process group can be
    /// asserted in a test without running anything.
    ///
    /// # Supplementary groups
    ///
    /// There is no `Command::groups` here, and that is not an oversight: the
    /// method is still unstable, and `setgid` alone would leave the child with
    /// whatever supplementary groups the privileged helper had — root's. The list
    /// is therefore applied to the *calling thread* by [`start`] (and by [`exec`])
    /// just before the spawn, because on Linux the supplementary groups are
    /// per-thread and a child inherits the thread that forked it.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);

        // A home directory that is not there would make `spawn` fail with ENOENT
        // and leave the person in front of the machine with a login screen that
        // says the session would not start for no visible reason. Starting in `/`
        // instead is a smaller lie than that: the session is told which home it has
        // through `HOME`, and a session that needs a writable directory has bigger
        // problems than its initial working directory.
        let working_directory = if self.account.home.is_dir() {
            self.account.home.as_path()
        } else {
            warn!(
                home = %self.account.home.display(),
                "the account's home directory is not there; the session starts in /"
            );
            Path::new("/")
        };

        command
            .args(&self.arguments)
            .current_dir(working_directory)
            // The privilege drop is `std`'s, performed in the child between `fork`
            // and `exec` in the order that works: gid, then uid.
            .gid(self.account.gid)
            .uid(self.account.uid)
            // Its own process group, so a signal aimed at the helper's group does
            // not reach the session. This is not `setsid`: a new *session* cannot be
            // created through `Command` without `pre_exec`, which this crate may not
            // use (`#![forbid(unsafe_code)]`), and the login's real session
            // leadership comes from `pam_systemd`/logind. The authentication boundary records it.
            .process_group(0)
            // What the session phase answered with (logind's session id, the seat,
            // the VT) first, so the account's own values below win.
            .envs(self.environment.iter().map(|(key, value)| (key, value)))
            // The values PAM also sets, made explicit: they describe the account
            // the session is for, and the session should not depend on which of the
            // two got there first.
            .env("USER", &self.account.username)
            .env("LOGNAME", &self.account.username)
            .env("HOME", &self.account.home)
            .env("SHELL", &self.account.shell)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir)
            // A session that wakes up with the login screen's stdin would read
            // keystrokes from the greeter's terminal.
            .stdin(Stdio::null());

        command
    }
}

/// Why a session was not started.
#[derive(Debug, Error)]
pub enum SessionError {
    /// The authenticator would not open a session for the account.
    #[error("the authenticator refused to open a session")]
    Boundary,
    /// The account's supplementary groups could not be applied.
    #[error("the account's supplementary groups could not be set: {0}")]
    Groups(#[source] io::Error),
    /// The session program could not be started.
    #[error("the session program could not be started: {0}")]
    Spawn(#[source] io::Error),
    /// No VT in the configured range is free.
    #[error("no VT in the configured range is free")]
    NoFreeVt,
    /// The session leader process could not be started.
    #[error("the session leader could not be started: {0}")]
    Leader(#[source] io::Error),
    /// The session leader did not confirm the session before its deadline.
    #[error("the session leader did not confirm the session in time")]
    LeaderTimeout,
    /// The session leader reported that it would not become the session.
    #[error("the session leader reported that the session did not open")]
    LeaderFailed,
}

/// Applies the account's supplementary groups to the calling thread.
///
/// On Linux the supplementary groups are per-thread credentials, so this has to
/// happen on the thread that forks or `exec`s the session: a child inherits the
/// groups of the thread that created it, and `setgid` does not touch the
/// supplementary list at all.
///
/// An unprivileged instance cannot call it for anybody (`setgroups` needs
/// `CAP_SETGID`), and it does not have to: a process that is not root can only
/// start a session as itself, so the groups it already has are the right ones. The
/// shipped deployment is root, where a failure here is fatal — starting a session
/// with the helper's own supplementary groups would hand it privileges nobody
/// granted.
fn apply_groups(account: &Account) -> Result<(), SessionError> {
    if rustix::process::geteuid().as_raw() != 0 {
        debug!(
            uid = account.uid,
            "running unprivileged: the session inherits this process's supplementary groups"
        );

        return Ok(());
    }

    let groups: Vec<Gid> = account.groups.iter().copied().map(Gid::from_raw).collect();
    rustix::thread::set_thread_groups(&groups)
        .map_err(|error| SessionError::Groups(io::Error::from(error)))
}

/// Starts the session, returning the child that is now running it.
///
/// The child is *not* waited for here: the caller reports the start to the client
/// first, so the login screen can take itself off the display while the session is
/// still coming up.
///
/// # Errors
///
/// Returns [`SessionError::Groups`] when the account's supplementary groups could
/// not be applied — the session is then *not* started, because starting one with
/// the helper's groups would hand it privileges nobody granted — and
/// [`SessionError::Spawn`] when the program could not be executed. That last one is
/// worth distinguishing: a missing `inox-session` is a packaging defect, and the
/// person in front of the machine should be told the session would not start rather
/// than be left with a blank screen.
pub fn start(plan: &SessionPlan) -> Result<Child, SessionError> {
    warn_about_missing_runtime_dir(plan);
    apply_groups(&plan.account)?;

    plan.command().spawn().map_err(SessionError::Spawn)
}

/// Becomes the session program, replacing this process.
///
/// This is the session leader's last act: the process that
/// opened the PAM session and claimed the VT `exec`s the configured program, which
/// is why the logind session, the cgroup and the PAM session survive as the login
/// rather than as a helper. The process environment is preserved because the plan's
/// environment is written into the `Command`, not inherited by accident.
///
/// # Returns
///
/// Only ever returns an error: a successful `exec` does not come back. The error is
/// the [`SessionError`] that stopped the session from starting, so the caller can
/// report one type on both paths.
pub fn exec(plan: &SessionPlan) -> SessionError {
    // The leader's stdout is the confirmation pipe its parent is reading. The
    // session must not keep it: the parent stops reading after the one line, and a
    // session writing into an unread pipe is a session that blocks on a full buffer
    // or dies of `SIGPIPE`.
    exec_with_stdout(plan, Stdio::null())
}

/// [`exec`], with the session's standard output given explicitly.
///
/// The split exists because `Command::exec` applies the command's redirections to
/// **this** process before `execve`, and does not undo them when `execve` fails. A
/// test that asserted the failure path through [`exec`] would therefore redirect
/// its own harness's stdout to `/dev/null` and lose the test report; it passes
/// [`Stdio::inherit`] instead, which leaves the descriptor alone.
fn exec_with_stdout(plan: &SessionPlan, stdout: Stdio) -> SessionError {
    warn_about_missing_runtime_dir(plan);

    if let Err(error) = apply_groups(&plan.account) {
        return error;
    }

    let mut command = plan.command();
    command.stdout(stdout);

    SessionError::Spawn(command.exec())
}

/// Says so when `pam_systemd`/logind has not created the session's runtime
/// directory. It is not fatal — the session may still start — but it is almost
/// always the sign that the PAM session phase did not run.
fn warn_about_missing_runtime_dir(plan: &SessionPlan) {
    if !plan.runtime_dir.is_dir() {
        warn!(
            uid = plan.account.uid,
            runtime_dir = %plan.runtime_dir.display(),
            "the session's runtime directory does not exist; pam_systemd or logind is expected to \
             create it before the session starts"
        );
    }
}

/// Whether `path` is a program this boundary can execute.
///
/// Used by the binary's argument handling: a mistyped `--session-program` should
/// fail at startup, not the first time somebody tries to log in.
pub fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account_fixture() -> Account {
        Account {
            username: "alice".to_owned(),
            uid: 1000,
            gid: 1000,
            groups: vec![1000, 27, 44],
            home: PathBuf::from("/home/alice"),
            shell: PathBuf::from("/bin/sh"),
        }
    }

    #[test]
    fn the_runtime_directory_is_the_users_own() {
        assert_eq!(runtime_dir(1000), PathBuf::from("/run/user/1000"));
        assert_eq!(runtime_dir(0), PathBuf::from("/run/user/0"));
    }

    #[test]
    fn a_plan_runs_the_configured_program_as_the_account() {
        let plan = plan(
            account_fixture(),
            PathBuf::from("/usr/bin/inox-session"),
            vec![OsString::from("--headless")],
        );
        let debugged = format!("{:?}", plan.command());

        // `Command` has no getters, so the assertions are made on the two settings
        // it does show when debugged: the program and its arguments. The privilege
        // fields are not printed by `Command`'s `Debug` (deliberately, upstream),
        // and the working directory is an environment detail whose rendering differs
        // between standard-library versions: all three are covered by the
        // integration test that actually starts a process as the current user.
        assert!(debugged.contains("/usr/bin/inox-session"), "{debugged}");
        assert!(debugged.contains("--headless"), "{debugged}");
    }

    #[test]
    fn a_plan_for_uid_1000_uses_that_users_runtime_directory() {
        let plan = plan(account_fixture(), PathBuf::from("/bin/true"), Vec::new());

        assert_eq!(plan.runtime_dir, PathBuf::from("/run/user/1000"));
    }

    #[test]
    fn the_session_program_is_never_an_argument_from_the_wire() {
        // A property of the types rather than of a check: `SessionPlan::program`
        // is only ever set from configuration, and nothing in `wire` can name a
        // path. This test exists so that the sentence is asserted somewhere.
        let plan = plan(account_fixture(), PathBuf::from("/bin/true"), Vec::new());

        assert_eq!(plan.program, PathBuf::from("/bin/true"));
        assert!(plan.arguments.is_empty());
    }

    #[test]
    fn roots_group_never_reaches_a_session() {
        // The shape of RUSTSEC-2025-0040: a group listing that ends in zeroes
        // because the library read past the count it was given.
        let groups = session_groups(1000, [1000, 27, 0, 0, 0, 44, 0]);

        assert_eq!(groups, vec![1000, 27, 44]);
        assert!(
            !groups.contains(&0),
            "a session must never run with root's group"
        );
    }

    #[test]
    fn the_session_group_list_is_the_primary_group_first_and_without_duplicates() {
        let groups = session_groups(1000, [44, 1000, 27, 44, 27]);

        assert_eq!(groups, vec![1000, 44, 27]);
        assert_eq!(groups[0], 1000, "the primary group comes first");
    }

    #[test]
    fn an_account_with_nothing_but_its_primary_group_is_still_a_list() {
        assert_eq!(session_groups(1000, []), vec![1000]);
        assert_eq!(session_groups(1000, [1000]), vec![1000]);
    }

    #[test]
    fn the_account_policy_refuses_root_system_accounts_and_root_s_group() {
        assert_eq!(
            eligible(0, 0, 1000),
            Err(AccountError::NotALoginAccount),
            "root is refused whatever the floor is"
        );
        assert_eq!(
            eligible(0, 0, 0),
            Err(AccountError::NotALoginAccount),
            "even when the floor is zero"
        );
        assert_eq!(
            eligible(999, 999, 1000),
            Err(AccountError::NotALoginAccount),
            "a system account is below the floor"
        );
        assert_eq!(
            eligible(1000, 0, 1000),
            Err(AccountError::NotALoginAccount),
            "an account whose primary group is root's would run with the root group"
        );
        assert_eq!(eligible(1000, 1000, 1000), Ok(()));
        assert_eq!(eligible(65534, 65534, 1000), Ok(()));
    }

    #[test]
    fn only_real_executables_are_accepted_as_a_session_program() {
        let executable = std::env::current_exe().expect("the test binary has a path");
        let not_executable = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");

        assert!(is_executable(&executable));
        assert!(!is_executable(&not_executable));
        assert!(!is_executable(Path::new("/nonexistent/program")));
    }

    /// The account the tests run as, so that `uid`/`gid` need no privilege.
    fn current_account() -> Account {
        let gid = rustix::process::getegid().as_raw();

        Account {
            username: "fixture".to_owned(),
            uid: rustix::process::geteuid().as_raw(),
            gid,
            groups: vec![gid],
            home: PathBuf::from("/"),
            shell: PathBuf::from("/bin/sh"),
        }
    }

    #[test]
    fn the_session_environment_reaches_the_program_and_the_accounts_values_win() {
        // The session leader learns its logind session id and its seat from the
        // session phase; both have to arrive at the session program. `HOME` is in
        // here as the counter-example: a variable this crate is responsible for is
        // not overridable by whatever the session phase happened to say.
        let plan = plan(current_account(), PathBuf::from("/usr/bin/env"), Vec::new())
            .with_environment(vec![
                ("XDG_SESSION_ID".to_owned(), "7".to_owned()),
                ("XDG_VTNR".to_owned(), "3".to_owned()),
                ("HOME".to_owned(), "/somewhere/else".to_owned()),
            ]);

        let output = plan.command().output().expect("`env` runs");
        let environment = String::from_utf8_lossy(&output.stdout);

        assert!(environment.contains("XDG_SESSION_ID=7"), "{environment}");
        assert!(environment.contains("XDG_VTNR=3"), "{environment}");
        assert!(
            environment.contains("HOME=/"),
            "the account's home wins over the session phase's idea of it: {environment}"
        );
        assert!(!environment.contains("/somewhere/else"), "{environment}");
    }

    #[test]
    fn a_session_that_cannot_be_execed_is_an_error_and_not_a_replaced_process() {
        // `exec` only returns when the program could not be executed; a successful
        // one would have replaced this test process.
        let plan = plan(
            current_account(),
            PathBuf::from("/nonexistent/inox-session"),
            Vec::new(),
        );

        // `Stdio::inherit` for the reason on `exec_with_stdout`: the failing
        // `exec` must not take the test harness's own stdout with it.
        let outcome = exec_with_stdout(&plan, Stdio::inherit());

        assert!(matches!(outcome, SessionError::Spawn(_)));
    }
}
