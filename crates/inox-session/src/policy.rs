//! Monotonic restart accounting shared by startup and runtime failures.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// At most three automatic restarts in a rolling sixty-second window.
#[derive(Debug, Default)]
pub struct RestartBudget {
    attempts: VecDeque<Instant>,
}

impl RestartBudget {
    /// Reserve a restart and return its backoff, or refuse when exhausted.
    /// A reservation counts even if spawning the replacement fails.
    pub fn reserve(&mut self, now: Instant) -> Option<Duration> {
        while self.attempts.front().is_some_and(|previous| {
            now.saturating_duration_since(*previous) >= Duration::from_secs(60)
        }) {
            self.attempts.pop_front();
        }
        if self.attempts.len() >= 3 {
            return None;
        }
        let delay = Duration::from_millis(250 * (1 << self.attempts.len()));
        self.attempts.push_back(now);
        Some(delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_budget_is_bounded_and_recovers_at_the_boundary() {
        let now = Instant::now();
        let mut budget = RestartBudget::default();
        assert_eq!(budget.reserve(now), Some(Duration::from_millis(250)));
        assert_eq!(budget.reserve(now), Some(Duration::from_millis(500)));
        assert_eq!(budget.reserve(now), Some(Duration::from_secs(1)));
        assert_eq!(budget.reserve(now + Duration::from_secs(59)), None);
        assert_eq!(
            budget.reserve(now + Duration::from_secs(60)),
            Some(Duration::from_millis(250))
        );
    }

    #[test]
    fn expiry_is_per_attempt_rather_than_a_fixed_window() {
        let now = Instant::now();
        let mut budget = RestartBudget::default();
        for seconds in [0, 10, 20] {
            assert!(budget.reserve(now + Duration::from_secs(seconds)).is_some());
        }
        assert!(budget.reserve(now + Duration::from_secs(60)).is_some());
        assert_eq!(budget.reserve(now + Duration::from_secs(61)), None);
    }
}
