//! The cross-process state contract of the interaction contract.
//!
//! The entries are separate processes and the state they show belongs to a
//! service elsewhere: the compositor owns windows, focus and outputs, and each
//! daemon owns what it is authoritative for. Four rules from the interaction contract's
//! "跨进程状态契约" section are what the components are written against, and
//! they are small enough to live as types rather than as prose:
//!
//! 1. **A view is a snapshot with a generation.** A client syncs first and
//!    subscribes afterwards, and a snapshot older than the one it already holds
//!    is discarded rather than applied ([`Snapshot`], [`SnapshotView`]).
//! 2. **A command has a result, and the stages are not interchangeable.** Sent,
//!    accepted and applied are three different claims, and the entries display
//!    the effect they read back, not the request they sent ([`CommandResult`]).
//! 3. **A request has a generation.** A slow answer to an old query must not
//!    overwrite the answer to a newer one ([`RequestGeneration`]).
//! 4. **A service that is not ready says so, and retrying is bounded.** Loading,
//!    error and retry are designed states (acceptance D05), and neither the
//!    spinner nor the retry loop may run for ever ([`ServiceState`],
//!    [`RetryPolicy`], [`RetrySchedule`]).
//!
//! Nothing here is specific to one entry: the panel, the launcher, the settings
//! window and the notification daemon all state the same facts about the
//! services they consume, which is what makes the contract worth sharing even
//! though each of them is its own process.

use std::fmt;
use std::time::Duration;

use crate::diagnostic::Diagnostic;
use crate::i18n::id;

/// The generation a service stamps on one snapshot of its authoritative state.
///
/// Only the service issuing it may interpret it beyond ordering: the client's
/// whole job is to compare it with the newest one it has seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotGeneration(u64);

impl SnapshotGeneration {
    /// Wraps a generation a service reported.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The number the service reported.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SnapshotGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One immutable view of a service's authoritative state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Snapshot<T> {
    /// The generation this view belongs to.
    pub generation: SnapshotGeneration,
    /// What the service said.
    pub value: T,
}

impl<T> Snapshot<T> {
    /// A snapshot of `value` at `generation`.
    pub const fn new(generation: SnapshotGeneration, value: T) -> Self {
        Self { generation, value }
    }

    /// Whether this snapshot is newer than the generation `seen`.
    ///
    /// A snapshot with the same generation is not newer: the client already
    /// holds everything it says, and re-applying it would rebuild a view that
    /// cannot have changed.
    pub fn is_newer_than(&self, seen: SnapshotGeneration) -> bool {
        self.generation > seen
    }
}

/// The client side of the snapshot contract: the newest view it has been given.
///
/// The rules of the interaction contract, rule 1 are enforced here rather than left to each
/// caller: the view only ever moves forward, so an answer that arrives late —
/// from a query sent before a reconnect, say — cannot roll the display back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnapshotView<T> {
    generation: SnapshotGeneration,
    value: T,
}

impl<T> SnapshotView<T> {
    /// A view that starts from `snapshot`.
    pub fn new(snapshot: Snapshot<T>) -> Self {
        Self {
            generation: snapshot.generation,
            value: snapshot.value,
        }
    }

    /// Applies `snapshot` if it is newer than the view's.
    ///
    /// Returns whether the view changed, which is also the signal to redisplay:
    /// `false` means the snapshot was discarded as stale, and the held value is
    /// untouched.
    pub fn apply(&mut self, snapshot: Snapshot<T>) -> bool {
        if !snapshot.is_newer_than(self.generation) {
            return false;
        }
        self.generation = snapshot.generation;
        self.value = snapshot.value;
        true
    }

    /// What the service last said, as the client believes it.
    pub fn current(&self) -> &T {
        &self.value
    }

    /// The generation of the held view.
    pub fn generation(&self) -> SnapshotGeneration {
        self.generation
    }
}

