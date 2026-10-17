//! Launching, and verifying that it happened.
//!
//! Acceptance D02 asks for two things that are easy to get wrong:
//!
//! - **exactly one launch.** The user's finger can reach Enter twice before the
//!   result of the first press exists, and a launcher that treats each press as
//!   a fresh wish opens two windows. The fix is an identity: an activation is
//!   keyed by the generation of the *selection* it belongs to, so the second
//!   press is recognisably the same request and is dropped.
//! - **a verified result.** "Started" is not the same claim as "we asked the
//!   process boundary to start it". [`Observation`] is the evidence the
//!   boundary reports, [`Readiness`] is what that evidence amounts to, and a
//!   launch that produced no evidence is reported as a failure with a reason
//!   and a retry rather than left spinning (acceptance D05).
//!
//! Nothing in this module touches Iced, and the state machine does no I/O: the
//! single blocking function is [`spawn_and_probe`], which the caller is
//! expected to run on a thread of its own (`crate::app` does).
//!
//! ## What "ready" can mean here
//!
//! ADR 0013 asks for readiness to be established from the observed evidence —
//! for a launcher, normally a window appearing or a D-Bus name being acquired.
//! Neither is available to this component today: the compositor exports no
//! window-list interface ([ADR 0013] leaves that to `inox-ipc`, which does not
//! carry it yet), and most applications own no D-Bus name. The evidence used
//! instead is the process still being alive after a grace period, which rules
//! out the failure that actually happens in practice — an `Exec` that names a
//! program that starts and immediately dies. That is a weaker claim than
//! ADR 0013's, and it is reported as such rather than dressed up: see the crate
//! README's "not implemented" list.
//!
//! [ADR 0013]: https://github.com/Inox-desktop/inox/blob/main/docs/adr/0013-shell-service-and-interaction-contracts.md

use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::time::Duration;

use tracing::debug;

use crate::contract::{CommandState, Failure};

/// How long a launch waits for evidence that the process is running.
///
/// Long enough for a program that fails on startup to have failed, short enough
/// that a user who launched the wrong entry is not left waiting. It is a
/// behavioural bound, not a motion duration: nothing animates for this long.
pub const READINESS_GRACE: Duration = Duration::from_millis(750);

/// How many activations are remembered.
///
/// The selection generation grows with every keystroke, so the history has to be
/// bounded; only settled attempts are evicted, and an activation is refused
/// outright once the bound is full of attempts that have not settled yet.
pub const TRACKED_ATTEMPTS: usize = 64;

/// What the process boundary observed about a launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// A child process exists.
    Spawned {
        /// Its process id.
        pid: u32,
    },
    /// The process could not be created.
    SpawnFailed {
        /// The underlying error, as text.
        detail: String,
    },
    /// The process was created and exited before the grace period ended.
    ExitedBeforeReady {
        /// The exit status, or the error reading it, as text.
        detail: String,
    },
    /// The process is still running after the grace period.
    Ready {
        /// Its process id.
        pid: u32,
    },
    /// The grace period passed with no evidence at all.
    TimedOut {
        /// The bound that expired, in seconds.
        seconds: u64,
    },
}

/// How far a launch has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Asked for; the process boundary has not answered.
    Pending,
    /// Evidence of a running process.
    Ready,
    /// The bounded wait expired without evidence.
    TimedOut,
    /// The launch was refused or the process died.
    Rejected,
}

/// The result of an activation.
///
/// ADR 0013's sent / applied / rejected distinction is [`LaunchOutcome::state`]'s
/// mapping: what the user is told is never stronger than what was observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOutcome {
    /// The request is on its way; nothing has been observed yet.
    Requested,
    /// Verified: the application is running.
    Started {
        /// Its process id.
        pid: u32,
    },
    /// The launch did not produce a running application.
    Failed {
        /// Why, in a form the UI can localise.
        failure: Failure,
    },
}

impl LaunchOutcome {
    /// Where this outcome sits in the service contract's vocabulary.
    pub fn state(&self) -> CommandState {
        match self {
            Self::Requested => CommandState::Sent,
            Self::Started { .. } => CommandState::Applied,
            Self::Failed { failure } => match failure.kind {
                crate::contract::FailureKind::NotReadyInTime { .. } => CommandState::TimedOut,
                _ => CommandState::Rejected,
            },
        }
    }

