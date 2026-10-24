//! The panel page: the strip along an output's edge.
//!
//! Every field is a field of [`inox_config::PanelConfig`], and every control is
//! built from that type's own shape: the edge is
//! [`inox_config::PanelPosition`], the height is the range `inox-config`
//! enforces while parsing `shell.toml`, and the two switches are booleans. A
//! value the schema rejects cannot be entered here (ADR 0009, decision 4).
//!
use iced::Element;
use inox_config::Theme as Tokens;

use crate::app::Message;
use crate::draft::Field;
use crate::view_model::{FieldView, PageView};

/// The fields this page edits, in the order the page draws them: where the strip
/// sits, then how it behaves, then its clock, then its task buttons.
/// [`crate::pages::PageId::groups`] splits them into the cards, and the keyboard
/// walks them in the order they are drawn (acceptance Q01).
pub const FIELDS: &[Field] = &[
    Field::PanelPosition,
    Field::PanelHeight,
    Field::PanelMargin,
    Field::PanelExclusive,
    Field::PanelAutohide,
    Field::PanelShowClock,
    Field::PanelClockFormat,
    Field::PanelMaxTaskButtons,
    Field::PanelIconSize,
];

/// Draws the page: its title, its cards and the fields listed above, in order.
pub fn view<'a>(page: &PageView, fields: &[FieldView], tokens: &'a Tokens) -> Element<'a, Message> {
    crate::view::page_section(page, fields, tokens)
}
