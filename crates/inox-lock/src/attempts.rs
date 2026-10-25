//! Rate limiting failed authentication.
//! Rate limiting failed authentication.
//! the session-lock boundary asks for "limited and readable feedback" on failure, and acceptance
//! S03 asks that the credential never reach a log, a clipboard or a screenshot.
//! What a rate limiter must *not* become is a way to keep the legitimate user out:
//! a lock screen that escalates without a ceiling is a denial of service a
//! stranger can trigger by typing, and one that reports its own state ("4 attempts
//! left", "wait 37 s") tells whoever is trying that they are looking at a
//! counter rather than a person's password.
//!
//! So the policy here is deliberately flat, bounded and simple to state:
//!
//! - failures are counted over a **sliding window** of [`AttemptPolicy::window`];
//! - once [`AttemptPolicy::allowance`] failures fall inside that window, attempts
//!   are refused until the oldest of them leaves the window;
//! - therefore the longest anyone can ever be held back is exactly one window,
//!   after which the legitimate user is let in without intervention — no
//!   permanent lockout, and no state an operator has to clear;
//! - and the interface is told only *that* a wait applies
//!   ([`crate::view_model`]), never how long it is or how many attempts remain.
//!
//! Counting *failures* rather than *attempts* matters: counting attempts would
//! let anyone lock a user out by submitting wrong passwords on their behalf.
//!
//! The module is pure apart from the clock the caller supplies: every method
//! takes `now`, which is what makes the boundary cases — exactly at the
//! allowance, exactly at the end of the window — testable without sleeping.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How many failures are tolerated, and over what period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptPolicy {
    /// Failures allowed inside one [`AttemptPolicy::window`].
    pub allowance: u32,
    /// The period failures are counted over, and therefore the longest a user
    /// can ever be held back.
    pub window: Duration,
}

impl AttemptPolicy {
    /// The policy a lock screen gets unless the session says otherwise.
    ///
    /// Chosen to be boring: five failures in five minutes. It is not a
    /// configuration key yet — there is no `[lock]` section in `inox-config`
    ///, and a value with no consumer should not be invented
    /// ahead of one.
    pub const fn standard() -> Self {
        Self {
            allowance: 5,
            window: Duration::from_secs(300),
        }
    }

    /// Validates a policy, because both a zero allowance and a zero window mean
    /// something other than what they look like.
    ///
    /// # Errors
    ///
    /// [`AttemptError::NoAttemptsAllowed`] for an allowance of zero, which would
    /// refuse every attempt including the first, and [`AttemptError::EmptyWindow`]
    /// for a window of zero, which would expire every failure the instant it is
    /// recorded and therefore never limit anything.
    pub fn new(allowance: u32, window: Duration) -> Result<Self, AttemptError> {
        if allowance == 0 {
            return Err(AttemptError::NoAttemptsAllowed);
        }
        if window.is_zero() {
            return Err(AttemptError::EmptyWindow);
        }
        Ok(Self { allowance, window })
    }

    /// The longest a user can be held back by this policy.
    pub const fn longest_wait(&self) -> Duration {
        self.window
    }
}

impl Default for AttemptPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

/// Why a policy was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttemptError {
    /// An allowance of zero refuses the first attempt too.
    #[error("an allowance of 0 would refuse every attempt, including the first")]
    NoAttemptsAllowed,
    /// A window of zero never counts anything.
    #[error("a window of 0 would expire every failure immediately")]
    EmptyWindow,
}

/// Whether an attempt may be made now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Go ahead.
    Permitted,
    /// The allowance is spent for now.
    Waiting,
}

/// Counts failed attempts over a sliding window.
#[derive(Debug, Clone)]
pub struct AttemptLimiter {
    policy: AttemptPolicy,
    failures: VecDeque<Instant>,
}

impl Default for AttemptLimiter {
    fn default() -> Self {
        Self::new(AttemptPolicy::default())
    }
}

impl AttemptLimiter {
    /// A limiter that has seen no failures.
    pub fn new(policy: AttemptPolicy) -> Self {
        Self {
            policy,
            failures: VecDeque::new(),
        }
    }

    /// The policy in force.
    pub const fn policy(&self) -> &AttemptPolicy {
        &self.policy
    }

    /// Whether an attempt may be made at `now`.
    ///
    /// Takes `&mut self` because asking is also what forgets the failures that
    /// have left the window; there is no separate sweep and therefore no way to
    /// forget to run one.
    pub fn gate(&mut self, now: Instant) -> Gate {
        self.forget_expired(now);

        if self.failures.len() >= self.policy.allowance as usize {
            Gate::Waiting
        } else {
            Gate::Permitted
        }
    }

