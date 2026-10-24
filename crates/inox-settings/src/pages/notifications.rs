//! The notifications page: where popups appear, how long they stay, and whether
//! the session starts quiet.
//!
//! The three timeout fields come from [`inox_config::TimeoutConfig`], whose
//! range `inox-config` enforces while parsing; the position comes from
//! [`inox_config::NotificationPosition`]; do-not-disturb is
//! `policy.do_not_disturb`.
//!
//! That last one is policy rather than presentation, and `inox-config` says so
//! on the field: it is the state the daemon *starts* in, so changing it while a
//! session is running changes what the next session starts with. The page's
//! per-field message ids and the status line are where that is told to the
//! user, rather than a value that silently does nothing.

use iced::Element;
use inox_config::Theme as Tokens;

use crate::app::Message;
use crate::draft::Field;
use crate::view_model::{FieldView, PageView};

/// The fields this page edits, in the order the page draws them: where a popup
/// appears and how big it is, how long each urgency stays, what the queue does
/// when it is full, and the policy the daemon starts with. The cards of
/// [`crate::pages::PageId::groups`] follow that order.
pub const FIELDS: &[Field] = &[
    Field::NotificationsPosition,
    Field::NotificationsWidth,
    Field::NotificationsMargin,
    Field::NotificationsIconSize,
    Field::NotificationsMaxVisible,
    Field::NotificationsLowTimeout,
    Field::NotificationsNormalTimeout,
    Field::NotificationsCriticalTimeout,
    Field::NotificationsMaxPending,
    Field::NotificationsQueuePolicy,
    Field::NotificationsDoNotDisturb,
    Field::NotificationsShowActions,
    Field::NotificationsHideBodyWhenLocked,
];

/// Draws the page: its title, its cards and the fields listed above, in order.
pub fn view<'a>(page: &PageView, fields: &[FieldView], tokens: &'a Tokens) -> Element<'a, Message> {
    crate::view::page_section(page, fields, tokens)
}
