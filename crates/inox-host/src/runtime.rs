//! The event loop that drives the supervisor and carries out its actions.
//!
//! This is the adapter half of the crate: [`crate::supervisor`] decides, this module
//! observes and executes. It owns the calloop loop, the two event sources (a 50 ms
//! supervision tick and a signal source for `SIGTERM`/`SIGINT`), the probe that
//! answers "does the compositor's Wayland socket exist yet", and the one mapping from
//! an [`Exit`] to the sentence that lands in the journal.
//!
//! Two ordering decisions are load-bearing.
//!
//! - **Signals are registered before any child exists.** calloop's signal source is a
//!   `signalfd`, which masks `SIGINT`/`SIGTERM` in this thread; children forked from a
//!   thread inherit its mask. Registering first and launching children through
//!   `--exec-child` (see [`crate::process`]) means the mask is established exactly
//!   once and every child has it reset.
//! - **Each tick reaps before it decides.** A compositor that has already exited must
//!   end the host even if its socket file happens to be lying around; a greeter that
//!   has already exited must not be restarted before the budget is consulted.

use std::collections::VecDeque;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use calloop::EventLoop;
use calloop::signals::{Signal as ReceivedSignal, Signals};
use calloop::timer::{TimeoutAction, Timer};
use inox_common::i18n::{Arg, Localizer};
use rustix::process::Signal;
use tracing::{debug, error, info, warn};

use crate::process::Children;
use crate::supervisor::{
    Action, Exit, Observed, Plan, RESTART_ATTEMPTS, RESTART_WINDOW, SOCKET_TIMEOUT, Supervisor,
    Target,
};

/// How often the host looks at its children and at its own runtime directory.
///
/// Fast enough that a stop request is acted on promptly and slow enough that the
/// stage costs nothing: two `waitpid` calls and one `stat` every fifty milliseconds.
const TICK: Duration = Duration::from_millis(50);

