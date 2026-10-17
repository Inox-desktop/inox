//! What a draft has to satisfy before it can be written.
//!
//! This module mirrors the rules `inox-config` enforces while parsing, and it
//! exists for one reason: **the message a user reads comes from here, not from
//! a parse error**. A value the schema refuses has to be reported against the
//! field that holds it and in the user's language (acceptance D04: "错误定位到
//! 字段"), so a [`FieldError`] carries the dotted path — the same one
//! `inox-config` names in a parse error — and a Fluent message id with its
//! arguments, never a sentence built in code (root `AGENTS.md` §7.3).
//!
//! Only the fields that *have* a rule appear in [`validate_field`]'s match:
//!
//! - `panel.height`, between 8 and 256 logical pixels;
//! - the three notification timeouts, between 0 and 600000 milliseconds;
//! - `locale.language`, a well-formed BCP-47 tag that is not the POSIX `C` or
//!   `POSIX` sentinel.
//!
//! The rest are covered by their types: an edge is one of two variants, and a
//! switch is a boolean, so there is nothing left for a rule to say. The ranges
//! are read from the field's own [`crate::draft::Control`], which is also what
//! the widgets clamp to, so the number the page offers and the number that is
//! accepted cannot drift apart.
//!
//! `tests/workflow.rs` proves the mirror is faithful rather than asserting it:
//! a draft the validator accepts is written to a real `shell.toml` and read
//! back through [`inox_config::InoxConfig::load`].

use inox_common::i18n::{Arg, Locale, Localizer};

use crate::draft::{Field, LocaleChoice, Settings, Value};

/// One message a user reads, as the catalog id and the values it interpolates.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationMessage {
    /// The Fluent message id.
    pub id: &'static str,
    /// The arguments the placeholders take.
    pub args: Vec<(&'static str, Arg)>,
}

impl ValidationMessage {
    /// A message with the arguments its placeholders expect.
    pub fn new(id: &'static str, args: Vec<(&'static str, Arg)>) -> Self {
        Self { id, args }
    }

    /// The sentence, in the locale the localizer is loaded for.
    pub fn render(&self, localizer: &Localizer) -> String {
        localizer.message_with(self.id, &self.args)
    }
}

/// A value the schema would refuse, against the field that holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldError {
    /// The dotted path of the key, as `inox-config` names it in a parse error.
    pub path: &'static str,
    /// What is wrong, as a message the user reads.
    pub message: ValidationMessage,
}

impl FieldError {
    /// The sentence, in the locale the localizer is loaded for.
    pub fn render(&self, localizer: &Localizer) -> String {
        self.message.render(localizer)
    }
}

/// Every value `settings` holds that the schema would refuse.
///
/// In the order the fields are declared, so a page can show the errors beside
/// their fields without sorting them.
pub fn validate(settings: &Settings) -> Vec<FieldError> {
    Field::ALL
        .into_iter()
        .filter_map(|field| validate_field(field, settings))
        .collect()
}