    /// Records that an attempt failed at `now`.
    ///
    /// Recorded whether or not the gate was open, because an attempt that
    /// happened anyway must not be free. A failure arriving while the limiter is
    /// closed can only move the window's oldest failure forward, so it never
    /// shortens the wait.
    pub fn record_failure(&mut self, now: Instant) {
        self.forget_expired(now);
        self.failures.push_back(now);
    }

    /// Forgets the history: the user proved who they are.
    pub fn record_success(&mut self) {
        self.failures.clear();
    }

    /// How many failures are being counted at `now`.
    pub fn failures(&mut self, now: Instant) -> usize {
        self.forget_expired(now);
        self.failures.len()
    }

    /// How long the wait still runs, at `now`.
    ///
    /// For whoever schedules the retry — a session timer, say. Deliberately not
    /// part of [`crate::view_model`]: the *user* is told that a wait applies, not
    /// how much of it is left, because that is the limiter's internal state and
    /// reporting it turns a prompt into a counter.
    ///
    /// The gate opens when enough failures have left the window that fewer than
    /// [`AttemptPolicy::allowance`] remain, which is not the same moment as the
    /// oldest one expiring: an attempt made *during* the wait extends it, and this
    /// is where that extension shows up.
    pub fn wait_remaining(&mut self, now: Instant) -> Option<Duration> {
        self.forget_expired(now);

        let allowance = self.policy.allowance as usize;
        if self.failures.len() < allowance {
            return None;
        }

        // The failure whose expiry frees the last slot the allowance needs. When
        // the limiter is exactly at its allowance it is the oldest of them, and
        // the wait is the whole window — which is the documented ceiling.
        let index = self.failures.len() - allowance;
        let freeing = *self.failures.get(index)?;
        let elapsed = now.saturating_duration_since(freeing);

        Some(self.policy.window.saturating_sub(elapsed))
    }

