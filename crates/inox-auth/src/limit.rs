//! The rolling allowance that bounds how often the boundary will try.
//!
//! The login screen already counts its own attempts (ADR 0021's model refuses a
//! fourth submission in a minute), and this is the same rule enforced on the side
//! that actually has the privilege. It matters because the two are not the same
//! trust domain: the client's counter is a courtesy to the person typing, and
//! this one is what stops a client that ignores it — or a different client
//! altogether — from spending `libpam`'s time without limit.
//!
//! What it deliberately is not: a lockout. The window slides, so the allowance
//! comes back on its own, and nothing here is persisted across restarts. A
//! display manager that can be locked out permanently by three wrong passwords is
//! a display manager that hands anyone at the keyboard a denial of service.
//!
//! The window is a parameter rather than a constant because the *policy* belongs
//! to the ADR and the *mechanism* to this type; the shipped binary passes the
//! values ADR 0029 fixes.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How many attempts are allowed inside one rolling window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatePolicy {
    /// The most attempts allowed inside [`RatePolicy::window`].
    pub attempts: usize,
    /// How long an attempt keeps counting.
    pub window: Duration,
}

impl Default for RatePolicy {
    /// Three attempts a minute: ADR 0015's lock-screen rule, reused so that the
    /// two components cannot drift apart in what they consider too many.
    fn default() -> Self {
        Self {
            attempts: 3,
            window: Duration::from_secs(60),
        }
    }
}

/// When the attempts happened, and how many of them still count.
#[derive(Debug)]
pub struct AttemptLog {
    policy: RatePolicy,
    attempts: VecDeque<Instant>,
}

impl AttemptLog {
    /// An empty log under `policy`.
    pub fn new(policy: RatePolicy) -> Self {
        Self {
            policy,
            attempts: VecDeque::new(),
        }
    }

    /// Whether another attempt is allowed at `now`.
    pub fn allows(&mut self, now: Instant) -> bool {
        self.discard_expired(now);
        self.attempts.len() < self.policy.attempts
    }

    /// Counts an attempt that is about to be made.
    ///
    /// Recorded before the authentication runs, not after it succeeds: an attempt
    /// that reaches `libpam` is what the allowance is for, and a failure is
    /// exactly the case this exists to bound.
    pub fn record(&mut self, now: Instant) {
        self.discard_expired(now);
        self.attempts.push_back(now);
    }

    /// How many attempts still count at `now`.
    pub fn counted(&mut self, now: Instant) -> usize {
        self.discard_expired(now);

        self.attempts.len()
    }

    /// Forgets attempts that have left the window.
    fn discard_expired(&mut self, now: Instant) {
        while let Some(oldest) = self.attempts.front() {
            if now.saturating_duration_since(*oldest) < self.policy.window {
                break;
            }
            self.attempts.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log() -> AttemptLog {
        AttemptLog::new(RatePolicy::default())
    }

    #[test]
    fn the_first_three_attempts_are_allowed_and_the_fourth_is_not() {
        let now = Instant::now();
        let mut log = log();

        for index in 0..3 {
            assert!(log.allows(now), "attempt {index} should be allowed");
            log.record(now);
        }

        assert!(!log.allows(now));
        assert_eq!(log.counted(now), 3);
    }

    #[test]
    fn the_window_slides_rather_than_locking_anyone_out() {
        let now = Instant::now();
        let mut log = log();

        for _ in 0..3 {
            log.record(now);
        }
        assert!(!log.allows(now));

        // One window later the allowance is back, and it is the *sliding* window
        // that says so: nothing had to be reset.
        assert!(log.allows(now + Duration::from_secs(60)));
        assert_eq!(log.counted(now + Duration::from_secs(60)), 0);
    }

    #[test]
    fn attempts_leave_the_window_one_at_a_time() {
        let now = Instant::now();
        let mut log = log();

        log.record(now);
        log.record(now + Duration::from_secs(30));
        log.record(now + Duration::from_secs(30));

        // The first attempt is 60 s old and no longer counts.
        assert!(log.allows(now + Duration::from_secs(60)));
        assert_eq!(log.counted(now + Duration::from_secs(60)), 2);

        // 30 s later the other two are gone as well.
        assert_eq!(log.counted(now + Duration::from_secs(90)), 0);
    }

    #[test]
    fn exactly_at_the_window_boundary_an_attempt_still_counts() {
        // The comparison is `< window`, so an attempt recorded exactly one window
        // ago has left. Stating it here keeps the boundary from drifting silently
        // in either direction.
        let now = Instant::now();
        let mut log = log();
        log.record(now);

        assert_eq!(
            log.counted(now + Duration::from_secs(59)),
            1,
            "inside the window"
        );
        assert_eq!(
            log.counted(now + Duration::from_secs(60)),
            0,
            "the window passed"
        );
    }

    #[test]
    fn a_zero_attempt_policy_refuses_everything() {
        let now = Instant::now();
        let mut log = AttemptLog::new(RatePolicy {
            attempts: 0,
            window: Duration::from_secs(60),
        });

        assert!(!log.allows(now));
        log.record(now);
        assert_eq!(log.counted(now), 1, "recording is not the same as allowing");
    }
}