/// The rule for one field, or `None` when the field is within its schema.
pub fn validate_field(field: Field, settings: &Settings) -> Option<FieldError> {
    if field.is_collection() {
        let Value::Collection(rows) = field.value(settings) else {
            return None;
        };
        let valid_rows = |rows: &[Vec<String>]| {
            crate::collections::toml(field, rows).is_some_and(|value| {
                let source =
                    crate::document::patch("", &[crate::document::Edit::set(field, value)]);
                match field.file() {
                    crate::document::SettingFile::Compositor => {
                        toml::from_str::<inox_config::CompositorConfig>(&source).is_ok()
                    }
                    crate::document::SettingFile::Shell => {
                        toml::from_str::<inox_config::ShellConfig>(&source).is_ok()
                    }
                    _ => false,
                }
            })
        };
        if valid_rows(&rows) {
            return None;
        }
        let row = (0..rows.len())
            .find(|index| !valid_rows(&rows[..=*index]))
            .unwrap_or(0);
        return Some(FieldError {
            path: field.path(),
            message: ValidationMessage::new(
                match field {
                    Field::InputMethodCommand => "settings-error-input-method-command",
                    Field::WallpaperPerOutput => "settings-error-wallpaper-entry",
                    _ => "settings-error-collection",
                },
                vec![("row", Arg::from((row + 1) as i64))],
            ),
        });
    }
    if field.control().range.is_some() {
        let id = match field {
            Field::PanelHeight => "settings-error-height-range",
            Field::NotificationsLowTimeout
            | Field::NotificationsNormalTimeout
            | Field::NotificationsCriticalTimeout => "settings-error-timeout-range",
            _ => "settings-error-number-range",
        };
        return range_error(field, settings, id);
    }
    if field == Field::LocaleLanguage {
        return locale_error(settings);
    }
    if field == Field::PanelClockFormat {
        let source = crate::document::patch(
            "",
            &[crate::document::Edit::set(
                field,
                crate::document::Toml::String(settings.panel_clock_format.clone()),
            )],
        );
        if toml::from_str::<inox_config::ShellConfig>(&source).is_err() {
            return Some(FieldError {
                path: field.path(),
                message: ValidationMessage::new("settings-error-clock-format", vec![]),
            });
        }
    }
    None
}

/// Whether the draft is in a state that may be written.
pub fn is_valid(settings: &Settings) -> bool {
    validate(settings).is_empty()
}

/// A number outside the range its control offers.
fn range_error(field: Field, settings: &Settings, id: &'static str) -> Option<FieldError> {
    let Value::Number(value) = field.value(settings) else {
        return None;
    };

    let range = field.control().range?;
    if range.contains(value) {
        return None;
    }

    Some(FieldError {
        path: field.path(),
        message: ValidationMessage::new(
            id,
            vec![
                ("min", Arg::from(range.min)),
                ("max", Arg::from(range.max)),
                ("value", Arg::from(value)),
            ],
        ),
    })
}

/// A language tag that cannot be one.
///
/// The rule is `inox-config`'s, on purpose: a tag this accepts but the loader
/// refuses would be a draft that can never be saved, and a tag the loader
/// refuses but this accepts would be a file that breaks the next session's
/// start. `C` and `POSIX` mean "no locale was set" rather than naming a
/// language, and the codeset and modifier parts (`C.UTF-8`, `POSIX@euro`) are
/// the same sentinel with a suffix.
fn locale_error(settings: &Settings) -> Option<FieldError> {
    let LocaleChoice::Tag(tag) = &settings.locale else {
        // Following the environment writes no key at all.
        return None;
    };

    if is_a_language(tag) {
        return None;
    }

    Some(FieldError {
        path: Field::LocaleLanguage.path(),
        message: ValidationMessage::new(
            "settings-error-locale",
            vec![("value", Arg::from(tag.as_str()))],
        ),
    })
}

/// Whether `tag` names a language the session can run in.
fn is_a_language(tag: &str) -> bool {
    let bare = tag.split(['.', '@']).next().unwrap_or(tag).trim();

    if bare.is_empty() || bare.eq_ignore_ascii_case("C") || bare.eq_ignore_ascii_case("POSIX") {
        return false;
    }

    Locale::parse(tag).is_ok()
}

