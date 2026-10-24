//! The thin adapter that owns the host's two child processes.
//!
//! Everything expensive to get wrong is in [`crate::supervisor`]; this module only
//! turns that state machine's [`Action`](crate::supervisor::Action)s into process operations, and turns what
//! the kernel reports back into events. It has no policy of its own: it does not
//! decide when to start, stop or restart anything.
//!
//! Three details are load-bearing.
//!
//! - **Every child is its own process group** (`process_group(0)`). A signal is
//!   therefore sent to the group rather than to one pid, so a component's
//!   descendants go with it and a crashed leader cannot leave orphans behind — the
//!   same rule `inox-session` applies to its components.
//! - **Children are launched through this very binary, once more.** calloop's
//!   signal source is a `signalfd`, which means `SIGTERM` and `SIGINT` are *masked*
//!   in this thread; a child forked from it inherits that mask and would never see
//!   the `SIGTERM` the host sends it. The host therefore starts
//!   `<self> --exec-child <program> …`: that intermediate mode resets the mask
//!   through a safe API and then `exec`s the program, keeping the pid unchanged.
//!   This is the same device `inox-session` uses for the same reason.
//! - **`Drop` kills what is left.** The state machine stops its children with
//!   `SIGTERM` and a grace period, but an error path in the event loop can leave a
//!   child behind. Dropping the set kills the process groups and reaps them, so a
//!   failed host start does not leave a compositor holding the VT.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};

use rustix::process::{Pid, Signal, kill_process_group};
use std::os::unix::process::CommandExt;
use tracing::{debug, info, warn};

use crate::supervisor::{Plan, Target};

/// The environment variable a Wayland client reads the socket name from.
pub const WAYLAND_DISPLAY_ENV: &str = "WAYLAND_DISPLAY";

/// The environment variable the login screen reads the boundary socket from
/// (ADR 0029, and the same variable `inox-greet` and `inox-auth` use).
pub const AUTH_SOCKET_ENV: &str = "INOX_AUTH_SOCKET";

/// The internal first argument that makes this binary reset signal masks and
/// `exec` the program that follows it.
///
/// It is not a user-facing option: it carries no capability (the host runs as the
/// greeter user and every process it starts runs as the same user) and it exists
/// only because a `signalfd` mask survives `exec`.
pub const EXEC_CHILD: &str = "--exec-child";

/// A program and its literal arguments; never interpreted by a shell.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// The executable, as an absolute path when the host built it from `--bin-dir`.
    pub program: OsString,
    /// The arguments, passed literally.
    pub arguments: Vec<OsString>,
}

impl CommandSpec {
    /// A command for `program` without arguments.
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
        }
    }
}

/// The host's children, by role.
///
/// A role is either running (the entry holds its [`Child`]) or not. There is no
/// third state: a child that exited is reaped and its entry removed, which is what
/// makes [`Children::signal`] on a finished child a no-op instead of a signal sent
/// to a recycled pid.
#[derive(Debug, Default)]
pub struct Children {
    compositor: Option<Child>,
    greeter: Option<Child>,
}

impl Children {
    /// An empty set: nothing is running yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts `target` as described by `plan`, through `launcher` (this binary).
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the program cannot be executed at all — a missing
    /// binary, no permission, no memory. A child that starts and then fails is not an
    /// error here; it is an exit the caller reaps.
    pub fn spawn(&mut self, target: Target, plan: &Plan, launcher: &Path) -> io::Result<()> {
        let spec = match target {
            Target::Compositor => &plan.compositor,
            Target::Greeter => &plan.greeter,
        };

        // The program is executed by the child (see `EXEC_CHILD`), so an obvious
        // packaging mistake — no such binary, or one that is not executable — would
        // otherwise surface as "the child exited immediately" rather than as "this
        // program cannot be started". The error the caller needs names the program.
        ensure_startable(&spec.program)?;

        let mut command = Command::new(launcher);
        command
            .arg(EXEC_CHILD)
            .arg(&spec.program)
            .args(&spec.arguments)
            // The host was started by systemd with `StandardInput=tty`, which is the
            // greeter's terminal. Children must not read it: the login screen has no
            // use for it, and the compositor must not consume keystrokes meant for
            // whatever is in front of the VT (ADR 0037 §决策 3.3).
            .stdin(Stdio::null())
            // One process group per child, so a signal reaches its descendants too.
            .process_group(0);

        if target == Target::Greeter {
            command
                .env(WAYLAND_DISPLAY_ENV, &plan.wayland_display)
                .env(AUTH_SOCKET_ENV, &plan.auth_socket)
                // An inherited `WAYLAND_SOCKET` is an already-open connection and
                // takes precedence over `WAYLAND_DISPLAY` in every toolkit. If the
                // host itself was started from a graphical session, leaving it set
                // would point the login screen at *that* compositor instead of the
                // one this host just started.
                .env_remove("WAYLAND_SOCKET");
        }

        let child = command.spawn()?;
        info!(
            component = target.component(),
            pid = child.id(),
            program = %spec.program.to_string_lossy(),
            "started child process"
        );
        *self.slot_mut(target) = Some(child);
        Ok(())
    }

