//! The settings pages: one per area of the configuration.
//!
//! A page is a **pure description** — the fields it edits, the label of its
//! entry in the navigation list, and the message ids it needs — plus the
//! builder that draws it (`view`). Nothing on a page holds state: the values
//! come from [`crate::draft`], what the page shows comes from
//! [`crate::view_model`], and the widgets come from [`crate::view`].
//!
//! The settings pages are the areas of PRD §9 the settings application owns:
//!
//! | page | file | what it edits |
//! |---|---|---|
//! | [`appearance`] | `config.toml` | the theme the session selects |
//! | [`panel`] | `shell.toml` | the panel's edge, height, autohiding and clock |
//! | [`notifications`] | `notifications.toml` | do-not-disturb, position, timeouts |
//! | [`locale`] | `config.toml` | the language the session runs in |
//!
//! Every field belongs to exactly one page, which `tests/workflow.rs` asserts
//! against [`Field::ALL`]: a field nobody draws would be editable only by
//! editing the file by hand, and a field drawn twice would be a second control
//! for one key.

pub mod appearance;
pub mod locale;
pub mod notifications;
pub mod panel;

use iced::Element;
use inox_config::Theme as Tokens;

use crate::app::Message;
use crate::draft::Field;
use crate::view_model::{FieldView, PageView, SettingsViewModel};

/// One page of the settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PageId {
    /// Trusted input method startup settings.
    InputMethod,
    /// Saved output preferences for the next compositor start.
    Displays,
    /// Input preferences.
    Input,
    /// Layout preferences.
    Layout,
    /// Launcher preferences.
    Launcher,
    /// Wallpaper preferences.
    Wallpaper,

    /// Which theme the session uses.
    Appearance,
    /// The panel's edge, height and clock.
    Panel,
    /// Popups: where they appear, how long they stay, and do-not-disturb.
    Notifications,
    /// The language the session runs in.
    Locale,
}

impl PageId {
    /// Every page, in the order the navigation list draws them.
    pub const ALL: [Self; 10] = [
        Self::Appearance,
        Self::Panel,
        Self::Notifications,
        Self::Locale,
        Self::Input,
        Self::Layout,
        Self::Launcher,
        Self::Wallpaper,
        Self::Displays,
        Self::InputMethod,
    ];

    /// The fields the page edits.
    pub fn fields(self) -> &'static [Field] {
        match self {
            Self::InputMethod => &[Field::InputMethodCommand],
            Self::Displays => &[Field::Outputs],
            Self::Input => &[
                Field::KeyboardLayout,
                Field::KeyboardVariant,
                Field::KeyboardRepeatDelay,
                Field::KeyboardRepeatRate,
                Field::TapToClick,
                Field::NaturalScroll,
            ],
            Self::Layout => &[
                Field::LayoutMasterRatio,
                Field::LayoutMasterCount,
                Field::LayoutGap,
            ],
            Self::Launcher => &[
                Field::LauncherTerminal,
                Field::LauncherMaxResults,
                Field::LauncherIconSize,
                Field::LauncherIncludeHidden,
                Field::LauncherMaxQueryChars,
            ],
            Self::Wallpaper => &[
                Field::WallpaperImage,
                Field::WallpaperMode,
                Field::WallpaperPerOutput,
            ],
            Self::Appearance => appearance::FIELDS,
            Self::Panel => panel::FIELDS,
            Self::Notifications => notifications::FIELDS,
            Self::Locale => locale::FIELDS,
        }
    }

    /// The message id of the page's name.
    pub const fn label_id(self) -> &'static str {
        match self {
            Self::InputMethod => "settings-page-input-method",
            Self::Displays => "settings-page-displays",
            Self::Input => "settings-page-input",
            Self::Layout => "settings-page-layout",
            Self::Launcher => "settings-page-launcher",
            Self::Wallpaper => "settings-page-wallpaper",
            Self::Appearance => "settings-page-appearance",
            Self::Panel => "settings-page-panel",
            Self::Notifications => "settings-page-notifications",
            Self::Locale => "settings-page-locale",
        }
    }

    /// Every message id the page's name and its fields need.
    pub fn message_ids(self) -> Vec<&'static str> {
        let mut ids = vec![self.label_id()];

        for field in self.fields() {
            ids.extend(field.message_ids());
        }

        ids
    }
}

/// Draws the open page.
///
/// The window asks the page that is open to draw itself, which is what gives
/// each module in this tree its own `view` rather than one function with four
/// arms.
pub fn view<'a>(
    page: PageId,
    model: &SettingsViewModel,
    tokens: &'a Tokens,
) -> Element<'a, Message> {
    let entry = model
        .pages
        .iter()
        .find(|entry| entry.page == page)
        .cloned()
        .unwrap_or_else(|| standalone_entry(page, model));

    let fields: Vec<FieldView> = page
        .fields()
        .iter()
        .filter_map(|field| model.field(*field).cloned())
        .collect();

    match page {
        PageId::InputMethod
        | PageId::Displays
        | PageId::Input
        | PageId::Layout
        | PageId::Launcher
        | PageId::Wallpaper => crate::view::page_section(&entry, &fields, tokens),
        PageId::Appearance => appearance::view(&entry, &fields, tokens),
        PageId::Panel => panel::view(&entry, &fields, tokens),
        PageId::Notifications => notifications::view(&entry, &fields, tokens),
        PageId::Locale => locale::view(&entry, &fields, tokens),
    }
}

/// A page entry for the case where the model has none.
///
/// The projection draws every page, so this is only reachable for a hand-built
/// model; the page is drawn with its own message id rather than not at all.
fn standalone_entry(page: PageId, model: &SettingsViewModel) -> PageView {
    PageView {
        page,
        label: page.label_id().to_owned(),
        selected: page == model.page,
        focused: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_field_is_on_exactly_one_page() {
        let drawn: Vec<Field> = PageId::ALL
            .iter()
            .flat_map(|page| page.fields().iter().copied())
            .collect();
        let unique: BTreeSet<Field> = drawn.iter().copied().collect();

        assert_eq!(
            unique.len(),
            drawn.len(),
            "a field is drawn on more than one page"
        );
        assert_eq!(
            unique,
            Field::ALL.into_iter().collect::<BTreeSet<Field>>(),
            "a field is on no page at all"
        );
    }

    #[test]
    fn a_field_is_on_the_page_it_says_it_is() {
        for page in PageId::ALL {
            for field in page.fields() {
                assert_eq!(field.page(), page, "`{field}`");
            }
        }
    }

    #[test]
    fn every_page_has_a_name_and_fields() {
        for page in PageId::ALL {
            assert!(!page.fields().is_empty(), "{page:?} edits nothing");
            assert!(
                page.label_id().starts_with("settings-page-"),
                "{page:?} is named `{}`",
                page.label_id()
            );
        }
    }

    #[test]
    fn a_page_needs_the_ids_of_its_name_and_of_its_fields() {
        let ids = PageId::Notifications.message_ids();

        assert!(ids.contains(&"settings-page-notifications"));
        assert!(ids.contains(&Field::NotificationsNormalTimeout.label_id()));
        assert!(ids.contains(&"settings-unit-ms"));
        assert!(!ids.contains(&"settings-page-panel"));
    }
}
