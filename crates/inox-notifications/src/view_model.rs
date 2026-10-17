//! What the popup stack draws, derived from what the daemon knows.
//!
//! The view is not allowed to make decisions (ADR 0013, and the same rule the
//! other L3 components follow). Every choice a user could notice — which
//! notifications are drawn, in what order, what each one says, which actions are
//! offered and **why one of them is disabled**, whether there are more waiting
//! than can be shown — is made here, once, from the queue, the display
//! configuration and the policy, and then drawn.
//!
//! Three of those are acceptance criteria rather than presentation choices:
//!
//! - **The overflow is said, not hidden.** When the queue holds more than the
//!   stack can show, the model carries a sentence saying how many are waiting
//!   (D05, D06), so the user is never looking at a stack that silently dropped
//!   their notifications.
//! - **A disabled action says why.** With `policy.show_actions` off, an action's
//!   button is drawn with
//!   [`notifications-action-disabled-policy`](../i18n/en-US/notifications.ftl),
//!   never as a button that appears to work.
//! - **A locked session cannot leak.** The content of every popup comes from
//!   [`Policy::content`], so a locked session's model holds
//!   [`PopupContent::Withheld`] — a value with no field a body or an application
//!   name could be in (ADR 0015).
//!
//! There is deliberately no focus order here: the stack never takes keyboard
//! focus, which is a property of its placement
//! ([`crate::placement::KeyboardInteractivity::None`]) rather than a list this
//! module could forget to fill in.

use inox_common::i18n::Localizer;
use inox_config::DisplayConfig;

use crate::actions::ActionId;
use crate::markup::Span;
use crate::policy::{Policy, PopupContent, Presentation, SessionState};
use crate::protocol::Urgency;
use crate::queue::{NotificationId, Queue};

/// The longest a summary or an application name may be, in characters.
///
/// A popup is a small surface and a title is one line of it: a 400-character
/// window title must not push the actions off the bottom, so the projection
/// bounds what it hands to the view.
pub const MAX_TITLE_CHARS: usize = 96;

/// The longest an action label may be, in characters.
pub const MAX_ACTION_CHARS: usize = 32;

/// The most body text one popup may carry, in characters.
///
/// A body is capped again here, below [`crate::markup::MAX_TEXT_CHARS`], because
/// what a popup can usefully show is much less than what a client may legally
/// send.
pub const MAX_BODY_CHARS: usize = 512;

/// What the stack as a whole is doing (the states D05 asks for).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackStatus {
    /// Nothing is held: the stack draws nothing.
    Empty,
    /// At least one popup is drawn.
    Showing,
    /// Notifications are held, and do not disturb is on, so nothing is drawn.
    Suppressed,
}

/// One button on a popup, for one of the notification's actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionButton {
    /// The action's key, which the client is told about when it is invoked.
    pub id: ActionId,
    /// The text on the button.
    pub label: String,
    /// Whether it can be pressed.
    pub enabled: bool,
    /// Why it cannot be, in the user's language (D05). `None` when it can.
    pub disabled_reason: Option<String>,
}

/// One popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    /// Which notification it is drawn for.
    pub id: NotificationId,
    /// How urgent it is, which decides its colour.
    pub urgency: Urgency,
    /// What it may show, after the session's policy.
    pub content: PopupContent,
    /// The actions it offers, as buttons.
    pub actions: Vec<ActionButton>,
    /// The label of its close button.
    pub close: String,
}

/// How many notifications are held but not drawn, said in the user's language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverflowNotice {
    /// How many notifications are waiting behind the ones on screen.
    pub hidden: usize,
    /// The sentence that says so, already localized.
    pub message: String,
}

/// Everything the stack draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupStack {
    /// The stack-level state.
    pub status: StackStatus,
    /// The popups, newest first: the one nearest the anchored corner comes
    /// first.
    pub popups: Vec<Popup>,
    /// How many notifications are held but not shown, and the sentence that
    /// says so.
    pub overflow: Option<OverflowNotice>,
    /// The daemon's own line, when it has no popup to draw: why the stack is
    /// empty, or why it is muted. `None` while a popup is drawn.
    pub notice: Option<String>,
}

impl PopupStack {
    /// Whether the stack draws no popup at all.
    pub fn is_empty(&self) -> bool {
        self.popups.is_empty()
    }

    /// The popup drawn for `id`, if it is one of them.
    pub fn popup(&self, id: NotificationId) -> Option<&Popup> {
        self.popups.iter().find(|popup| popup.id == id)
    }

