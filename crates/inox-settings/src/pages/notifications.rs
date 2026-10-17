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

/// The fields this page edits.
pub const FIELDS: &[Field] = &[
    Field::NotificationsPosition,
    Field::NotificationsDoNotDisturb,
    Field::NotificationsLowTimeout,
    Field::NotificationsNormalTimeout,
    Field::NotificationsCriticalTimeout,
    Field::NotificationsMaxVisible,
    Field::NotificationsWidth,
    Field::NotificationsMargin,
    Field::NotificationsIconSize,
    Field::NotificationsMaxPending,
    Field::NotificationsQueuePolicy,
    Field::NotificationsShowActions,
    Field::NotificationsHideBodyWhenLocked,
];

/// Draws the page: its heading and the fields listed above, in that order.
pub fn view<'a>(page: &PageView, fields: &[FieldView], tokens: &'a Tokens) -> Element<'a, Message> {
    crate::view::page_section(page, fields, tokens)
}
