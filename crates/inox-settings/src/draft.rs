//! The draft: the settings a user is editing, laid over the ones on disk.
//!
//! ADR 0014 asks the settings application to keep two things apart — what has
//! been **saved** and what is **in effect** — and this module is the first half
//! of that: a [`Draft`] holds the values the window is showing, the values the
//! file was read with, and the set of fields the user actually changed. A save
//! writes the changed fields and nothing else, so a preference the user never
//! opened cannot be rewritten by the act of saving an unrelated one.
//!
//! Three rules are load-bearing, and each has tests:
//!
//! - **A field that was never touched is never written.** [`Draft::changed`] is
//!   the only thing a save acts on, and a field whose value is put back to what
//!   the file already had stops being a change.
//! - **Discarding writes nothing at all.** [`Draft::discard`] — what the Cancel
//!   action calls — returns every value to the loaded one and forgets every
//!   touch. Nothing else in the crate has a way to reach the file.
//! - **A reload does not throw the user's work away.** [`Draft::rebase`] takes a
//!   freshly loaded configuration and keeps the fields the user changed, which
//!   is what makes an external edit findable (acceptance D04) without silently
//!   reverting an open window.
//!
//! [`Field`] is the closed set of settings this application edits, each one
//! identified by its dotted path *inside its file* — `panel.height`,
//! `timeouts.normal_ms` — which is the same vocabulary `inox-config` uses in a
//! parse error, so a value the user changed here and a value the loader refused
//! at startup are named the same way.

use std::collections::{BTreeMap, BTreeSet};

use inox_config::{
    Appearance, InoxConfig, NotificationPosition, NotificationsConfig, PanelPosition, RootConfig,
    ShellConfig,
};

use crate::document::{Edit, SettingFile, Toml};
use crate::pages::PageId;

/// The choice that means "no `locale.language` in the file".
///
/// Following the environment is the absence of the key rather than a value of
/// it: `config.toml` is read by every component, and a tag written here would
/// pin the session's language even when the user's environment asks for
/// another one.
pub const ENVIRONMENT: &str = "environment";

/// The identifier of `light` in the appearance choices.
const LIGHT: &str = "light";

/// The identifier of `dark` in the appearance choices.
const DARK: &str = "dark";

/// The identifier of `system` in the appearance choices.
const SYSTEM: &str = "system";

/// One setting the application can edit.
///
/// The set is closed on purpose: it is what the pages draw, what validation
/// walks, what the draft tracks and what a save writes. Adding a field means
/// adding an arm here and its message ids to both catalogs; the parity test
/// fails until the catalogs agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// `input.input_method`: trusted engine executable and literal arguments.
    InputMethodCommand,
    /// `outputs` preferences.
    Outputs,
    /// `launcher.terminal` preferences.
    LauncherTerminal,
    /// `wallpaper.per_output` preferences.
    WallpaperPerOutput,

    /// `panel.margin` in `shell.toml`.
    PanelMargin,
    /// `panel.exclusive` in `shell.toml`.
    PanelExclusive,
    /// `panel.clock_format` in `shell.toml`.
    PanelClockFormat,
    /// `panel.max_task_buttons` in `shell.toml`.
    PanelMaxTaskButtons,
    /// `panel.icon_size` in `shell.toml`.
    PanelIconSize,
    /// `launcher.max_results` in `shell.toml`.
    LauncherMaxResults,
    /// `launcher.icon_size` in `shell.toml`.
    LauncherIconSize,
    /// `launcher.include_hidden` in `shell.toml`.
    LauncherIncludeHidden,
    /// `launcher.max_query_chars` in `shell.toml`.
    LauncherMaxQueryChars,
    /// `wallpaper.image` in `shell.toml`.
    WallpaperImage,
    /// `wallpaper.mode` in `shell.toml`.
    WallpaperMode,
    /// `input.keyboard_layout` in `compositor.toml`.
    KeyboardLayout,
    /// `input.keyboard_variant` in `compositor.toml`.
    KeyboardVariant,
    /// `input.keyboard_repeat_delay_ms` in `compositor.toml`.
    KeyboardRepeatDelay,
    /// `input.keyboard_repeat_rate_hz` in `compositor.toml`.
    KeyboardRepeatRate,
    /// `input.tap_to_click` in `compositor.toml`.
    TapToClick,
    /// `input.natural_scroll` in `compositor.toml`.
    NaturalScroll,
    /// `layout.master_ratio` in `compositor.toml`.
    LayoutMasterRatio,
    /// `layout.master_count` in `compositor.toml`.
    LayoutMasterCount,
    /// `layout.gap` in `compositor.toml`.
    LayoutGap,
    /// `display.max_visible` in `notifications.toml`.
    NotificationsMaxVisible,
    /// `display.width` in `notifications.toml`.
    NotificationsWidth,
    /// `display.margin` in `notifications.toml`.
    NotificationsMargin,
    /// `display.icon_size` in `notifications.toml`.
    NotificationsIconSize,
    /// `queue.max_pending` in `notifications.toml`.
    NotificationsMaxPending,
    /// `queue.policy` in `notifications.toml`.
    NotificationsQueuePolicy,
    /// `policy.show_actions` in `notifications.toml`.
    NotificationsShowActions,
    /// `policy.hide_body_when_locked` in `notifications.toml`.
    NotificationsHideBodyWhenLocked,
    /// `theme.appearance` in `config.toml`: light, dark, or the session's.
    ThemeAppearance,
    /// `locale.language` in `config.toml`: the session's language.
    LocaleLanguage,
    /// `panel.position` in `shell.toml`.
    PanelPosition,
    /// `panel.height` in `shell.toml`.
    PanelHeight,
    /// `panel.autohide` in `shell.toml`.
    PanelAutohide,
    /// `panel.show_clock` in `shell.toml`.
    PanelShowClock,
    /// `policy.do_not_disturb` in `notifications.toml`.
    NotificationsDoNotDisturb,
    /// `display.position` in `notifications.toml`.
    NotificationsPosition,
    /// `timeouts.low_ms` in `notifications.toml`.
    NotificationsLowTimeout,
    /// `timeouts.normal_ms` in `notifications.toml`.
    NotificationsNormalTimeout,
    /// `timeouts.critical_ms` in `notifications.toml`.
    NotificationsCriticalTimeout,
}

/// The choices of the appearance field, in the order they are drawn.
const APPEARANCE_CHOICES: &[Choice] = &[
    Choice {
        id: LIGHT,
        label_id: "settings-choice-theme-light",
    },
    Choice {
        id: DARK,
        label_id: "settings-choice-theme-dark",
    },
    Choice {
        id: SYSTEM,
        label_id: "settings-choice-theme-system",
    },
];

/// The choices of the language field.
const LOCALE_CHOICES: &[Choice] = &[
    Choice {
        id: ENVIRONMENT,
        label_id: "settings-choice-locale-environment",
    },
    Choice {
        id: "en-US",
        label_id: "settings-choice-locale-en-us",
    },
    Choice {
        id: "zh-CN",
        label_id: "settings-choice-locale-zh-cn",
    },
];

/// The choices of the panel position field.
const PANEL_POSITION_CHOICES: &[Choice] = &[
    Choice {
        id: "top",
        label_id: "settings-choice-panel-top",
    },
    Choice {
        id: "bottom",
        label_id: "settings-choice-panel-bottom",
    },
];

/// The choices of the notification position field, in reading order.
const NOTIFICATION_POSITION_CHOICES: &[Choice] = &[
    Choice {
        id: "top-left",
        label_id: "settings-choice-notifications-top-left",
    },
    Choice {
        id: "top-right",
        label_id: "settings-choice-notifications-top-right",
    },
    Choice {
        id: "bottom-left",
        label_id: "settings-choice-notifications-bottom-left",
    },
    Choice {
        id: "bottom-right",
        label_id: "settings-choice-notifications-bottom-right",
    },
];

/// The height range `inox-config` accepts for `panel.height`.
///
/// The limits are the ones `inox-config` enforces while parsing, which are
/// private to that crate; `tests/workflow.rs` proves the mirror agrees by
/// handing the values either side of each limit to `InoxConfig::load` and
/// checking which ones it accepts. The widget clamps to this range, so a value
/// the schema rejects cannot be entered.
const PANEL_HEIGHT_RANGE: Range = Range {
    min: 8,
    max: 256,
    step: 1,
    unit_id: "settings-unit-px",
};

/// The lifetime range `inox-config` accepts for each notification timeout.
const TIMEOUT_RANGE: Range = Range {
    min: 0,
    max: 600_000,
    step: 1_000,
    unit_id: "settings-unit-ms",
};

const WALLPAPER_MODE_CHOICES: &[Choice] = &[
    Choice {
        id: "fill",
        label_id: "settings-choice-fill",
    },
    Choice {
        id: "fit",
        label_id: "settings-choice-fit",
    },
    Choice {
        id: "stretch",
        label_id: "settings-choice-stretch",
    },
    Choice {
        id: "center",
        label_id: "settings-choice-center",
    },
    Choice {
        id: "tile",
        label_id: "settings-choice-tile",
    },
];

const QUEUE_POLICY_CHOICES: &[Choice] = &[
    Choice {
        id: "drop-oldest",
        label_id: "settings-choice-drop-oldest",
    },
    Choice {
        id: "coalesce-by-app",
        label_id: "settings-choice-coalesce-by-app",
    },
];