/// The fields whose control edits a number.
///
/// `tests` uses it to check that every number field has a rule, which is the
/// property that keeps a new field from being added without one.
/// Checks the complete candidate against its schema and verifies every requested edit.
///
/// This catches invalid patches before any file in a multi-file save is queued.
///
/// # Errors
///
/// Returns a diagnostic naming the file or key that failed parsing or readback.
pub fn document(
    file: crate::document::SettingFile,
    source: &str,
    edits: &[crate::document::Edit],
) -> Result<(), String> {
    use crate::document::SettingFile;
    let parsed = match file {
        SettingFile::Root => toml::from_str::<inox_config::RootConfig>(source).map(|_| ()),
        SettingFile::Compositor => {
            toml::from_str::<inox_config::CompositorConfig>(source).map(|_| ())
        }
        SettingFile::Shell => toml::from_str::<inox_config::ShellConfig>(source).map(|_| ()),
        SettingFile::Notifications => {
            toml::from_str::<inox_config::NotificationsConfig>(source).map(|_| ())
        }
    };
    parsed.map_err(|error| format!("{}: {error}", file.name()))?;
    let actual: toml::Value = toml::from_str(source).map_err(|error| error.to_string())?;
    for edit in edits {
        let found = edit
            .path()
            .split('.')
            .try_fold(&actual, |value, key| value.get(key));
        let expected = edit
            .value()
            .map(|value| {
                toml::from_str::<toml::Value>(&format!("value = {}", value.render()))
                    .map(|mut table| table.get_mut("value").map(|value| value.clone()))
            })
            .transpose()
            .map_err(|error| error.to_string())?
            .flatten();
        if found != expected.as_ref() {
            return Err(format!(
                "{}: `{}` does not match the requested edit",
                file.name(),
                edit.path()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
fn number_fields() -> Vec<Field> {
    Field::ALL
        .into_iter()
        .filter(|field| field.control().kind == crate::draft::ValueKind::Number)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::ENVIRONMENT;

    fn localizer() -> Localizer {
        crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse")
    }

    #[test]
    fn the_defaults_satisfy_every_rule() {
        let settings = Settings::defaults();

        assert_eq!(validate(&settings), Vec::new());
        assert!(is_valid(&settings));
    }

    #[test]
    fn every_field_is_clean_at_its_own_default() {
        let settings = Settings::defaults();

        for field in Field::ALL {
            assert_eq!(validate_field(field, &settings), None, "`{field}`");
        }
    }

    #[test]
    fn every_number_field_has_a_rule() {
        let fields = number_fields();

        assert!(!fields.is_empty(), "the schema has numeric fields");

        for field in fields {
            let range = field.control().range.expect("a number field has a range");
            let mut settings = Settings::defaults();
            field
                .set(&mut settings, Value::Number(range.max + 1))
                .expect("a number");

            assert!(
                validate_field(field, &settings).is_some(),
                "`{field}` accepts a value above its range"
            );
        }
    }

    #[test]
    fn a_height_outside_its_range_is_reported_against_its_field() {
        let mut settings = Settings::defaults();
        settings.panel_height = 4_000;

        let error = validate_field(Field::PanelHeight, &settings).expect("out of range");

        assert_eq!(error.path, "panel.height");
        assert_eq!(error.message.id, "settings-error-height-range");

        let sentence = error.render(&localizer());
        assert!(sentence.contains("256"), "rendered `{sentence}`");
        assert!(sentence.contains("4000"), "rendered `{sentence}`");
    }

    #[test]
    fn a_height_below_its_range_is_reported_too() {
        let mut settings = Settings::defaults();
        settings.panel_height = -1;

        let error = validate_field(Field::PanelHeight, &settings).expect("out of range");

        assert_eq!(error.path, "panel.height");
        assert!(
            error.render(&localizer()).contains("8"),
            "{}",
            error.render(&localizer())
        );
    }

    #[test]
    fn both_ends_of_a_range_are_accepted() {
        let range = Field::PanelHeight
            .control()
            .range
            .expect("a number field has a range");

        for value in [range.min, range.max] {
            let mut settings = Settings::defaults();
            settings.panel_height = value;

            assert_eq!(
                validate_field(Field::PanelHeight, &settings),
                None,
                "{value}"
            );
        }
    }

    #[test]
    fn an_out_of_range_timeout_names_its_own_field() {
        let mut settings = Settings::defaults();
        settings.notifications_critical_ms = 900_000;

        let error =
            validate_field(Field::NotificationsCriticalTimeout, &settings).expect("out of range");

        assert_eq!(error.path, "timeouts.critical_ms");
        assert_eq!(error.message.id, "settings-error-timeout-range");
        assert!(
            error.render(&localizer()).contains("600000"),
            "{}",
            error.render(&localizer())
        );
    }

    #[test]
    fn a_timeout_of_zero_is_the_schema_default_and_is_accepted() {
        let mut settings = Settings::defaults();
        settings.notifications_normal_ms = 0;

        assert_eq!(
            validate_field(Field::NotificationsNormalTimeout, &settings),
            None
        );
    }

    #[test]
    fn the_shipped_languages_and_following_the_environment_are_accepted() {
        for choice in [ENVIRONMENT, "en-US", "zh-CN"] {
            let mut settings = Settings::defaults();
            settings.locale = if choice == ENVIRONMENT {
                LocaleChoice::Environment
            } else {
                LocaleChoice::Tag(choice.to_owned())
            };

            assert_eq!(
                validate_field(Field::LocaleLanguage, &settings),
                None,
                "{choice}"
            );
        }
    }

    #[test]
    fn a_tag_that_is_not_a_language_is_reported() {
        for tag in ["not a tag !", "", "  ", "_"] {
            let mut settings = Settings::defaults();
            settings.locale = LocaleChoice::Tag(tag.to_owned());

            let error =
                validate_field(Field::LocaleLanguage, &settings).expect("not a language tag");

            assert_eq!(error.path, "locale.language");
            assert_eq!(error.message.id, "settings-error-locale");
        }
    }

    #[test]
    fn the_posix_sentinels_are_reported_rather_than_read_as_languages() {
        // `C` and `POSIX` mean "no locale was set". `Locale::parse` would read
        // the second one as a language called `posix`, and the session would
        // then be pinned to it instead of following the environment, so the
        // rule refuses them exactly as `inox-config` does.
        for tag in ["C", "POSIX", "c", "posix", " c ", "C.UTF-8", "POSIX@euro"] {
            let mut settings = Settings::defaults();
            settings.locale = LocaleChoice::Tag(tag.to_owned());

            assert!(
                validate_field(Field::LocaleLanguage, &settings).is_some(),
                "`{tag}` was read as a language"
            );
        }
    }

    #[test]
    fn a_language_tag_inox_has_no_catalog_for_is_still_a_language() {
        // `LocaleConfig::resolve` negotiates it to the default locale, so the
        // session runs; refusing to save it would be this component inventing a
        // rule the schema does not have.
        let mut settings = Settings::defaults();
        settings.locale = LocaleChoice::Tag("fr-FR".to_owned());

        assert_eq!(validate_field(Field::LocaleLanguage, &settings), None);
    }

    #[test]
    fn one_broken_field_does_not_report_another() {
        let mut settings = Settings::defaults();
        settings.panel_height = 1_000;
        settings.notifications_low_ms = 1_000_000;

        let errors = validate(&settings);

        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].path, "panel.height");
        assert_eq!(errors[1].path, "timeouts.low_ms");
    }

    #[test]
    fn the_errors_are_reported_in_the_order_the_fields_are_declared() {
        let mut settings = Settings::defaults();
        settings.notifications_critical_ms = -1;
        settings.panel_height = 4_000;

        let errors = validate(&settings);

        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors[0].path,
            Field::PanelHeight.path(),
            "the panel comes before the timeouts in the order the fields are declared"
        );
        assert_eq!(errors[1].path, Field::NotificationsCriticalTimeout.path());
    }

    #[test]
    fn a_message_is_an_id_and_its_arguments_not_a_sentence() {
        let mut settings = Settings::defaults();
        settings.panel_height = 0;

        let error = validate_field(Field::PanelHeight, &settings).expect("out of range");

        assert!(
            error.message.id.starts_with("settings-"),
            "{}",
            error.message.id
        );
        let names: Vec<&str> = error.message.args.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, vec!["min", "max", "value"]);
    }
}
