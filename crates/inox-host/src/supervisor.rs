//! The host's orchestration rules, as a pure state machine.
//!
//! ADR 0037 §决策 3 puts the login chain in one small process: the compositor and
//! the login screen are the host's children, and everything the host decides —
//! when to start them, when to restart the login screen, when a failure ends the
//! whole stage — is a rule about *events* and *time*, not about the kernel. This
//! module holds exactly those rules.
//!
//! It performs no I/O and reads no clock. Every transition takes the current
//! [`Instant`] and returns the [`Action`]s the caller has to carry out:
//!
//! ```text
//!         boundary(true)              socket appears
//! Preflight ─────────► CompositorSpawning ► WaitingForSocket ─────────► Supervising
//!     │ boundary(false)                          │ deadline                    │
//!     ▼                                         ▼                             │ greeter exits
//!   Done(BoundaryUnavailable)          Stopping(SocketTimeout)                │ (budget left)
//!                                                                             ▼
//!                                       Stopping(‹reason›) ◄──────────── Supervising
//!                                             │ both children gone
//!                                             ▼
//!                                     Done(‹reason›) ── Finish ──► the loop ends
//! ```
//!
//! Four decisions are worth reading before changing anything here.
//!
//! - **The boundary is checked before anything is started.** A host that spawned a
//!   compositor and only then discovered there was nobody to check a password would
//!   be a login screen that lies; [`Supervisor::boundary`] is the only transition out
//!   of `Preflight`, and it either starts the compositor or ends the host.
//! - **The compositor is never restarted.** Its exit is the end of this stage
//!   (ADR 0037 §决策 3.4): the login screen's clients have lost their display, and
//!   systemd restarts the whole unit. Only the login screen has a restart budget.
//! - **Only the login screen has a restart budget.** Three restarts inside sixty
//!   seconds with a bounded backoff (ADR 0015), after which the host exits and lets
//!   systemd record one failed start instead of looping forever.
//! - **Shutdown is ordered, not simultaneous.** The login screen is stopped first and
//!   the compositor only once it is gone (or once its grace period has expired), so
//!   the login screen gets to close its window while the display it draws on still
//!   exists — the same order [`inox-session`] uses for its children.
//!
//! [`inox-session`]: https://github.com/Inox-desktop/inox/tree/main/crates/inox-session

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::process::CommandSpec;

/// How long the compositor has to create its Wayland socket (ADR 0037 §决策 3.2).
pub const SOCKET_TIMEOUT: Duration = Duration::from_secs(10);

/// The rolling window the login screen's restart budget is accounted over (ADR 0015).
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);

/// How many times the login screen may be restarted inside [`RESTART_WINDOW`].
pub const RESTART_ATTEMPTS: usize = 3;

/// How long the login screen has to leave after `SIGTERM` before it is killed.
pub const GREETER_GRACE: Duration = Duration::from_secs(4);

/// How long the compositor is left alone while the login screen is still leaving.
pub const COMPOSITOR_GRACE: Duration = Duration::from_secs(5);

/// How long the compositor has to leave after `SIGTERM` before it is killed.
pub const COMPOSITOR_DEADLINE: Duration = Duration::from_secs(8);

/// The delay before the first restart, doubled for each restart already spent.
const BACKOFF_STEP: Duration = Duration::from_millis(250);

/// One of the two processes the host owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Target {
    /// The session compositor the login screen draws on.
    Compositor,
    /// The login screen (`inox-greet`).
    Greeter,
}

impl Target {
    /// The component this target names, for a log line or a message.
    pub fn component(self) -> &'static str {
        match self {
            Target::Compositor => "compositor",
            Target::Greeter => "greeter",
        }
    }
}

/// Something the host has to do to its children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Start `target` (or start it again).
    Start(Target),
    /// `SIGTERM` to `target`'s process group.
    Stop(Target),
    /// `SIGKILL` to `target`'s process group: the grace period expired.
    Kill(Target),
    /// The host is finished; release the process with this reason.
    Finish(Exit),
}