    /// How many notifications are held but not drawn.
    pub fn hidden(&self) -> usize {
        self.overflow.as_ref().map_or(0, |overflow| overflow.hidden)
    }
}

/// The state the projection reads.
pub struct StackInput<'a> {
    /// The notifications the daemon holds.
    pub queue: &'a Queue,
    /// How the stack is drawn and how many popups it shows at once.
    pub display: &'a DisplayConfig,
    /// Do not disturb, and whether a locked session withholds content.
    pub policy: &'a Policy,
    /// Whether the session is locked.
    pub session: SessionState,
}

impl StackInput<'_> {
    /// Projects the input into what the view draws.
    pub fn project(&self, localizer: &Localizer) -> PopupStack {
        if self.policy.presentation() == Presentation::Suppressed {
            return PopupStack {
                status: StackStatus::Suppressed,
                popups: Vec::new(),
                overflow: None,
                notice: Some(localizer.message("notifications-suppressed")),
            };
        }

        let generic_summary = localizer.message("notifications-generic-summary");
        let close = localizer.message("notifications-close");
        let disabled_reason = (!self.policy.show_actions())
            .then(|| localizer.message("notifications-action-disabled-policy"));
        let shows_actions = self.policy.show_actions();

        let popups: Vec<Popup> = self
            .queue
            .newest(usize::from(self.display.max_visible))
            .into_iter()
            .map(|notification| Popup {
                id: notification.id,
                urgency: notification.urgency(),
                content: content(
                    self.policy
                        .content(notification, self.session, &generic_summary),
                    localizer,
                ),
                actions: notification
                    .actions
                    .buttons()
                    .map(|action| ActionButton {
                        id: action.id.clone(),
                        label: truncate(&action.label, MAX_ACTION_CHARS),
                        enabled: shows_actions,
                        disabled_reason: disabled_reason.clone(),
                    })
                    .collect(),
                close: close.clone(),
            })
            .collect();

        // What is held and not drawn, which is not the same as what the queue
        // calls waiting: the projection is what knows how many popups it drew.
        let hidden = self.queue.len().saturating_sub(popups.len());
        let overflow = (hidden > 0).then(|| OverflowNotice {
            hidden,
            message: localizer.message_with("notifications-overflow", &[("count", hidden.into())]),
        });

        let status = if popups.is_empty() {
            StackStatus::Empty
        } else {
            StackStatus::Showing
        };
        let notice =
            (status == StackStatus::Empty).then(|| localizer.message("notifications-empty"));

        PopupStack {
            status,
            popups,
            overflow,
            notice,
        }
    }
}

/// Bounds the text a popup carries, and gives an image its alt text.
fn content(content: PopupContent, localizer: &Localizer) -> PopupContent {
    match content {
        PopupContent::Full {
            app_name,
            icon,
            summary,
            body,
        } => PopupContent::Full {
            app_name: truncate(&app_name, MAX_TITLE_CHARS),
            icon,
            summary: truncate(&summary, MAX_TITLE_CHARS),
            body: body_spans(&body, localizer),
        },
        PopupContent::Withheld { summary } => PopupContent::Withheld { summary },
    }
}

/// The body a popup draws: bounded, and with every image spelled out.
///
/// An image this daemon does not decode is drawn as its alt text; one that
/// carries no alt text at all gets the localized word for it, so a body never
/// renders as a blank gap. That word is counted against [`MAX_BODY_CHARS`] like
/// any other, because the bound is on what is drawn.
fn body_spans(body: &[Span], localizer: &Localizer) -> Vec<Span> {
    let placeholder = localizer.message("notifications-image-placeholder");
    let mut spans: Vec<Span> = Vec::new();
    let mut budget = MAX_BODY_CHARS;

    for span in body {
        if budget == 0 {
            break;
        }

        // What the popup will draw for this span: an image with no alt text is
        // drawn as the localized word for it.
        let shown = match span {
            Span::Image { alt, .. } if alt.is_empty() => placeholder.as_str(),
            other => other.text(),
        };
        let length = shown.chars().count();

        if length <= budget {
            budget -= length;
            spans.push(match span {
                Span::Image { src, alt, emphasis } if alt.is_empty() => Span::Image {
                    src: src.clone(),
                    alt: placeholder.clone(),
                    emphasis: *emphasis,
                },
                other => other.clone(),
            });
            continue;
        }

        // Whatever is left of the budget is all this span may keep, and the rest
        // of the body is dropped: the cap bounds the body, not each span, so two
        // long runs cannot add up to twice it.
        let cut: String = shown.chars().take(budget).collect();
        budget = 0;

        spans.push(match span {
            Span::Text { emphasis, .. } => Span::Text {
                text: cut,
                emphasis: *emphasis,
            },
            Span::Link { href, emphasis, .. } => Span::Link {
                text: cut,
                href: href.clone(),
                emphasis: *emphasis,
            },
            Span::Image { src, emphasis, .. } => Span::Image {
                src: src.clone(),
                alt: cut,
                emphasis: *emphasis,
            },
        });
    }

    spans
}

