//! Prepare-to-suspend ordering.
//! Prepare-to-suspend ordering.
//! the session-lock boundary states the order as a rule and gives no room to hope:
//!
//! > 触发顺序为准备锁定→收到锁完成确认→允许挂起；锁定失败按明确策略阻止/取消挂起，
//! > 不能先休眠再寄望恢复时补锁
//!
//! which is to say: lock, wait for the *confirmation* that the lock completed,
//! and only then let the suspend proceed. If the lock does not land, the suspend
//! is refused or cancelled by a policy that is written down — never left to be
//! established on resume, when the machine has already been asleep with whatever
//! was on screen.
//!
//! This module is that ordering, and nothing else. It is a small state machine
//! because the rule has three parts that can each be got wrong:
//!
//! - **A request does not authorise a suspend.** [`SuspendEvent::Requested`]
//!   returns [`SuspendDecision::Lock`] — ask for the lock, and suspend nothing.
//! - **The confirmation must be for *this* request.** Each request mints a
//!   [`Generation`], and a confirmation carrying any other generation is
//!   [`SuspendDecision::Ignore`]d. A confirmation that was already in flight when
//!   the request was made says nothing about the lock that was just asked for.
//! - **A failure is decided, not deferred.** Refusing keeps the session awake
//!   and the user in front of it; cancelling takes back a suspend that was
//!   already under way. Which of the two is [`LockFailure`].
//!
//! There is no clock here on purpose. The bound on the confirmation is data
//! ([`SuspendPolicy::confirmation_timeout`]) that the session — which owns the
//! timers, and is the component the session-lock boundary gives the supervision to — uses to send
//! [`SuspendEvent::TimedOut`]. Keeping the timeout as an *event* rather than a
//! deadline in two places is what keeps the ordering testable without sleeping
//! and without a second source of truth about when the wait is over.

use std::time::Duration;

use crate::contract::Generation;

/// What to do when the lock cannot be confirmed before a suspend.
///
/// Both answers are in the session-lock boundary's "refused or cancelled"; which one is used is a
/// decision a session makes once, not a fallback chosen at runtime.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LockFailure {
    /// Do not suspend: the session stays awake, locked, with the user in front
    /// of it. The default, because it is the answer that cannot lose a frame.
    #[default]
    Refuse,
    /// Take back a suspend that was already under way.
    Cancel,
}

/// How the session coordinates a suspend with the lock screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuspendPolicy {
    /// How long the session waits for the lock to confirm before it sends
    /// [`SuspendEvent::TimedOut`].
    pub confirmation_timeout: Duration,
    /// What happens when the lock does not confirm.
    pub on_failure: LockFailure,
}

impl SuspendPolicy {
    /// A policy, validated.
    ///
    /// # Errors
    ///
    /// [`SuspendError::ZeroTimeout`] for a timeout of zero, which would time the
    /// lock out before it could possibly confirm and turn every suspend into a
    /// refusal.
    pub fn new(
        confirmation_timeout: Duration,
        on_failure: LockFailure,
    ) -> Result<Self, SuspendError> {
        if confirmation_timeout.is_zero() {
            return Err(SuspendError::ZeroTimeout);
        }
        Ok(Self {
            confirmation_timeout,
            on_failure,
        })
    }
}

impl Default for SuspendPolicy {
    fn default() -> Self {
        Self {
            confirmation_timeout: Duration::from_secs(5),
            on_failure: LockFailure::Refuse,
        }
    }
}

/// Why a suspend policy was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SuspendError {
    /// A zero timeout leaves the lock no time to confirm.
    #[error("a confirmation timeout of 0 would refuse every suspend")]
    ZeroTimeout,
}

/// Where the coordination is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SuspendPhase {
    /// No suspend is being prepared.
    #[default]
    Idle,
    /// A lock was asked for; the confirmation has not arrived.
    Locking,
    /// The lock confirmed: the suspend may proceed.
    Ready,
    /// The lock could not be confirmed and the suspend was refused.
    Refused,
    /// The lock could not be confirmed and a suspend under way was cancelled.
    Cancelled,
}

