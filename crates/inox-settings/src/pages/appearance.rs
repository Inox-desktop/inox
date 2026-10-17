//! The appearance page: which theme the session selects.
//!
//! PRD §9.2 asks for light and dark, and for following the session. All three
//! are one field, `theme.appearance` in `config.toml`, whose value
//! [`inox_config::Appearance`] already models — including that `system` may
//! only be asked for at the selection layer, never inside a theme file.
//!
//! The page does not edit `theme.name`. That key selects a theme *directory*,
//! and choosing one needs a list of the directories under `themes/`, which
//! belongs with the theme loader rather than with this page; the field is left
//! alone, so a user who selected a custom theme keeps it, and only the
//! appearance the fallback resolves against changes here.

use iced::Element;
use inox_config::Theme as Tokens;

use crate::app::Message;
use crate::draft::Field;
use crate::view_model::{FieldView, PageView};

/// The fields this page edits.
pub const FIELDS: &[Field] = &[Field::ThemeAppearance];

/// Draws the page: its heading and the fields listed above, in that order.
pub fn view<'a>(page: &PageView, fields: &[FieldView], tokens: &'a Tokens) -> Element<'a, Message> {
    crate::view::page_section(page, fields, tokens)
}
