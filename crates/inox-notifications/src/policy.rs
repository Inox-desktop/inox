//! Do not disturb, and what a locked session may show.
//!
//! Two decisions live here, and both are about *presentation*, never about
//! delivery:
//!
//! - **Do not disturb** is owned by the daemon, not by the configuration file:
//!   `[policy] do_not_disturb` is the value the daemon **starts** with, and the
//!   panel toggles the live one at runtime. Turning it on suppresses the popups
//!   and nothing else — notifications are still admitted, still counted, still
//!   held, and the queue's bound still applies (acceptance D06: "勿扰只抑制呈现，
//!   不抑制投递"). It also never takes the keyboard: the popup stack is placed
//!   with [`crate::placement::KeyboardInteractivity::None`] whatever this policy
//!   says, which is asserted where the placement is derived.
//! - **A locked session hides the sensitive parts of a notification**
//!   (ADR 0015, acceptance D06). [`Policy::content`] is the only way to get a
//!   popup's content, and while the session is locked it returns
//!   [`PopupContent::Withheld`], which **has no field** for the body, the
//!   application name or the icon. That is the point of modelling it as a
//!   projection type rather than a boolean the view has to remember to check: a
//!   drawing path cannot leak what the type cannot carry.

use inox_config::PolicyConfig;

use crate::markup::Span;
use crate::queue::Notification;

/// Whether the session is locked.
///
/// The compositor owns this (ADR 0015: the lock state is not a UI client's to
/// keep), and the daemon is told; it decides nothing about locking itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// The session is unlocked, which is the ordinary case.
    Unlocked,
    /// The session is locked: sensitive content is withheld.
    Locked,
}

impl SessionState {
    /// Whether the session is locked.
    pub const fn is_locked(self) -> bool {
        matches!(self, Self::Locked)
    }
}

/// Whether a popup may be presented at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    /// The popup stack is drawn.
    Shown,
    /// Do not disturb is on: the stack is not drawn, but nothing was lost.
    Suppressed,
}

/// What one popup may show.
///
/// The two variants are the whole point of this module: [`Self::Withheld`] has
/// no field for the body, the application name or the icon, so no drawing path
/// can show them while the session is locked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupContent {
    /// Everything the notification carries.
    Full {
        /// The application that sent it.
        app_name: String,
        /// The icon it asked for, if any.
        icon: Option<String>,
        /// The summary line.
        summary: String,
        /// The sanitised body.
        body: Vec<Span>,
    },
    /// A locked session: only a generic summary is shown.
    Withheld {
        /// The generic sentence that stands in for the summary.
        summary: String,
    },
}

impl PopupContent {
    /// The line a popup draws as its title.
    pub fn summary(&self) -> &str {
        match self {
            Self::Full { summary, .. } | Self::Withheld { summary } => summary,
        }
    }

    /// The body, when the popup is allowed to have one.
    pub fn body(&self) -> Option<&[Span]> {
        match self {
            Self::Full { body, .. } => Some(body),
            Self::Withheld { .. } => None,
        }
    }

    /// The application name, when the popup is allowed to name it.
    pub fn app_name(&self) -> Option<&str> {
        match self {
            Self::Full { app_name, .. } => Some(app_name),
            Self::Withheld { .. } => None,
        }
    }

    /// The icon, when the popup is allowed to draw it.
    pub fn icon(&self) -> Option<&str> {
        match self {
            Self::Full { icon, .. } => icon.as_deref(),
            Self::Withheld { .. } => None,
        }
    }

    /// Whether the sensitive parts were withheld.
    pub const fn is_withheld(&self) -> bool {
        matches!(self, Self::Withheld { .. })
    }
}

/// The policy the daemon applies to what it presents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    do_not_disturb: bool,
    show_actions: bool,
    hide_body_when_locked: bool,
}

impl Policy {
    /// The policy a configuration starts the daemon with.
    pub fn from_config(config: &PolicyConfig) -> Self {
        Self {
            do_not_disturb: config.do_not_disturb,
            show_actions: config.show_actions,
            hide_body_when_locked: config.hide_body_when_locked,
        }
    }

    /// Applies a reloaded configuration.
    ///
    /// Do not disturb is deliberately untouched: the field in the file is the
    /// value the **next** session starts with, and the daemon owns the running
    /// one (the panel is what toggles it). A reload that reset it would make a
    /// user's click undoable by editing a file they are not editing.
    pub fn apply_config(&mut self, config: &PolicyConfig) {
        self.show_actions = config.show_actions;
        self.hide_body_when_locked = config.hide_body_when_locked;
    }