/// The wallpapers `wallpaper.image` can be set to, in the order they are drawn.
///
/// The identifiers are the ones `inox-config` resolves ([`inox_config::wallpapers`]):
/// the empty string for "whatever the session ships as its default", each entry
/// of the shipped catalogue, and [`inox_config::BACKGROUND_ID`] for the theme
/// background. A path a user typed by hand is deliberately *not* here — the
/// projection appends whatever the configuration holds when it is not one of
/// these, so a hand-written path still shows as the chosen option.
///
/// The label ids are the window's business (they name Fluent messages), which is
/// why this list is not built from the catalogue; a unit test keeps the two in
/// step instead.
const WALLPAPER_CHOICES: &[Choice] = &[
    Choice {
        id: "",
        label_id: "settings-choice-wallpaper-default",
    },
    Choice {
        id: "nebula",
        label_id: "settings-choice-wallpaper-nebula",
    },
    Choice {
        id: "fuji",
        label_id: "settings-choice-wallpaper-fuji",
    },
    Choice {
        id: "bricks",
        label_id: "settings-choice-wallpaper-bricks",
    },
    Choice {
        id: "lake",
        label_id: "settings-choice-wallpaper-lake",
    },
    Choice {
        id: "rainbow",
        label_id: "settings-choice-wallpaper-rainbow",
    },
    Choice {
        id: "monument",
        label_id: "settings-choice-wallpaper-monument",
    },
    Choice {
        id: inox_config::BACKGROUND_ID,
        label_id: "settings-choice-wallpaper-background",
    },
];

impl Field {
    /// Every field, in the order the pages draw them.
    pub const ALL: [Self; 43] = [
        Self::InputMethodCommand,
        Self::Outputs,
        Self::LauncherTerminal,
        Self::WallpaperPerOutput,
        Self::PanelMargin,
        Self::PanelExclusive,
        Self::PanelClockFormat,
        Self::PanelMaxTaskButtons,
        Self::PanelIconSize,
        Self::LauncherMaxResults,
        Self::LauncherIconSize,
        Self::LauncherIncludeHidden,
        Self::LauncherMaxQueryChars,
        Self::WallpaperImage,
        Self::WallpaperMode,
        Self::KeyboardLayout,
        Self::KeyboardVariant,
        Self::KeyboardRepeatDelay,
        Self::KeyboardRepeatRate,
        Self::TapToClick,
        Self::NaturalScroll,
        Self::LayoutMasterRatio,
        Self::LayoutMasterCount,
        Self::LayoutGap,
        Self::NotificationsMaxVisible,
        Self::NotificationsWidth,
        Self::NotificationsMargin,
        Self::NotificationsIconSize,
        Self::NotificationsMaxPending,
        Self::NotificationsQueuePolicy,
        Self::NotificationsShowActions,
        Self::NotificationsHideBodyWhenLocked,
        Self::ThemeAppearance,
        Self::LocaleLanguage,
        Self::PanelPosition,
        Self::PanelHeight,
        Self::PanelAutohide,
        Self::PanelShowClock,
        Self::NotificationsPosition,
        Self::NotificationsDoNotDisturb,
        Self::NotificationsLowTimeout,
        Self::NotificationsNormalTimeout,
        Self::NotificationsCriticalTimeout,
    ];

