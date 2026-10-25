//! The display-trial state machine.
//!
//! A change to an output's mode, scale or position is applied live and kept only
//! if the consumer confirms it before [`TRIAL_DEADLINE`]. This module holds the
//! pure part of that rule: what one armed trial is, and what a `Confirm`, a
//! `Rollback` or the deadline itself means for the trial that is armed right now.
//! It depends on neither Smithay nor calloop nor any I/O, and every decision takes
//! `now` as an argument, so the rules below are unit-tested instead of being
//! inferred from a display.
//!
//! The rules [`decide`] implements:
//!
//! - A `Confirm` naming the armed generation before the deadline keeps the applied
//!   state and disarms the trial.
//! - A `Confirm` naming any other generation, or naming a trial already past its
//!   deadline, keeps nothing: a late or foreign confirmation must never fix a mode
//!   the user did not see. The caller reports both as failures.
//! - A `Rollback` naming the armed generation restores the previous preferences
//!   whether or not the deadline has passed: the consumer's countdown and the
//!   compositor's timer are the same rule, so both may run and the result is the
//!   same.
//! - A `Rollback` naming any other generation (already confirmed, already rolled
//!   back, timed out or superseded) changes nothing and is still a success. It is
//!   idempotent because the settings window races the compositor's own timer at
//!   the 30 second mark.
//! - The deadline restores the previous preferences exactly once. It is not
//!   reported before the deadline, and after the trial has been disarmed it is a
//!   no-op.

use std::time::{Duration, Instant};

use inox_config::OutputConfig;

/// How long a display trial stays armed without a confirmation.
pub const TRIAL_DEADLINE: Duration = Duration::from_secs(30);

/// One display trial that has been applied and is waiting for a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmedTrial {
    generation: u64,
    previous: Vec<OutputConfig>,
    deadline: Instant,
}

impl ArmedTrial {
    /// Arm a trial for `generation` that expires at `deadline`.
    ///
    /// `previous` is the set of output preferences that was in force before the
    /// trial was applied; it is what a rollback and the deadline put back.
    pub fn new(generation: u64, previous: Vec<OutputConfig>, deadline: Instant) -> Self {
        Self {
            generation,
            previous,
            deadline,
        }
    }

    /// The consumer generation this trial answers to.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The output preferences that were in force before the trial was applied.
    pub fn previous(&self) -> &[OutputConfig] {
        &self.previous
    }

    /// The instant the trial expires.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Whether `now` is at or past the deadline.
    pub fn expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }
}

/// Which request the runtime is asking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// `Confirm(generation)` from the trial interface.
    Confirm(u64),
    /// `Rollback(generation)` from the trial interface.
    Rollback(u64),
    /// The armed trial's deadline has been reached.
    Deadline,
}

/// What one [`Request`] means for the trial that is armed right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Keep the applied state and disarm the trial.
    Keep,
    /// Restore the previous output preferences and disarm the trial.
    ///
    /// Reached by a `Rollback` naming the armed trial — in time or already
    /// expired — and by a `Confirm` or the deadline once the deadline has passed.
    Revert,
    /// The request named a generation that is not the armed trial.
    ///
    /// Nothing changes. A `Confirm` must be reported as a failure; a `Rollback`
    /// still succeeds with the current snapshot.
    Stale,
    /// No trial is armed and a `Confirm` has nothing to confirm: it must fail.
    NoTrial,
    /// No trial is armed and a `Rollback` is an idempotent no-op that succeeds.
    Idle,
    /// The deadline has not been reached yet: leave the trial armed.
    NotYet,
}

