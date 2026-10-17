//! The notification daemon configuration, `notifications.toml`.
//!
//! PRD §9.1 leaves the file without a schema; what it models here is what the
//! notification daemon needs to be told, and nothing else: how a popup is drawn
//! ([`DisplayConfig`]), how long each urgency stays on screen
//! ([`TimeoutConfig`]), what happens to a notification that arrives while
//! others are showing ([`QueueConfig`]), and the startup policy
//! ([`PolicyConfig`]).
//!
//! Two of these are policy rather than presentation, and both say so in the
//! field they belong to: `[policy] do_not_disturb` is the state the daemon
//! starts in — the panel toggles it at runtime and the daemon owns it from
//! there on — and `[policy] hide_body_when_locked` is a failure boundary rather
//! than a preference: a locked session must not show notification bodies
//! (ADR 0015), so its default is `true`.
//!
//! Every range is enforced while the file is parsed, so a value outside it is a
//! parse error naming the field rather than a silent clamp (ADR 0009,
//! decision 4): a popup width of zero, a queue that can hold nothing or a
//! timeout beyond ten minutes are all rejected by [`crate::store::InoxConfig::load`].

use serde::{Deserialize, Deserializer};

/// Default number of notifications shown at the same time.
const DEFAULT_MAX_VISIBLE: u8 = 3;

/// Smallest accepted number of visible notifications.
const MAX_VISIBLE_MIN: u8 = 1;

/// Largest accepted number of visible notifications.
const MAX_VISIBLE_MAX: u8 = 16;

/// Default width of a notification popup, in logical pixels.
const DEFAULT_WIDTH: u16 = 340;

/// Smallest accepted popup width, in logical pixels.
const WIDTH_MIN: u16 = 160;

/// Largest accepted popup width, in logical pixels.
const WIDTH_MAX: u16 = 1024;

/// Default distance between a popup and the output edge, in logical pixels.
const DEFAULT_MARGIN: u16 = 12;

/// Smallest accepted distance between a popup and the output edge.
const MARGIN_MIN: u16 = 0;

/// Largest accepted distance between a popup and the output edge.
const MARGIN_MAX: u16 = 256;

/// Default size of a notification icon, in logical pixels.
const DEFAULT_ICON_SIZE: u16 = 32;

/// Smallest accepted notification icon size, in logical pixels.
const ICON_SIZE_MIN: u16 = 8;

/// Largest accepted notification icon size, in logical pixels.
const ICON_SIZE_MAX: u16 = 128;

/// Default lifetime of a low-urgency notification, in milliseconds.
const DEFAULT_LOW_MS: u32 = 5_000;

/// Default lifetime of a normal-urgency notification, in milliseconds.
const DEFAULT_NORMAL_MS: u32 = 8_000;

/// Default lifetime of a critical-urgency notification, in milliseconds.
const DEFAULT_CRITICAL_MS: u32 = 0;

/// Smallest accepted lifetime, in milliseconds.
const TIMEOUT_MIN: u32 = 0;

/// Largest accepted lifetime, in milliseconds: ten minutes.
const TIMEOUT_MAX: u32 = 600_000;

/// Default number of notifications the queue may hold.
const DEFAULT_MAX_PENDING: u16 = 32;

/// Smallest accepted queue length.
const MAX_PENDING_MIN: u16 = 1;

/// Largest accepted queue length.
const MAX_PENDING_MAX: u16 = 1024;

/// Everything `notifications.toml` describes.
///
/// The default of a section is the default of each of its fields, so this type
/// derives it: an empty `notifications.toml` is the same configuration as no
/// `notifications.toml` at all.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct NotificationsConfig {
    /// How a popup is drawn.
    pub display: DisplayConfig,
    /// How long a notification stays on screen, per urgency.
    pub timeouts: TimeoutConfig,
    /// What happens to notifications waiting behind the visible ones.
    pub queue: QueueConfig,
    /// Startup policy of the daemon.
    pub policy: PolicyConfig,
}