    /// The dotted path of the key inside its file.
    ///
    /// These are the paths `inox-config` reports in a parse error, so one
    /// vocabulary covers both what this window edits and what a startup failure
    /// names.
    pub const fn path(self) -> &'static str {
        match self {
            Self::InputMethodCommand => "input.input_method",
            Self::Outputs => "outputs",
            Self::LauncherTerminal => "launcher.terminal",
            Self::WallpaperPerOutput => "wallpaper.per_output",

            Self::PanelMargin => "panel.margin",
            Self::PanelExclusive => "panel.exclusive",
            Self::PanelClockFormat => "panel.clock_format",
            Self::PanelMaxTaskButtons => "panel.max_task_buttons",
            Self::PanelIconSize => "panel.icon_size",
            Self::LauncherMaxResults => "launcher.max_results",
            Self::LauncherIconSize => "launcher.icon_size",
            Self::LauncherIncludeHidden => "launcher.include_hidden",
            Self::LauncherMaxQueryChars => "launcher.max_query_chars",
            Self::WallpaperImage => "wallpaper.image",
            Self::WallpaperMode => "wallpaper.mode",
            Self::KeyboardLayout => "input.keyboard_layout",
            Self::KeyboardVariant => "input.keyboard_variant",
            Self::KeyboardRepeatDelay => "input.keyboard_repeat_delay_ms",
            Self::KeyboardRepeatRate => "input.keyboard_repeat_rate_hz",
            Self::TapToClick => "input.tap_to_click",
            Self::NaturalScroll => "input.natural_scroll",
            Self::LayoutMasterRatio => "layout.master_ratio",
            Self::LayoutMasterCount => "layout.master_count",
            Self::LayoutGap => "layout.gap",
            Self::NotificationsMaxVisible => "display.max_visible",
            Self::NotificationsWidth => "display.width",
            Self::NotificationsMargin => "display.margin",
            Self::NotificationsIconSize => "display.icon_size",
            Self::NotificationsMaxPending => "queue.max_pending",
            Self::NotificationsQueuePolicy => "queue.policy",
            Self::NotificationsShowActions => "policy.show_actions",
            Self::NotificationsHideBodyWhenLocked => "policy.hide_body_when_locked",

            Self::ThemeAppearance => "theme.appearance",
            Self::LocaleLanguage => "locale.language",
            Self::PanelPosition => "panel.position",
            Self::PanelHeight => "panel.height",
            Self::PanelAutohide => "panel.autohide",
            Self::PanelShowClock => "panel.show_clock",
            Self::NotificationsDoNotDisturb => "policy.do_not_disturb",
            Self::NotificationsPosition => "display.position",
            Self::NotificationsLowTimeout => "timeouts.low_ms",
            Self::NotificationsNormalTimeout => "timeouts.normal_ms",
            Self::NotificationsCriticalTimeout => "timeouts.critical_ms",
        }
    }

    /// The file the key lives in.
    pub const fn file(self) -> SettingFile {
        match self {
            Self::InputMethodCommand => SettingFile::Compositor,
            Self::Outputs => SettingFile::Compositor,
            Self::LauncherTerminal => SettingFile::Shell,
            Self::WallpaperPerOutput => SettingFile::Shell,

            Self::PanelMargin => SettingFile::Shell,
            Self::PanelExclusive => SettingFile::Shell,
            Self::PanelClockFormat => SettingFile::Shell,
            Self::PanelMaxTaskButtons => SettingFile::Shell,
            Self::PanelIconSize => SettingFile::Shell,
            Self::LauncherMaxResults => SettingFile::Shell,
            Self::LauncherIconSize => SettingFile::Shell,
            Self::LauncherIncludeHidden => SettingFile::Shell,
            Self::LauncherMaxQueryChars => SettingFile::Shell,
            Self::WallpaperImage => SettingFile::Shell,
            Self::WallpaperMode => SettingFile::Shell,
            Self::KeyboardLayout => SettingFile::Compositor,
            Self::KeyboardVariant => SettingFile::Compositor,
            Self::KeyboardRepeatDelay => SettingFile::Compositor,
            Self::KeyboardRepeatRate => SettingFile::Compositor,
            Self::TapToClick => SettingFile::Compositor,
            Self::NaturalScroll => SettingFile::Compositor,
            Self::LayoutMasterRatio => SettingFile::Compositor,
            Self::LayoutMasterCount => SettingFile::Compositor,
            Self::LayoutGap => SettingFile::Compositor,
            Self::NotificationsMaxVisible => SettingFile::Notifications,
            Self::NotificationsWidth => SettingFile::Notifications,
            Self::NotificationsMargin => SettingFile::Notifications,
            Self::NotificationsIconSize => SettingFile::Notifications,
            Self::NotificationsMaxPending => SettingFile::Notifications,
            Self::NotificationsQueuePolicy => SettingFile::Notifications,
            Self::NotificationsShowActions => SettingFile::Notifications,
            Self::NotificationsHideBodyWhenLocked => SettingFile::Notifications,

            Self::ThemeAppearance | Self::LocaleLanguage => SettingFile::Root,
            Self::PanelPosition
            | Self::PanelHeight
            | Self::PanelAutohide
            | Self::PanelShowClock => SettingFile::Shell,
            Self::NotificationsDoNotDisturb
            | Self::NotificationsPosition
            | Self::NotificationsLowTimeout
            | Self::NotificationsNormalTimeout
            | Self::NotificationsCriticalTimeout => SettingFile::Notifications,
        }
    }

    /// The page the field is edited on.
    pub const fn page(self) -> PageId {
        match self {
            Self::InputMethodCommand => PageId::InputMethod,
            Self::Outputs => PageId::Displays,
            Self::LauncherTerminal => PageId::Launcher,
            Self::WallpaperPerOutput => PageId::Wallpaper,

            Self::PanelMargin => PageId::Panel,
            Self::PanelExclusive => PageId::Panel,
            Self::PanelClockFormat => PageId::Panel,
            Self::PanelMaxTaskButtons => PageId::Panel,
            Self::PanelIconSize => PageId::Panel,
            Self::LauncherMaxResults => PageId::Launcher,
            Self::LauncherIconSize => PageId::Launcher,
            Self::LauncherIncludeHidden => PageId::Launcher,
            Self::LauncherMaxQueryChars => PageId::Launcher,
            Self::WallpaperImage => PageId::Wallpaper,
            Self::WallpaperMode => PageId::Wallpaper,
            Self::KeyboardLayout => PageId::Input,
            Self::KeyboardVariant => PageId::Input,
            Self::KeyboardRepeatDelay => PageId::Input,
            Self::KeyboardRepeatRate => PageId::Input,
            Self::TapToClick => PageId::Input,
            Self::NaturalScroll => PageId::Input,
            Self::LayoutMasterRatio => PageId::Layout,
            Self::LayoutMasterCount => PageId::Layout,
            Self::LayoutGap => PageId::Layout,
            Self::NotificationsMaxVisible => PageId::Notifications,
            Self::NotificationsWidth => PageId::Notifications,
            Self::NotificationsMargin => PageId::Notifications,
            Self::NotificationsIconSize => PageId::Notifications,
            Self::NotificationsMaxPending => PageId::Notifications,
            Self::NotificationsQueuePolicy => PageId::Notifications,
            Self::NotificationsShowActions => PageId::Notifications,
            Self::NotificationsHideBodyWhenLocked => PageId::Notifications,

            Self::ThemeAppearance => PageId::Appearance,
            Self::LocaleLanguage => PageId::Locale,
            Self::PanelPosition
            | Self::PanelHeight
            | Self::PanelAutohide
            | Self::PanelShowClock => PageId::Panel,
            Self::NotificationsDoNotDisturb
            | Self::NotificationsPosition
            | Self::NotificationsLowTimeout
            | Self::NotificationsNormalTimeout
            | Self::NotificationsCriticalTimeout => PageId::Notifications,
        }
    }

    /// The message id of the field's label.
    pub const fn label_id(self) -> &'static str {
        match self {
            Self::InputMethodCommand => "settings-field-input-method-command",
            Self::Outputs => "settings-field-outputs",
            Self::LauncherTerminal => "settings-field-launcherterminal",
            Self::WallpaperPerOutput => "settings-field-wallpaperperoutput",

            Self::PanelMargin => "settings-field-panel-margin",
            Self::PanelExclusive => "settings-field-panel-exclusive",
            Self::PanelClockFormat => "settings-field-panel-clock-format",
            Self::PanelMaxTaskButtons => "settings-field-panel-max-task-buttons",
            Self::PanelIconSize => "settings-field-panel-icon-size",
            Self::LauncherMaxResults => "settings-field-launcher-max-results",
            Self::LauncherIconSize => "settings-field-launcher-icon-size",
            Self::LauncherIncludeHidden => "settings-field-launcher-include-hidden",
            Self::LauncherMaxQueryChars => "settings-field-launcher-max-query-chars",
            Self::WallpaperImage => "settings-field-wallpaper-image",
            Self::WallpaperMode => "settings-field-wallpaper-mode",
            Self::KeyboardLayout => "settings-field-keyboard-layout",
            Self::KeyboardVariant => "settings-field-keyboard-variant",
            Self::KeyboardRepeatDelay => "settings-field-keyboard-repeat-delay",
            Self::KeyboardRepeatRate => "settings-field-keyboard-repeat-rate",
            Self::TapToClick => "settings-field-tap-to-click",
            Self::NaturalScroll => "settings-field-natural-scroll",
            Self::LayoutMasterRatio => "settings-field-layout-master-ratio",
            Self::LayoutMasterCount => "settings-field-layout-master-count",
            Self::LayoutGap => "settings-field-layout-gap",
            Self::NotificationsMaxVisible => "settings-field-notifications-max-visible",
            Self::NotificationsWidth => "settings-field-notifications-width",
            Self::NotificationsMargin => "settings-field-notifications-margin",
            Self::NotificationsIconSize => "settings-field-notifications-icon-size",
            Self::NotificationsMaxPending => "settings-field-notifications-max-pending",
            Self::NotificationsQueuePolicy => "settings-field-notifications-queue-policy",
            Self::NotificationsShowActions => "settings-field-notifications-show-actions",
            Self::NotificationsHideBodyWhenLocked => {
                "settings-field-notifications-hide-body-when-locked"
            }

            Self::ThemeAppearance => "settings-field-theme-appearance",
            Self::LocaleLanguage => "settings-field-locale-language",
            Self::PanelPosition => "settings-field-panel-position",
            Self::PanelHeight => "settings-field-panel-height",
            Self::PanelAutohide => "settings-field-panel-autohide",
            Self::PanelShowClock => "settings-field-panel-show-clock",
            Self::NotificationsDoNotDisturb => "settings-field-notifications-do-not-disturb",
            Self::NotificationsPosition => "settings-field-notifications-position",
            Self::NotificationsLowTimeout => "settings-field-notifications-low-timeout",
            Self::NotificationsNormalTimeout => "settings-field-notifications-normal-timeout",
            Self::NotificationsCriticalTimeout => "settings-field-notifications-critical-timeout",
        }
    }

    /// How the field is edited: a row of choices, a switch, or a number between
    /// two limits.
    pub const fn control(self) -> Control {
        match self {
            Self::InputMethodCommand => Control::COLLECTION,
            Self::Outputs => Control::COLLECTION,
            Self::LauncherTerminal => Control::COLLECTION,
            Self::WallpaperPerOutput => Control::COLLECTION,

            Self::PanelMargin => Control::number(Range {
                min: 0,
                max: 64,
                step: 1,
                unit_id: "settings-unit-px",
            }),
            Self::PanelExclusive => Control::FLAG,
            Self::PanelClockFormat => Control::TEXT,
            Self::PanelMaxTaskButtons => Control::number(Range {
                min: 1,
                max: 64,
                step: 1,
                unit_id: "settings-unit-count",
            }),
            Self::PanelIconSize => Control::number(Range {
                min: 8,
                max: 128,
                step: 1,
                unit_id: "settings-unit-px",
            }),
            Self::LauncherMaxResults => Control::number(Range {
                min: 1,
                max: 64,
                step: 1,
                unit_id: "settings-unit-count",
            }),
            Self::LauncherIconSize => Control::number(Range {
                min: 8,
                max: 256,
                step: 1,
                unit_id: "settings-unit-px",
            }),
            Self::LauncherIncludeHidden => Control::FLAG,
            Self::LauncherMaxQueryChars => Control::number(Range {
                min: 1,
                max: 512,
                step: 1,
                unit_id: "settings-unit-count",
            }),
            Self::WallpaperImage => Control::choices(WALLPAPER_CHOICES),
            Self::WallpaperMode => Control::choices(WALLPAPER_MODE_CHOICES),
            Self::KeyboardLayout => Control::TEXT,
            Self::KeyboardVariant => Control::TEXT,
            Self::KeyboardRepeatDelay => Control::number(Range {
                min: 0,
                max: 2147483647,
                step: 50,
                unit_id: "settings-unit-ms",
            }),
            Self::KeyboardRepeatRate => Control::number(Range {
                min: 0,
                max: 2147483647,
                step: 1,
                unit_id: "settings-unit-hz",
            }),
            Self::TapToClick => Control::FLAG,
            Self::NaturalScroll => Control::FLAG,
            Self::LayoutMasterRatio => Control::number(Range {
                min: 10,
                max: 90,
                step: 1,
                unit_id: "settings-unit-percent",
            }),
            Self::LayoutMasterCount => Control::number(Range {
                min: 1,
                max: 4294967295,
                step: 1,
                unit_id: "settings-unit-count",
            }),
            Self::LayoutGap => Control::number(Range {
                min: 0,
                max: 2147483647,
                step: 1,
                unit_id: "settings-unit-px",
            }),
            Self::NotificationsMaxVisible => Control::number(Range {
                min: 1,
                max: 16,
                step: 1,
                unit_id: "settings-unit-count",
            }),
            Self::NotificationsWidth => Control::number(Range {
                min: 160,
                max: 1024,
                step: 1,
                unit_id: "settings-unit-px",
            }),
            Self::NotificationsMargin => Control::number(Range {
                min: 0,
                max: 256,
                step: 1,
                unit_id: "settings-unit-px",
            }),
            Self::NotificationsIconSize => Control::number(Range {
                min: 8,
                max: 128,
                step: 1,
                unit_id: "settings-unit-px",
            }),
            Self::NotificationsMaxPending => Control::number(Range {
                min: 1,
                max: 1024,
                step: 1,
                unit_id: "settings-unit-count",
            }),
            Self::NotificationsQueuePolicy => Control::choices(QUEUE_POLICY_CHOICES),
            Self::NotificationsShowActions => Control::FLAG,
            Self::NotificationsHideBodyWhenLocked => Control::FLAG,

            Self::ThemeAppearance => Control::choices(APPEARANCE_CHOICES),
            Self::LocaleLanguage => Control::choices(LOCALE_CHOICES),
            Self::PanelPosition => Control::choices(PANEL_POSITION_CHOICES),
            Self::NotificationsPosition => Control::choices(NOTIFICATION_POSITION_CHOICES),
            Self::PanelHeight => Control::number(PANEL_HEIGHT_RANGE),
            Self::NotificationsLowTimeout
            | Self::NotificationsNormalTimeout
            | Self::NotificationsCriticalTimeout => Control::number(TIMEOUT_RANGE),
            Self::PanelAutohide | Self::PanelShowClock | Self::NotificationsDoNotDisturb => {
                Control::FLAG
            }
        }
    }

    /// Whether the field is edited as a structured collection.
    pub const fn is_collection(self) -> bool {
        matches!(
            self,
            Self::InputMethodCommand
                | Self::Outputs
                | Self::LauncherTerminal
                | Self::WallpaperPerOutput
        )
    }

    /// Every message id the field's label, choices and unit need.
    ///
    /// `tests/catalog.rs` walks this for every field, so a label that is renamed
    /// in one catalog and not the other — or a choice added without its
    /// translation — fails there rather than rendering as a raw id.
    pub fn message_ids(self) -> Vec<&'static str> {
        let control = self.control();
        let mut ids = vec![self.label_id()];
        ids.extend(control.choices.iter().map(|choice| choice.label_id));

        if let Some(range) = control.range {
            ids.push(range.unit_id);
        }

        ids
    }

    /// The value the field has in `settings`.
    ///
    /// Always a value of the shape [`Field::control`] offers.
    pub fn value(self, settings: &Settings) -> Value {
        match self {
            Self::InputMethodCommand => Value::Collection(
                settings
                    .input_method
                    .iter()
                    .cloned()
                    .map(|arg| vec![arg])
                    .collect(),
            ),
            Self::Outputs => Value::Collection(settings.outputs.clone()),
            Self::LauncherTerminal => Value::Collection(
                settings
                    .launcher_terminal
                    .iter()
                    .cloned()
                    .map(|arg| vec![arg])
                    .collect(),
            ),
            Self::WallpaperPerOutput => Value::Collection(settings.wallpaper_per_output.clone()),

            Self::PanelMargin => Value::Number(settings.panel_margin),
            Self::PanelExclusive => Value::Flag(settings.panel_exclusive),
            Self::PanelClockFormat => Value::Text(settings.panel_clock_format.clone()),
            Self::PanelMaxTaskButtons => Value::Number(settings.panel_max_task_buttons),
            Self::PanelIconSize => Value::Number(settings.panel_icon_size),
            Self::LauncherMaxResults => Value::Number(settings.launcher_max_results),
            Self::LauncherIconSize => Value::Number(settings.launcher_icon_size),
            Self::LauncherIncludeHidden => Value::Flag(settings.launcher_include_hidden),
            Self::LauncherMaxQueryChars => Value::Number(settings.launcher_max_query_chars),
            Self::WallpaperImage => Value::Choice(settings.wallpaper_image.clone()),
            Self::WallpaperMode => Value::Choice(settings.wallpaper_mode.clone()),
            Self::KeyboardLayout => Value::Text(settings.input_keyboard_layout.clone()),
            Self::KeyboardVariant => Value::Text(settings.input_keyboard_variant.clone()),
            Self::KeyboardRepeatDelay => Value::Number(settings.input_keyboard_repeat_delay_ms),
            Self::KeyboardRepeatRate => Value::Number(settings.input_keyboard_repeat_rate_hz),
            Self::TapToClick => Value::Flag(settings.input_tap_to_click),
            Self::NaturalScroll => Value::Flag(settings.input_natural_scroll),
            Self::LayoutMasterRatio => Value::Number(settings.layout_master_ratio),
            Self::LayoutMasterCount => Value::Number(settings.layout_master_count),
            Self::LayoutGap => Value::Number(settings.layout_gap),
            Self::NotificationsMaxVisible => Value::Number(settings.display_max_visible),
            Self::NotificationsWidth => Value::Number(settings.display_width),
            Self::NotificationsMargin => Value::Number(settings.display_margin),
            Self::NotificationsIconSize => Value::Number(settings.display_icon_size),
            Self::NotificationsMaxPending => Value::Number(settings.queue_max_pending),
            Self::NotificationsQueuePolicy => Value::Choice(settings.queue_policy.clone()),
            Self::NotificationsShowActions => Value::Flag(settings.policy_show_actions),
            Self::NotificationsHideBodyWhenLocked => {
                Value::Flag(settings.policy_hide_body_when_locked)
            }

            Self::ThemeAppearance => Value::Choice(appearance_id(settings.theme_appearance)),
            Self::LocaleLanguage => Value::Choice(settings.locale.id()),
            Self::PanelPosition => Value::Choice(panel_position_id(settings.panel_position)),
            Self::PanelHeight => Value::Number(settings.panel_height),
            Self::PanelAutohide => Value::Flag(settings.panel_autohide),
            Self::PanelShowClock => Value::Flag(settings.panel_show_clock),
            Self::NotificationsDoNotDisturb => Value::Flag(settings.notifications_do_not_disturb),
            Self::NotificationsPosition => {
                Value::Choice(notification_position_id(settings.notifications_position))
            }
            Self::NotificationsLowTimeout => Value::Number(settings.notifications_low_ms),
            Self::NotificationsNormalTimeout => Value::Number(settings.notifications_normal_ms),
            Self::NotificationsCriticalTimeout => Value::Number(settings.notifications_critical_ms),
        }
    }

    /// Writes `value` into `settings`.
    ///
    /// # Errors
    ///
    /// Returns [`DraftError::Mismatched`] when the value is not of the shape the
    /// field's control edits — a flag handed to a choice, say. This is the only
    /// check here: what the value *means* is [`mod@crate::validate`]'s job, so that
    /// a value the schema refuses can be shown to the user as a field error
    /// rather than disappearing into a rejected click.
    pub fn set(self, settings: &mut Settings, value: Value) -> Result<(), DraftError> {
        let expected = self.control().kind;
        let found = value.kind();

        if expected != found {
            return Err(DraftError::Mismatched {
                field: self,
                expected,
                found,
            });
        }

        self.assign(settings, value);

        Ok(())
    }

    /// The line this field's value becomes, or `None` when the field's setting
    /// is the absence of its key.
    ///
    /// # Errors
    ///
    /// Same as [`Field::set`].
    pub fn toml_value(self, value: &Value) -> Result<Option<Toml>, DraftError> {
        if self.control().kind != value.kind() {
            return Err(DraftError::Mismatched {
                field: self,
                expected: self.control().kind,
                found: value.kind(),
            });
        }

        if let Value::Collection(rows) = value {
            return crate::collections::toml(self, rows)
                .map(Some)
                .ok_or(DraftError::InvalidCollection { field: self });
        }
        if self == Self::LocaleLanguage
            && let Value::Choice(id) = value
            && id == ENVIRONMENT
        {
            return Ok(None);
        }

        // The default choice is the absence of the key, exactly as "follow the
        // environment" is for the locale: choosing it must remove the line, not
        // write an empty string the loader would then have to interpret.
        if self == Self::WallpaperImage && matches!(value, Value::Choice(id) if id.is_empty()) {
            return Ok(None);
        }

        Ok(Some(match value {
            Value::Collection(_) => return Err(DraftError::InvalidCollection { field: self }),
            Value::Choice(id) | Value::Text(id) => Toml::String(id.clone()),
            Value::Flag(flag) => Toml::Bool(*flag),
            Value::Number(number) => Toml::Integer(*number),
        }))
    }

    /// Writes the value without checking its shape.
    ///
    /// Only [`Field::value`] and the choices of [`Field::control`] may be handed
    /// to it, which is what [`Draft::rebase`] relies on: the value it carries
    /// over came from `value(settings)` a moment earlier.
    fn assign(self, settings: &mut Settings, value: Value) {
        match (self, value) {
            (Self::InputMethodCommand, Value::Collection(rows)) => {
                settings.input_method = rows
                    .into_iter()
                    .filter_map(|row| row.into_iter().next())
                    .collect()
            }
            (Self::Outputs, Value::Collection(rows)) => settings.outputs = rows,
            (Self::LauncherTerminal, Value::Collection(rows)) => {
                settings.launcher_terminal = rows
                    .into_iter()
                    .filter_map(|row| row.into_iter().next())
                    .collect()
            }
            (Self::WallpaperPerOutput, Value::Collection(rows)) => {
                settings.wallpaper_per_output = rows
            }

            (Self::PanelMargin, Value::Number(value)) => settings.panel_margin = value,
            (Self::PanelExclusive, Value::Flag(value)) => settings.panel_exclusive = value,
            (Self::PanelClockFormat, Value::Text(value)) => settings.panel_clock_format = value,
            (Self::PanelMaxTaskButtons, Value::Number(value)) => {
                settings.panel_max_task_buttons = value
            }
            (Self::PanelIconSize, Value::Number(value)) => settings.panel_icon_size = value,
            (Self::LauncherMaxResults, Value::Number(value)) => {
                settings.launcher_max_results = value
            }
            (Self::LauncherIconSize, Value::Number(value)) => settings.launcher_icon_size = value,
            (Self::LauncherIncludeHidden, Value::Flag(value)) => {
                settings.launcher_include_hidden = value
            }
            (Self::LauncherMaxQueryChars, Value::Number(value)) => {
                settings.launcher_max_query_chars = value
            }
            (Self::WallpaperImage, Value::Choice(value)) => settings.wallpaper_image = value,
            (Self::WallpaperMode, Value::Choice(value)) => settings.wallpaper_mode = value,
            (Self::KeyboardLayout, Value::Text(value)) => settings.input_keyboard_layout = value,
            (Self::KeyboardVariant, Value::Text(value)) => settings.input_keyboard_variant = value,
            (Self::KeyboardRepeatDelay, Value::Number(value)) => {
                settings.input_keyboard_repeat_delay_ms = value
            }
            (Self::KeyboardRepeatRate, Value::Number(value)) => {
                settings.input_keyboard_repeat_rate_hz = value
            }
            (Self::TapToClick, Value::Flag(value)) => settings.input_tap_to_click = value,
            (Self::NaturalScroll, Value::Flag(value)) => settings.input_natural_scroll = value,
            (Self::LayoutMasterRatio, Value::Number(value)) => settings.layout_master_ratio = value,
            (Self::LayoutMasterCount, Value::Number(value)) => settings.layout_master_count = value,
            (Self::LayoutGap, Value::Number(value)) => settings.layout_gap = value,
            (Self::NotificationsMaxVisible, Value::Number(value)) => {
                settings.display_max_visible = value
            }
            (Self::NotificationsWidth, Value::Number(value)) => settings.display_width = value,
            (Self::NotificationsMargin, Value::Number(value)) => settings.display_margin = value,
            (Self::NotificationsIconSize, Value::Number(value)) => {
                settings.display_icon_size = value
            }
            (Self::NotificationsMaxPending, Value::Number(value)) => {
                settings.queue_max_pending = value
            }
            (Self::NotificationsQueuePolicy, Value::Choice(value)) => settings.queue_policy = value,
            (Self::NotificationsShowActions, Value::Flag(value)) => {
                settings.policy_show_actions = value
            }
            (Self::NotificationsHideBodyWhenLocked, Value::Flag(value)) => {
                settings.policy_hide_body_when_locked = value
            }

            (Self::ThemeAppearance, Value::Choice(id)) => {
                settings.theme_appearance = appearance_of(&id);
            }
            (Self::LocaleLanguage, Value::Choice(id)) => settings.locale = LocaleChoice::new(id),
            (Self::PanelPosition, Value::Choice(id)) => {
                settings.panel_position = panel_position_of(&id);
            }
            (Self::PanelHeight, Value::Number(number)) => settings.panel_height = number,
            (Self::PanelAutohide, Value::Flag(flag)) => settings.panel_autohide = flag,
            (Self::PanelShowClock, Value::Flag(flag)) => settings.panel_show_clock = flag,
            (Self::NotificationsDoNotDisturb, Value::Flag(flag)) => {
                settings.notifications_do_not_disturb = flag;
            }
            (Self::NotificationsPosition, Value::Choice(id)) => {
                settings.notifications_position = notification_position_of(&id);
            }
            (Self::NotificationsLowTimeout, Value::Number(number)) => {
                settings.notifications_low_ms = number;
            }
            (Self::NotificationsNormalTimeout, Value::Number(number)) => {
                settings.notifications_normal_ms = number;
            }
            (Self::NotificationsCriticalTimeout, Value::Number(number)) => {
                settings.notifications_critical_ms = number;
            }
            // Unreachable: `set` compares the two kinds before calling this,
            // and `rebase` carries a value that came out of `value`.
            _ => {}
        }
    }
}

