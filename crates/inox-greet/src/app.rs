//! Qt-free UI logic of the preview: the one place a QML event is interpreted.
//!
//! The QML surface only renders and emits [`Action`]s. Turning an action into a
//! [`Greeter`] event — and every authentication/session decision that follows
//! from it — stays here, in safe Rust (ADR 0036 §失败、恢复与安全). Credentials
//! are not part of this type: the preview collects a public account name and
//! nothing else, and the shipped binary always constructs the greeter with the
//! reviewed transport unavailable.

use std::time::Instant;

use crate::state::{Command, Event, Greeter, valid_username};

/// A user action from the preview surface.
///
/// There is deliberately no "submit credentials" variant: the login button is
/// disabled and [`Action::Activate`] cannot start a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Edit the public account name.
    EditUsername(String),
    /// Clear the identity.
    Clear,
    /// Activate the focused control; login stays disabled.
    Activate,
}

/// The explanation the surface shows below the account field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// Login is unavailable until the authentication boundary is reviewed.
    Blocked,
    /// The typed identity is not a valid account name.
    InvalidUser,
}

impl Notice {
    /// The Fluent message id that explains this notice.
    pub fn message_id(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::InvalidUser => "invalid-user",
        }
    }
}

/// The preview's view model: the greeter state machine plus what QML shows.
///
/// The state machine is unchanged — this type only feeds it the events the
/// surface produces and remembers which notice to render. It never executes a
/// [`Command`]; the returned command is the contract a future reviewed
/// transport would honour, and the preview drops it.
#[derive(Debug)]
pub struct Preview {
    greeter: Greeter,
    notice: Notice,
}

impl Preview {
    /// A preview with no authenticator attached.
    ///
    /// `Greeter::new(false)` is what keeps the preview from authenticating: with
    /// the boundary unavailable, `Submit` is rejected by the state machine
    /// itself, not by the UI.
    pub fn new() -> Self {
        Self {
            greeter: Greeter::new(false),
            notice: Notice::Blocked,
        }
    }

    /// The account name being edited.
    pub fn username(&self) -> &str {
        self.greeter.username()
    }

    /// The explanation currently shown below the account field.
    pub fn notice(&self) -> Notice {
        self.notice
    }

    /// Whether the login control is enabled; always `false` without a reviewed
    /// transport, including the rolling attempt limit.
    pub fn can_login(&self, now: Instant) -> bool {
        self.greeter.can_submit(now)
    }

    /// Apply one surface action and return the (never executed) transport command.
    pub fn apply(&mut self, action: Action, now: Instant) -> Command {
        let event = match action {
            Action::EditUsername(value) => {
                // The notice is derived from the value as typed: the greeter
                // truncates an oversized identity, which can turn it into a
                // syntactically valid name that is still rejected.
                self.notice = if value.len() > 128 || (!value.is_empty() && !valid_username(&value))
                {
                    Notice::InvalidUser
                } else {
                    Notice::Blocked
                };
                Event::Username(value)
            }
            Action::Clear => {
                self.notice = Notice::Blocked;
                Event::Cancel
            }
            Action::Activate => Event::Submit,
        };
        let command = self.greeter.apply(event, now);
        tracing::debug!(
            notice = self.notice.message_id(),
            ?command,
            "preview action applied"
        );
        command
    }
}

impl Default for Preview {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_preview_never_authenticates_or_starts_a_session() {
        let now = Instant::now();
        let mut preview = Preview::new();
        assert!(!preview.can_login(now));
        preview.apply(Action::EditUsername("alice".into()), now);
        assert_eq!(preview.apply(Action::Activate, now), Command::None);
        assert!(!preview.can_login(now));
        assert_eq!(preview.username(), "alice");
    }

    #[test]
    fn clearing_resets_the_identity_and_the_notice() {
        let now = Instant::now();
        let mut preview = Preview::new();
        preview.apply(Action::EditUsername("alice".into()), now);
        assert_eq!(preview.notice(), Notice::Blocked);
        assert_eq!(preview.apply(Action::Clear, now), Command::None);
        assert_eq!(preview.username(), "");
        assert_eq!(preview.notice(), Notice::Blocked);
    }

    #[test]
    fn an_invalid_or_oversized_identity_is_explained() {
        let now = Instant::now();
        for invalid in ["-user", "alice;id", "a b", "alice\n"] {
            let mut preview = Preview::new();
            preview.apply(Action::EditUsername(invalid.into()), now);
            assert_eq!(
                preview.notice(),
                Notice::InvalidUser,
                "{invalid:?} must be explained"
            );
        }

        let mut preview = Preview::new();
        preview.apply(Action::EditUsername("a".repeat(4096)), now);
        assert_eq!(preview.notice(), Notice::InvalidUser);
        assert_eq!(preview.username().len(), 128);
        assert!(!preview.can_login(now));
    }

    #[test]
    fn a_valid_name_keeps_the_blocked_notice() {
        let now = Instant::now();
        let mut preview = Preview::new();
        preview.apply(Action::EditUsername("alice-1.test".into()), now);
        assert_eq!(preview.notice(), Notice::Blocked);
    }

    #[test]
    fn notice_ids_name_messages_that_exist_in_both_catalogs() {
        assert_eq!(Notice::Blocked.message_id(), "blocked");
        assert_eq!(Notice::InvalidUser.message_id(), "invalid-user");
    }
}