/// What a suspend asks of the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendDecision {
    /// Ask the compositor to lock, under `generation`, and suspend nothing yet.
    Lock {
        /// The generation the confirmation must carry.
        generation: Generation,
    },
    /// The lock is confirmed for this request: the suspend may proceed.
    AllowSuspend {
        /// The request that was confirmed.
        generation: Generation,
    },
    /// Do not suspend.
    Refuse {
        /// The generation whose lock failed.
        generation: Generation,
        /// Why the suspend was refused.
        reason: SuspendRefusal,
    },
    /// Take back the suspend under way.
    Cancel {
        /// The generation whose lock failed.
        generation: Generation,
        /// Why the suspend was cancelled.
        reason: SuspendRefusal,
    },
    /// Nothing to do, and why.
    Ignore(SuspendIgnore),
}

/// Why a suspend was refused or cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendRefusal {
    /// The lock reported a failure.
    LockFailed,
    /// The lock did not confirm inside its budget.
    TimedOut,
    /// The lock protocol is not available at all, so no lock can be asked for.
    NoLockProtocol,
}

impl SuspendRefusal {
    /// The catalog message that explains this to the user.
    pub const fn message_id(self) -> &'static str {
        match self {
            Self::LockFailed => "lock-suspend-refused-lock-failed",
            Self::TimedOut => "lock-suspend-refused-timeout",
            Self::NoLockProtocol => "lock-suspend-refused-no-protocol",
        }
    }
}

/// Why an event asked for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendIgnore {
    /// A suspend is already being prepared.
    AlreadyInProgress,
    /// The confirmation belongs to a different request.
    StaleGeneration,
    /// The confirmation has already been given.
    AlreadyConfirmed,
    /// No suspend is being prepared.
    NotPreparing,
    /// The session came back from a suspend, which is not something the lock
    /// state has an opinion about.
    OutOfScope,
}

/// What the session tells the coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspendEvent {
    /// A suspend is being considered.
    Requested,
    /// The lock is complete, for `generation`.
    LockConfirmed(Generation),
    /// The lock failed, for `generation`.
    LockFailed(Generation),
    /// The lock did not confirm inside [`SuspendPolicy::confirmation_timeout`],
    /// for `generation`.
    TimedOut(Generation),
    /// The session has come back, or the suspend was abandoned.
    Resumed,
}

/// The prepare-to-suspend ordering.
#[derive(Debug, Clone)]
pub struct SuspendCoordinator {
    policy: SuspendPolicy,
    phase: SuspendPhase,
    generation: Generation,
}

impl Default for SuspendCoordinator {
    fn default() -> Self {
        Self::new(SuspendPolicy::default())
    }
}

impl SuspendCoordinator {
    /// A coordinator that is preparing nothing.
    pub fn new(policy: SuspendPolicy) -> Self {
        Self {
            policy,
            phase: SuspendPhase::Idle,
            generation: Generation::new(0),
        }
    }

    /// The policy in force.
    pub const fn policy(&self) -> &SuspendPolicy {
        &self.policy
    }

    /// Where the coordination is.
    pub const fn phase(&self) -> SuspendPhase {
        self.phase
    }

    /// The current request's generation, once there has been one.
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Applies an event and says what the session should do.
    ///
    /// A suspend is never allowed by anything but a
    /// [`SuspendEvent::LockConfirmed`] for the current generation: no refusal,
    /// no timeout, no resume and no confirmation from an earlier request
    /// produces [`SuspendDecision::AllowSuspend`].
    pub fn apply(&mut self, event: SuspendEvent) -> SuspendDecision {
        match event {
            SuspendEvent::Requested => self.request(),
            SuspendEvent::LockConfirmed(generation) => self.confirmed(generation),
            SuspendEvent::LockFailed(generation) => {
                self.failed(generation, SuspendRefusal::LockFailed)
            }
            SuspendEvent::TimedOut(generation) => self.failed(generation, SuspendRefusal::TimedOut),
            SuspendEvent::Resumed => {
                self.phase = SuspendPhase::Idle;
                SuspendDecision::Ignore(SuspendIgnore::OutOfScope)
            }
        }
    }