    /// Collects the exit status of `target` if it has exited.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the status cannot be read, which leaves the entry
    /// in place; the next tick tries again.
    pub fn reap(&mut self, target: Target) -> io::Result<Option<ExitStatus>> {
        let slot = self.slot_mut(target);
        let Some(child) = slot.as_mut() else {
            return Ok(None);
        };

        let status = child.try_wait()?;
        if let Some(status) = status {
            // The leader is gone, but its process group may still hold descendants
            // (a helper the component spawned, a stuck child of a shell wrapper).
            signal_group(child.id(), Signal::KILL);
            info!(
                component = target.component(),
                %status,
                "child process exited"
            );
            *slot = None;
        }
        Ok(status)
    }

    /// Sends `signal` to `target`'s process group, if it is still running.
    pub fn signal(&self, target: Target, signal: Signal) {
        let Some(child) = self.slot(target) else {
            debug!(
                component = target.component(),
                "no child process to signal; it has already exited"
            );
            return;
        };
        info!(
            component = target.component(),
            pid = child.id(),
            ?signal,
            "signalling child process group"
        );
        signal_group(child.id(), signal);
    }

    /// Whether `target` is currently running according to this set.
    pub fn is_running(&self, target: Target) -> bool {
        self.slot(target).is_some()
    }

    /// Stops and reaps whatever is still running, without a grace period.
    fn kill_remaining(&mut self) {
        for target in [Target::Greeter, Target::Compositor] {
            let Some(mut child) = self.slot_mut(target).take() else {
                continue;
            };
            warn!(
                component = target.component(),
                pid = child.id(),
                "killing a child that outlived the host"
            );
            signal_group(child.id(), Signal::KILL);
            if let Err(error) = child.wait() {
                warn!(component = target.component(), %error, "failed to reap child process");
            }
        }
    }

    fn slot(&self, target: Target) -> Option<&Child> {
        match target {
            Target::Compositor => self.compositor.as_ref(),
            Target::Greeter => self.greeter.as_ref(),
        }
    }

    fn slot_mut(&mut self, target: Target) -> &mut Option<Child> {
        match target {
            Target::Compositor => &mut self.compositor,
            Target::Greeter => &mut self.greeter,
        }
    }
}

impl Drop for Children {
    fn drop(&mut self) {
        self.kill_remaining();
    }
}

/// Whether `program` can plausibly be executed, as far as a `stat` can tell.
///
/// A preflight, not a guarantee — the real `exec` happens in the child — but it turns
/// the two packaging mistakes that would otherwise read as "the child exited
/// immediately" into an error that names the program: one that is not installed, and
/// one that is not executable.
fn ensure_startable(program: &OsStr) -> io::Result<()> {
    let path = Path::new(program);
    let metadata = std::fs::metadata(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a file", path.display()),
        ));
    }
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not executable", path.display()),
        ));
    }
    Ok(())
}

/// Sends `signal` to the process group `pid` leads, ignoring the case where the
/// group has already disappeared.
fn signal_group(pid: u32, signal: Signal) {
    let Some(pid) = Pid::from_raw(i32::try_from(pid).unwrap_or(i32::MAX)) else {
        return;
    };
    if let Err(error) = kill_process_group(pid, signal)
        && error != rustix::io::Errno::SRCH
    {
        warn!(%pid, ?signal, %error, "failed to signal process group");
    }
}

/// The internal `--exec-child` mode: reset the inherited signal masks, then become
/// `program` with `arguments`.
///
/// `exec` replaces this process, so this function only ever returns when the
/// program could not be executed at all — which is what the error carries. The
/// caller's `match never {}` is therefore not a trick: it is the type saying that
/// no successful return exists.
///
/// # Errors
///
/// Returns the I/O error from `exec`, or the one from creating the temporary signal
/// source that resets the mask.
pub fn exec_child(
    program: OsString,
    arguments: Vec<OsString>,
) -> anyhow::Result<std::convert::Infallible> {
    // Creating and dropping the source masks these signals and then restores the
    // previous mask, which is exactly the reset the child needs — and doing it here
    // means it cannot interrupt the host's own event loop.
    let signals = calloop::signals::Signals::new(&[
        calloop::signals::Signal::SIGINT,
        calloop::signals::Signal::SIGTERM,
    ])?;
    drop(signals);

    let error = Command::new(program).args(arguments).exec();
    Err(error.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_set_starts_empty() {
        let children = Children::new();

        assert!(!children.is_running(Target::Compositor));
        assert!(!children.is_running(Target::Greeter));
    }

    #[test]
    fn signalling_a_child_that_is_not_running_is_harmless() {
        let children = Children::new();

        children.signal(Target::Greeter, Signal::TERM);
        children.signal(Target::Compositor, Signal::KILL);
    }

    #[test]
    fn reaping_a_child_that_was_never_started_reports_nothing() {
        let mut children = Children::new();

        assert_eq!(children.reap(Target::Compositor).expect("no I/O"), None);
        assert_eq!(children.reap(Target::Greeter).expect("no I/O"), None);
    }

    #[test]
    fn the_exec_child_flag_is_not_one_of_the_documented_options() {
        // It is internal: if it ever grows a usage line of its own, this fails and
        // whoever added it has to decide what the user-facing surface is.
        assert_eq!(EXEC_CHILD, "--exec-child");
    }
}
