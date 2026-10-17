//! The locale page: the language the session runs in.
//!
//! One field, `locale.language` in `config.toml`, with three states the session
//! actually has: follow the environment — which is the *absence* of the key, so
//! `config.toml` is written without one — or a language Inox ships a catalog
//! for ([`inox_common::i18n::SUPPORTED_LOCALES`]: `en-US` and `zh-CN`).
//!
//! A tag a user wrote by hand and this page does not offer is kept and shown as
//! it stands rather than being silently replaced by one of the three: the
//! control's options come from the model, and the model carries the file's own
//! value ([`crate::view_model`]).

use iced::Element;
use inox_config::Theme as Tokens;

use crate::app::Message;
use crate::draft::Field;
use crate::view_model::{FieldView, PageView};

/// The fields this page edits.
pub const FIELDS: &[Field] = &[Field::LocaleLanguage];

/// Draws the page: its heading and the fields listed above, in that order.
pub fn view<'a>(page: &PageView, fields: &[FieldView], tokens: &'a Tokens) -> Element<'a, Message> {
    crate::view::page_section(page, fields, tokens)
}