impl std::fmt::Display for Field {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.path())
    }
}

/// One option of a choice control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// The value written into the file.
    pub id: &'static str,
    /// The message id of the option's label.
    pub label_id: &'static str,
}

/// A number between two limits, stepping by a whole step.
///
/// The range is treated as a cycle: stepping past the end comes back to the
/// beginning, which is what lets a keyboard user reach every stop of a wide
/// range — the notification timeouts span six hundred of them — with one key
/// (acceptance Q01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// Smallest value the schema accepts.
    pub min: i64,
    /// Largest value the schema accepts.
    pub max: i64,
    /// Distance between two stops.
    pub step: i64,
    /// The message id of the unit shown beside the number.
    pub unit_id: &'static str,
}

impl Range {
    /// How many stops the range has.
    pub const fn stops(&self) -> i64 {
        (self.max - self.min) / self.step + 1
    }

    /// Whether the schema accepts `value`.
    pub const fn contains(&self, value: i64) -> bool {
        value >= self.min && value <= self.max
    }

    /// The stop `steps` away from the one nearest `value`, wrapping around.
    ///
    /// A value outside the range — which only a hand-edited draft can hold —
    /// snaps to the nearest stop rather than staying outside.
    pub fn advance(&self, value: i64, steps: i64) -> i64 {
        let current = value
            .saturating_sub(self.min)
            .div_euclid(self.step)
            .clamp(0, self.stops() - 1);

        self.min + (current + steps.rem_euclid(self.stops())).rem_euclid(self.stops()) * self.step
    }
}