/// How a notification popup is drawn.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct DisplayConfig {
    /// Corner of the output the popups appear in.
    pub position: NotificationPosition,
    /// Notifications shown at the same time, between 1 and 16.
    #[serde(deserialize_with = "validate_max_visible")]
    pub max_visible: u8,
    /// Width of a popup, between 160 and 1024 logical pixels.
    #[serde(deserialize_with = "validate_width")]
    pub width: u16,
    /// Distance between the popups and the edges they sit against, between 0
    /// and 256 logical pixels.
    #[serde(deserialize_with = "validate_margin")]
    pub margin: u16,
    /// Size of the application icon on a popup, between 8 and 128 logical
    /// pixels.
    #[serde(deserialize_with = "validate_icon_size")]
    pub icon_size: u16,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            position: NotificationPosition::TopRight,
            max_visible: DEFAULT_MAX_VISIBLE,
            width: DEFAULT_WIDTH,
            margin: DEFAULT_MARGIN,
            icon_size: DEFAULT_ICON_SIZE,
        }
    }
}

/// The corner of an output popups appear in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationPosition {
    /// Upper right corner.
    #[default]
    TopRight,
    /// Upper left corner.
    TopLeft,
    /// Lower right corner.
    BottomRight,
    /// Lower left corner.
    BottomLeft,
}

/// How long a notification stays on screen, by urgency.
///
/// A lifetime of `0` means the notification never expires on its own; it is the
/// default for critical urgency, which is also the urgency the specification
/// says must not time out silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TimeoutConfig {
    /// Lifetime of a low-urgency notification, in milliseconds, between 0 and
    /// 600000.
    #[serde(deserialize_with = "validate_timeout")]
    pub low_ms: u32,
    /// Lifetime of a normal-urgency notification, in milliseconds, between 0
    /// and 600000.
    #[serde(deserialize_with = "validate_timeout")]
    pub normal_ms: u32,
    /// Lifetime of a critical-urgency notification, in milliseconds, between 0
    /// and 600000.
    #[serde(deserialize_with = "validate_timeout")]
    pub critical_ms: u32,
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        Self {
            low_ms: DEFAULT_LOW_MS,
            normal_ms: DEFAULT_NORMAL_MS,
            critical_ms: DEFAULT_CRITICAL_MS,
        }
    }
}

/// What happens to notifications waiting behind the visible ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct QueueConfig {
    /// Notifications the queue may hold, between 1 and 1024.
    #[serde(deserialize_with = "validate_max_pending")]
    pub max_pending: u16,
    /// What the daemon does when the queue is full.
    pub policy: QueuePolicy,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_pending: DEFAULT_MAX_PENDING,
            policy: QueuePolicy::DropOldest,
        }
    }
}

/// What the daemon does when the queue is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueuePolicy {
    /// Discard the oldest waiting notification.
    #[default]
    DropOldest,
    /// Keep one notification per application, replacing the waiting one the
    /// same application left behind.
    CoalesceByApp,
}

/// Startup policy of the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PolicyConfig {
    /// Whether the daemon starts with do-not-disturb on.
    ///
    /// This is the startup state only: the panel toggles it at runtime and the
    /// daemon owns it from there on, so editing this field while a session is
    /// running changes what the next session starts with, not the current one.
    pub do_not_disturb: bool,
    /// Whether notifications offering actions show them.
    pub show_actions: bool,
    /// Whether a locked session hides notification bodies (ADR 0015).
    pub hide_body_when_locked: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            do_not_disturb: false,
            show_actions: true,
            hide_body_when_locked: true,
        }
    }
}

/// Reject a visible notification count outside its accepted range.
fn validate_max_visible<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let visible = u8::deserialize(deserializer)?;

    if !(MAX_VISIBLE_MIN..=MAX_VISIBLE_MAX).contains(&visible) {
        return Err(serde::de::Error::custom(format!(
            "notifications.display.max_visible must be between {MAX_VISIBLE_MIN} and {MAX_VISIBLE_MAX} notifications, found {visible}"
        )));
    }

    Ok(visible)
}

