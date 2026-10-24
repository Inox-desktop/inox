//! The settings pages: one per area of the configuration.
//!
//! A page is a **pure description** — the fields it edits, the label of its
//! entry in the navigation list, the sentence under its title, and the cards its
//! fields are grouped into — plus the builder that draws it (`view`). Nothing on
//! a page holds state: the values come from [`crate::draft`], what the page shows
//! comes from [`crate::view_model`], and the widgets come from [`crate::view`].
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
//! Every field belongs to exactly one page and to exactly one card of it, which
//! `tests/workflow.rs` asserts against [`Field::ALL`] and the unit tests here
//! assert against [`PageId::fields`]: a field nobody draws would be editable only
//! by editing the file by hand, and a field drawn twice would be a second control
//! for one key.
//!
//! **Grouping is part of the page, not of the drawing.** Which fields sit in one
//! card is what the guidelines mean by "相关字段同卡片" (ui-guidelines §4), and it
//! is decided here so the drawing code cannot invent a second grouping — or drop
//! a field between two of them.

pub mod appearance;
pub mod locale;
pub mod notifications;
pub mod panel;

use iced::Element;
use inox_config::Theme as Tokens;

use crate::app::Message;
use crate::draft::Field;
use crate::view_model::{FieldView, PageView, SettingsViewModel};