/// Bounds a label to `max` characters, on a character boundary.
///
/// An ellipsis marks that something was cut: a silently shortened title is a
/// title a user cannot tell apart from the one that was sent.
fn truncate(label: &str, max: usize) -> String {
    if label.chars().count() <= max {
        return label.to_owned();
    }

    let mut cut: String = label.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');

    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::Actions;
    use crate::markup::{Emphasis, sanitize};
    use crate::queue::{Draft, Queue};
    use inox_config::{PolicyConfig, QueueConfig, TimeoutConfig};
    use std::time::Instant;

    fn localizer() -> Localizer {
        crate::i18n::load(&inox_common::i18n::Locale::default_locale()).expect("catalogs")
    }

    fn queue_with(notifications: &[(&str, &str)]) -> Queue {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);

        for (app, summary) in notifications {
            queue.notify(
                Draft {
                    app_name: (*app).to_owned(),
                    summary: (*summary).to_owned(),
                    ..Draft::default()
                },
                Instant::now(),
            );
        }

        queue
    }

    fn input<'a>(
        queue: &'a Queue,
        display: &'a DisplayConfig,
        policy: &'a Policy,
    ) -> StackInput<'a> {
        StackInput {
            queue,
            display,
            policy,
            session: SessionState::Unlocked,
        }
    }

    fn display() -> DisplayConfig {
        DisplayConfig::default()
    }

    #[test]
    fn nothing_held_is_the_empty_state() {
        let queue = queue_with(&[]);
        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());

        assert_eq!(stack.status, StackStatus::Empty);
        assert!(stack.is_empty());
        assert!(stack.overflow.is_none());
        assert_eq!(stack.notice.as_deref(), Some("No notifications"));
    }

    #[test]
    fn one_held_notification_is_one_popup_newest_first() {
        let queue = queue_with(&[("terminal", "a build finished")]);
        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());

        assert_eq!(stack.status, StackStatus::Showing);
        assert!(stack.notice.is_none(), "a drawn popup speaks for itself");
        assert_eq!(stack.popups.len(), 1);
        assert_eq!(stack.popups[0].content.summary(), "a build finished");
        assert_eq!(stack.popups[0].content.app_name(), Some("terminal"));
        assert_eq!(stack.popups[0].close, "Close");
        assert_eq!(stack.popups[0].id, NotificationId::new(1));
        assert_eq!(stack.popups[0].urgency, Urgency::Normal);
    }

    #[test]
    fn at_most_the_configured_number_of_popups_are_drawn() {
        let queue = queue_with(&[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4"), ("e", "5")]);
        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());

        assert_eq!(stack.popups.len(), 3, "display.max_visible is 3 by default");
        assert_eq!(
            stack
                .popups
                .iter()
                .map(|popup| popup.content.summary())
                .collect::<Vec<_>>(),
            ["5", "4", "3"],
            "the newest are drawn, newest first"
        );
    }

    #[test]
    fn more_waiting_than_can_be_shown_is_said_rather_than_hidden() {
        let queue = queue_with(&[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4"), ("e", "5")]);
        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());

        assert_eq!(
            stack.overflow,
            Some(OverflowNotice {
                hidden: 2,
                message: "2 more waiting".to_owned()
            })
        );
        assert_eq!(stack.hidden(), 2);
    }

    #[test]
    fn a_configuration_that_shows_more_has_nothing_to_say_about_overflow() {
        let queue = queue_with(&[("a", "1"), ("b", "2")]);
        let display = DisplayConfig {
            max_visible: 8,
            ..display()
        };
        let stack = input(&queue, &display, &Policy::default()).project(&localizer());

        assert!(stack.overflow.is_none());
        assert_eq!(stack.popups.len(), 2);
    }

    #[test]
    fn do_not_disturb_suppresses_the_stack_and_loses_nothing() {
        let queue = queue_with(&[("a", "1"), ("b", "2")]);
        let mut policy = Policy::default();
        policy.set_do_not_disturb(true);

        let stack = input(&queue, &display(), &policy).project(&localizer());

        assert_eq!(stack.status, StackStatus::Suppressed);
        assert!(stack.popups.is_empty());
        assert!(stack.overflow.is_none());
        assert_eq!(
            stack.notice.as_deref(),
            Some("Notifications are muted while do not disturb is on")
        );
        assert_eq!(
            queue.len(),
            2,
            "delivery is not suppressed: the queue still holds both"
        );
    }

    #[test]
    fn turning_do_not_disturb_off_shows_what_was_held() {
        let queue = queue_with(&[("a", "1"), ("b", "2")]);
        let mut policy = Policy::default();
        policy.set_do_not_disturb(true);
        let _ = input(&queue, &display(), &policy).project(&localizer());

        policy.set_do_not_disturb(false);
        let stack = input(&queue, &display(), &policy).project(&localizer());

        assert_eq!(stack.status, StackStatus::Showing);
        assert_eq!(stack.popups.len(), 2, "nothing was dropped while muted");
    }

    #[test]
    fn a_locked_session_draws_neither_the_body_nor_the_name() {
        let queue = queue_with(&[("secret-app", "a secret summary")]);
        let display = display();
        let policy = Policy::default();
        let mut input = input(&queue, &display, &policy);
        input.session = SessionState::Locked;

        let stack = input.project(&localizer());

        assert_eq!(stack.popups.len(), 1);
        let popup = &stack.popups[0];
        assert!(popup.content.is_withheld());
        assert_eq!(popup.content.summary(), "A notification arrived");
        assert!(popup.content.body().is_none());
        assert!(popup.content.app_name().is_none());
        assert!(popup.content.icon().is_none());
    }

    #[test]
    fn no_body_the_notification_carried_is_anywhere_in_the_locked_model() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                app_name: "secret-app".to_owned(),
                app_icon: Some("secret-icon".to_owned()),
                summary: "a secret summary".to_owned(),
                body: sanitize("a secret body"),
                ..Draft::default()
            },
            Instant::now(),
        );
        let display = display();
        let policy = Policy::default();
        let mut input = input(&queue, &display, &policy);
        input.session = SessionState::Locked;

        let stack = input.project(&localizer());
        let rendered = format!("{stack:?}");

        // "Absent from the model" rather than "not drawn by the view": a test
        // that walked the drawing would pass just as well if the string were
        // sitting in a field nobody read.
        for secret in [
            "a secret body",
            "a secret summary",
            "secret-app",
            "secret-icon",
        ] {
            assert!(
                !rendered.contains(secret),
                "`{secret}` is still in the locked model"
            );
        }
    }

    #[test]
    fn an_unlocked_session_keeps_the_body_it_was_sent() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                body: sanitize("with <b>markup</b>"),
                ..Draft::default()
            },
            Instant::now(),
        );

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());
        let content = &stack.popups[0].content;

        assert_eq!(
            content.body().map(crate::markup::plain_text).as_deref(),
            Some("with markup")
        );
    }

    #[test]
    fn an_action_becomes_a_button_with_its_label() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                actions: Actions::from_flat(&[
                    "default".to_owned(),
                    String::new(),
                    "open".to_owned(),
                    "Open the file".to_owned(),
                ])
                .expect("actions"),
                ..Draft::default()
            },
            Instant::now(),
        );

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());

        assert_eq!(
            stack.popups[0].actions.len(),
            1,
            "the default action is not a button"
        );
        assert_eq!(stack.popups[0].actions[0].id, ActionId::new("open"));
        assert_eq!(stack.popups[0].actions[0].label, "Open the file");
        assert!(stack.popups[0].actions[0].enabled);
        assert!(stack.popups[0].actions[0].disabled_reason.is_none());
    }

    #[test]
    fn an_action_that_cannot_be_used_says_why() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                actions: Actions::from_flat(&["open".to_owned(), "Open".to_owned()])
                    .expect("actions"),
                ..Draft::default()
            },
            Instant::now(),
        );

        let policy = Policy::from_config(&PolicyConfig {
            show_actions: false,
            ..PolicyConfig::default()
        });
        let stack = input(&queue, &display(), &policy).project(&localizer());
        let button = &stack.popups[0].actions[0];

        assert!(!button.enabled);
        assert_eq!(
            button.disabled_reason.as_deref(),
            Some("Actions are turned off in notifications.toml")
        );
    }

    #[test]
    fn a_long_summary_is_cut_with_an_ellipsis() {
        let long = "t".repeat(MAX_TITLE_CHARS * 3);
        let queue = queue_with(&[("a", &long)]);

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());
        let summary = stack.popups[0].content.summary();

        assert_eq!(summary.chars().count(), MAX_TITLE_CHARS);
        assert!(summary.ends_with('…'), "the cut is visible: {summary}");
    }

    #[test]
    fn a_long_body_is_cut_to_what_a_popup_can_show() {
        let long = "b".repeat(MAX_BODY_CHARS * 4);
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                body: sanitize(&long),
                ..Draft::default()
            },
            Instant::now(),
        );

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());
        let body = stack.popups[0]
            .content
            .body()
            .map(crate::markup::plain_text)
            .expect("a body");

        assert_eq!(body.chars().count(), MAX_BODY_CHARS);
    }

    #[test]
    fn a_long_body_is_cut_to_the_cap_across_all_of_its_spans() {
        // Two runs, each longer than half the budget: what is bounded is the
        // body, so the two together may not add up to twice the cap.
        let run = "b".repeat(MAX_BODY_CHARS * 3 / 4);
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                body: sanitize(&format!("<b>{run}</b>{run}")),
                ..Draft::default()
            },
            Instant::now(),
        );

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());
        let body = stack.popups[0].content.body().expect("a body");

        assert_eq!(
            crate::markup::plain_text(body).chars().count(),
            MAX_BODY_CHARS
        );
        assert_eq!(body.len(), 2, "the second run is cut, not dropped whole");
    }

    #[test]
    fn an_image_without_alt_text_is_counted_against_the_body_cap() {
        // Every `<img>` with no alt text is drawn as the localized word for it,
        // and two hundred of them are more text than a popup may carry.
        let image = "<img src=\"/a.png\">";
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                body: sanitize(&image.repeat(200)),
                ..Draft::default()
            },
            Instant::now(),
        );

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());
        let body = stack.popups[0].content.body().expect("a body");

        assert_eq!(
            crate::markup::plain_text(body).chars().count(),
            MAX_BODY_CHARS
        );
        assert!(
            body.len() < 200,
            "the cap stops the stack drawing every image: {}",
            body.len()
        );
    }

    #[test]
    fn a_long_action_label_is_cut() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                actions: Actions::from_flat(&["open".to_owned(), "o".repeat(200)])
                    .expect("actions"),
                ..Draft::default()
            },
            Instant::now(),
        );

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());
        let label = &stack.popups[0].actions[0].label;

        assert_eq!(label.chars().count(), MAX_ACTION_CHARS);
        assert!(label.ends_with('…'));
    }

    #[test]
    fn a_body_image_becomes_its_alt_text_or_the_placeholder() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                body: sanitize("<img src=\"/a.png\" alt=\"a chart\"> <img src=\"/b.png\"> after"),
                ..Draft::default()
            },
            Instant::now(),
        );

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());
        let body = stack.popups[0].content.body().expect("a body");

        assert_eq!(
            crate::markup::plain_text(body),
            "a chart Image after",
            "an image with no alt text is named rather than left blank"
        );
    }

    #[test]
    fn a_link_keeps_its_target_and_its_styling() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                body: sanitize("<b>see</b> <a href=\"https://example.com\">here</a>"),
                ..Draft::default()
            },
            Instant::now(),
        );

        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());
        let body = stack.popups[0].content.body().expect("a body");

        assert_eq!(
            body,
            &[
                Span::Text {
                    text: "see".to_owned(),
                    emphasis: Emphasis {
                        bold: true,
                        ..Emphasis::PLAIN
                    },
                },
                Span::Text {
                    text: " ".to_owned(),
                    emphasis: Emphasis::PLAIN
                },
                Span::Link {
                    text: "here".to_owned(),
                    href: "https://example.com".to_owned(),
                    emphasis: Emphasis::PLAIN,
                },
            ]
        );
    }

    #[test]
    fn a_popup_can_be_looked_up_by_its_identifier() {
        let queue = queue_with(&[("a", "1"), ("b", "2")]);
        let stack = input(&queue, &display(), &Policy::default()).project(&localizer());

        assert!(stack.popup(NotificationId::new(2)).is_some());
        assert!(stack.popup(NotificationId::new(9)).is_none());
    }

    #[test]
    fn truncating_leaves_a_short_label_alone() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("exactly-ten", 11), "exactly-ten");
        assert_eq!(truncate("中文字符串", 3), "中文…");
    }
}