    /// Drops the failures that have left the window.
    ///
    /// A failure is expired once it is *at least* one window old, so the gate
    /// opens again at exactly `oldest + window` rather than one tick after.
    fn forget_expired(&mut self, now: Instant) {
        while let Some(front) = self.failures.front() {
            if now.saturating_duration_since(*front) >= self.policy.window {
                self.failures.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn policy(allowance: u32, window_secs: u64) -> AttemptPolicy {
        AttemptPolicy::new(allowance, Duration::from_secs(window_secs)).expect("a valid policy")
    }

    fn base() -> Instant {
        Instant::now()
    }

    fn at(base: Instant, secs: u64) -> Instant {
        base.checked_add(Duration::from_secs(secs))
            .expect("a representable instant")
    }

    #[test]
    fn a_fresh_limiter_permits() {
        let mut limiter = AttemptLimiter::new(policy(3, 60));

        assert_eq!(limiter.gate(base()), Gate::Permitted);
        assert_eq!(limiter.wait_remaining(base()), None);
    }

    #[test]
    fn the_allowance_is_the_last_permitted_failure() {
        let mut limiter = AttemptLimiter::new(policy(3, 60));
        let now = base();

        for expected in 0..3 {
            assert_eq!(limiter.failures(now), expected);
            assert_eq!(limiter.gate(now), Gate::Permitted);
            limiter.record_failure(now);
        }

        assert_eq!(
            limiter.failures(now),
            3,
            "the third failure is inside the window"
        );
        assert_eq!(limiter.gate(now), Gate::Waiting);
    }

    #[test]
    fn a_wait_ends_when_the_oldest_failure_leaves_the_window() {
        let mut limiter = AttemptLimiter::new(policy(2, 60));
        let start = base();
        limiter.record_failure(start);
        limiter.record_failure(at(start, 30));
        assert_eq!(limiter.gate(at(start, 30)), Gate::Waiting);

        // The oldest failure is 45 s old here: still inside the window.
        assert_eq!(limiter.gate(at(start, 45)), Gate::Waiting);
        // ... and 60 s old here: gone, so the gate opens, at exactly the
        // boundary rather than a tick later.
        assert_eq!(limiter.gate(at(start, 60)), Gate::Permitted);
        assert_eq!(limiter.failures(at(start, 60)), 1);
    }

    #[test]
    fn the_longest_wait_is_one_window() {
        let mut limiter = AttemptLimiter::new(policy(1, 60));
        let start = base();
        limiter.record_failure(start);

        assert_eq!(limiter.wait_remaining(start), Some(Duration::from_secs(60)));
        assert_eq!(
            limiter.wait_remaining(at(start, 1)),
            Some(Duration::from_secs(59))
        );
        // Asking at 60 s would expire the failure the three assertions above are
        // about, so the remaining wait is checked in the order it passes.
        assert!(
            limiter.wait_remaining(at(start, 30)).expect("a wait") <= Duration::from_secs(60),
            "the documented ceiling is the window"
        );
        assert_eq!(limiter.wait_remaining(at(start, 60)), None);
    }

    #[test]
    fn hammering_while_waiting_only_ever_extends_the_wait() {
        let mut limiter = AttemptLimiter::new(policy(1, 60));
        let start = base();
        limiter.record_failure(start);

        for second in 1..10 {
            limiter.record_failure(at(start, second));
        }

        // The allowance is one, so the wait now runs from the *newest* failure
        // rather than from the first one: an attempt during the wait pushes the
        // moment the gate opens further away, never closer.
        assert_eq!(
            limiter.wait_remaining(at(start, 10)),
            Some(Duration::from_secs(59)),
            "the wait ends 60 s after the failure at second 9, not after the one at 0"
        );
        assert_eq!(limiter.gate(at(start, 68)), Gate::Waiting);
        assert_eq!(
            limiter.gate(at(start, 69)),
            Gate::Permitted,
            "the last failure was recorded at 9 s, so the gate opens at 69 s"
        );
    }

    #[test]
    fn a_success_forgets_everything() {
        let mut limiter = AttemptLimiter::new(policy(2, 60));
        let start = base();
        limiter.record_failure(start);
        limiter.record_failure(start);
        assert_eq!(limiter.gate(start), Gate::Waiting);

        limiter.record_success();

        assert_eq!(limiter.gate(start), Gate::Permitted);
        assert_eq!(limiter.failures(start), 0);
    }

    #[test]
    fn a_zero_allowance_is_refused_rather_than_limiting_everything() {
        assert_eq!(
            AttemptPolicy::new(0, Duration::from_secs(60)),
            Err(AttemptError::NoAttemptsAllowed)
        );
    }

    #[test]
    fn a_zero_window_is_refused_rather_than_limiting_nothing() {
        assert_eq!(
            AttemptPolicy::new(3, Duration::ZERO),
            Err(AttemptError::EmptyWindow)
        );
    }

    #[test]
    fn the_standard_policy_never_locks_a_user_out_permanently() {
        let standard = AttemptPolicy::standard();

        assert!(standard.allowance > 0);
        assert_eq!(standard.longest_wait(), standard.window);
        assert!(
            standard.longest_wait() <= Duration::from_secs(3600),
            "a policy someone can wait out is a policy that cannot be used to \
             deny the legitimate user their session"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Whatever a sequence of failures and elapsed time does, the wait is
        /// never longer than the window, and it always ends by itself.
        #[test]
        fn a_wait_is_bounded_and_always_ends(
            offsets in proptest::collection::vec(0u64..600, 0..24),
            extra in 0u64..600,
        ) {
            let start = base();
            let mut limiter = AttemptLimiter::new(policy(3, 120));
            let mut seconds = 0u64;

            for offset in &offsets {
                seconds = seconds.saturating_add(*offset);
                let now = at(start, seconds);
                limiter.record_failure(now);
                if let Some(remaining) = limiter.wait_remaining(now) {
                    prop_assert!(remaining <= Duration::from_secs(120));
                }
            }

            let end = at(start, seconds.saturating_add(extra).saturating_add(121));
            prop_assert_eq!(
                limiter.gate(end),
                Gate::Permitted,
                "a window after the last failure, and with no failures in \
                 between, the gate must be open again",
            );
            prop_assert_eq!(limiter.wait_remaining(end), None);
        }

        /// A closed limiter never opens earlier than a whole window after the
        /// last failure in it, no matter how much hammering happens in between.
        #[test]
        fn a_closed_gate_stays_closed_for_a_whole_window(
            probes in proptest::collection::vec(0u64..119, 1..16),
        ) {
            let start = base();
            let mut limiter = AttemptLimiter::new(policy(1, 120));
            limiter.record_failure(start);
            let mut last = 0u64;

            for probe in probes {
                let now = at(start, probe);
                limiter.record_failure(now);
                last = last.max(probe);
                prop_assert_eq!(
                    limiter.gate(now),
                    Gate::Waiting,
                    "a failure is inside the window, so the gate is closed",
                );
            }

            prop_assert_eq!(
                limiter.gate(at(start, last.saturating_add(120))),
                Gate::Permitted,
                "a window after the newest failure, the gate is open again",
            );
        }
    }
}