    /// Whether the launch is still undecided.
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Requested)
    }
}

/// One attempt at launching one application.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Attempt {
    /// The entry's desktop file id.
    entry: String,
    /// The entry's localized name, for the failure message.
    name: String,
    /// The argv the attempt started.
    argv: Vec<String>,
    /// How far the attempt has got.
    outcome: LaunchOutcome,
}

/// The activation the component is reporting on.
///
/// It carries the entry's *name* rather than only its id, because the sentence
/// the user reads is about the application, not about a desktop file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    /// The selection generation it belongs to.
    pub selection: u64,
    /// The entry's desktop file id.
    pub entry: String,
    /// The entry's localized name.
    pub name: String,
    /// How far it has got.
    pub outcome: LaunchOutcome,
}

/// A request to start a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    /// The selection generation this request is for.
    pub selection: u64,
    /// The argv to start.
    pub argv: Vec<String>,
    /// The entry's desktop file id.
    pub entry: String,
    /// The entry's localized name.
    pub name: String,
}

/// Which selections have been activated, and how they turned out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activator {
    attempts: VecDeque<(u64, Attempt)>,
}

impl Default for Activator {
    fn default() -> Self {
        Self::new()
    }
}

impl Activator {
    /// An activator with nothing activated.
    pub fn new() -> Self {
        Self {
            attempts: VecDeque::new(),
        }
    }

    /// Asks to launch `entry` for the selection generation `selection`.
    ///
    /// Returns `None` — and does nothing — when this selection has already been
    /// activated, whether the attempt is still in flight or has settled. That
    /// is the whole of "at most once": the caller may call this on every Enter
    /// press and only the first one produces a request.
    pub fn request(
        &mut self,
        selection: u64,
        entry: &str,
        name: &str,
        argv: Vec<String>,
    ) -> Option<LaunchRequest> {
        if self.attempts.iter().any(|(key, _)| *key == selection) {
            debug!(selection, entry, "activation already requested");
            return None;
        }

        if self.attempts.len() >= TRACKED_ATTEMPTS && self.evict_settled(selection).is_none() {
            debug!(selection, entry, "no room to track another activation");
            return None;
        }

        self.attempts.push_back((
            selection,
            Attempt {
                entry: entry.to_owned(),
                name: name.to_owned(),
                argv: argv.clone(),
                outcome: LaunchOutcome::Requested,
            },
        ));

        Some(LaunchRequest {
            selection,
            argv,
            entry: entry.to_owned(),
            name: name.to_owned(),
        })
    }

    /// Retries a selection whose attempt failed.
    ///
    /// A retry is a new attempt, not a second press of Enter: it is refused
    /// while the current attempt is undecided, and refused when it succeeded,
    /// so the "at most once" rule is about the *outcome* rather than about the
    /// key. The caller decides which selection generation the retry belongs to
    /// and asks with that one.
    pub fn retry(&mut self, selection: u64, failed: u64) -> Option<LaunchRequest> {
        let (_, attempt) = self.attempts.iter().find(|(key, _)| *key == failed)?;
        if attempt.outcome.is_pending() {
            return None;
        }
        if matches!(attempt.outcome, LaunchOutcome::Started { .. }) {
            return None;
        }
        let entry = attempt.entry.clone();
        let name = attempt.name.clone();
        let argv = attempt.argv.clone();
        self.request(selection, &entry, &name, argv)
    }