/// One card of a page: a heading and the fields under it.
///
/// A field appears in exactly one group of its page, and the groups of a page
/// are in the order they are drawn — which is also the order the keyboard walks
/// them (acceptance Q01: 键盘顺序与视觉顺序一致).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    /// The message id of the card's heading.
    pub title_id: &'static str,
    /// The fields the card holds, in the order they are drawn.
    pub fields: &'static [Field],
}

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

    /// The fields the page edits, in the order the page draws them.
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

    /// The cards the page's fields are drawn in, in order.
    ///
    /// The fields of every group, read in order, are exactly [`Self::fields`] —
    /// that is what keeps the keyboard's walk, the drawing and the file's keys
    /// in one order, and the unit tests below assert it for every page.
    pub fn groups(self) -> &'static [Group] {
        match self {
            Self::Appearance => &[Group {
                title_id: "settings-group-appearance-theme",
                fields: appearance::FIELDS,
            }],
            Self::Panel => &[
                Group {
                    title_id: "settings-group-panel-strip",
                    fields: &[
                        Field::PanelPosition,
                        Field::PanelHeight,
                        Field::PanelMargin,
                        Field::PanelExclusive,
                    ],
                },
                Group {
                    title_id: "settings-group-panel-behaviour",
                    fields: &[Field::PanelAutohide],
                },
                Group {
                    title_id: "settings-group-panel-clock",
                    fields: &[Field::PanelShowClock, Field::PanelClockFormat],
                },
                Group {
                    title_id: "settings-group-panel-tasks",
                    fields: &[Field::PanelMaxTaskButtons, Field::PanelIconSize],
                },
            ],
            Self::Notifications => &[
                Group {
                    title_id: "settings-group-notifications-display",
                    fields: &[
                        Field::NotificationsPosition,
                        Field::NotificationsWidth,
                        Field::NotificationsMargin,
                        Field::NotificationsIconSize,
                        Field::NotificationsMaxVisible,
                    ],
                },
                Group {
                    title_id: "settings-group-notifications-timeouts",
                    fields: &[
                        Field::NotificationsLowTimeout,
                        Field::NotificationsNormalTimeout,
                        Field::NotificationsCriticalTimeout,
                    ],
                },
                Group {
                    title_id: "settings-group-notifications-queue",
                    fields: &[
                        Field::NotificationsMaxPending,
                        Field::NotificationsQueuePolicy,
                    ],
                },
                Group {
                    title_id: "settings-group-notifications-policy",
                    fields: &[
                        Field::NotificationsDoNotDisturb,
                        Field::NotificationsShowActions,
                        Field::NotificationsHideBodyWhenLocked,
                    ],
                },
            ],
            Self::Locale => &[Group {
                title_id: "settings-group-locale-language",
                fields: locale::FIELDS,
            }],
            Self::Input => &[
                Group {
                    title_id: "settings-group-input-keyboard",
                    fields: &[Field::KeyboardLayout, Field::KeyboardVariant],
                },
                Group {
                    title_id: "settings-group-input-repeat",
                    fields: &[Field::KeyboardRepeatDelay, Field::KeyboardRepeatRate],
                },
                Group {
                    title_id: "settings-group-input-pointer",
                    fields: &[Field::TapToClick, Field::NaturalScroll],
                },
            ],
            Self::Layout => &[Group {
                title_id: "settings-group-layout-tiling",
                fields: &[
                    Field::LayoutMasterRatio,
                    Field::LayoutMasterCount,
                    Field::LayoutGap,
                ],
            }],
            Self::Launcher => &[
                Group {
                    title_id: "settings-group-launcher-command",
                    fields: &[Field::LauncherTerminal],
                },
                Group {
                    title_id: "settings-group-launcher-results",
                    fields: &[
                        Field::LauncherMaxResults,
                        Field::LauncherIconSize,
                        Field::LauncherIncludeHidden,
                        Field::LauncherMaxQueryChars,
                    ],
                },
            ],
            Self::Wallpaper => &[
                Group {
                    title_id: "settings-group-wallpaper-image",
                    fields: &[Field::WallpaperImage, Field::WallpaperMode],
                },
                Group {
                    title_id: "settings-group-wallpaper-outputs",
                    fields: &[Field::WallpaperPerOutput],
                },
            ],
            Self::Displays => &[Group {
                title_id: "settings-group-displays-outputs",
                fields: &[Field::Outputs],
            }],
            Self::InputMethod => &[Group {
                title_id: "settings-group-input-method-command",
                fields: &[Field::InputMethodCommand],
            }],
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

    /// The message id of the sentence under the page's title.
    ///
    /// One screen carries one title and one explanation of what the screen is
    /// for; the cards under it carry the headings (ui-guidelines §2).
    pub const fn description_id(self) -> &'static str {
        match self {
            Self::InputMethod => "settings-page-input-method-description",
            Self::Displays => "settings-page-displays-description",
            Self::Input => "settings-page-input-description",
            Self::Layout => "settings-page-layout-description",
            Self::Launcher => "settings-page-launcher-description",
            Self::Wallpaper => "settings-page-wallpaper-description",
            Self::Appearance => "settings-page-appearance-description",
            Self::Panel => "settings-page-panel-description",
            Self::Notifications => "settings-page-notifications-description",
            Self::Locale => "settings-page-locale-description",
        }
    }

    /// Every message id the page's name, its explanation, its cards and its
    /// fields need.
    pub fn message_ids(self) -> Vec<&'static str> {
        let mut ids = vec![self.label_id(), self.description_id()];

        for group in self.groups() {
            ids.push(group.title_id);
        }

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
        description: page.description_id().to_owned(),
        groups: page
            .groups()
            .iter()
            .map(|group| crate::view_model::GroupView {
                title: group.title_id.to_owned(),
                fields: group.fields.to_vec(),
            })
            .collect(),
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
            assert!(
                page.description_id().starts_with("settings-page-"),
                "{page:?} is explained by `{}`",
                page.description_id()
            );
        }
    }

    #[test]
    fn the_cards_of_a_page_hold_exactly_its_fields_in_the_order_it_draws_them() {
        for page in PageId::ALL {
            let grouped: Vec<Field> = page
                .groups()
                .iter()
                .flat_map(|group| group.fields.iter().copied())
                .collect();

            assert_eq!(
                grouped,
                page.fields().to_vec(),
                "{page:?}: the cards and the page disagree, so the keyboard would walk \
                 the window in another order than it is drawn"
            );

            let unique: BTreeSet<Field> = grouped.iter().copied().collect();
            assert_eq!(
                unique.len(),
                grouped.len(),
                "{page:?}: a field is in two cards"
            );

            for group in page.groups() {
                assert!(!group.fields.is_empty(), "{page:?}: an empty card");
                assert!(
                    group.title_id.starts_with("settings-group-"),
                    "{page:?} has a card titled `{}`",
                    group.title_id
                );
            }
        }
    }

    #[test]
    fn a_page_needs_the_ids_of_its_name_and_of_its_fields() {
        let ids = PageId::Notifications.message_ids();

        assert!(ids.contains(&"settings-page-notifications"));
        assert!(ids.contains(&"settings-page-notifications-description"));
        assert!(ids.contains(&"settings-group-notifications-timeouts"));
        assert!(ids.contains(&Field::NotificationsNormalTimeout.label_id()));
        assert!(ids.contains(&"settings-unit-ms"));
        assert!(!ids.contains(&"settings-page-panel"));
    }
}