    /// Refuses a suspend because nothing can be locked at all.
    ///
    /// Used when the compositor does not offer the lock protocol: there is no
    /// lock to wait for, so the answer comes from the policy immediately rather
    /// than from a timeout.
    pub fn refuse_unlockable(&mut self) -> SuspendDecision {
        self.generation = self.generation.next();
        let generation = self.generation;

        match self.policy.on_failure {
            LockFailure::Refuse => {
                self.phase = SuspendPhase::Refused;
                SuspendDecision::Refuse {
                    generation,
                    reason: SuspendRefusal::NoLockProtocol,
                }
            }
            LockFailure::Cancel => {
                self.phase = SuspendPhase::Cancelled;
                SuspendDecision::Cancel {
                    generation,
                    reason: SuspendRefusal::NoLockProtocol,
                }
            }
        }
    }

    fn request(&mut self) -> SuspendDecision {
        if self.phase == SuspendPhase::Locking || self.phase == SuspendPhase::Ready {
            return SuspendDecision::Ignore(SuspendIgnore::AlreadyInProgress);
        }

        self.generation = self.generation.next();
        self.phase = SuspendPhase::Locking;

        SuspendDecision::Lock {
            generation: self.generation,
        }
    }

    fn confirmed(&mut self, generation: Generation) -> SuspendDecision {
        if generation != self.generation {
            return SuspendDecision::Ignore(SuspendIgnore::StaleGeneration);
        }

        match self.phase {
            SuspendPhase::Locking => {
                self.phase = SuspendPhase::Ready;
                SuspendDecision::AllowSuspend {
                    generation: self.generation,
                }
            }
            SuspendPhase::Ready => SuspendDecision::Ignore(SuspendIgnore::AlreadyConfirmed),
            SuspendPhase::Idle | SuspendPhase::Refused | SuspendPhase::Cancelled => {
                SuspendDecision::Ignore(SuspendIgnore::NotPreparing)
            }
        }
    }