/// Why the host stopped, and with which exit code.
///
/// The codes are distinct so that an operator can classify a failed start from
/// `systemctl status inox-greeter@tty1` without reading the journal; the only
/// successful one is [`Exit::Requested`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// `SIGTERM`/`SIGINT`: the children were asked to stop and the host exits cleanly.
    Requested,
    /// The authentication boundary could not be reached, so nothing was started.
    BoundaryUnavailable,
    /// The compositor exited; the stage is over.
    CompositorExited,
    /// The compositor did not create its Wayland socket within [`SOCKET_TIMEOUT`].
    SocketTimeout,
    /// A child could not be started at all.
    SpawnFailed(Target),
    /// The login screen used up its restart budget.
    RestartBudgetExhausted,
}

impl Exit {
    /// Whether this reason means the host did what it was asked to do.
    pub fn is_success(self) -> bool {
        matches!(self, Exit::Requested)
    }

    /// The process exit code that goes with this reason.
    pub fn code(self) -> u8 {
        match self {
            Exit::Requested => 0,
            Exit::BoundaryUnavailable => 3,
            Exit::CompositorExited => 4,
            Exit::SocketTimeout => 5,
            Exit::SpawnFailed(_) => 6,
            Exit::RestartBudgetExhausted => 7,
        }
    }
}

/// What the adapter saw in `$XDG_RUNTIME_DIR` when it looked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observed {
    /// Whether the Wayland socket file the compositor was asked for exists.
    pub wayland_socket: bool,
}

/// Everything the host needs to know before it starts anything.
///
/// A plain value: it carries the commands and the two socket paths, and no state.
#[derive(Debug, Clone)]
pub struct Plan {
    /// The Wayland socket name, passed to the compositor and to the login screen
    /// as `WAYLAND_DISPLAY` (ADR 0037 §决策 3.2).
    pub wayland_display: String,
    /// The socket file that has to appear before the login screen is started.
    pub wayland_socket: PathBuf,
    /// The authentication boundary the login screen is pointed at, and the very
    /// socket this host checks before it starts a compositor.
    pub auth_socket: PathBuf,
    /// The compositor, already carrying `--socket <name>` and any extra arguments.
    pub compositor: CommandSpec,
    /// The login screen.
    pub greeter: CommandSpec,
}

/// How the login screen's restarts are accounted for.
///
/// The rule is ADR 0015's: at most [`RESTART_ATTEMPTS`] restarts inside a rolling
/// [`RESTART_WINDOW`], each one delayed by a backoff that doubles. A reservation
/// counts even if the replacement then fails to start, so a login screen that
/// cannot be executed at all exhausts the budget just like one that crashes.
#[derive(Debug, Default)]
struct RestartBudget {
    attempts: VecDeque<Instant>,
}

impl RestartBudget {
    /// Reserves one restart and returns its backoff, or refuses when exhausted.
    fn reserve(&mut self, now: Instant) -> Option<Duration> {
        while self
            .attempts
            .front()
            .is_some_and(|previous| now.saturating_duration_since(*previous) >= RESTART_WINDOW)
        {
            self.attempts.pop_front();
        }
        if self.attempts.len() >= RESTART_ATTEMPTS {
            return None;
        }
        let exponent = u32::try_from(self.attempts.len()).unwrap_or(u32::MAX);
        self.attempts.push_back(now);
        Some(BACKOFF_STEP.saturating_mul(1u32 << exponent.min(8)))
    }
}

/// The stage the host is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Nothing has been started, and the boundary has not answered yet.
    Preflight,
    /// The compositor was asked for; its spawn has not been reported yet.
    CompositorSpawning,
    /// The compositor is running; its Wayland socket is awaited until `deadline`.
    WaitingForSocket { deadline: Instant },
    /// The compositor's socket exists; the login screen is supervised.
    Supervising,
    /// The children are being stopped, in reverse order.
    Stopping(Stopping),
    /// The host is done, with this reason.
    Done(Exit),
}