/// What became of a command the shell sent to a service.
///
/// The variants are ordered by how much is known: a command that was only
/// [`CommandResult::Sent`] may still be refused, one that was
/// [`CommandResult::Accepted`] has been promised rather than performed, and
/// only [`CommandResult::Applied`] means the effect was read back. A component
/// that shows the requested state as if it were the actual one is showing an
/// optimistic view, which is the mistake this type exists to prevent.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandResult {
    /// The command reached the transport; nothing has come back.
    Sent,
    /// The service acknowledged the command but has not carried it out.
    Accepted,
    /// The effect the command asked for was observed in the service's state.
    Applied,
    /// The service refused the command.
    Rejected {
        /// Why, in the service's own words.
        reason: Diagnostic,
    },
    /// No answer arrived before the caller's deadline.
    TimedOut,
}

impl CommandResult {
    /// Whether the effect of the command is known to have happened.
    ///
    /// Only a read-back makes this true: [`CommandResult::Sent`] and
    /// [`CommandResult::Accepted`] are both "not applied yet".
    pub fn is_applied(&self) -> bool {
        matches!(self, CommandResult::Applied)
    }

    /// Whether the outcome is still in flight.
    pub fn is_pending(&self) -> bool {
        matches!(self, CommandResult::Sent | CommandResult::Accepted)
    }

    /// Whether the command will not produce the effect it asked for.
    pub fn is_failed(&self) -> bool {
        matches!(
            self,
            CommandResult::Rejected { .. } | CommandResult::TimedOut
        )
    }

    /// A user-readable explanation of the stage the command reached.
    pub fn describe(&self) -> Diagnostic {
        match self {
            CommandResult::Sent => Diagnostic::new(id::COMMAND_SENT),
            CommandResult::Accepted => Diagnostic::new(id::COMMAND_ACCEPTED),
            CommandResult::Applied => Diagnostic::new(id::COMMAND_APPLIED),
            CommandResult::Rejected { reason } => {
                Diagnostic::new(id::COMMAND_REJECTED).with_message("reason", reason.clone())
            }
            CommandResult::TimedOut => Diagnostic::new(id::COMMAND_TIMED_OUT),
        }
    }
}

/// The generation of one asynchronous request.
///
/// A caller — a search box, a settings form, a task view refresh — issues
/// requests at its own pace and receives the answers out of order. It keeps the
/// newest generation it issued, stamps each request with a copy, and applies an
/// answer only when the stamp is still current:
///
/// ```
/// # use inox_shell::contract::RequestGeneration;
/// let mut newest = RequestGeneration::first();
/// let first = newest;
///
/// newest = newest.supersede();
/// let second = newest;
///
/// assert!(!first.is_current(newest));
/// assert!(second.is_current(newest));
/// ```
///
/// An older search result therefore cannot overwrite a newer one, which is what
/// D02 asks for when the query is cleared while an answer is still on its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestGeneration(u64);

impl RequestGeneration {
    /// The generation of the first request a caller issues.
    pub const fn first() -> Self {
        Self(1)
    }

    /// The generation a request issued after this one carries.
    ///
    /// The caller replaces the generation it keeps with the returned one; every
    /// answer stamped with the previous value becomes inapplicable.
    pub const fn supersede(self) -> Self {
        Self(self.0 + 1)
    }

    /// Whether a request stamped `self` is still the newest one, given that
    /// `newest` is the generation the caller issued most recently.
    pub fn is_current(self, newest: Self) -> bool {
        self == newest
    }

    /// The number itself.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for RequestGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What the session knows about one entry or service (acceptance D05).
///
/// The states are the ones a component has to render: a spinner that belongs to
/// [`ServiceState::Starting`], a working view in [`ServiceState::Ready`], an
/// error the user can act on in [`ServiceState::Unavailable`], and a
/// [`ServiceState::Failed`] state that explains itself. A component that shows
/// the same thing for all of them has not designed them.
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceState {
    /// Starting up; nothing is available yet.
    Starting,
    /// Running and usable.
    Ready,
    /// Not usable right now, with something the user can do about it.
    Unavailable {
        /// What the user can do — a message, not a bare error code.
        actionable: Diagnostic,
    },
    /// Not usable, and retrying is not what will fix it.
    Failed {
        /// Why it failed.
        reason: Diagnostic,
    },
}

impl ServiceState {
    /// Whether the service can be used right now.
    pub fn is_ready(&self) -> bool {
        matches!(self, ServiceState::Ready)
    }