    /// Reports evidence about an attempt.
    ///
    /// Returns the outcome after the observation, or `None` when the selection
    /// is not being tracked. An observation for an already settled attempt does
    /// not change it: the first verified result wins.
    pub fn observe(&mut self, selection: u64, observation: Observation) -> Option<LaunchOutcome> {
        let (_, attempt) = self
            .attempts
            .iter_mut()
            .find(|(key, _)| *key == selection)?;

        if !attempt.outcome.is_pending() {
            return Some(attempt.outcome.clone());
        }

        // `Spawned` is progress, not a result: the process existing is what
        // makes the wait meaningful, and the outcome stays "requested" until
        // there is evidence either way.
        attempt.outcome = match observation {
            Observation::Spawned { .. } => LaunchOutcome::Requested,
            Observation::Ready { pid } => LaunchOutcome::Started { pid },
            Observation::SpawnFailed { detail } => LaunchOutcome::Failed {
                failure: Failure::for_entry(
                    crate::contract::FailureKind::SpawnFailed { detail },
                    &attempt.name,
                ),
            },
            Observation::ExitedBeforeReady { detail } => LaunchOutcome::Failed {
                failure: Failure::for_entry(
                    crate::contract::FailureKind::ExitedBeforeReady { detail },
                    &attempt.name,
                ),
            },
            Observation::TimedOut { seconds } => LaunchOutcome::Failed {
                failure: Failure::for_entry(
                    crate::contract::FailureKind::NotReadyInTime { seconds },
                    &attempt.name,
                ),
            },
        };

        Some(attempt.outcome.clone())
    }

    /// The outcome of a selection, if it is being tracked.
    pub fn outcome(&self, selection: u64) -> Option<&LaunchOutcome> {
        self.attempts
            .iter()
            .find(|(key, _)| *key == selection)
            .map(|(_, attempt)| &attempt.outcome)
    }

    /// How far a selection has got.
    pub fn readiness(&self, selection: u64) -> Readiness {
        match self.outcome(selection) {
            None => Readiness::Pending,
            Some(LaunchOutcome::Requested) => Readiness::Pending,
            Some(LaunchOutcome::Started { .. }) => Readiness::Ready,
            Some(LaunchOutcome::Failed { failure }) => match failure.kind {
                crate::contract::FailureKind::NotReadyInTime { .. } => Readiness::TimedOut,
                _ => Readiness::Rejected,
            },
        }
    }

    /// Whether a selection has been activated.
    pub fn is_tracked(&self, selection: u64) -> bool {
        self.attempts.iter().any(|(key, _)| *key == selection)
    }

    /// How many attempts are being tracked.
    ///
    /// Bounded by [`TRACKED_ATTEMPTS`], plus the attempts that were started
    /// while the bound was full of unfinished ones.
    pub fn tracked(&self) -> usize {
        self.attempts.len()
    }

    /// Drops the oldest settled attempt, returning its selection.
    fn evict_settled(&mut self, keep: u64) -> Option<u64> {
        let index = self
            .attempts
            .iter()
            .position(|(key, attempt)| *key != keep && !attempt.outcome.is_pending())?;
        self.attempts.remove(index).map(|(key, _)| key)
    }
}