/// The bookkeeping of one ordered shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stopping {
    since: Instant,
    exit: Exit,
    greeter_signalled: bool,
    greeter_killed: bool,
    compositor_signalled: bool,
    compositor_killed: bool,
}

impl Stopping {
    fn new(since: Instant, exit: Exit) -> Self {
        Self {
            since,
            exit,
            greeter_signalled: false,
            greeter_killed: false,
            compositor_signalled: false,
            compositor_killed: false,
        }
    }
}

/// The host's rules, and the little state they need.
#[derive(Debug)]
pub struct Supervisor {
    plan: Plan,
    budget: RestartBudget,
    phase: Phase,
    /// When the login screen is due to be started again, while it is down.
    restart_at: Option<Instant>,
    compositor_alive: bool,
    greeter_alive: bool,
}

impl Supervisor {
    /// A supervisor that has started nothing and will start nothing until
    /// [`Supervisor::boundary`] says a boundary answered.
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            budget: RestartBudget::default(),
            phase: Phase::Preflight,
            restart_at: None,
            compositor_alive: false,
            greeter_alive: false,
        }
    }

    /// The plan this host executes.
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    /// The reason the host is finished, once it is.
    pub fn exit(&self) -> Option<Exit> {
        match self.phase {
            Phase::Done(exit) => Some(exit),
            _ => None,
        }
    }

    /// Whether the host has nothing left to do.
    pub fn is_finished(&self) -> bool {
        self.exit().is_some()
    }

    /// Answers the one question asked before a single process exists.
    ///
    /// `reachable == false` finishes the host without starting anything: ADR 0037
    /// §决策 3.1 puts the refusal *before* the compositor, so the screen never shows
    /// a login interface that cannot check a password.
    pub fn boundary(&mut self, reachable: bool) -> Vec<Action> {
        if !matches!(self.phase, Phase::Preflight) {
            return Vec::new();
        }
        if !reachable {
            return self.finish(Exit::BoundaryUnavailable);
        }
        self.phase = Phase::CompositorSpawning;
        vec![Action::Start(Target::Compositor)]
    }

    /// Records that `target` is running.
    pub fn spawned(&mut self, now: Instant, target: Target) -> Vec<Action> {
        match target {
            Target::Compositor => {
                self.compositor_alive = true;
                if matches!(self.phase, Phase::CompositorSpawning) {
                    self.phase = Phase::WaitingForSocket {
                        deadline: now + SOCKET_TIMEOUT,
                    };
                }
                Vec::new()
            }
            Target::Greeter => {
                self.greeter_alive = true;
                Vec::new()
            }
        }
    }

    /// Records that `target` could not be started.
    pub fn spawn_failed(&mut self, now: Instant, target: Target) -> Vec<Action> {
        match target {
            // A compositor that cannot be executed leaves nothing running and
            // nothing to wait for: the stage fails immediately.
            Target::Compositor => self.finish(Exit::SpawnFailed(Target::Compositor)),
            Target::Greeter => self.greeter_gone(now),
        }
    }

    /// Records that `target` exited.
    ///
    /// A login-screen exit is a crash for this purpose whether or not its status was
    /// successful: ADR 0037 §决策 4 makes the login screen stay resident after a
    /// session starts, so any exit means the screen is gone and has to come back.
    pub fn exited(&mut self, now: Instant, target: Target) -> Vec<Action> {
        match target {
            Target::Compositor => {
                self.compositor_alive = false;
                match self.phase {
                    Phase::Done(_) => Vec::new(),
                    // A shutdown in progress finishes as soon as the last child is
                    // gone; the grace-period bookkeeping has nothing left to do.
                    Phase::Stopping(stopping) => {
                        if self.greeter_alive {
                            Vec::new()
                        } else {
                            self.finish(stopping.exit)
                        }
                    }
                    _ => self.begin_stopping(now, Exit::CompositorExited),
                }
            }
            Target::Greeter => self.greeter_gone(now),
        }
    }

    /// Advances the timers that are running with nothing else to report.
    pub fn tick(&mut self, now: Instant, observed: Observed) -> Vec<Action> {
        match self.phase {
            Phase::WaitingForSocket { deadline } => {
                if observed.wayland_socket {
                    // The compositor is up and its socket exists: the login screen
                    // now has a display to connect to (ADR 0037 §决策 3.3).
                    self.phase = Phase::Supervising;
                    self.restart_at = None;
                    return vec![Action::Start(Target::Greeter)];
                }
                if now >= deadline {
                    return self.begin_stopping(now, Exit::SocketTimeout);
                }
                Vec::new()
            }
            Phase::Supervising => match self.restart_at {
                Some(at) if now >= at => {
                    self.restart_at = None;
                    vec![Action::Start(Target::Greeter)]
                }
                _ => Vec::new(),
            },
            Phase::Stopping(_) => self.advance_shutdown(now),
            _ => Vec::new(),
        }
    }

    /// Starts an ordered shutdown, whatever the host was doing.
    pub fn shutdown(&mut self, now: Instant) -> Vec<Action> {
        match self.phase {
            Phase::Done(_) | Phase::Stopping(_) => Vec::new(),
            _ => self.begin_stopping(now, Exit::Requested),
        }
    }

    /// The login screen is gone: restart it within the budget, or stop the host.
    fn greeter_gone(&mut self, now: Instant) -> Vec<Action> {
        self.greeter_alive = false;
        match self.phase {
            Phase::Stopping(stopping) => {
                if !self.compositor_alive {
                    return self.finish(stopping.exit);
                }
                // The login screen has left, so the display can go now instead of
                // after `COMPOSITOR_GRACE`; `advance_shutdown` would get here a tick
                // later, and the prompt version keeps a stop from waiting on a timer.
                if stopping.compositor_signalled {
                    return Vec::new();
                }
                self.set_stopping(Stopping {
                    compositor_signalled: true,
                    ..stopping
                });
                vec![Action::Stop(Target::Compositor)]
            }
            Phase::Supervising => match self.budget.reserve(now) {
                Some(backoff) => {
                    self.restart_at = Some(now + backoff);
                    Vec::new()
                }
                None => self.begin_stopping(now, Exit::RestartBudgetExhausted),
            },
            _ => Vec::new(),
        }
    }

    /// Begins stopping both children, in reverse order, without emitting
    /// anything twice.
    fn begin_stopping(&mut self, now: Instant, exit: Exit) -> Vec<Action> {
        if !self.compositor_alive && !self.greeter_alive {
            return self.finish(exit);
        }
        let mut stopping = Stopping::new(now, exit);
        let mut actions = Vec::new();
        if self.greeter_alive {
            stopping.greeter_signalled = true;
            actions.push(Action::Stop(Target::Greeter));
        }
        if self.compositor_alive && !self.greeter_alive {
            // Nothing is waiting on the display any more, so the compositor can go
            // at once instead of after `COMPOSITOR_GRACE`.
            stopping.compositor_signalled = true;
            actions.push(Action::Stop(Target::Compositor));
        }
        self.restart_at = None;
        self.phase = Phase::Stopping(stopping);
        actions
    }

    /// Moves a shutdown along: escalate the grace periods, and finish when both
    /// children are gone.
    fn advance_shutdown(&mut self, now: Instant) -> Vec<Action> {
        let Phase::Stopping(mut stopping) = self.phase else {
            return Vec::new();
        };
        let elapsed = now.saturating_duration_since(stopping.since);
        let mut actions = Vec::new();

        if self.greeter_alive && !stopping.greeter_signalled {
            stopping.greeter_signalled = true;
            actions.push(Action::Stop(Target::Greeter));
        }
        if self.greeter_alive && !stopping.greeter_killed && elapsed >= GREETER_GRACE {
            stopping.greeter_killed = true;
            actions.push(Action::Kill(Target::Greeter));
        }
        if self.compositor_alive
            && !stopping.compositor_signalled
            && (!self.greeter_alive || elapsed >= COMPOSITOR_GRACE)
        {
            stopping.compositor_signalled = true;
            actions.push(Action::Stop(Target::Compositor));
        }
        if self.compositor_alive && !stopping.compositor_killed && elapsed >= COMPOSITOR_DEADLINE {
            stopping.compositor_killed = true;
            actions.push(Action::Kill(Target::Compositor));
        }

        if self.greeter_alive || self.compositor_alive {
            self.phase = Phase::Stopping(stopping);
            return actions;
        }
        let mut finished = self.finish(stopping.exit);
        actions.append(&mut finished);
        actions
    }

    /// Writes a shutdown's progress back into the phase.
    fn set_stopping(&mut self, stopping: Stopping) {
        self.phase = Phase::Stopping(stopping);
    }

    /// Ends the host with `exit`; the returned action is what releases the process.
    fn finish(&mut self, exit: Exit) -> Vec<Action> {
        self.phase = Phase::Done(exit);
        self.restart_at = None;
        vec![Action::Finish(exit)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// A plan that is never executed: the tests only read the rules.
    fn plan() -> Plan {
        Plan {
            wayland_display: "wayland-inox-greet".to_owned(),
            wayland_socket: PathBuf::from("/run/user/1000/wayland-inox-greet"),
            auth_socket: PathBuf::from("/run/inox/auth.sock"),
            compositor: CommandSpec {
                program: OsString::from("/usr/bin/inox-compositor"),
                arguments: vec![
                    OsString::from("--socket"),
                    OsString::from("wayland-inox-greet"),
                ],
            },
            greeter: CommandSpec::new("/usr/bin/inox-greet"),
        }
    }

    /// A supervisor whose compositor is up and whose socket has appeared, with the
    /// login screen running: the state every restart rule applies in.
    fn supervising(now: Instant) -> Supervisor {
        let mut supervisor = Supervisor::new(plan());
        assert_eq!(
            supervisor.boundary(true),
            vec![Action::Start(Target::Compositor)]
        );
        assert!(supervisor.spawned(now, Target::Compositor).is_empty());
        assert_eq!(
            supervisor.tick(
                now,
                Observed {
                    wayland_socket: true
                }
            ),
            vec![Action::Start(Target::Greeter)]
        );
        assert!(supervisor.spawned(now, Target::Greeter).is_empty());
        supervisor
    }

    #[test]
    fn an_unreachable_boundary_finishes_the_host_before_anything_starts() {
        let mut supervisor = Supervisor::new(plan());

        let actions = supervisor.boundary(false);

        assert_eq!(
            actions,
            vec![Action::Finish(Exit::BoundaryUnavailable)],
            "a host without a boundary must not start a compositor"
        );
        assert!(supervisor.is_finished());
        assert_eq!(supervisor.exit(), Some(Exit::BoundaryUnavailable));
        assert!(!Exit::BoundaryUnavailable.is_success());
        assert_ne!(Exit::BoundaryUnavailable.code(), 0);
    }

    #[test]
    fn a_reachable_boundary_starts_the_compositor_and_nothing_else() {
        let now = Instant::now();
        let mut supervisor = Supervisor::new(plan());

        assert_eq!(
            supervisor.boundary(true),
            vec![Action::Start(Target::Compositor)]
        );
        assert!(!supervisor.is_finished());
        assert_eq!(supervisor.exit(), None);

        // A socket that appears before the spawn is reported changes nothing: the
        // login screen waits for a compositor that this host has not seen start.
        assert!(
            supervisor
                .tick(
                    now,
                    Observed {
                        wayland_socket: true
                    }
                )
                .is_empty()
        );
    }

    #[test]
    fn the_login_screen_starts_only_once_the_wayland_socket_exists() {
        let now = Instant::now();
        let mut supervisor = Supervisor::new(plan());
        let _ = supervisor.boundary(true);
        let _ = supervisor.spawned(now, Target::Compositor);

        assert!(
            supervisor
                .tick(
                    now,
                    Observed {
                        wayland_socket: false
                    }
                )
                .is_empty()
        );
        assert_eq!(
            supervisor.tick(
                now,
                Observed {
                    wayland_socket: true
                }
            ),
            vec![Action::Start(Target::Greeter)]
        );
    }

    #[test]
    fn a_compositor_that_never_creates_its_socket_is_stopped() {
        let now = Instant::now();
        let mut supervisor = Supervisor::new(plan());
        let _ = supervisor.boundary(true);
        let _ = supervisor.spawned(now, Target::Compositor);

        // One tick before the deadline is silent.
        assert!(
            supervisor
                .tick(
                    now + SOCKET_TIMEOUT - Duration::from_millis(1),
                    Observed {
                        wayland_socket: false
                    }
                )
                .is_empty()
        );
        assert_eq!(
            supervisor.tick(now + SOCKET_TIMEOUT, Observed::default()),
            vec![Action::Stop(Target::Compositor)]
        );
        // Nothing is waiting for the display, so there is no grace period to serve:
        // the compositor is killed at the shutdown deadline.
        assert!(
            supervisor
                .tick(
                    now + SOCKET_TIMEOUT + COMPOSITOR_DEADLINE - Duration::from_millis(1),
                    Observed::default()
                )
                .is_empty()
        );
        assert_eq!(
            supervisor.tick(
                now + SOCKET_TIMEOUT + COMPOSITOR_DEADLINE,
                Observed::default()
            ),
            vec![Action::Kill(Target::Compositor)]
        );
        let actions = supervisor.exited(
            now + SOCKET_TIMEOUT + COMPOSITOR_DEADLINE,
            Target::Compositor,
        );
        assert_eq!(actions, vec![Action::Finish(Exit::SocketTimeout)]);
        assert_eq!(supervisor.exit(), Some(Exit::SocketTimeout));
    }

    #[test]
    fn a_compositor_that_exits_early_ends_the_host() {
        let now = Instant::now();
        let mut supervisor = Supervisor::new(plan());
        let _ = supervisor.boundary(true);
        let _ = supervisor.spawned(now, Target::Compositor);

        // The compositor left while its socket was still being awaited; there is no
        // login screen to wait for, so the host finishes at once.
        assert_eq!(
            supervisor.exited(now, Target::Compositor),
            vec![Action::Finish(Exit::CompositorExited)]
        );
        assert_eq!(supervisor.exit(), Some(Exit::CompositorExited));
    }

    #[test]
    fn a_compositor_exit_while_supervising_stops_the_login_screen_first() {
        let now = Instant::now();
        let mut supervisor = supervising(now);

        assert_eq!(
            supervisor.exited(now, Target::Compositor),
            vec![Action::Stop(Target::Greeter)],
            "the login screen is stopped before the host finishes"
        );
        // The compositor is already gone, so nothing is signalled for it; once the
        // login screen leaves, the host finishes.
        assert_eq!(
            supervisor.exited(now + Duration::from_millis(10), Target::Greeter),
            vec![Action::Finish(Exit::CompositorExited)]
        );
        assert_eq!(supervisor.exit(), Some(Exit::CompositorExited));
    }

    #[test]
    fn the_login_screen_is_restarted_within_the_budget() {
        let now = Instant::now();
        let mut supervisor = supervising(now);

        assert!(supervisor.exited(now, Target::Greeter).is_empty());
        // The backoff is served, not skipped: nothing starts before it expires.
        assert!(
            supervisor
                .tick(now + Duration::from_millis(249), Observed::default())
                .is_empty()
        );
        assert_eq!(
            supervisor.tick(now + Duration::from_millis(250), Observed::default()),
            vec![Action::Start(Target::Greeter)]
        );
        assert!(!supervisor.is_finished());
    }

    #[test]
    fn the_backoff_doubles_and_the_budget_is_bounded() {
        let now = Instant::now();
        let mut budget = RestartBudget::default();

        assert_eq!(budget.reserve(now), Some(Duration::from_millis(250)));
        assert_eq!(budget.reserve(now), Some(Duration::from_millis(500)));
        assert_eq!(budget.reserve(now), Some(Duration::from_secs(1)));
        assert_eq!(budget.reserve(now), None, "a fourth restart is refused");
        // The window is rolling: the oldest reservation expires on its own.
        assert_eq!(
            budget.reserve(now + RESTART_WINDOW),
            Some(Duration::from_millis(250))
        );
    }

    #[test]
    fn a_login_screen_that_keeps_crashing_ends_the_host() {
        let now = Instant::now();
        let mut supervisor = supervising(now);

        for restart in 0..RESTART_ATTEMPTS {
            assert!(supervisor.exited(now, Target::Greeter).is_empty());
            let backoff = BACKOFF_STEP * (1 << restart);
            assert_eq!(
                supervisor.tick(now + backoff, Observed::default()),
                vec![Action::Start(Target::Greeter)]
            );
            let _ = supervisor.spawned(now + backoff, Target::Greeter);
        }

        // The fourth exit has no budget left: the compositor is stopped and the host
        // will exit once it is gone.
        assert_eq!(
            supervisor.exited(now, Target::Greeter),
            vec![Action::Stop(Target::Compositor)]
        );
        assert_eq!(
            supervisor.exited(now, Target::Compositor),
            vec![Action::Finish(Exit::RestartBudgetExhausted)]
        );
        assert_eq!(supervisor.exit(), Some(Exit::RestartBudgetExhausted));
        assert_ne!(Exit::RestartBudgetExhausted.code(), 0);
    }

    #[test]
    fn a_login_screen_that_cannot_be_started_spends_the_budget_too() {
        let now = Instant::now();
        let mut supervisor = supervising(now);

        for _ in 0..RESTART_ATTEMPTS {
            assert!(supervisor.spawn_failed(now, Target::Greeter).is_empty());
            let _ = supervisor.tick(now + Duration::from_secs(1), Observed::default());
        }

        assert_eq!(
            supervisor.spawn_failed(now, Target::Greeter),
            vec![Action::Stop(Target::Compositor)]
        );
        assert_eq!(
            supervisor.exited(now, Target::Compositor),
            vec![Action::Finish(Exit::RestartBudgetExhausted)]
        );
    }

    #[test]
    fn a_compositor_that_cannot_be_started_ends_the_host_without_a_restart() {
        let now = Instant::now();
        let mut supervisor = Supervisor::new(plan());
        let _ = supervisor.boundary(true);

        assert_eq!(
            supervisor.spawn_failed(now, Target::Compositor),
            vec![Action::Finish(Exit::SpawnFailed(Target::Compositor))]
        );
        assert_eq!(
            supervisor.exit(),
            Some(Exit::SpawnFailed(Target::Compositor))
        );
        assert_ne!(Exit::SpawnFailed(Target::Compositor).code(), 0);
    }

    #[test]
    fn a_requested_shutdown_stops_the_login_screen_then_the_compositor() {
        let now = Instant::now();
        let mut supervisor = supervising(now);

        assert_eq!(
            supervisor.shutdown(now),
            vec![Action::Stop(Target::Greeter)]
        );
        assert_eq!(
            supervisor.exited(now + Duration::from_millis(20), Target::Greeter),
            vec![Action::Stop(Target::Compositor)]
        );
        assert_eq!(
            supervisor.exited(now + Duration::from_millis(40), Target::Compositor),
            vec![Action::Finish(Exit::Requested)]
        );
        assert_eq!(supervisor.exit(), Some(Exit::Requested));
        assert!(Exit::Requested.is_success());
        assert_eq!(Exit::Requested.code(), 0);
    }

    #[test]
    fn a_shutdown_escalates_to_sigkill_when_the_grace_period_expires() {
        let now = Instant::now();
        let mut supervisor = supervising(now);
        let _ = supervisor.shutdown(now);

        assert!(
            supervisor
                .tick(
                    now + GREETER_GRACE - Duration::from_millis(1),
                    Observed::default()
                )
                .is_empty()
        );
        assert_eq!(
            supervisor.tick(now + GREETER_GRACE, Observed::default()),
            vec![Action::Kill(Target::Greeter)]
        );
        // A login screen that ignores `SIGTERM` must not hold the compositor forever:
        // after `COMPOSITOR_GRACE` the display is stopped anyway.
        assert!(
            supervisor
                .tick(now + COMPOSITOR_GRACE, Observed::default())
                .contains(&Action::Stop(Target::Compositor))
        );
        assert_eq!(
            supervisor.tick(now + COMPOSITOR_DEADLINE, Observed::default()),
            vec![Action::Kill(Target::Compositor)]
        );
        // Both children really left; the host finishes with the shutdown it was asked
        // for, not with a failure.
        assert!(
            supervisor
                .exited(now + COMPOSITOR_DEADLINE, Target::Greeter)
                .is_empty()
        );
        assert_eq!(
            supervisor.exited(now + COMPOSITOR_DEADLINE, Target::Compositor),
            vec![Action::Finish(Exit::Requested)]
        );
    }

    #[test]
    fn a_shutdown_is_only_asked_for_once() {
        let now = Instant::now();
        let mut supervisor = supervising(now);

        let first = supervisor.shutdown(now);
        assert_eq!(first, vec![Action::Stop(Target::Greeter)]);
        assert!(
            supervisor
                .shutdown(now + Duration::from_millis(10))
                .is_empty(),
            "a second signal must not signal the children again"
        );
        assert!(supervisor.shutdown(now + Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn a_shutdown_before_anything_started_finishes_at_once() {
        let now = Instant::now();
        let mut supervisor = Supervisor::new(plan());

        assert_eq!(
            supervisor.shutdown(now),
            vec![Action::Finish(Exit::Requested)]
        );
        assert_eq!(supervisor.exit(), Some(Exit::Requested));
    }

    #[test]
    fn events_after_the_end_change_nothing() {
        let now = Instant::now();
        let mut supervisor = Supervisor::new(plan());
        let _ = supervisor.boundary(false);

        for actions in [
            supervisor.boundary(true),
            supervisor.spawned(now, Target::Compositor),
            supervisor.spawn_failed(now, Target::Greeter),
            supervisor.exited(now, Target::Compositor),
            supervisor.tick(
                now,
                Observed {
                    wayland_socket: true,
                },
            ),
            supervisor.shutdown(now),
        ] {
            assert!(actions.is_empty());
        }
        assert_eq!(supervisor.exit(), Some(Exit::BoundaryUnavailable));
    }

    #[test]
    fn every_failure_code_is_distinct_and_nonzero() {
        let failures = [
            Exit::BoundaryUnavailable,
            Exit::CompositorExited,
            Exit::SocketTimeout,
            Exit::SpawnFailed(Target::Compositor),
            Exit::SpawnFailed(Target::Greeter),
            Exit::RestartBudgetExhausted,
        ];
        let mut codes: Vec<u8> = failures.iter().map(|exit| exit.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 5, "the two spawn failures share one code");
        assert!(codes.iter().all(|code| *code != 0));

        for exit in failures {
            assert!(!exit.is_success());
        }
    }

    #[test]
    fn the_targets_name_themselves_like_the_components_they_are() {
        assert_eq!(Target::Compositor.component(), "compositor");
        assert_eq!(Target::Greeter.component(), "greeter");
    }
}