    /// Whether do not disturb is on.
    pub const fn do_not_disturb(&self) -> bool {
        self.do_not_disturb
    }

    /// Turns do not disturb on or off, as the panel asks.
    pub const fn set_do_not_disturb(&mut self, on: bool) {
        self.do_not_disturb = on;
    }

    /// Turns do not disturb the other way, and reports what it became.
    pub const fn toggle_do_not_disturb(&mut self) -> bool {
        self.do_not_disturb = !self.do_not_disturb;

        self.do_not_disturb
    }

    /// Whether a notification's actions are drawn as buttons.
    pub const fn show_actions(&self) -> bool {
        self.show_actions
    }

    /// Whether a locked session withholds what a notification carries.
    pub const fn hides_body_when_locked(&self) -> bool {
        self.hide_body_when_locked
    }

    /// Whether the popup stack is presented at all.
    pub const fn presentation(&self) -> Presentation {
        if self.do_not_disturb {
            Presentation::Suppressed
        } else {
            Presentation::Shown
        }
    }

    /// What a popup may show of `notification`.
    ///
    /// `generic_summary` is the localized sentence that stands in for the
    /// summary while the session is locked; this module has no catalog of its
    /// own (the view model localizes it).
    pub fn content(
        &self,
        notification: &Notification,
        session: SessionState,
        generic_summary: &str,
    ) -> PopupContent {
        let withheld = session.is_locked() && self.hide_body_when_locked;

        if withheld {
            return PopupContent::Withheld {
                summary: generic_summary.to_owned(),
            };
        }

        PopupContent::Full {
            app_name: notification.app_name.clone(),
            icon: notification.icon_reference().map(str::to_owned),
            summary: notification.summary.clone(),
            body: notification.body.clone(),
        }
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::from_config(&PolicyConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup::sanitize;
    use crate::protocol::{Expiry, Hints, Urgency};
    use crate::queue::{Draft, Notification, NotificationId};

    const GENERIC: &str = "A notification arrived";

    fn notification() -> Notification {
        let mut queue = crate::queue::Queue::new(
            inox_config::QueueConfig::default(),
            inox_config::TimeoutConfig::default(),
            3,
        );

        let id = queue
            .notify(
                Draft {
                    app_name: "secret-app".to_owned(),
                    app_icon: Some("secret-icon".to_owned()),
                    summary: "the summary is sensitive too".to_owned(),
                    body: sanitize("a body that must not be drawn while locked"),
                    hints: Hints {
                        urgency: Urgency::Normal,
                        ..Hints::default()
                    },
                    expiry: Expiry::ServerDefault,
                    ..Draft::default()
                },
                std::time::Instant::now(),
            )
            .id;

        queue.get(id).cloned().unwrap_or(Notification {
            id: NotificationId::NONE,
            app_name: String::new(),
            app_icon: None,
            summary: String::new(),
            body: Vec::new(),
            actions: crate::actions::Actions::new(),
            hints: Hints::default(),
            lifetime: crate::queue::Lifetime::Never,
            arrived: std::time::Instant::now(),
            deadline: None,
        })
    }

    #[test]
    fn the_policy_starts_from_the_configuration() {
        let policy = Policy::from_config(&PolicyConfig {
            do_not_disturb: true,
            show_actions: false,
            hide_body_when_locked: false,
        });

        assert!(policy.do_not_disturb());
        assert!(!policy.show_actions());
        assert!(!policy.hides_body_when_locked());
        assert_eq!(policy.presentation(), Presentation::Suppressed);
    }

    #[test]
    fn the_default_policy_presents_actions_and_hides_bodies_while_locked() {
        let policy = Policy::default();

        assert!(!policy.do_not_disturb());
        assert!(policy.show_actions());
        assert!(policy.hides_body_when_locked());
        assert_eq!(policy.presentation(), Presentation::Shown);
    }

    #[test]
    fn do_not_disturb_can_be_toggled_at_runtime() {
        let mut policy = Policy::default();

        assert!(policy.toggle_do_not_disturb(), "it is now on");
        assert_eq!(policy.presentation(), Presentation::Suppressed);
        assert!(!policy.toggle_do_not_disturb(), "it is now off again");
        assert_eq!(policy.presentation(), Presentation::Shown);

        policy.set_do_not_disturb(true);
        assert!(policy.do_not_disturb());
        policy.set_do_not_disturb(false);
        assert!(!policy.do_not_disturb());
    }

    #[test]
    fn a_reload_does_not_reset_the_running_do_not_disturb() {
        let mut policy = Policy::default();
        policy.set_do_not_disturb(true);

        policy.apply_config(&PolicyConfig {
            do_not_disturb: false,
            show_actions: false,
            hide_body_when_locked: false,
        });

        assert!(
            policy.do_not_disturb(),
            "the file is the value the next session starts with, not this one's"
        );
        assert!(!policy.show_actions(), "the other two do follow the file");
        assert!(!policy.hides_body_when_locked());
    }

    #[test]
    fn a_reload_can_turn_do_not_disturb_on_for_the_next_start() {
        let next = Policy::from_config(&PolicyConfig {
            do_not_disturb: true,
            ..PolicyConfig::default()
        });

        assert!(next.do_not_disturb());
    }

    #[test]
    fn an_unlocked_session_shows_everything_the_notification_carries() {
        let policy = Policy::default();
        let notification = notification();

        let content = policy.content(&notification, SessionState::Unlocked, GENERIC);

        assert_eq!(content.app_name(), Some("secret-app"));
        assert_eq!(content.icon(), Some("secret-icon"));
        assert_eq!(content.summary(), "the summary is sensitive too");
        assert_eq!(
            content.body().map(crate::markup::plain_text).as_deref(),
            Some("a body that must not be drawn while locked")
        );
        assert!(!content.is_withheld());
    }

    #[test]
    fn a_locked_session_withholds_the_body_the_name_and_the_icon() {
        let policy = Policy::default();

        let content = policy.content(&notification(), SessionState::Locked, GENERIC);

        assert!(content.is_withheld());
        assert_eq!(content.summary(), GENERIC);
        assert_eq!(content.body(), None, "there is no body to draw");
        assert_eq!(content.app_name(), None, "and no application name");
        assert_eq!(content.icon(), None, "and no icon");
    }

    #[test]
    fn nothing_sensitive_is_left_anywhere_in_the_withheld_content() {
        let policy = Policy::default();
        let content = policy.content(&notification(), SessionState::Locked, GENERIC);

        // The strongest form of "the body is absent from the view model": not a
        // hidden field, an absent one — the rendering of the whole value cannot
        // contain it because the value does not.
        let rendered = format!("{content:?}");

        for secret in [
            "a body that must not be drawn while locked",
            "the summary is sensitive too",
            "secret-app",
            "secret-icon",
        ] {
            assert!(
                !rendered.contains(secret),
                "`{secret}` is still in the locked content: {rendered}"
            );
        }
    }

    #[test]
    fn a_projector_that_forgot_to_withhold_still_cannot_carry_a_body() {
        // The type itself is the guarantee: a `Withheld` popup has no field for
        // the body, the application name or the icon.
        let content = PopupContent::Withheld {
            summary: GENERIC.to_owned(),
        };

        assert!(content.body().is_none());
        assert!(content.app_name().is_none());
        assert!(content.icon().is_none());
    }

    #[test]
    fn locking_does_not_withhold_when_the_configuration_says_not_to() {
        let policy = Policy::from_config(&PolicyConfig {
            hide_body_when_locked: false,
            ..PolicyConfig::default()
        });

        let content = policy.content(&notification(), SessionState::Locked, GENERIC);

        assert!(
            !content.is_withheld(),
            "an operator who turned the redaction off gets the notification"
        );
        assert!(content.body().is_some());
    }

    #[test]
    fn do_not_disturb_and_the_locked_redaction_are_independent() {
        let mut policy = Policy::default();
        policy.set_do_not_disturb(true);

        let content = policy.content(&notification(), SessionState::Unlocked, GENERIC);

        assert_eq!(policy.presentation(), Presentation::Suppressed);
        assert!(
            !content.is_withheld(),
            "suppressing the popups is not the same as withholding their content: \
             turning do not disturb off must not need the notification to be re-sent"
        );
    }

    #[test]
    fn a_notification_without_an_icon_has_none_to_show() {
        let mut notification = notification();
        notification.app_icon = None;

        let content = Policy::default().content(&notification, SessionState::Unlocked, GENERIC);

        assert_eq!(content.icon(), None);
    }

    #[test]
    fn a_notification_with_an_empty_body_still_has_one() {
        let mut notification = notification();
        notification.body = Vec::new();

        let content = Policy::default().content(&notification, SessionState::Unlocked, GENERIC);

        assert_eq!(content.body(), Some([].as_slice()));
    }

    #[test]
    fn session_state_knows_whether_it_is_locked() {
        assert!(!SessionState::Unlocked.is_locked());
        assert!(SessionState::Locked.is_locked());
    }
}