/// Starts `argv` and waits up to `grace` for evidence that it is running.
///
/// This is the crate's only blocking call. It is blocking on purpose — it waits
/// on a child process — and its caller is expected to run it on a thread of its
/// own, which is what `crate::app` does; it must not be called from `update`.
///
/// Returning [`Observation::Ready`] means the process was alive when the grace
/// period ended. It does not mean a window exists; see the module
/// documentation.
pub fn spawn_and_probe(argv: &[String], grace: Duration) -> Observation {
    let Some((program, arguments)) = argv.split_first() else {
        return Observation::SpawnFailed {
            detail: "no program to start".to_owned(),
        };
    };

    let child = Command::new(program)
        .args(arguments)
        // The application must not inherit the launcher's streams: a GUI
        // process holding the launcher's standard output would keep a pipe
        // open of a process that is about to exit.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            return Observation::SpawnFailed {
                detail: error.to_string(),
            };
        }
    };

    let pid = child.id();
    std::thread::sleep(grace);

    match child.try_wait() {
        Ok(None) => {
            // The application outlives the launcher, and the launcher is not
            // its parent any more than the session manager is; reaping it here
            // keeps it from becoming a zombie for as long as the launcher lives.
            std::thread::spawn(move || {
                if let Err(error) = child.wait() {
                    debug!(%error, pid, "could not reap a launched process");
                }
            });
            Observation::Ready { pid }
        }
        Ok(Some(status)) => Observation::ExitedBeforeReady {
            detail: status.to_string(),
        },
        Err(error) => Observation::ExitedBeforeReady {
            detail: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::FailureKind;

    fn argv() -> Vec<String> {
        vec!["example".to_owned()]
    }

    fn activator() -> Activator {
        Activator::new()
    }

    #[test]
    fn a_selection_is_activated_at_most_once() {
        let mut activator = activator();

        let first = activator.request(1, "a.desktop", "A", argv());
        assert!(first.is_some());

        // The user's finger reaches Enter again before the first launch has a
        // result: the second and third presses are the same request.
        assert!(activator.request(1, "a.desktop", "A", argv()).is_none());
        assert!(activator.request(1, "a.desktop", "A", argv()).is_none());
        assert_eq!(activator.tracked(), 1);
    }

    #[test]
    fn a_new_selection_is_a_new_activation() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());
        assert!(activator.request(2, "a.desktop", "A", argv()).is_some());
        assert_eq!(activator.tracked(), 2);
    }

    #[test]
    fn the_request_carries_the_selection_and_the_command() {
        let mut activator = activator();
        let request = activator
            .request(7, "a.desktop", "A", argv())
            .expect("the first request");
        assert_eq!(request.selection, 7);
        assert_eq!(request.entry, "a.desktop");
        assert_eq!(request.name, "A");
        assert_eq!(request.argv, ["example"]);
    }

    #[test]
    fn the_grace_period_ends_in_ready_or_exited() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());

        assert_eq!(
            activator.observe(1, Observation::Spawned { pid: 42 }),
            Some(LaunchOutcome::Requested),
            "a process existing is progress, not a result",
        );
        assert_eq!(activator.readiness(1), Readiness::Pending);

        assert_eq!(
            activator.observe(1, Observation::Ready { pid: 42 }),
            Some(LaunchOutcome::Started { pid: 42 }),
        );
        assert_eq!(activator.readiness(1), Readiness::Ready);
        assert_eq!(
            activator.outcome(1).map(LaunchOutcome::state),
            Some(crate::contract::CommandState::Applied),
        );
    }

    #[test]
    fn a_process_that_exits_before_it_is_ready_fails_with_a_reason() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());

        let outcome = activator
            .observe(
                1,
                Observation::ExitedBeforeReady {
                    detail: "exit status: 1".to_owned(),
                },
            )
            .expect("the attempt is tracked");

        match outcome {
            LaunchOutcome::Failed { failure } => {
                assert_eq!(failure.entry.as_deref(), Some("A"));
                assert_eq!(
                    failure.kind,
                    FailureKind::ExitedBeforeReady {
                        detail: "exit status: 1".to_owned()
                    }
                );
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(activator.readiness(1), Readiness::Rejected);
    }

    #[test]
    fn a_spawn_that_could_not_happen_fails_with_the_kernels_words() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());
        let outcome = activator
            .observe(
                1,
                Observation::SpawnFailed {
                    detail: "No such file or directory (os error 2)".to_owned(),
                },
            )
            .expect("tracked");
        assert!(matches!(outcome, LaunchOutcome::Failed { .. }));
        assert_eq!(outcome.state(), crate::contract::CommandState::Rejected);
    }

    #[test]
    fn a_timeout_is_a_timeout_not_a_rejection() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());
        let outcome = activator
            .observe(1, Observation::TimedOut { seconds: 5 })
            .expect("tracked");
        assert_eq!(outcome.state(), crate::contract::CommandState::TimedOut);
        assert_eq!(activator.readiness(1), Readiness::TimedOut);
    }

    #[test]
    fn the_first_result_wins() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());
        assert!(
            activator
                .observe(1, Observation::Ready { pid: 1 })
                .is_some()
        );

        assert_eq!(
            activator.observe(
                1,
                Observation::ExitedBeforeReady {
                    detail: "much later".to_owned()
                }
            ),
            Some(LaunchOutcome::Started { pid: 1 }),
            "evidence that arrives after a verified result does not undo it",
        );
    }

    #[test]
    fn evidence_for_a_selection_that_was_never_activated_is_ignored() {
        let mut activator = activator();
        assert_eq!(activator.observe(9, Observation::Ready { pid: 1 }), None);
        assert_eq!(activator.outcome(9), None);
        assert_eq!(activator.readiness(9), Readiness::Pending);
        assert!(!activator.is_tracked(9));
    }

    #[test]
    fn a_failed_launch_can_be_retried_under_a_new_selection() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());
        assert!(
            activator
                .observe(
                    1,
                    Observation::SpawnFailed {
                        detail: "not found".to_owned()
                    }
                )
                .is_some()
        );

        let retry = activator
            .retry(2, 1)
            .expect("a failed launch can be tried again");
        assert_eq!(retry.selection, 2);
        assert_eq!(retry.entry, "a.desktop");
        assert_eq!(activator.tracked(), 2);
    }

    #[test]
    fn a_retry_is_refused_while_the_first_attempt_is_undecided() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());
        assert_eq!(activator.retry(2, 1), None);
    }

    #[test]
    fn a_successful_launch_cannot_be_retried() {
        let mut activator = activator();
        assert!(activator.request(1, "a.desktop", "A", argv()).is_some());
        assert!(
            activator
                .observe(1, Observation::Ready { pid: 3 })
                .is_some()
        );
        assert_eq!(activator.retry(2, 1), None);
    }

    #[test]
    fn retrying_a_selection_that_does_not_exist_is_refused() {
        let mut activator = activator();
        assert_eq!(activator.retry(2, 99), None);
    }

    #[test]
    fn the_history_is_bounded_and_drops_settled_attempts() {
        let mut activator = activator();
        for selection in 1..=u64::from(TRACKED_ATTEMPTS as u32) {
            assert!(
                activator
                    .request(selection, "a.desktop", "A", argv())
                    .is_some()
            );
            assert!(
                activator
                    .observe(selection, Observation::Ready { pid: 1 })
                    .is_some()
            );
        }
        assert_eq!(activator.tracked(), TRACKED_ATTEMPTS);

        // One more settled attempt evicts the oldest.
        assert!(activator.request(1000, "a.desktop", "A", argv()).is_some());
        assert_eq!(activator.tracked(), TRACKED_ATTEMPTS);
        assert!(!activator.is_tracked(1));
        assert!(activator.is_tracked(1000));
    }

    #[test]
    fn an_activation_is_refused_when_the_history_is_full_of_unfinished_ones() {
        let mut activator = activator();
        for selection in 1..=u64::from(TRACKED_ATTEMPTS as u32) {
            assert!(
                activator
                    .request(selection, "a.desktop", "A", argv())
                    .is_some()
            );
        }
        assert_eq!(activator.tracked(), TRACKED_ATTEMPTS);
        assert_eq!(
            activator.request(9999, "a.desktop", "A", argv()),
            None,
            "a hard bound beats unbounded memory",
        );
    }

    #[test]
    fn an_empty_argv_is_a_failure_not_a_panic() {
        assert_eq!(
            spawn_and_probe(&[], Duration::from_millis(1)),
            Observation::SpawnFailed {
                detail: "no program to start".to_owned()
            },
        );
    }

    #[test]
    fn a_program_that_exits_immediately_is_not_ready() {
        let argv = vec!["sh".to_owned(), "-c".to_owned(), "exit 3".to_owned()];
        match spawn_and_probe(&argv, Duration::from_millis(200)) {
            Observation::ExitedBeforeReady { detail } => {
                assert!(detail.contains('3'), "{detail}");
            }
            other => panic!("expected the exit to be observed, got {other:?}"),
        }
    }

    #[test]
    fn a_program_that_stays_alive_is_ready() {
        let argv = vec!["sh".to_owned(), "-c".to_owned(), "sleep 5".to_owned()];
        match spawn_and_probe(&argv, Duration::from_millis(50)) {
            Observation::Ready { pid } => assert!(pid > 0),
            other => panic!("expected a running process, got {other:?}"),
        }
    }

    #[test]
    fn a_program_that_does_not_exist_could_not_be_spawned() {
        let argv = vec!["definitely-not-a-program-inox".to_owned()];
        match spawn_and_probe(&argv, Duration::from_millis(1)) {
            Observation::SpawnFailed { detail } => assert!(!detail.is_empty()),
            other => panic!("expected a spawn failure, got {other:?}"),
        }
    }
}