/// Whether an authentication boundary is listening at `socket`.
///
/// ADR 0037 §决策 3.1 makes this the *first* thing the host does, before a compositor
/// or a window exists, because a login screen that could collect a password and only
/// then discover there is nobody to check it would waste the one thing it must not
/// waste. A `connect` — not the existence of the file — is what proves somebody is
/// there: a stale socket left by a crashed boundary refuses the connection.
pub fn boundary_is_reachable(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// Whether the compositor has bound its Wayland socket yet.
///
/// ADR 0037 §决策 3.2 waits for the socket to appear; the host does not connect to it.
/// "Appear" is checked as "the path is a socket", not "the path exists": a leftover
/// regular file — a log someone pointed at the same name, a half-written artefact, a
/// directory — would otherwise declare the stage ready while the compositor can still
/// accept no client at all, and the login screen would be started against nothing.
///
/// A leftover *socket* from an earlier compositor still satisfies this probe, and
/// deliberately so: distinguishing a listening socket from a dead one takes a
/// connection, and the host does not make one. The two failures that leaves are both
/// already covered — the compositor here is a live child whose exit ends the stage, and
/// a login screen that cannot use the socket it was given exits and is restarted within
/// its budget.
fn wayland_socket_is_bound(socket: &Path) -> bool {
    std::fs::symlink_metadata(socket)
        .map(|metadata| metadata.file_type().is_socket())
        .unwrap_or(false)
}

/// Runs the greeter stage until it ends, and returns the reason it ended.
///
/// `launcher` is this executable: children are started through its internal
/// `--exec-child` mode so they do not inherit this thread's signal mask.
///
/// # Errors
///
/// Returns an error when the event loop, or one of its two sources, cannot be
/// created — a broken environment rather than a stage failure. Any child that is
/// already running is killed on the way out by [`Children`]'s `Drop`.
pub fn run(plan: Plan, launcher: PathBuf, localizer: Localizer) -> anyhow::Result<Exit> {
    let mut event_loop: EventLoop<Host> =
        EventLoop::try_new().context("the greeter host event loop could not be created")?;
    let handle = event_loop.handle();

    let signals = Signals::new(&[ReceivedSignal::SIGINT, ReceivedSignal::SIGTERM])
        .context("the greeter host could not listen for SIGTERM and SIGINT")?;
    handle
        .insert_source(signals, |event, _, host: &mut Host| {
            host.received(event.signal());
        })
        .map_err(|error| anyhow::anyhow!("the signal source could not be registered: {error}"))?;

    handle
        .insert_source(Timer::immediate(), |_, _, host: &mut Host| {
            host.step();
            TimeoutAction::ToDuration(TICK)
        })
        .map_err(|error| {
            anyhow::anyhow!("the supervision timer could not be registered: {error}")
        })?;

    let mut host = Host {
        supervisor: Supervisor::new(plan),
        children: Children::new(),
        launcher,
        localizer,
    };

    // Before the loop: the one decision that has to happen before anything runs.
    host.begin();

    while !host.supervisor.is_finished() {
        event_loop
            .dispatch(None, &mut host)
            .context("the greeter host event loop failed")?;
    }

    host.supervisor
        .exit()
        .ok_or_else(|| anyhow::anyhow!("the greeter host stopped without recording why"))
}

/// Everything one host run owns.
struct Host {
    supervisor: Supervisor,
    children: Children,
    launcher: PathBuf,
    localizer: Localizer,
}

impl Host {
    /// Checks the boundary and, if it answers, starts the compositor.
    fn begin(&mut self) {
        let socket = self.supervisor.plan().auth_socket.clone();
        let reachable = boundary_is_reachable(&socket);
        if reachable {
            debug!(socket = %socket.display(), "the authentication boundary answered");
        } else {
            warn!(
                socket = %socket.display(),
                "no authentication boundary answered; the greeter host will not start"
            );
        }
        let actions = self.supervisor.boundary(reachable);
        self.execute(actions);
    }

    /// One supervision tick: reap, then let the state machine advance.
    fn step(&mut self) {
        self.reap();
        if self.supervisor.is_finished() {
            return;
        }

        let observed = Observed {
            wayland_socket: wayland_socket_is_bound(&self.supervisor.plan().wayland_socket),
        };
        let actions = self.supervisor.tick(Instant::now(), observed);
        self.execute(actions);
    }

    /// Collects the exit status of every child that has left.
    fn reap(&mut self) {
        // The login screen first: when both left in the same tick, the ordering a
        // shutdown cares about is the one the state machine is told about.
        for target in [Target::Greeter, Target::Compositor] {
            match self.children.reap(target) {
                Ok(Some(status)) => {
                    debug!(
                        component = target.component(),
                        %status,
                        "reporting a child's exit to the supervisor"
                    );
                    let actions = self.supervisor.exited(Instant::now(), target);
                    self.execute(actions);
                }
                Ok(None) => {}
                Err(error) => warn!(
                    component = target.component(),
                    %error,
                    "could not read a child's exit status; the next tick tries again"
                ),
            }
        }
    }

    /// `SIGTERM`/`SIGINT`: stop the children in reverse order and exit cleanly.
    fn received(&mut self, signal: ReceivedSignal) {
        info!(?signal, "the greeter host received a termination signal");
        let actions = self.supervisor.shutdown(Instant::now());
        self.execute(actions);
    }

    /// Carries out `actions`, including the actions they cause in turn.
    ///
    /// A queue rather than recursion: starting a child reports back into the state
    /// machine, which can itself produce another action (a spawn failure becomes a
    /// shutdown, a shutdown becomes a stop).
    fn execute(&mut self, actions: Vec<Action>) {
        let mut queue: VecDeque<Action> = actions.into();
        while let Some(action) = queue.pop_front() {
            match action {
                Action::Start(target) => {
                    let outcome = {
                        let plan = self.supervisor.plan();
                        self.children.spawn(target, plan, &self.launcher)
                    };
                    let now = Instant::now();
                    match outcome {
                        Ok(()) => queue.extend(self.supervisor.spawned(now, target)),
                        Err(error) => {
                            warn!(
                                component = target.component(),
                                %error,
                                "a child process could not be started"
                            );
                            queue.extend(self.supervisor.spawn_failed(now, target));
                        }
                    }
                }
                Action::Stop(target) => self.children.signal(target, Signal::TERM),
                Action::Kill(target) => {
                    warn!(
                        component = target.component(),
                        "the grace period expired; killing the child process group"
                    );
                    self.children.signal(target, Signal::KILL);
                }
                Action::Finish(exit) => self.report(exit),
            }
        }
    }

    /// Says why the stage ended, in the locale the host was started with.
    fn report(&self, exit: Exit) {
        let plan = self.supervisor.plan();
        let message = match exit {
            Exit::Requested => self.localizer.message("host-stopping"),
            Exit::BoundaryUnavailable => self.localizer.message_with(
                "host-boundary-missing",
                &[("socket", path_argument(&plan.auth_socket))],
            ),
            Exit::CompositorExited => self.localizer.message("host-compositor-exited"),
            Exit::SocketTimeout => self.localizer.message_with(
                "host-socket-timeout",
                &[
                    ("socket", Arg::from(plan.wayland_display.clone())),
                    ("seconds", seconds_argument(SOCKET_TIMEOUT)),
                ],
            ),
            Exit::SpawnFailed(Target::Compositor) => self.localizer.message_with(
                "host-compositor-unstartable",
                &[(
                    "program",
                    path_argument(Path::new(&plan.compositor.program)),
                )],
            ),
            Exit::SpawnFailed(Target::Greeter) => self.localizer.message_with(
                "host-greeter-unstartable",
                &[("program", path_argument(Path::new(&plan.greeter.program)))],
            ),
            Exit::RestartBudgetExhausted => self.localizer.message_with(
                "host-restarts-exhausted",
                &[
                    ("count", Arg::from(RESTART_ATTEMPTS)),
                    ("window", seconds_argument(RESTART_WINDOW)),
                ],
            ),
        };

        if exit.is_success() {
            info!("{message}");
        } else {
            error!(code = exit.code(), "{message}");
        }
    }
}

/// A path as a message argument, rendered the way an operator would type it.
fn path_argument(path: &Path) -> Arg {
    Arg::from(path.display().to_string())
}

/// A duration as a whole number of seconds, for a message placeholder.
fn seconds_argument(duration: Duration) -> Arg {
    Arg::from(duration.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn a_socket_nobody_listens_on_is_not_reachable() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("auth.sock");

        assert!(!boundary_is_reachable(&socket));

        // A file that is not a socket must not count either: `connect` fails, which
        // is the point of probing with a connection rather than a `stat`.
        std::fs::write(&socket, b"").expect("the marker file is writable");
        assert!(!boundary_is_reachable(&socket));
    }

    #[test]
    fn a_listening_boundary_is_reachable() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("auth.sock");
        let _listener = UnixListener::bind(&socket).expect("the socket is bindable");

        assert!(
            boundary_is_reachable(&socket),
            "a listening socket has to answer a connect"
        );
    }

    #[test]
    fn a_stale_socket_file_does_not_count_as_a_boundary() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("auth.sock");
        {
            let listener = UnixListener::bind(&socket).expect("the socket is bindable");
            drop(listener);
        }

        assert!(
            socket.exists(),
            "the test needs a socket file that outlived its listener"
        );
        assert!(
            !boundary_is_reachable(&socket),
            "a socket nobody listens on is not a boundary"
        );
    }

    #[test]
    fn a_missing_path_is_not_a_bound_wayland_socket() {
        let directory = tempfile::tempdir().expect("a temporary directory");

        assert!(!wayland_socket_is_bound(
            &directory.path().join("wayland-inox-greet")
        ));
    }

    #[test]
    fn a_leftover_file_or_directory_is_not_a_bound_wayland_socket() {
        let directory = tempfile::tempdir().expect("a temporary directory");

        // A regular file with the socket's name, which is what `exists` would have
        // accepted: the compositor has still bound nothing.
        let file = directory.path().join("regular");
        std::fs::write(&file, b"").expect("the marker file is writable");
        assert!(!wayland_socket_is_bound(&file));

        let folder = directory.path().join("folder");
        std::fs::create_dir(&folder).expect("the directory is creatable");
        assert!(!wayland_socket_is_bound(&folder));
    }

    #[test]
    fn a_bound_socket_is_a_bound_wayland_socket() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-inox-greet");
        let _listener = UnixListener::bind(&socket).expect("the socket is bindable");

        assert!(
            wayland_socket_is_bound(&socket),
            "a listening socket is what the compositor leaves behind"
        );
    }

    #[test]
    fn a_stale_socket_still_counts_and_the_restart_budget_covers_it() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-inox-greet");
        {
            let listener = UnixListener::bind(&socket).expect("the socket is bindable");
            drop(listener);
        }

        // Documented, not desired: telling a live listener from a dead one needs a
        // connection, which this probe deliberately does not make. The login screen
        // fails to use the stale socket, exits, and is restarted within the budget.
        assert!(wayland_socket_is_bound(&socket));
    }
}