/// How a field is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Control {
    /// The shape of the value the control produces.
    pub kind: ValueKind,
    /// The options, for a choice control; empty otherwise.
    pub choices: &'static [Choice],
    /// The limits, for a number control.
    pub range: Option<Range>,
}

impl Control {
    /// A row of options.
    pub const fn choices(choices: &'static [Choice]) -> Self {
        Self {
            kind: ValueKind::Choice,
            choices,
            range: None,
        }
    }

    /// A structured list of rows.
    pub const COLLECTION: Self = Self {
        kind: ValueKind::Collection,
        choices: &[],
        range: None,
    };

    /// Free-form text.
    pub const TEXT: Self = Self {
        kind: ValueKind::Text,
        choices: &[],
        range: None,
    };

    /// A switch.
    pub const FLAG: Self = Self {
        kind: ValueKind::Flag,
        choices: &[],
        range: None,
    };

    /// A number with limits.
    pub const fn number(range: Range) -> Self {
        Self {
            kind: ValueKind::Number,
            choices: &[],
            range: Some(range),
        }
    }
}

/// The shape of a value a control edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// A structured collection.
    Collection,
    /// Free-form text.
    Text,
    /// One of a fixed set of identifiers.
    Choice,
    /// A switch.
    Flag,
    /// A whole number.
    Number,
}

impl std::fmt::Display for ValueKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Collection => "collection",
            Self::Text => "text",
            Self::Choice => "choice",
            Self::Flag => "switch",
            Self::Number => "number",
        })
    }
}

/// A value a field can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A list of rows; each row holds its editable column values.
    Collection(Vec<Vec<String>>),
    /// Text entered without shell interpretation.
    Text(String),
    /// One of the identifiers the field's control offers, or — for a choice the
    /// control does not offer, such as a language tag a user wrote by hand — the
    /// value as it stands in the file.
    Choice(String),
    /// A switch.
    Flag(bool),
    /// A whole number.
    Number(i64),
}

impl Value {
    /// The shape of the value.
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Collection(_) => ValueKind::Collection,
            Self::Text(_) => ValueKind::Text,
            Self::Choice(_) => ValueKind::Choice,
            Self::Flag(_) => ValueKind::Flag,
            Self::Number(_) => ValueKind::Number,
        }
    }

    /// The identifier, for a choice value.
    pub fn choice_id(&self) -> Option<&str> {
        match self {
            Self::Choice(id) => Some(id),
            Self::Flag(_) | Self::Number(_) | Self::Text(_) | Self::Collection(_) => None,
        }
    }
}

/// The language the session runs in, as the file records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocaleChoice {
    /// No `locale.language`: the session follows `LC_ALL`/`LC_MESSAGES`/`LANG`.
    Environment,
    /// A language tag, exactly as it stands in the file.
    ///
    /// It is not validated here: a tag the schema would reject has to be
    /// *reportable* — that is what [`mod@crate::validate`] is for — rather than
    /// silently dropped on the way in.
    Tag(String),
}

impl LocaleChoice {
    /// The choice for a tag as it was read from the file.
    pub fn from_config(language: Option<&str>) -> Self {
        match language {
            Some(tag) => Self::Tag(tag.to_owned()),
            None => Self::Environment,
        }
    }

    /// The choice for an identifier a control offered.
    pub fn new(id: String) -> Self {
        if id == ENVIRONMENT {
            Self::Environment
        } else {
            Self::Tag(id)
        }
    }

    /// The identifier of the choice, for a control and for a file.
    pub fn id(&self) -> String {
        match self {
            Self::Environment => ENVIRONMENT.to_owned(),
            Self::Tag(tag) => tag.clone(),
        }
    }

    /// The tag the file should hold, or `None` to follow the environment.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Environment => None,
            Self::Tag(tag) => Some(tag),
        }
    }
}

/// Every setting this application edits, with the values their files have.
///
/// The numeric fields are `i64` rather than the widths `inox-config` uses, so
/// that a value outside the schema's range can be held and reported instead of
/// being clamped on the way in. `Field::set` is the only way in, and
/// [`mod@crate::validate`] is the gate on the way out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Trusted input method command for the next physical session.
    pub input_method: Vec<String>,
    /// Output connector, width, height, refresh, scale, x and y, in display order.
    pub outputs: Vec<Vec<String>>,
    /// Terminal executable followed by arguments, without shell interpretation.
    pub launcher_terminal: Vec<String>,
    /// Per-output wallpaper names and paths, kept in editable order.
    pub wallpaper_per_output: Vec<Vec<String>>,
    /// `panel.margin`.
    pub panel_margin: i64,
    /// `panel.exclusive`.
    pub panel_exclusive: bool,
    /// `panel.clock_format`.
    pub panel_clock_format: String,
    /// `panel.max_task_buttons`.
    pub panel_max_task_buttons: i64,
    /// `panel.icon_size`.
    pub panel_icon_size: i64,
    /// `launcher.max_results`.
    pub launcher_max_results: i64,
    /// `launcher.icon_size`.
    pub launcher_icon_size: i64,
    /// `launcher.include_hidden`.
    pub launcher_include_hidden: bool,
    /// `launcher.max_query_chars`.
    pub launcher_max_query_chars: i64,
    /// `wallpaper.image`.
    pub wallpaper_image: String,
    /// `wallpaper.mode`.
    pub wallpaper_mode: String,
    /// `input.keyboard_layout`.
    pub input_keyboard_layout: String,
    /// `input.keyboard_variant`.
    pub input_keyboard_variant: String,
    /// `input.keyboard_repeat_delay_ms`.
    pub input_keyboard_repeat_delay_ms: i64,
    /// `input.keyboard_repeat_rate_hz`.
    pub input_keyboard_repeat_rate_hz: i64,
    /// `input.tap_to_click`.
    pub input_tap_to_click: bool,
    /// `input.natural_scroll`.
    pub input_natural_scroll: bool,
    /// `layout.master_ratio`.
    pub layout_master_ratio: i64,
    /// `layout.master_count`.
    pub layout_master_count: i64,
    /// `layout.gap`.
    pub layout_gap: i64,
    /// `display.max_visible`.
    pub display_max_visible: i64,
    /// `display.width`.
    pub display_width: i64,
    /// `display.margin`.
    pub display_margin: i64,
    /// `display.icon_size`.
    pub display_icon_size: i64,
    /// `queue.max_pending`.
    pub queue_max_pending: i64,
    /// `queue.policy`.
    pub queue_policy: String,
    /// `policy.show_actions`.
    pub policy_show_actions: bool,
    /// `policy.hide_body_when_locked`.
    pub policy_hide_body_when_locked: bool,

    /// `theme.appearance`.
    pub theme_appearance: Appearance,
    /// `locale.language`.
    pub locale: LocaleChoice,
    /// `panel.position`.
    pub panel_position: PanelPosition,
    /// `panel.height`.
    pub panel_height: i64,
    /// `panel.autohide`.
    pub panel_autohide: bool,
    /// `panel.show_clock`.
    pub panel_show_clock: bool,
    /// `policy.do_not_disturb`.
    pub notifications_do_not_disturb: bool,
    /// `display.position`.
    pub notifications_position: NotificationPosition,
    /// `timeouts.low_ms`.
    pub notifications_low_ms: i64,
    /// `timeouts.normal_ms`.
    pub notifications_normal_ms: i64,
    /// `timeouts.critical_ms`.
    pub notifications_critical_ms: i64,
}

