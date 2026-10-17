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
/// another one (ADR 0009, [`inox_config::LocaleConfig::resolve`]).
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
/// the schema rejects cannot be entered (ADR 0009, decision 4).
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

impl Field {
    /// Every field, in the order the pages draw them.
    pub const ALL: [Self; 11] = [
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

        if self == Self::LocaleLanguage
            && let Value::Choice(id) = value
            && id == ENVIRONMENT
        {
            return Ok(None);
        }

        Ok(Some(match value {
            Value::Choice(id) => Toml::String(id.clone()),
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
        let current = (value - self.min)
            .div_euclid(self.step)
            .clamp(0, self.stops() - 1);

        self.min + (current + steps).rem_euclid(self.stops()) * self.step
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
            Self::Choice => "choice",
            Self::Flag => "switch",
            Self::Number => "number",
        })
    }
}

/// A value a field can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
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
            Self::Choice(_) => ValueKind::Choice,
            Self::Flag(_) => ValueKind::Flag,
            Self::Number(_) => ValueKind::Number,
        }
    }

    /// The identifier, for a choice value.
    pub fn choice_id(&self) -> Option<&str> {
        match self {
            Self::Choice(id) => Some(id),
            Self::Flag(_) | Self::Number(_) => None,
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
        let root = RootConfig::default();
        let shell = ShellConfig::default();
        let notifications = NotificationsConfig::default();

        Self {
            theme_appearance: root.theme.appearance,
            locale: LocaleChoice::from_config(root.locale.language.as_deref()),
            panel_position: shell.panel.position,
            panel_height: i64::from(shell.panel.height),
            panel_autohide: shell.panel.autohide,
            panel_show_clock: shell.panel.show_clock,
            notifications_do_not_disturb: notifications.policy.do_not_disturb,
            notifications_position: notifications.display.position,
            notifications_low_ms: i64::from(notifications.timeouts.low_ms),
            notifications_normal_ms: i64::from(notifications.timeouts.normal_ms),
            notifications_critical_ms: i64::from(notifications.timeouts.critical_ms),
        }
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
}

impl Draft {
    /// A draft showing the values `base` holds, with nothing changed.
    pub fn new(base: Settings) -> Self {
        Self {
            current: base.clone(),
            base,
            touched: BTreeMap::new(),
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
        !self.touched.is_empty()
    }

    /// The fields a save would write, in the order they are declared.
    pub fn changed(&self) -> Vec<Field> {
        let mut changed: Vec<Field> = self.touched.keys().copied().collect();
        changed.sort_unstable();

        changed
    }

    /// Why a field is a change, or `None` when it is not one.
    pub fn origin(&self, field: Field) -> Option<Origin> {
        self.touched.get(&field).copied()
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

        Ok(self.retouch(field, Origin::User))
    }

    /// Returns one field to the value a missing key would have.
    ///
    /// This is a change the user asked for: a key that is in the file and has a
    /// default is written back to that default, rather than being deleted.
    ///
    /// Returns whether the field is now a change.
    pub fn reset_to_default(&mut self, field: Field) -> bool {
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
            assert_eq!(field.path().split('.').count(), 2, "`{field}`");
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
}