/// Decide what `request` means for `trial` at `now`.
///
/// This is the whole rule; the runtime only carries out what it returns.
pub fn decide(trial: Option<&ArmedTrial>, request: Request, now: Instant) -> Verdict {
    let Some(trial) = trial else {
        return match request {
            Request::Confirm(_) => Verdict::NoTrial,
            Request::Rollback(_) | Request::Deadline => Verdict::Idle,
        };
    };

    match request {
        Request::Confirm(generation) => {
            if generation != trial.generation {
                Verdict::Stale
            } else if trial.expired(now) {
                Verdict::Revert
            } else {
                Verdict::Keep
            }
        }
        Request::Rollback(generation) => {
            if generation != trial.generation {
                Verdict::Stale
            } else {
                Verdict::Revert
            }
        }
        Request::Deadline => {
            if trial.expired(now) {
                Verdict::Revert
            } else {
                Verdict::NotYet
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armed(generation: u64, now: Instant) -> ArmedTrial {
        ArmedTrial::new(
            generation,
            vec![OutputConfig::default()],
            now + TRIAL_DEADLINE,
        )
    }

    #[test]
    fn a_matching_generation_confirms() {
        let now = Instant::now();
        let trial = armed(7, now);

        assert_eq!(
            decide(Some(&trial), Request::Confirm(7), now),
            Verdict::Keep
        );
        assert_eq!(
            decide(Some(&trial), Request::Confirm(7), now + TRIAL_DEADLINE),
            Verdict::Revert,
            "a confirmation at the deadline is late"
        );
    }

    #[test]
    fn the_previous_preferences_are_what_a_trial_rolls_back_to() {
        let now = Instant::now();
        let previous = vec![OutputConfig {
            name: Some("HDMI-A-1".to_owned()),
            ..OutputConfig::default()
        }];
        let deadline = now + TRIAL_DEADLINE;
        let trial = ArmedTrial::new(3, previous.clone(), deadline);

        assert_eq!(trial.generation(), 3);
        assert_eq!(trial.deadline(), deadline);
        assert_eq!(trial.previous(), previous.as_slice());
        assert!(!trial.expired(now));
        assert!(trial.expired(deadline));
    }

    #[test]
    fn a_stale_generation_never_disarms_or_keeps_a_trial() {
        let now = Instant::now();
        let trial = armed(7, now);

        assert_eq!(
            decide(Some(&trial), Request::Confirm(8), now),
            Verdict::Stale
        );
        assert_eq!(
            decide(Some(&trial), Request::Rollback(8), now),
            Verdict::Stale
        );
        // The stale requests changed nothing: the armed trial still confirms.
        assert_eq!(
            decide(Some(&trial), Request::Confirm(7), now),
            Verdict::Keep
        );
    }

    #[test]
    fn requests_without_a_trial_are_idle_or_fail() {
        let now = Instant::now();

        assert_eq!(decide(None, Request::Confirm(1), now), Verdict::NoTrial);
        assert_eq!(decide(None, Request::Rollback(1), now), Verdict::Idle);
        assert_eq!(decide(None, Request::Deadline, now), Verdict::Idle);
        assert_eq!(
            decide(None, Request::Rollback(1), now + TRIAL_DEADLINE),
            Verdict::Idle,
            "rollback of an unknown generation stays idempotent"
        );
    }

    #[test]
    fn expiry_is_reported_once_the_deadline_passes_and_not_before() {
        let now = Instant::now();
        let trial = armed(1, now);

        assert_eq!(
            decide(Some(&trial), Request::Deadline, now),
            Verdict::NotYet
        );
        assert_eq!(
            decide(
                Some(&trial),
                Request::Deadline,
                now + TRIAL_DEADLINE - Duration::from_nanos(1)
            ),
            Verdict::NotYet
        );
        assert_eq!(
            decide(Some(&trial), Request::Deadline, now + TRIAL_DEADLINE),
            Verdict::Revert
        );
        assert_eq!(
            decide(
                Some(&trial),
                Request::Deadline,
                now + TRIAL_DEADLINE + Duration::from_secs(1)
            ),
            Verdict::Revert
        );
    }

    #[test]
    fn a_rollback_of_the_armed_generation_reverts_even_after_the_deadline() {
        let now = Instant::now();
        let trial = armed(11, now);

        assert_eq!(
            decide(Some(&trial), Request::Rollback(11), now),
            Verdict::Revert
        );
        assert_eq!(
            decide(
                Some(&trial),
                Request::Rollback(11),
                now + TRIAL_DEADLINE + Duration::from_secs(5)
            ),
            Verdict::Revert,
            "the consumer's countdown and the timer are the same rule"
        );
    }
}