impl Settings {
    /// The values the loaded configuration holds.
    pub fn from_config(config: &InoxConfig) -> Self {
        let panel = &config.shell.panel;
        let display = &config.notifications.display;
        let timeouts = &config.notifications.timeouts;

        Self {
            input_method: config.compositor.input.input_method.clone(),
            outputs: config
                .compositor
                .outputs
                .iter()
                .map(|output| {
                    vec![
                        output.name.clone().unwrap_or_default(),
                        output.width.to_string(),
                        output.height.to_string(),
                        output.refresh_hz.to_string(),
                        output.scale.to_string(),
                        output
                            .position
                            .map(|position| position.x.to_string())
                            .unwrap_or_default(),
                        output
                            .position
                            .map(|position| position.y.to_string())
                            .unwrap_or_default(),
                    ]
                })
                .collect(),
            launcher_terminal: config.shell.launcher.terminal.clone(),
            wallpaper_per_output: config
                .shell
                .wallpaper
                .per_output
                .iter()
                .map(|(name, path)| vec![name.clone(), path.to_string_lossy().into_owned()])
                .collect(),

            panel_margin: i64::from(config.shell.panel.margin),
            panel_exclusive: config.shell.panel.exclusive,
            panel_clock_format: config.shell.panel.clock_format.clone(),
            panel_max_task_buttons: i64::from(config.shell.panel.max_task_buttons),
            panel_icon_size: i64::from(config.shell.panel.icon_size),
            launcher_max_results: i64::from(config.shell.launcher.max_results),
            launcher_icon_size: i64::from(config.shell.launcher.icon_size),
            launcher_include_hidden: config.shell.launcher.include_hidden,
            launcher_max_query_chars: i64::from(config.shell.launcher.max_query_chars),
            wallpaper_image: config
                .shell
                .wallpaper
                .image
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            wallpaper_mode: match config.shell.wallpaper.mode {
                inox_config::WallpaperMode::Fill => "fill",
                inox_config::WallpaperMode::Fit => "fit",
                inox_config::WallpaperMode::Stretch => "stretch",
                inox_config::WallpaperMode::Center => "center",
                inox_config::WallpaperMode::Tile => "tile",
            }
            .to_owned(),
            input_keyboard_layout: config.compositor.input.keyboard_layout.clone(),
            input_keyboard_variant: config.compositor.input.keyboard_variant.clone(),
            input_keyboard_repeat_delay_ms: i64::from(
                config.compositor.input.keyboard_repeat_delay_ms,
            ),
            input_keyboard_repeat_rate_hz: i64::from(
                config.compositor.input.keyboard_repeat_rate_hz,
            ),
            input_tap_to_click: config.compositor.input.tap_to_click,
            input_natural_scroll: config.compositor.input.natural_scroll,
            layout_master_ratio: i64::from(config.compositor.layout.master_ratio),
            layout_master_count: i64::from(config.compositor.layout.master_count),
            layout_gap: i64::from(config.compositor.layout.gap),
            display_max_visible: i64::from(config.notifications.display.max_visible),
            display_width: i64::from(config.notifications.display.width),
            display_margin: i64::from(config.notifications.display.margin),
            display_icon_size: i64::from(config.notifications.display.icon_size),
            queue_max_pending: i64::from(config.notifications.queue.max_pending),
            queue_policy: match config.notifications.queue.policy {
                inox_config::QueuePolicy::DropOldest => "drop-oldest",
                inox_config::QueuePolicy::CoalesceByApp => "coalesce-by-app",
            }
            .to_owned(),
            policy_show_actions: config.notifications.policy.show_actions,
            policy_hide_body_when_locked: config.notifications.policy.hide_body_when_locked,

            theme_appearance: config.root.theme.appearance,
            locale: LocaleChoice::from_config(config.root.locale.language.as_deref()),
            panel_position: panel.position,
            panel_height: i64::from(panel.height),
            panel_autohide: panel.autohide,
            panel_show_clock: panel.show_clock,
            notifications_do_not_disturb: config.notifications.policy.do_not_disturb,
            notifications_position: display.position,
            notifications_low_ms: i64::from(timeouts.low_ms),
            notifications_normal_ms: i64::from(timeouts.normal_ms),
            notifications_critical_ms: i64::from(timeouts.critical_ms),
        }
    }

    /// The values the schema gives each field when the file says nothing.
    ///
    /// These are `inox-config`'s own defaults — [`RootConfig::default`],
    /// [`ShellConfig::default`] and [`NotificationsConfig::default`] — rather
    /// than a second copy of the numbers, so "reset to default" means what a
    /// missing key means.
    pub fn defaults() -> Self {
        Self::from_config(&InoxConfig {
            root: RootConfig::default(),
            shell: ShellConfig::default(),
            notifications: NotificationsConfig::default(),
            compositor: inox_config::CompositorConfig::default(),
            theme: inox_config::Theme::builtin_dark(),
            locale: inox_common::i18n::Locale::default_locale(),
        })
    }
}

/// How a field came to be changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The user changed the value on a page.
    User,
    /// The user asked for the schema's default back.
    Reset,
}

/// Why a value could not be written into a draft.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    /// A collection contains incomplete or invalid entries.
    #[error("invalid collection for `{field}`")]
    InvalidCollection {
        /// The collection field.
        field: Field,
    },
    /// The value's shape is not the one the field's control edits.
    #[error("`{field}` takes a {expected} value, not a {found}")]
    Mismatched {
        /// The field.
        field: Field,
        /// The shape the field edits.
        expected: ValueKind,
        /// The shape that was handed over.
        found: ValueKind,
    },
}

/// The values a window is editing, over the ones the files hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    base: Settings,
    current: Settings,
    touched: BTreeMap<Field, Origin>,
    number_inputs: BTreeMap<Field, String>,
}

impl Draft {
    /// A draft showing the values `base` holds, with nothing changed.
    pub fn new(base: Settings) -> Self {
        Self {
            current: base.clone(),
            base,
            touched: BTreeMap::new(),
            number_inputs: BTreeMap::new(),
        }
    }

    /// The values the files hold.
    pub fn base(&self) -> &Settings {
        &self.base
    }

    /// The values the window is showing.
    pub fn settings(&self) -> &Settings {
        &self.current
    }

    /// The value of one field.
    pub fn field(&self, field: Field) -> Value {
        field.value(&self.current)
    }

    /// Whether anything differs from what the files hold.
    pub fn is_modified(&self) -> bool {
        !self.touched.is_empty() || !self.number_inputs.is_empty()
    }

    /// The fields a save would write, in the order they are declared.
    pub fn changed(&self) -> Vec<Field> {
        let mut changed: Vec<Field> = self.touched.keys().copied().collect();
        changed.sort_unstable();

        changed
    }

    /// Why a field is a change, or `None` when it is not one.
    pub fn origin(&self, field: Field) -> Option<Origin> {
        self.touched.get(&field).copied().or_else(|| {
            self.number_inputs
                .contains_key(&field)
                .then_some(Origin::User)
        })
    }

    /// The lines a save would write, one per changed field.
    ///
    /// # Errors
    ///
    /// Returns [`DraftError::Mismatched`] for a field whose stored value is not
    /// of the shape its control edits, which no sequence of drafts can produce.
    pub fn edits(&self) -> Result<Vec<Edit>, DraftError> {
        self.changed()
            .into_iter()
            .map(|field| {
                let value = self.field(field);

                Ok(match field.toml_value(&value)? {
                    Some(value) => Edit::set(field, value),
                    None => Edit::remove(field),
                })
            })
            .collect()
    }

    /// The files a save would write.
    pub fn files(&self) -> BTreeSet<SettingFile> {
        self.changed().into_iter().map(Field::file).collect()
    }

    /// Text currently being entered into a numeric field, including incomplete input.
    pub fn number_input(&self, field: Field) -> String {
        self.number_inputs
            .get(&field)
            .cloned()
            .unwrap_or_else(|| match self.field(field) {
                Value::Number(value) => value.to_string(),
                _ => String::new(),
            })
    }

    /// Keeps incomplete or overflowing numbers visible without writing an older value.
    pub fn edit_number(&mut self, field: Field, text: String) {
        if field.control().kind != ValueKind::Number {
            return;
        }
        match text.parse::<i64>() {
            Ok(number) => {
                self.set_current(field, Value::Number(number));
                self.retouch(field, Origin::User);
                self.number_inputs.remove(&field);
            }
            Err(_) => {
                self.number_inputs.insert(field, text);
            }
        }
    }

