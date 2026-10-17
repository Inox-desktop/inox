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

/// The fields this page edits.
pub const FIELDS: &[Field] = &[
    Field::PanelPosition,
    Field::PanelHeight,
    Field::PanelAutohide,
    Field::PanelShowClock,
    Field::PanelMargin,
    Field::PanelExclusive,
    Field::PanelClockFormat,
    Field::PanelMaxTaskButtons,
    Field::PanelIconSize,
];

/// Draws the page: its heading and the fields listed above, in that order.
pub fn view<'a>(page: &PageView, fields: &[FieldView], tokens: &'a Tokens) -> Element<'a, Message> {
    crate::view::page_section(page, fields, tokens)
}