    fn failed(&mut self, generation: Generation, reason: SuspendRefusal) -> SuspendDecision {
        if generation != self.generation {
            return SuspendDecision::Ignore(SuspendIgnore::StaleGeneration);
        }
        if self.phase != SuspendPhase::Locking {
            return SuspendDecision::Ignore(SuspendIgnore::NotPreparing);
        }

        let generation = self.generation;

        match self.policy.on_failure {
            LockFailure::Refuse => {
                self.phase = SuspendPhase::Refused;
                SuspendDecision::Refuse { generation, reason }
            }
            LockFailure::Cancel => {
                self.phase = SuspendPhase::Cancelled;
                SuspendDecision::Cancel { generation, reason }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn coordinator(on_failure: LockFailure) -> SuspendCoordinator {
        SuspendCoordinator::new(
            SuspendPolicy::new(Duration::from_secs(5), on_failure).expect("a valid policy"),
        )
    }

    #[test]
    fn a_request_locks_and_does_not_suspend() {
        let mut suspend = coordinator(LockFailure::Refuse);

        let decision = suspend.apply(SuspendEvent::Requested);

        assert_eq!(
            decision,
            SuspendDecision::Lock {
                generation: Generation::new(1)
            }
        );
        assert_eq!(suspend.phase(), SuspendPhase::Locking);
    }

    #[test]
    fn only_a_confirmation_of_this_request_allows_the_suspend() {
        let mut suspend = coordinator(LockFailure::Refuse);
        let SuspendDecision::Lock { generation } = suspend.apply(SuspendEvent::Requested) else {
            panic!("a request locks");
        };

        let decision = suspend.apply(SuspendEvent::LockConfirmed(generation));

        assert_eq!(decision, SuspendDecision::AllowSuspend { generation });
        assert_eq!(suspend.phase(), SuspendPhase::Ready);
    }

    #[test]
    fn a_confirmation_from_an_earlier_request_allows_nothing() {
        let mut suspend = coordinator(LockFailure::Refuse);
        let SuspendDecision::Lock { generation: first } = suspend.apply(SuspendEvent::Requested)
        else {
            panic!("a request locks");
        };
        suspend.apply(SuspendEvent::Resumed);
        let SuspendDecision::Lock { generation: second } = suspend.apply(SuspendEvent::Requested)
        else {
            panic!("a request locks");
        };
        assert_ne!(first, second);

        let decision = suspend.apply(SuspendEvent::LockConfirmed(first));

        assert_eq!(
            decision,
            SuspendDecision::Ignore(SuspendIgnore::StaleGeneration)
        );
        assert_eq!(suspend.phase(), SuspendPhase::Locking);
    }

    #[test]
    fn a_failed_lock_refuses_the_suspend() {
        let mut suspend = coordinator(LockFailure::Refuse);
        let SuspendDecision::Lock { generation } = suspend.apply(SuspendEvent::Requested) else {
            panic!("a request locks");
        };

        let decision = suspend.apply(SuspendEvent::LockFailed(generation));

        assert_eq!(
            decision,
            SuspendDecision::Refuse {
                generation,
                reason: SuspendRefusal::LockFailed
            }
        );
        assert_eq!(suspend.phase(), SuspendPhase::Refused);
    }

    #[test]
    fn a_timeout_refuses_the_suspend_by_default() {
        let mut suspend = coordinator(LockFailure::Refuse);
        let SuspendDecision::Lock { generation } = suspend.apply(SuspendEvent::Requested) else {
            panic!("a request locks");
        };

        let decision = suspend.apply(SuspendEvent::TimedOut(generation));

        assert_eq!(
            decision,
            SuspendDecision::Refuse {
                generation,
                reason: SuspendRefusal::TimedOut
            }
        );
    }

    #[test]
    fn the_cancel_policy_takes_back_the_suspend_instead() {
        let mut suspend = coordinator(LockFailure::Cancel);
        let SuspendDecision::Lock { generation } = suspend.apply(SuspendEvent::Requested) else {
            panic!("a request locks");
        };

        let decision = suspend.apply(SuspendEvent::TimedOut(generation));

        assert_eq!(
            decision,
            SuspendDecision::Cancel {
                generation,
                reason: SuspendRefusal::TimedOut
            }
        );
        assert_eq!(suspend.phase(), SuspendPhase::Cancelled);
    }

    #[test]
    fn a_failure_for_another_generation_is_ignored() {
        let mut suspend = coordinator(LockFailure::Refuse);
        suspend.apply(SuspendEvent::Requested);

        let decision = suspend.apply(SuspendEvent::LockFailed(Generation::new(99)));

        assert_eq!(
            decision,
            SuspendDecision::Ignore(SuspendIgnore::StaleGeneration)
        );
        assert_eq!(suspend.phase(), SuspendPhase::Locking);
    }

    #[test]
    fn a_failure_with_nothing_being_prepared_is_ignored() {
        let mut suspend = coordinator(LockFailure::Refuse);

        let decision = suspend.apply(SuspendEvent::TimedOut(suspend.generation()));

        assert_eq!(
            decision,
            SuspendDecision::Ignore(SuspendIgnore::NotPreparing)
        );
    }

    #[test]
    fn a_second_request_while_one_is_prepared_is_ignored() {
        let mut suspend = coordinator(LockFailure::Refuse);
        suspend.apply(SuspendEvent::Requested);

        let decision = suspend.apply(SuspendEvent::Requested);

        assert_eq!(
            decision,
            SuspendDecision::Ignore(SuspendIgnore::AlreadyInProgress)
        );
    }

    #[test]
    fn a_second_confirmation_allows_nothing_new() {
        let mut suspend = coordinator(LockFailure::Refuse);
        let SuspendDecision::Lock { generation } = suspend.apply(SuspendEvent::Requested) else {
            panic!("a request locks");
        };
        suspend.apply(SuspendEvent::LockConfirmed(generation));

        let decision = suspend.apply(SuspendEvent::LockConfirmed(generation));

        assert_eq!(
            decision,
            SuspendDecision::Ignore(SuspendIgnore::AlreadyConfirmed)
        );
    }

    #[test]
    fn a_resume_returns_to_idle_and_allows_nothing() {
        let mut suspend = coordinator(LockFailure::Refuse);
        suspend.apply(SuspendEvent::Requested);

        let decision = suspend.apply(SuspendEvent::Resumed);

        assert_eq!(decision, SuspendDecision::Ignore(SuspendIgnore::OutOfScope));
        assert_eq!(suspend.phase(), SuspendPhase::Idle);
    }

    #[test]
    fn a_request_after_a_refusal_starts_a_new_generation() {
        let mut suspend = coordinator(LockFailure::Refuse);
        let SuspendDecision::Lock { generation: first } = suspend.apply(SuspendEvent::Requested)
        else {
            panic!("a request locks");
        };
        suspend.apply(SuspendEvent::LockFailed(first));

        let decision = suspend.apply(SuspendEvent::Requested);

        assert_eq!(
            decision,
            SuspendDecision::Lock {
                generation: Generation::new(2)
            },
            "the user gets another chance rather than a permanent refusal"
        );
    }

    #[test]
    fn an_unlockable_session_refuses_without_waiting_for_a_timeout() {
        let mut suspend = coordinator(LockFailure::Refuse);

        let decision = suspend.refuse_unlockable();

        assert_eq!(
            decision,
            SuspendDecision::Refuse {
                generation: Generation::new(1),
                reason: SuspendRefusal::NoLockProtocol
            }
        );
    }

    #[test]
    fn an_unlockable_session_respects_the_cancel_policy() {
        let mut suspend = coordinator(LockFailure::Cancel);

        let decision = suspend.refuse_unlockable();

        assert!(matches!(decision, SuspendDecision::Cancel { .. }));
    }

    #[test]
    fn a_zero_timeout_is_refused() {
        assert_eq!(
            SuspendPolicy::new(Duration::ZERO, LockFailure::Refuse),
            Err(SuspendError::ZeroTimeout)
        );
        assert!(SuspendPolicy::default().confirmation_timeout > Duration::ZERO);
    }

    #[test]
    fn every_refusal_names_a_catalog_message() {
        for reason in [
            SuspendRefusal::LockFailed,
            SuspendRefusal::TimedOut,
            SuspendRefusal::NoLockProtocol,
        ] {
            assert!(reason.message_id().starts_with("lock-"));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Whatever the session sends, a suspend is allowed only by a
        /// confirmation of the request that is currently being prepared, and a
        /// failure or a timeout never turns into a permission.
        #[test]
        fn only_the_current_confirmation_allows_a_suspend(
            steps in proptest::collection::vec(0usize..5, 0..48),
        ) {
            let mut suspend = coordinator(LockFailure::Refuse);
            let mut current = Generation::new(0);

            for step in &steps {
                let (event, decision) = match step {
                    0 => {
                        let decision = suspend.apply(SuspendEvent::Requested);
                        if let SuspendDecision::Lock { generation } = decision {
                            current = generation;
                        }
                        ("a request", decision)
                    }
                    1 => (
                        "a confirmation of the current request",
                        suspend.apply(SuspendEvent::LockConfirmed(current)),
                    ),
                    2 => (
                        "a failure of the current request",
                        suspend.apply(SuspendEvent::LockFailed(current)),
                    ),
                    3 => (
                        "a timeout of the current request",
                        suspend.apply(SuspendEvent::TimedOut(current)),
                    ),
                    _ => ("a resume", suspend.apply(SuspendEvent::Resumed)),
                };

                match decision {
                    SuspendDecision::AllowSuspend { generation } => {
                        prop_assert_eq!(
                            event,
                            "a confirmation of the current request",
                            "only a confirmation allows a suspend: {} did",
                            event,
                        );
                        prop_assert_eq!(generation, current);
                        prop_assert_eq!(suspend.phase(), SuspendPhase::Ready);
                    }
                    SuspendDecision::Refuse { generation, .. }
                    | SuspendDecision::Cancel { generation, .. } => {
                        prop_assert!(
                            event.ends_with("of the current request")
                                && !event.starts_with("a confirmation"),
                            "only a failure leaves a suspend refused: {} did",
                            event,
                        );
                        prop_assert_eq!(generation, current);
                        prop_assert_ne!(
                            suspend.phase(),
                            SuspendPhase::Ready,
                            "a refused suspend must not leave the coordinator ready",
                        );
                    }
                    SuspendDecision::Lock { generation } => {
                        prop_assert_eq!(event, "a request");
                        prop_assert_eq!(generation, current);
                    }
                    SuspendDecision::Ignore(_) => {}
                }
            }
        }
    }
}