    /// Numeric input that has not yet become an integer.
    pub fn invalid_numbers(&self) -> impl Iterator<Item = Field> + '_ {
        self.number_inputs.keys().copied()
    }

    /// Changes a field.
    ///
    /// Returns whether the field is now a change. A field put back to the value
    /// its file holds stops being one, so a user who changes their mind about a
    /// setting does not cause it to be rewritten.
    ///
    /// # Errors
    ///
    /// Returns [`DraftError::Mismatched`] when the value is not of the shape the
    /// field edits.
    pub fn set(&mut self, field: Field, value: Value) -> Result<bool, DraftError> {
        let mut candidate = self.current.clone();
        field.set(&mut candidate, value)?;
        self.current = candidate;
        self.number_inputs.remove(&field);

        Ok(self.retouch(field, Origin::User))
    }

    /// Returns one field to the value a missing key would have.
    ///
    /// This is a change the user asked for: a key that is in the file and has a
    /// default is written back to that default, rather than being deleted.
    ///
    /// Returns whether the field is now a change.
    pub fn reset_to_default(&mut self, field: Field) -> bool {
        self.number_inputs.remove(&field);
        let default = field.value(&Settings::defaults());
        self.set_current(field, default);

        self.retouch(field, Origin::Reset)
    }

    /// Returns every field to the value a missing key would have.
    pub fn reset_all(&mut self) {
        for field in Field::ALL {
            self.reset_to_default(field);
        }
    }

    /// Throws the draft away: every field goes back to the value its file
    /// holds, and nothing is a change any more.
    ///
    /// This is what the Cancel action does, and it writes nothing at all — the
    /// only code that touches a file is [`crate::app`]'s save, which acts on
    /// [`Draft::changed`].
    pub fn discard(&mut self) {
        self.current = self.base.clone();
        self.touched.clear();
        self.number_inputs.clear();
    }

    /// Takes a freshly loaded configuration as the new baseline.
    ///
    /// The fields the user changed keep the user's values; every other field
    /// takes the loaded one. That is what makes an external edit visible — the
    /// window stops claiming a value the file no longer has — without reverting
    /// work that is still open (acceptance D04).
    pub fn rebase(&mut self, base: Settings) {
        let previous = std::mem::replace(&mut self.current, base.clone());
        self.base = base;

        let touched = std::mem::take(&mut self.touched);
        for (field, origin) in touched {
            let value = field.value(&previous);
            self.set_current(field, value);

            if field.value(&self.current) != field.value(&self.base) {
                self.touched.insert(field, origin);
            }
        }
    }

    /// Writes a value that came out of [`Field::value`] or
    /// [`Field::control`], whose shape the field therefore already has.
    fn set_current(&mut self, field: Field, value: Value) {
        let mut candidate = self.current.clone();
        field.assign(&mut candidate, value);
        self.current = candidate;
    }

    /// Records whether `field` differs from the baseline now.
    fn retouch(&mut self, field: Field, origin: Origin) -> bool {
        if field.value(&self.current) == field.value(&self.base) {
            self.touched.remove(&field);
            false
        } else {
            self.touched.insert(field, origin);
            true
        }
    }
}

/// The identifier of an appearance.
fn appearance_id(appearance: Appearance) -> String {
    match appearance {
        Appearance::Light => LIGHT.to_owned(),
        Appearance::Dark => DARK.to_owned(),
        Appearance::System => SYSTEM.to_owned(),
    }
}

/// The appearance an identifier names.
///
/// An identifier the control does not offer is read as [`Appearance::System`],
/// which is the schema's default; the choices the view draws are the ones in
/// [`APPEARANCE_CHOICES`], so this is the answer for a value built by hand.
fn appearance_of(id: &str) -> Appearance {
    match id {
        LIGHT => Appearance::Light,
        DARK => Appearance::Dark,
        _ => Appearance::System,
    }
}

/// The identifier of a panel position.
fn panel_position_id(position: PanelPosition) -> String {
    match position {
        PanelPosition::Top => "top".to_owned(),
        PanelPosition::Bottom => "bottom".to_owned(),
    }
}

/// The panel position an identifier names.
fn panel_position_of(id: &str) -> PanelPosition {
    match id {
        "top" => PanelPosition::Top,
        _ => PanelPosition::Bottom,
    }
}

/// The identifier of a notification position.
fn notification_position_id(position: NotificationPosition) -> String {
    match position {
        NotificationPosition::TopRight => "top-right",
        NotificationPosition::TopLeft => "top-left",
        NotificationPosition::BottomRight => "bottom-right",
        NotificationPosition::BottomLeft => "bottom-left",
    }
    .to_owned()
}

