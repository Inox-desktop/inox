//! Login transitions. Only a current reply from the reviewed boundary may start a session.
use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

impl fmt::Display for AttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AttemptId {
    /// An identifier no model ever issued.
    ///
    /// Test-only: it exists so a test can express "an answer arrived from
    /// nowhere", which is the shape a forged or replayed success takes, without
    /// making the identifier constructible in the code that has to trust it.
    #[cfg(test)]
    pub const fn that_was_never_issued(raw: u64) -> Self {
        Self(raw)
    }
}

/// The most bytes of identity data the model retains.
///
/// A floor rather than a policy: it bounds what a paste can put into the model,
/// so a window that receives a megabyte of text still has a finite amount of it
/// to keep, to lay out and to hand to the system authenticator.
pub const MAX_IDENTITY_BYTES: usize = 128;

/// Correlation identifier for a single authentication and session-start attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptId(u64);

/// Visible lifecycle; no authentication implementation is embedded here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No reviewed authentication transport is connected.
    Unavailable,
    /// A user may submit a name.
    Ready,
    /// Waiting for the system authenticator.
    Authenticating,
    /// Authenticated; waiting for the display manager to confirm session startup.
    Starting,
    /// A rejected attempt or failed transport. A new attempt may be submitted.
    Failed,
    /// The display manager confirmed session startup.
    SessionStarted,
}

/// Result supplied exclusively by the reviewed system boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthReply {
    /// The boundary authenticated the current user.
    Accepted,
    /// Authentication was rejected.
    Rejected,
    /// The authenticator failed or disconnected.
    Unavailable,
}

/// Events from UI actions, a monotonic timer and the trusted system transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Change the username while no request is in flight.
    Username(String),
    /// Begin authentication.
    Submit,
    /// Cancel and clear the user identity.
    Cancel,
    /// Reply to the current authentication request.
    AuthAnswered(AttemptId, AuthReply),
    /// The display manager finished launching the authenticated session.
    SessionAnswered(AttemptId, bool),
    /// Enforce the finite reply deadline.
    Tick,
}

/// Requests for a future reviewed transport; these are never executed by the preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Authenticate the named user through the system boundary.
    Authenticate {
        /// Request to correlate.
        id: AttemptId,
        /// Validated user identity.
        username: String,
    },
    /// Start the fixed Inox session for the authenticated identity.
    StartSession(AttemptId),
    /// Cancel the old conversation on the system boundary.
    Cancel(AttemptId),
    /// No transport request.
    None,
}

/// User identity and correlation state. This type never stores a password.
#[derive(Debug)]
pub struct Greeter {
    username: String,
    identity_too_long: bool,
    phase: Phase,
    available: bool,
    serial: u64,
    pending: Option<(AttemptId, Instant)>,
    attempts: VecDeque<Instant>,
}
impl Greeter {
    /// Create a greeter with the actual reviewed transport availability.
    /// The shipped binary always supplies false until that transport is approved.
    pub fn new(available: bool) -> Self {
        Self {
            username: String::new(),
            identity_too_long: false,
            phase: if available {
                Phase::Ready
            } else {
                Phase::Unavailable
            },
            available,
            serial: 0,
            pending: None,
            attempts: VecDeque::new(),
        }
    }
    /// The current display phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether a reviewed authentication transport is connected at all.
    pub const fn available(&self) -> bool {
        self.available
    }
    /// The username being edited or authenticated.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Whether the last identity the user supplied was longer than the model
    /// keeps, and was therefore cut down to [`MAX_IDENTITY_BYTES`].
    ///
    /// The interface shows this rather than silently submitting a name the user
    /// did not type.
    pub const fn identity_too_long(&self) -> bool {
        self.identity_too_long
    }
    /// Whether submission is currently enabled, including the rolling rate limit.
    pub fn can_submit(&self, now: Instant) -> bool {
        self.available
            && !self.identity_too_long
            && matches!(self.phase, Phase::Ready | Phase::Failed)
            && valid_username(&self.username)
            && self
                .attempts
                .iter()
                .filter(|t| now.saturating_duration_since(**t) < Duration::from_secs(60))
                .count()
                < 3
    }
    /// Apply one event, rejecting stale, duplicated and expired successes.
    pub fn apply(&mut self, event: Event, now: Instant) -> Command {
        if self.phase == Phase::SessionStarted {
            return Command::None;
        }
        if self.pending.is_some_and(|(_, deadline)| now >= deadline) {
            let old = self.pending.take();
            self.phase = Phase::Failed;
            if let Some((id, _)) = old {
                return Command::Cancel(id);
            }
        }
        match event {
            Event::Username(value)
                if self.pending.is_none() && self.phase != Phase::SessionStarted =>
            {
                // Bound retained identity data even when the UI receives a large paste.
                self.identity_too_long = value.len() > MAX_IDENTITY_BYTES;
                self.username = value.chars().take(MAX_IDENTITY_BYTES).collect();
                Command::None
            }
            Event::Submit if self.can_submit(now) => {
                let Some(serial) = self.serial.checked_add(1) else {
                    self.available = false;
                    self.phase = Phase::Unavailable;
                    return Command::None;
                };
                self.serial = serial;
                let id = AttemptId(serial);
                self.pending = Some((id, now + Duration::from_secs(30)));
                self.phase = Phase::Authenticating;
                self.attempts
                    .retain(|t| now.saturating_duration_since(*t) < Duration::from_secs(60));
                self.attempts.push_back(now);
                Command::Authenticate {
                    id,
                    username: self.username.clone(),
                }
            }
            Event::Cancel => {
                self.username.clear();
                self.identity_too_long = false;
                self.phase = if self.available {
                    Phase::Ready
                } else {
                    Phase::Unavailable
                };
                self.pending
                    .take()
                    .map_or(Command::None, |(id, _)| Command::Cancel(id))
            }
            Event::AuthAnswered(id, reply)
                if self.phase == Phase::Authenticating
                    && self.pending.is_some_and(|(current, _)| current == id) =>
            {
                match reply {
                    AuthReply::Accepted => {
                        self.phase = Phase::Starting;
                        self.pending = Some((id, now + Duration::from_secs(30)));
                        Command::StartSession(id)
                    }
                    AuthReply::Rejected => {
                        self.pending = None;
                        self.phase = Phase::Failed;
                        Command::None
                    }
                    AuthReply::Unavailable => {
                        self.pending = None;
                        self.available = false;
                        self.phase = Phase::Unavailable;
                        Command::None
                    }
                }
            }
            Event::SessionAnswered(id, success)
                if self.phase == Phase::Starting
                    && self.pending.is_some_and(|(current, _)| current == id) =>
            {
                self.pending = None;
                self.phase = if success {
                    Phase::SessionStarted
                } else {
                    Phase::Failed
                };
                Command::None
            }
            _ => Command::None,
        }
    }
}