    /// A user-readable explanation of the state.
    pub fn describe(&self) -> Diagnostic {
        match self {
            ServiceState::Starting => Diagnostic::new(id::SERVICE_STARTING),
            ServiceState::Ready => Diagnostic::new(id::SERVICE_READY),
            ServiceState::Unavailable { actionable } => Diagnostic::new(id::SERVICE_UNAVAILABLE)
                .with_message("actionable", actionable.clone()),
            ServiceState::Failed { reason } => {
                Diagnostic::new(id::SERVICE_FAILED).with_message("reason", reason.clone())
            }
        }
    }
}

/// How often and how far apart a failing service is retried.
///
/// Bounded on construction, so "nothing spins for ever" (D05) is a property of
/// the value rather than a habit of the caller: the number of attempts has a
/// ceiling and the delay has both a floor and a ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RetryPolicy {
    attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl RetryPolicy {
    /// The largest number of attempts a policy may allow.
    pub const MAX_ATTEMPTS: u32 = 10;

    /// The longest delay between two attempts a policy may ask for.
    pub const MAX_DELAY: Duration = Duration::from_secs(300);

    /// A policy that allows `attempts` attempts in total, the first one
    /// immediately and the rest `base_delay` apart, doubling up to `max_delay`.
    ///
    /// # Errors
    ///
    /// Returns a [`RetryPolicyError`] for a policy that could retry for ever or
    /// not retry at all: zero attempts, more than [`RetryPolicy::MAX_ATTEMPTS`],
    /// a zero base delay, a base delay above the ceiling, or a ceiling above
    /// [`RetryPolicy::MAX_DELAY`].
    pub fn new(
        attempts: u32,
        base_delay: Duration,
        max_delay: Duration,
    ) -> Result<Self, RetryPolicyError> {
        if attempts == 0 || attempts > Self::MAX_ATTEMPTS {
            return Err(RetryPolicyError::AttemptsOutOfRange { attempts });
        }
        if base_delay.is_zero() {
            return Err(RetryPolicyError::ZeroBaseDelay);
        }
        if base_delay > max_delay {
            return Err(RetryPolicyError::BaseDelayAboveMax {
                base_delay,
                max_delay,
            });
        }
        if max_delay > Self::MAX_DELAY {
            return Err(RetryPolicyError::MaxDelayOutOfRange { max_delay });
        }

        Ok(Self {
            attempts,
            base_delay,
            max_delay,
        })
    }

    /// How many attempts the policy allows in total.
    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// How long a policy waits before attempt number `attempt` (1-based).
    ///
    /// The delay doubles with each attempt and stops at the ceiling, so it is
    /// computed by repetition rather than by an exponent that could overflow.
    fn backoff(&self, attempt: u32) -> Duration {
        let mut delay = self.base_delay;
        for _ in 1..attempt {
            delay = delay.saturating_mul(2).min(self.max_delay);
        }
        delay
    }
}

impl Default for RetryPolicy {
    /// Three attempts, a quarter of a second apart, no more than fifteen
    /// seconds apart: enough to ride out a service that is still coming up,
    /// short enough that a user does not wait for it.
    fn default() -> Self {
        Self {
            attempts: 3,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(15),
        }
    }
}

/// Why a retry policy was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RetryPolicyError {
    /// The attempt count is zero or above [`RetryPolicy::MAX_ATTEMPTS`].
    #[error(
        "a retry policy must allow between 1 and {max} attempts, not {attempts}",
        max = RetryPolicy::MAX_ATTEMPTS
    )]
    AttemptsOutOfRange {
        /// The rejected count.
        attempts: u32,
    },
    /// The first delay is zero, which would retry in a tight loop.
    #[error("a retry policy needs a non-zero delay before it retries")]
    ZeroBaseDelay,
    /// The first delay is longer than the longest one.
    #[error("the first retry delay ({base_delay:?}) is longer than the maximum ({max_delay:?})")]
    BaseDelayAboveMax {
        /// The rejected base delay.
        base_delay: Duration,
        /// The ceiling it was compared with.
        max_delay: Duration,
    },
    /// The longest delay is above the ceiling.
    #[error(
        "the longest retry delay must not exceed {limit:?}, not {max_delay:?}",
        limit = RetryPolicy::MAX_DELAY
    )]
    MaxDelayOutOfRange {
        /// The rejected ceiling.
        max_delay: Duration,
    },
}