/// The notification position an identifier names.
fn notification_position_of(id: &str) -> NotificationPosition {
    match id {
        "top-right" => NotificationPosition::TopRight,
        "top-left" => NotificationPosition::TopLeft,
        "bottom-left" => NotificationPosition::BottomLeft,
        _ => NotificationPosition::BottomRight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::PanelConfig;

    fn draft() -> Draft {
        Draft::new(Settings::defaults())
    }

    #[test]
    fn a_field_path_is_the_one_inox_config_reports() {
        // The vocabulary in `inox-config`'s `serde_path_to_error` messages: the
        // table then the key, inside the file the field lives in.
        assert_eq!(Field::PanelHeight.path(), "panel.height");
        assert_eq!(
            Field::NotificationsNormalTimeout.path(),
            "timeouts.normal_ms"
        );
        assert_eq!(
            Field::NotificationsDoNotDisturb.path(),
            "policy.do_not_disturb"
        );
        assert_eq!(Field::ThemeAppearance.path(), "theme.appearance");
        assert_eq!(Field::LocaleLanguage.path(), "locale.language");
    }

    #[test]
    fn every_field_belongs_to_a_file_and_a_page() {
        for field in Field::ALL {
            assert_eq!(
                field.path().split('.').count(),
                if field == Field::Outputs { 1 } else { 2 },
                "`{field}`"
            );
            assert_eq!(field.file(), field.page().fields()[0].file(), "`{field}`");
        }
    }

    #[test]
    fn every_field_has_its_own_path() {
        let mut paths: Vec<&str> = Field::ALL.iter().map(|field| field.path()).collect();
        paths.sort_unstable();
        let before = paths.len();
        paths.dedup();

        assert_eq!(paths.len(), before, "two fields share a path");
    }

    #[test]
    fn a_choice_field_offers_choices_and_nothing_else() {
        for field in [
            Field::ThemeAppearance,
            Field::LocaleLanguage,
            Field::PanelPosition,
            Field::NotificationsPosition,
        ] {
            let control = field.control();

            assert_eq!(control.kind, ValueKind::Choice, "`{field}`");
            assert!(!control.choices.is_empty(), "`{field}` offers nothing");
            assert!(control.range.is_none(), "`{field}` has a range");
        }
    }

    #[test]
    fn a_number_field_has_a_range_and_no_choices() {
        for field in [
            Field::PanelHeight,
            Field::NotificationsLowTimeout,
            Field::NotificationsNormalTimeout,
            Field::NotificationsCriticalTimeout,
        ] {
            let control = field.control();

            assert_eq!(control.kind, ValueKind::Number, "`{field}`");
            assert!(control.choices.is_empty(), "`{field}`");
            assert!(control.range.is_some_and(|range| range.min < range.max));
        }
    }

    #[test]
    fn the_defaults_are_the_values_the_schema_gives_a_missing_key() {
        let defaults = Settings::defaults();

        assert_eq!(defaults.theme_appearance, Appearance::System);
        assert_eq!(defaults.locale, LocaleChoice::Environment);
        assert_eq!(defaults.panel_position, PanelPosition::Bottom);
        assert_eq!(
            defaults.panel_height,
            i64::from(PanelConfig::default().height)
        );
        assert_eq!(
            defaults.notifications_normal_ms,
            i64::from(NotificationsConfig::default().timeouts.normal_ms)
        );
    }

    #[test]
    fn a_draft_with_nothing_changed_writes_nothing() {
        let draft = draft();

        assert!(!draft.is_modified());
        assert!(draft.changed().is_empty());
        assert!(
            draft
                .edits()
                .expect("the defaults are well shaped")
                .is_empty()
        );
        assert!(draft.files().is_empty());
    }

    #[test]
    fn changing_a_field_makes_exactly_that_field_a_change() {
        let mut draft = draft();

        assert_eq!(draft.set(Field::PanelHeight, Value::Number(48)), Ok(true));

        assert!(draft.is_modified());
        assert_eq!(draft.changed(), vec![Field::PanelHeight]);
        assert_eq!(draft.origin(Field::PanelHeight), Some(Origin::User));
        assert_eq!(draft.files(), BTreeSet::from([SettingFile::Shell]));
        assert_eq!(draft.settings().panel_height, 48);
        assert_eq!(draft.base().panel_height, 32, "the baseline is untouched");
    }

    #[test]
    fn a_field_put_back_where_it_was_stops_being_a_change() {
        let mut draft = draft();
        let base = i64::from(PanelConfig::default().height);

        draft
            .set(Field::PanelHeight, Value::Number(base + 4))
            .expect("a number");
        assert!(draft.is_modified());

        draft
            .set(Field::PanelHeight, Value::Number(base))
            .expect("a number");

        assert!(
            !draft.is_modified(),
            "nothing differs from the file any more"
        );
        assert!(draft.changed().is_empty());
    }

    #[test]
    fn changes_are_reported_in_the_order_the_fields_are_declared() {
        let mut draft = draft();

        draft
            .set(Field::NotificationsDoNotDisturb, Value::Flag(true))
            .expect("a switch");
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");
        draft
            .set(Field::ThemeAppearance, Value::Choice(LIGHT.to_owned()))
            .expect("a choice");

        assert_eq!(
            draft.changed(),
            vec![
                Field::ThemeAppearance,
                Field::PanelHeight,
                Field::NotificationsDoNotDisturb
            ]
        );
        assert_eq!(
            draft.files(),
            BTreeSet::from([
                SettingFile::Root,
                SettingFile::Shell,
                SettingFile::Notifications
            ])
        );
    }

    #[test]
    fn a_value_of_the_wrong_shape_is_refused() {
        let mut draft = draft();

        let error = draft
            .set(Field::PanelHeight, Value::Flag(true))
            .expect_err("a switch is not a height");

        assert_eq!(
            error,
            DraftError::Mismatched {
                field: Field::PanelHeight,
                expected: ValueKind::Number,
                found: ValueKind::Flag,
            }
        );
        assert!(!draft.is_modified(), "a refused value changes nothing");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn discarding_returns_every_value_and_cancels_every_change() {
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");
        draft
            .set(Field::PanelShowClock, Value::Flag(true))
            .expect("a switch");

        draft.discard();

        assert!(!draft.is_modified());
        assert_eq!(draft.settings(), draft.base());
        assert!(draft.edits().expect("well shaped").is_empty());
    }

    #[test]
    fn resetting_a_field_writes_its_default_even_when_the_file_says_otherwise() {
        let mut base = Settings::defaults();
        base.panel_position = PanelPosition::Top;
        base.notifications_normal_ms = 20_000;
        let mut draft = Draft::new(base);

        assert!(draft.reset_to_default(Field::PanelPosition));

        assert_eq!(draft.settings().panel_position, PanelPosition::Bottom);
        assert_eq!(draft.origin(Field::PanelPosition), Some(Origin::Reset));
        assert_eq!(draft.changed(), vec![Field::PanelPosition]);
        assert_eq!(
            draft.edits().expect("well shaped"),
            vec![Edit::set(
                Field::PanelPosition,
                Toml::String("bottom".to_owned())
            )]
        );
    }

    #[test]
    fn resetting_a_field_that_already_has_its_default_is_not_a_change() {
        let mut draft = draft();

        assert!(!draft.reset_to_default(Field::PanelHeight));
        assert!(!draft.is_modified());
        assert_eq!(draft.origin(Field::PanelHeight), None);
    }

    #[test]
    fn resetting_everything_returns_the_whole_draft_to_the_schema_defaults() {
        let mut base = Settings::defaults();
        base.panel_height = 64;
        base.notifications_position = NotificationPosition::BottomLeft;
        let mut draft = Draft::new(base);

        draft.reset_all();

        assert_eq!(draft.settings(), &Settings::defaults());
        assert!(draft.is_modified(), "the file said otherwise");
    }

    #[test]
    fn following_the_environment_removes_the_key_rather_than_writing_a_tag() {
        let mut base = Settings::defaults();
        base.locale = LocaleChoice::Tag("zh-CN".to_owned());
        let mut draft = Draft::new(base);

        draft
            .set(Field::LocaleLanguage, Value::Choice(ENVIRONMENT.to_owned()))
            .expect("a choice");

        assert_eq!(
            draft.edits().expect("well shaped"),
            vec![Edit::remove(Field::LocaleLanguage)]
        );
    }

    #[test]
    fn pinning_a_language_writes_the_tag() {
        let mut draft = draft();

        draft
            .set(Field::LocaleLanguage, Value::Choice("zh-CN".to_owned()))
            .expect("a choice");

        assert_eq!(
            draft.edits().expect("well shaped"),
            vec![Edit::set(
                Field::LocaleLanguage,
                Toml::String("zh-CN".to_owned())
            )]
        );
    }

    #[test]
    fn a_rebase_keeps_what_the_user_changed_and_takes_the_rest_from_the_file() {
        let mut base = Settings::defaults();
        base.panel_height = 32;
        let mut draft = Draft::new(base);

        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let mut reloaded = Settings::defaults();
        reloaded.panel_height = 64;
        reloaded.panel_show_clock = false;
        draft.rebase(reloaded);

        assert_eq!(
            draft.settings().panel_height,
            48,
            "the user's edit survives"
        );
        assert!(
            !draft.settings().panel_show_clock,
            "the file's value is taken"
        );
        assert_eq!(draft.base().panel_height, 64);
        assert_eq!(draft.changed(), vec![Field::PanelHeight]);
        assert_eq!(draft.origin(Field::PanelHeight), Some(Origin::User));
    }

    #[test]
    fn a_rebase_that_agrees_with_the_user_ends_the_change() {
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let mut reloaded = Settings::defaults();
        reloaded.panel_height = 48;
        draft.rebase(reloaded);

        assert!(
            !draft.is_modified(),
            "the file now says what the draft says"
        );
    }

    #[test]
    fn a_range_wraps_so_one_key_reaches_every_stop() {
        let range = Field::PanelHeight.control().range.expect("a range");

        assert_eq!(range.stops(), 249);
        assert_eq!(range.advance(range.min, 1), range.min + 1);
        assert_eq!(range.advance(range.max, 1), range.min, "past the end wraps");
        assert_eq!(
            range.advance(range.min, -1),
            range.max,
            "before the start wraps"
        );
    }

    #[test]
    fn a_range_snaps_a_value_that_is_out_of_it_to_the_nearest_stop() {
        let range = TIMEOUT_RANGE;

        assert_eq!(range.advance(-5_000, 1), range.min + range.step);
        assert_eq!(range.advance(0, 1), range.step);
        assert_eq!(range.advance(range.max + 10_000, 1), range.min);
    }

    #[test]
    fn a_range_knows_what_the_schema_accepts() {
        let range = Field::PanelHeight.control().range.expect("a range");

        assert!(range.contains(range.min));
        assert!(range.contains(range.max));
        assert!(!range.contains(range.min - 1));
        assert!(!range.contains(range.max + 1));
    }

    #[test]
    fn a_language_tag_written_by_hand_is_kept_rather_than_dropped() {
        let choice = LocaleChoice::from_config(Some("fr-FR"));

        assert_eq!(choice.id(), "fr-FR");
        assert_eq!(choice.tag(), Some("fr-FR"));
        assert_eq!(LocaleChoice::from_config(None), LocaleChoice::Environment);
        assert_eq!(LocaleChoice::Environment.tag(), None);
    }

    #[test]
    fn a_hand_built_settings_can_hold_a_value_the_schema_would_refuse() {
        // The draft holds what it is given; refusing a value is the validator's
        // job, so that the user reads a field error instead of a dead click.
        let mut settings = Settings::defaults();
        Field::PanelHeight
            .set(&mut settings, Value::Number(9_000))
            .expect("a number");

        assert_eq!(settings.panel_height, 9_000);
        assert!(
            !Field::PanelHeight
                .control()
                .range
                .expect("a range")
                .contains(9_000)
        );
    }

    #[test]
    fn the_wallpaper_choices_are_the_catalogue_plus_the_default_and_the_background() {
        let offered: BTreeSet<&str> = WALLPAPER_CHOICES.iter().map(|choice| choice.id).collect();
        let mut expected: BTreeSet<&str> = inox_config::wallpapers::BUNDLED
            .iter()
            .map(|wallpaper| wallpaper.id)
            .collect();
        expected.insert(inox_config::BACKGROUND_ID);
        expected.insert("");

        assert_eq!(
            offered, expected,
            "the chooser and the shipped catalogue drifted apart"
        );
        assert!(
            offered.contains(inox_config::wallpapers::DEFAULT_ID),
            "the shipped default must be one of the offered wallpapers"
        );
    }

    #[test]
    fn an_absent_wallpaper_image_is_shown_as_the_default_choice() {
        let settings = Settings::defaults();

        assert_eq!(
            Field::WallpaperImage.value(&settings),
            Value::Choice(String::new())
        );
    }

    #[test]
    fn choosing_a_wallpaper_writes_its_identifier_and_nothing_else() {
        let mut settings = Settings::defaults();
        Field::WallpaperImage
            .set(&mut settings, Value::Choice("fuji".to_owned()))
            .expect("a choice");

        assert_eq!(settings.wallpaper_image, "fuji");
        assert_eq!(
            Field::WallpaperImage
                .toml_value(&Value::Choice("fuji".to_owned()))
                .expect("a string"),
            Some(Toml::String("fuji".to_owned()))
        );
    }

    #[test]
    fn the_default_choice_removes_the_key_rather_than_writing_an_empty_string() {
        assert_eq!(
            Field::WallpaperImage
                .toml_value(&Value::Choice(String::new()))
                .expect("a choice"),
            None
        );
    }

    #[test]
    fn the_theme_background_is_written_as_the_reserved_word() {
        assert_eq!(
            Field::WallpaperImage
                .toml_value(&Value::Choice(inox_config::BACKGROUND_ID.to_owned()))
                .expect("a string"),
            Some(Toml::String(inox_config::BACKGROUND_ID.to_owned()))
        );
    }

    #[test]
    fn a_path_written_by_hand_is_kept_as_the_chosen_value() {
        // The window offers identifiers, but a configuration may hold a path;
        // the draft reads it back as-is so the option the projection appends
        // shows the path the file actually names.
        let mut settings = Settings::defaults();
        settings.wallpaper_image = "/images/主壁纸.png".to_owned();

        assert_eq!(
            Field::WallpaperImage.value(&settings),
            Value::Choice("/images/主壁纸.png".to_owned())
        );
        Field::WallpaperImage
            .set(&mut settings, Value::Choice("/images/other.png".to_owned()))
            .expect("a choice");
        assert_eq!(settings.wallpaper_image, "/images/other.png");
    }
}