/// Reject a popup width outside its accepted range.
fn validate_width<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let width = u16::deserialize(deserializer)?;

    if !(WIDTH_MIN..=WIDTH_MAX).contains(&width) {
        return Err(serde::de::Error::custom(format!(
            "notifications.display.width must be between {WIDTH_MIN} and {WIDTH_MAX} logical pixels, found {width}"
        )));
    }

    Ok(width)
}

/// Reject a popup margin outside its accepted range.
fn validate_margin<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let margin = u16::deserialize(deserializer)?;

    if !(MARGIN_MIN..=MARGIN_MAX).contains(&margin) {
        return Err(serde::de::Error::custom(format!(
            "notifications.display.margin must be between {MARGIN_MIN} and {MARGIN_MAX} logical pixels, found {margin}"
        )));
    }

    Ok(margin)
}

/// Reject a notification icon size outside its accepted range.
fn validate_icon_size<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let size = u16::deserialize(deserializer)?;

    if !(ICON_SIZE_MIN..=ICON_SIZE_MAX).contains(&size) {
        return Err(serde::de::Error::custom(format!(
            "notifications.display.icon_size must be between {ICON_SIZE_MIN} and {ICON_SIZE_MAX} logical pixels, found {size}"
        )));
    }

    Ok(size)
}

/// Reject a timeout outside its accepted range.
///
/// Shared by the three urgencies; which one is at fault comes from the field
/// path in the error.
fn validate_timeout<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let timeout = u32::deserialize(deserializer)?;

    if !(TIMEOUT_MIN..=TIMEOUT_MAX).contains(&timeout) {
        return Err(serde::de::Error::custom(format!(
            "a notification timeout must be between {TIMEOUT_MIN} and {TIMEOUT_MAX} milliseconds, found {timeout}"
        )));
    }

    Ok(timeout)
}