/// Conservative portable account names, never shell command strings.
pub fn valid_username(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && name.len() <= MAX_IDENTITY_BYTES
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn submit(greeter: &mut Greeter, now: Instant) -> AttemptId {
        greeter.apply(Event::Username("alice".into()), now);
        match greeter.apply(Event::Submit, now) {
            Command::Authenticate { id, .. } => id,
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn no_boundary_never_authenticates_or_starts_a_session() {
        let now = Instant::now();
        let mut greeter = Greeter::new(false);
        greeter.apply(Event::Username("alice".into()), now);
        assert_eq!(greeter.apply(Event::Submit, now), Command::None);
        assert_eq!(
            greeter.apply(Event::AuthAnswered(AttemptId(1), AuthReply::Accepted), now),
            Command::None
        );
        assert_eq!(greeter.phase(), Phase::Unavailable);
    }
    #[test]
    fn only_one_current_success_can_request_session_start() {
        let now = Instant::now();
        let mut greeter = Greeter::new(true);
        let old = submit(&mut greeter, now);
        assert_eq!(greeter.apply(Event::Cancel, now), Command::Cancel(old));
        assert_eq!(greeter.username(), "");
        let current = submit(&mut greeter, now);
        assert_eq!(
            greeter.apply(Event::AuthAnswered(old, AuthReply::Accepted), now),
            Command::None
        );
        assert_eq!(
            greeter.apply(Event::SessionAnswered(current, true), now),
            Command::None
        );
        assert_eq!(greeter.phase(), Phase::Authenticating);
        assert_eq!(
            greeter.apply(Event::AuthAnswered(current, AuthReply::Accepted), now),
            Command::StartSession(current)
        );
        assert_eq!(
            greeter.apply(Event::AuthAnswered(current, AuthReply::Accepted), now),
            Command::None
        );
        assert_eq!(greeter.phase(), Phase::Starting);
        greeter.apply(Event::SessionAnswered(current, true), now);
        assert_eq!(greeter.phase(), Phase::SessionStarted);
        assert_eq!(greeter.apply(Event::Cancel, now), Command::None);
        assert_eq!(greeter.phase(), Phase::SessionStarted);
    }
    #[test]
    fn failures_and_expired_success_never_start_sessions() {
        let now = Instant::now();
        for reply in [AuthReply::Rejected, AuthReply::Unavailable] {
            let mut greeter = Greeter::new(true);
            let id = submit(&mut greeter, now);
            assert_eq!(
                greeter.apply(Event::AuthAnswered(id, reply), now),
                Command::None
            );
            assert_ne!(greeter.phase(), Phase::Starting);
        }
        let mut greeter = Greeter::new(true);
        let id = submit(&mut greeter, now);
        assert_eq!(
            greeter.apply(
                Event::AuthAnswered(id, AuthReply::Accepted),
                now + Duration::from_secs(30)
            ),
            Command::Cancel(id)
        );
        assert_eq!(greeter.phase(), Phase::Failed);
    }
    #[test]
    fn cancellations_do_not_bypass_the_rolling_attempt_limit() {
        let now = Instant::now();
        let mut greeter = Greeter::new(true);
        for _ in 0..3 {
            submit(&mut greeter, now);
            greeter.apply(Event::Cancel, now);
        }
        greeter.apply(Event::Username("alice".into()), now);
        assert!(!greeter.can_submit(now));
        assert!(greeter.can_submit(now + Duration::from_secs(60)));
    }
    #[test]
    fn invalid_names_and_oversized_identity_data_are_bounded() {
        for name in [
            "", "-user", "0user", "alice\n", "alice;id", "../root", "a b",
        ] {
            assert!(!valid_username(name));
        }
        assert!(valid_username("alice-1.test"));
        let mut greeter = Greeter::new(true);
        greeter.apply(Event::Username("a".repeat(10000)), Instant::now());
        assert_eq!(greeter.username().len(), MAX_IDENTITY_BYTES);
        assert!(!greeter.can_submit(Instant::now()));
        assert!(
            greeter.identity_too_long(),
            "the interface has to be able to say that the name was cut down"
        );

        let mut greeter = Greeter::new(true);
        greeter.apply(Event::Username("alice".into()), Instant::now());
        assert!(!greeter.identity_too_long());
    }
}