/// Counts the attempts made against a policy.
///
/// The counter is what actually stops a retry loop, and it lives next to the
/// policy rather than in every component so that no component can forget it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetrySchedule {
    policy: RetryPolicy,
    attempts_used: u32,
}

impl RetrySchedule {
    /// A schedule that has not tried anything yet.
    pub fn new(policy: RetryPolicy) -> Self {
        Self {
            policy,
            attempts_used: 0,
        }
    }

    /// Records a failed attempt and says how long to wait before the next one.
    ///
    /// `None` means the policy's budget is spent: the caller moves to
    /// [`ServiceState::Failed`] and offers the user a retry rather than looping.
    pub fn attempt_failed(&mut self) -> Option<Duration> {
        if self.attempts_used >= self.policy.attempts {
            return None;
        }
        self.attempts_used += 1;

        if self.attempts_used >= self.policy.attempts {
            None
        } else {
            Some(self.policy.backoff(self.attempts_used + 1))
        }
    }

    /// Forgets the failures, so a service that came back gets the budget again.
    pub fn reset(&mut self) {
        self.attempts_used = 0;
    }

    /// How many attempts have been made.
    pub fn attempts_used(&self) -> u32 {
        self.attempts_used
    }

    /// The policy this schedule counts against.
    pub fn policy(&self) -> RetryPolicy {
        self.policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{id, localizer};
    use inox_common::i18n::Locale;

    fn generation(value: u64) -> SnapshotGeneration {
        SnapshotGeneration::new(value)
    }

    fn snapshot(value: u64, windows: u8) -> Snapshot<u8> {
        Snapshot::new(generation(value), windows)
    }

    #[test]
    fn a_newer_snapshot_replaces_the_view() {
        let mut view = SnapshotView::new(snapshot(4, 2));

        assert!(view.apply(snapshot(5, 3)));

        assert_eq!(*view.current(), 3);
        assert_eq!(view.generation(), generation(5));
    }

    #[test]
    fn an_older_snapshot_is_discarded() {
        let mut view = SnapshotView::new(snapshot(4, 2));

        assert!(!view.apply(snapshot(3, 9)));

        assert_eq!(*view.current(), 2);
        assert_eq!(view.generation(), generation(4));
    }

    #[test]
    fn a_repeated_snapshot_is_not_newer() {
        let mut view = SnapshotView::new(snapshot(4, 2));

        assert!(!view.apply(snapshot(4, 2)));
        assert!(!snapshot(4, 2).is_newer_than(generation(4)));
        assert!(snapshot(5, 2).is_newer_than(generation(4)));
    }

    #[test]
    fn sent_and_applied_are_different_claims() {
        for result in [CommandResult::Sent, CommandResult::Accepted] {
            assert!(!result.is_applied(), "{result:?}");
            assert!(result.is_pending(), "{result:?}");
            assert!(!result.is_failed(), "{result:?}");
        }

        assert!(CommandResult::Applied.is_applied());
        assert!(!CommandResult::Applied.is_pending());
        assert!(!CommandResult::Applied.is_failed());

        assert!(CommandResult::TimedOut.is_failed());
        assert!(
            CommandResult::Rejected {
                reason: Diagnostic::new(id::SERVICE_FAILED)
            }
            .is_failed()
        );
    }

    #[test]
    fn every_command_result_has_a_readable_explanation() {
        let results = [
            CommandResult::Sent,
            CommandResult::Accepted,
            CommandResult::Applied,
            CommandResult::Rejected {
                reason: Diagnostic::new(id::SERVICE_FAILED),
            },
            CommandResult::TimedOut,
        ];

        for result in results {
            let diagnostic = result.describe();
            assert!(id::ALL.contains(&diagnostic.id()), "{}", diagnostic.id());
        }
    }

    #[test]
    fn a_superseded_request_is_no_longer_current() {
        let first = RequestGeneration::first();
        let second = first.supersede();
        let third = second.supersede();

        assert!(first.is_current(first));
        assert!(!first.is_current(second));
        assert!(!first.is_current(third));
        assert!(second.is_current(second));
        assert!(!second.is_current(third));
        assert!(third.is_current(third));
    }

    #[test]
    fn a_service_state_explains_itself() {
        let states = [
            ServiceState::Starting,
            ServiceState::Ready,
            ServiceState::Unavailable {
                actionable: Diagnostic::new(id::SERVICE_STARTING),
            },
            ServiceState::Failed {
                reason: Diagnostic::new(id::SERVICE_READY),
            },
        ];

        for state in states {
            let diagnostic = state.describe();
            assert!(id::ALL.contains(&diagnostic.id()), "{}", diagnostic.id());
            assert_eq!(state.is_ready(), state == ServiceState::Ready);
        }
    }

    #[test]
    fn a_policy_must_be_bounded() {
        assert!(RetryPolicy::new(0, Duration::from_millis(1), Duration::from_millis(2)).is_err());
        assert!(
            RetryPolicy::new(
                RetryPolicy::MAX_ATTEMPTS + 1,
                Duration::from_millis(1),
                Duration::from_millis(2)
            )
            .is_err()
        );
        assert!(RetryPolicy::new(3, Duration::ZERO, Duration::from_millis(2)).is_err());
        assert!(RetryPolicy::new(3, Duration::from_secs(2), Duration::from_secs(1)).is_err());
        assert!(
            RetryPolicy::new(
                3,
                Duration::from_millis(1),
                RetryPolicy::MAX_DELAY + Duration::from_secs(1)
            )
            .is_err()
        );

        let policy = RetryPolicy::new(3, Duration::from_millis(1), Duration::from_secs(1));
        assert_eq!(policy.map(|policy| policy.attempts()), Ok(3));
    }

    #[test]
    fn a_rejected_policy_says_which_limit_it_broke() {
        assert_eq!(
            RetryPolicy::new(0, Duration::from_millis(1), Duration::from_millis(2)),
            Err(RetryPolicyError::AttemptsOutOfRange { attempts: 0 })
        );
        assert_eq!(
            RetryPolicy::new(2, Duration::ZERO, Duration::from_millis(2)),
            Err(RetryPolicyError::ZeroBaseDelay)
        );
        assert_eq!(
            RetryPolicy::new(2, Duration::from_secs(3), Duration::from_secs(2)),
            Err(RetryPolicyError::BaseDelayAboveMax {
                base_delay: Duration::from_secs(3),
                max_delay: Duration::from_secs(2),
            })
        );
        assert_eq!(
            RetryPolicy::new(2, Duration::from_secs(1), Duration::from_secs(301)),
            Err(RetryPolicyError::MaxDelayOutOfRange {
                max_delay: Duration::from_secs(301),
            })
        );
    }

    #[test]
    fn the_schedule_stops_when_the_budget_is_spent() {
        let policy = RetryPolicy::new(3, Duration::from_millis(100), Duration::from_secs(1))
            .expect("a valid policy");
        let mut schedule = RetrySchedule::new(policy);

        assert_eq!(schedule.attempt_failed(), Some(Duration::from_millis(200)));
        assert_eq!(schedule.attempt_failed(), Some(Duration::from_millis(400)));
        assert_eq!(schedule.attempt_failed(), None);
        assert_eq!(schedule.attempt_failed(), None);
        assert_eq!(schedule.attempts_used(), 3);
    }

    #[test]
    fn a_single_attempt_policy_never_retries() {
        let policy = RetryPolicy::new(1, Duration::from_millis(1), Duration::from_millis(1))
            .expect("a valid policy");
        let mut schedule = RetrySchedule::new(policy);

        assert_eq!(schedule.attempt_failed(), None);
        assert_eq!(schedule.attempts_used(), 1);
    }

    #[test]
    fn a_success_gives_the_budget_back() {
        let policy = RetryPolicy::default();
        let mut schedule = RetrySchedule::new(policy);
        while schedule.attempt_failed().is_some() {}
        assert_eq!(schedule.attempt_failed(), None);

        schedule.reset();

        assert_eq!(schedule.attempts_used(), 0);
        assert!(schedule.attempt_failed().is_some());
        assert_eq!(schedule.policy(), policy);
    }

    #[test]
    fn the_default_policy_is_bounded() {
        let policy = RetryPolicy::default();

        assert!(policy.attempts() >= 1);
        assert!(policy.attempts() <= RetryPolicy::MAX_ATTEMPTS);
    }

    #[test]
    fn the_catalog_has_an_id_for_the_default_services() {
        let english = localizer(Locale::default_locale()).expect("the shipped catalogs parse");

        for state in [ServiceState::Starting, ServiceState::Ready] {
            let rendered = state.describe().render(&english);
            assert_ne!(rendered, state.describe().id(), "{rendered}");
        }
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    fn snapshot(generation: u64, windows: u8) -> Snapshot<u8> {
        Snapshot::new(SnapshotGeneration::new(generation), windows)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// A retry schedule always terminates, never exceeds its policy's
        /// attempt budget, and never waits longer than the policy allows.
        #[test]
        fn a_retry_schedule_is_bounded(
            attempts in 1u32..=RetryPolicy::MAX_ATTEMPTS,
            base_ms in 1u64..=1_000,
            factor in 1u64..=60,
        ) {
            let base_delay = Duration::from_millis(base_ms);
            let max_delay = (base_delay * u32::try_from(factor).unwrap_or(1)).min(RetryPolicy::MAX_DELAY);
            let policy = RetryPolicy::new(attempts, base_delay, max_delay).expect("a valid policy");
            let mut schedule = RetrySchedule::new(policy);

            let mut failures = 0;
            let mut delays = Vec::new();
            while let Some(delay) = schedule.attempt_failed() {
                failures += 1;
                delays.push(delay);
                prop_assert!(
                    delay <= max_delay,
                    "the delay {} exceeded the ceiling {:?}",
                    delay.as_millis(),
                    max_delay
                );
                prop_assert!(failures < attempts, "more attempts than the policy allows");
            }

            prop_assert_eq!(schedule.attempts_used(), attempts);
            prop_assert!(delays.windows(2).all(|pair| pair[0] <= pair[1]), "the delay must not shrink: {delays:?}");
        }

        /// Only the newest generation a caller issued is current, whatever order
        /// the answers come back in.
        #[test]
        fn only_the_newest_request_is_current(count in 1u32..64) {
            let generations: Vec<RequestGeneration> = (0..count)
                .scan(RequestGeneration::first(), |newest, _| {
                    let current = *newest;
                    *newest = newest.supersede();
                    Some(current)
                })
                .collect();
            let newest = *generations.last().expect("at least one generation");

            for (index, generation) in generations.iter().enumerate() {
                prop_assert_eq!(
                    generation.is_current(newest),
                    index + 1 == generations.len()
                );
            }
        }

        /// A view only ever moves forward, whatever order snapshots arrive in.
        #[test]
        fn a_view_never_rolls_back(values in prop::collection::vec((0u64..32, 0u8..8), 0..64)) {
            let mut view = SnapshotView::new(snapshot(0, 0));
            let mut highest = 0;

            for (generation, value) in values {
                let applied = view.apply(snapshot(generation, value));
                if applied {
                    prop_assert!(generation > highest);
                    highest = generation;
                    prop_assert_eq!(*view.current(), value);
                } else {
                    prop_assert!(generation <= highest);
                }
                prop_assert_eq!(view.generation(), SnapshotGeneration::new(highest));
            }
        }
    }
}