/// Reject a queue length outside its accepted range.
fn validate_max_pending<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let pending = u16::deserialize(deserializer)?;

    if !(MAX_PENDING_MIN..=MAX_PENDING_MAX).contains(&pending) {
        return Err(serde::de::Error::custom(format!(
            "notifications.queue.max_pending must be between {MAX_PENDING_MIN} and {MAX_PENDING_MAX} notifications, found {pending}"
        )));
    }

    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ConfigError;
    use crate::error::test_support::{column_of, line_of};
    use std::path::Path;

    const NOTIFICATIONS_PATH: &str = "/cfg/inox/notifications.toml";

    fn parse_notifications(text: &str) -> Result<NotificationsConfig, ConfigError> {
        crate::error::parse(Path::new(NOTIFICATIONS_PATH), text)
    }

    #[test]
    fn an_empty_document_uses_every_default() {
        let config = parse_notifications("").expect("empty document");

        assert_eq!(config.display.position, NotificationPosition::TopRight);
        assert_eq!(config.display.max_visible, 3);
        assert_eq!(config.display.width, 340);
        assert_eq!(config.display.margin, 12);
        assert_eq!(config.display.icon_size, 32);
        assert_eq!(config.timeouts.low_ms, 5_000);
        assert_eq!(config.timeouts.normal_ms, 8_000);
        assert_eq!(config.timeouts.critical_ms, 0);
        assert_eq!(config.queue.max_pending, 32);
        assert_eq!(config.queue.policy, QueuePolicy::DropOldest);
        assert!(!config.policy.do_not_disturb);
        assert!(config.policy.show_actions);
        assert!(config.policy.hide_body_when_locked);
    }

    #[test]
    fn the_defaults_of_the_four_sections_are_the_ones_this_schema_declares() {
        assert_eq!(
            NotificationsConfig::default(),
            parse_notifications("").expect("empty document")
        );
        assert_eq!(
            DisplayConfig::default(),
            NotificationsConfig::default().display
        );
        assert_eq!(
            TimeoutConfig::default(),
            NotificationsConfig::default().timeouts
        );
        assert_eq!(QueueConfig::default(), NotificationsConfig::default().queue);
        assert_eq!(
            PolicyConfig::default(),
            NotificationsConfig::default().policy
        );
    }

    #[test]
    fn display_timeouts_queue_and_policy_are_parsed() {
        let config = parse_notifications(
            "[display]\n\
             position = \"bottom-left\"\n\
             max_visible = 5\n\
             width = 420\n\
             margin = 24\n\
             icon_size = 48\n\
             \n\
             [timeouts]\n\
             low_ms = 3000\n\
             normal_ms = 6000\n\
             critical_ms = 15000\n\
             \n\
             [queue]\n\
             max_pending = 64\n\
             policy = \"coalesce-by-app\"\n\
             \n\
             [policy]\n\
             do_not_disturb = true\n\
             show_actions = false\n\
             hide_body_when_locked = false\n",
        )
        .expect("document");

        assert_eq!(config.display.position, NotificationPosition::BottomLeft);
        assert_eq!(config.display.max_visible, 5);
        assert_eq!(config.display.width, 420);
        assert_eq!(config.display.margin, 24);
        assert_eq!(config.display.icon_size, 48);
        assert_eq!(config.timeouts.low_ms, 3_000);
        assert_eq!(config.timeouts.normal_ms, 6_000);
        assert_eq!(config.timeouts.critical_ms, 15_000);
        assert_eq!(config.queue.max_pending, 64);
        assert_eq!(config.queue.policy, QueuePolicy::CoalesceByApp);
        assert!(config.policy.do_not_disturb);
        assert!(!config.policy.show_actions);
        assert!(!config.policy.hide_body_when_locked);
    }

    #[test]
    fn a_partial_section_keeps_the_defaults_of_the_fields_it_leaves_out() {
        let config = parse_notifications("[timeouts]\nnormal_ms = 2000\n").expect("document");

        assert_eq!(config.timeouts.normal_ms, 2_000);
        assert_eq!(config.timeouts.low_ms, 5_000);
        assert_eq!(config.timeouts.critical_ms, 0);
        assert_eq!(config.display, DisplayConfig::default());
        assert_eq!(config.queue, QueueConfig::default());
        assert_eq!(config.policy, PolicyConfig::default());
    }

    #[test]
    fn every_position_spelling_is_parsed() {
        for (text, expected) in [
            ("top-right", NotificationPosition::TopRight),
            ("top-left", NotificationPosition::TopLeft),
            ("bottom-right", NotificationPosition::BottomRight),
            ("bottom-left", NotificationPosition::BottomLeft),
        ] {
            let config =
                parse_notifications(&format!("[display]\nposition = \"{text}\"\n")).expect("doc");

            assert_eq!(config.display.position, expected);
        }
    }

    #[test]
    fn a_misspelled_position_is_rejected() {
        let error = parse_notifications("[display]\nposition = \"topright\"\n")
            .expect_err("unknown variant");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("display.position"));
                assert_eq!(line, Some(2));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn every_queue_policy_spelling_is_parsed() {
        for (text, expected) in [
            ("drop-oldest", QueuePolicy::DropOldest),
            ("coalesce-by-app", QueuePolicy::CoalesceByApp),
        ] {
            let config =
                parse_notifications(&format!("[queue]\npolicy = \"{text}\"\n")).expect("document");

            assert_eq!(config.queue.policy, expected);
        }
    }

    #[test]
    fn a_misspelled_queue_policy_is_rejected() {
        let error = parse_notifications("[queue]\npolicy = \"drop_newest\"\n")
            .expect_err("unknown variant");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("queue.policy"));
                assert_eq!(line, Some(2));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn out_of_range_values_are_rejected_while_parsing() {
        for (document, field, needle, reason) in [
            (
                "[display]\nmax_visible = 0\n",
                "display.max_visible",
                "0",
                "between 1 and 16",
            ),
            (
                "[display]\nmax_visible = 17\n",
                "display.max_visible",
                "17",
                "between 1 and 16",
            ),
            (
                "[display]\nwidth = 100\n",
                "display.width",
                "100",
                "between 160 and 1024",
            ),
            (
                "[display]\nmargin = 257\n",
                "display.margin",
                "257",
                "between 0 and 256",
            ),
            (
                "[display]\nicon_size = 4\n",
                "display.icon_size",
                "4",
                "between 8 and 128",
            ),
            (
                "[timeouts]\nlow_ms = 600_001\n",
                "timeouts.low_ms",
                "600_001",
                "between 0 and 600000",
            ),
            (
                "[timeouts]\ncritical_ms = 700000\n",
                "timeouts.critical_ms",
                "700000",
                "between 0 and 600000 milliseconds",
            ),
            (
                "[queue]\nmax_pending = 0\n",
                "queue.max_pending",
                "0",
                "between 1 and 1024",
            ),
            (
                "[queue]\nmax_pending = 1025\n",
                "queue.max_pending",
                "1025",
                "between 1 and 1024",
            ),
        ] {
            let outcome = parse_notifications(document);
            assert!(
                outcome.is_err(),
                "expected `{document:?}` to be rejected, got {outcome:?}"
            );

            let error = outcome.expect_err("checked above");
            let expected_line = line_of(document, needle);
            let rendered = error.to_string();

            assert!(
                rendered.contains(&format!("notifications.toml:{expected_line}:")),
                "rendered `{rendered}`"
            );
            assert!(
                rendered.contains(&format!("(field `{field}`)")),
                "rendered `{rendered}`"
            );
            assert!(rendered.contains(reason), "rendered `{rendered}`");

            match error {
                ConfigError::Parse {
                    field: reported,
                    line,
                    column,
                    ..
                } => {
                    assert_eq!(reported.as_deref(), Some(field));
                    assert_eq!(line, Some(expected_line));
                    assert_eq!(column, Some(column_of(document, needle)));
                }
                other => panic!("expected a parse error, got {other:?}"),
            }
        }
    }

    #[test]
    fn values_at_the_edge_of_their_range_are_accepted() {
        let config = parse_notifications(
            "[display]\n\
             max_visible = 1\n\
             width = 160\n\
             margin = 256\n\
             icon_size = 8\n\
             \n\
             [timeouts]\n\
             low_ms = 0\n\
             normal_ms = 600000\n\
             \n\
             [queue]\n\
             max_pending = 1\n",
        )
        .expect("values at the edge of their ranges");

        assert_eq!(config.display.max_visible, 1);
        assert_eq!(config.display.width, 160);
        assert_eq!(config.display.margin, 256);
        assert_eq!(config.display.icon_size, 8);
        assert_eq!(config.timeouts.low_ms, 0);
        assert_eq!(config.timeouts.normal_ms, 600_000);
        assert_eq!(config.queue.max_pending, 1);

        let config = parse_notifications(
            "[display]\n\
             max_visible = 16\n\
             width = 1024\n\
             margin = 0\n\
             icon_size = 128\n\
             \n\
             [queue]\n\
             max_pending = 1024\n",
        )
        .expect("values at the edge of their ranges");

        assert_eq!(config.display.max_visible, 16);
        assert_eq!(config.display.width, 1024);
        assert_eq!(config.display.margin, 0);
        assert_eq!(config.display.icon_size, 128);
        assert_eq!(config.queue.max_pending, 1024);
    }

    #[test]
    fn an_unknown_key_is_rejected_with_its_field_path() {
        let error = parse_notifications("[queue]\nmax_pending = 8\nnonsense = 1\n")
            .expect_err("unknown key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("queue.nonsense"));
                assert_eq!(line, Some(3));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_section_is_rejected() {
        let error = parse_notifications("[popups]\nwidth = 300\n").expect_err("unknown section");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("popups"));
                assert_eq!(line, Some(1));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_critical_timeout_of_zero_means_never_expire() {
        // Documented rather than merely defaulted: the daemon reads `0` as "no
        // timeout", so the default critical lifetime is not a typo for 5000.
        let config = parse_notifications("[timeouts]\ncritical_ms = 0\n").expect("document");

        assert_eq!(config.timeouts.critical_ms, 0);
    }
}
