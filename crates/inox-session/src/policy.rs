//! The two pieces of policy the supervisor owns: restart accounting, and the
//! capability the session hands to its trusted lock client.
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

/// The length in bytes of a session lock token.
///
/// Sixteen bytes is the usual "unguessable" size for a capability that is only
/// ever compared once per claim; the token is not a key and is never stored.
const LOCK_TOKEN_BYTES: usize = 16;

/// Why a session lock token could not be minted.
#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    /// The kernel's random source was unavailable, which means the token would not
    /// have been unguessable.
    #[error("the system random source is unavailable: {0}")]
    Random(rustix::io::Errno),
}

/// Mint the token that authorizes one session's lock client.
///
/// The token is the capability the session-lock and notification bus work, rule 3 describes: the compositor hands
/// it to nobody, the session delivers it to the trusted lock client, and a claim
/// without it is refused. Generating it here rather than in the compositor is what
/// keeps the compositor's policy — "a claim must match the token I was started
/// with" — independent of where the value came from.
///
/// # Errors
///
/// Returns [`TokenError::Random`] when `getrandom` fails. Falling back to a weaker
/// source would silently make the capability guessable, so this is fatal.
pub fn session_lock_token() -> Result<String, TokenError> {
    let mut bytes = [0u8; LOCK_TOKEN_BYTES];
    rustix::rand::getrandom(&mut bytes, rustix::rand::GetRandomFlags::empty())
        .map_err(TokenError::Random)?;

    let mut token = String::with_capacity(LOCK_TOKEN_BYTES * 2);
    for byte in bytes {
        // Lower-case hex, written out rather than pulled in: one dependency for
        // sixteen bytes of formatting is not a trade this workspace makes.
        token.push(HEX[(byte >> 4) as usize] as char);
        token.push(HEX[(byte & 0x0f) as usize] as char);
    }

    Ok(token)
}

/// The alphabet for the token's hexadecimal encoding.
const HEX: &[u8; 16] = b"0123456789abcdef";

#[cfg(test)]
mod token_tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn a_token_is_lower_case_hex_of_the_right_length() {
        let token = session_lock_token().expect("the system random source is available");

        assert_eq!(token.len(), LOCK_TOKEN_BYTES * 2);
        assert!(
            token
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()),
            "{token}"
        );
    }

    #[test]
    fn tokens_do_not_repeat() {
        // A capability that repeats is a capability that leaks across sessions; the
        // point of the assertion is that the source is actually consulted.
        let mut seen = HashSet::new();
        for _ in 0..64 {
            let token = session_lock_token().expect("the system random source is available");
            assert!(seen.insert(token), "a token repeated within 64 draws");
        }
    }
}
