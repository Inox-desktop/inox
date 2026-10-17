//! The top-level `config.toml`.
//!
//! PRD §9.1 describes `config.toml` as the file that points at the per-component
//! configuration, carries the environment variables the session exports and
//! selects the theme. This module models exactly that: [`ComponentFiles`] holds
//! the paths, and the paths themselves are resolved against the configuration
//! directory by [`crate::paths::ConfigPaths::file`] rather than being joined
//! here.
//!
//! `shell.toml` and `notifications.toml` appear in this file as names only:
//! [`ComponentFiles`] holds their paths, and [`crate::shell`] and
//! [`crate::notifications`] model what is inside them. The one thing that does
//! belong to `config.toml` alone is `[locale]` ([`LocaleConfig`]): every L3
//! component resolves the same language, and there is nowhere else
//! session-wide to put it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use inox_common::i18n::Locale;
use serde::{Deserialize, Deserializer};

use crate::paths::{COMPOSITOR_FILE, NOTIFICATIONS_FILE, SHELL_FILE};

/// The top-level configuration file, `<config>/config.toml`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct RootConfig {
    /// Environment variables the session exports to the components it starts.
    ///
    /// A map rather than a struct: the set of variables is open, and a typo can
    /// only be caught by whoever reads them.
    pub environment: BTreeMap<String, String>,
    /// Where each component finds its own configuration file.
    pub components: ComponentFiles,
    /// Which theme the session should use.
    pub theme: ThemeSelection,
    /// Which language the session runs in.
    pub locale: LocaleConfig,
}

/// The files the individual components read their own configuration from.
///
/// Every path is relative to the configuration directory; absolute paths and
/// paths that escape it are rejected by
/// [`crate::paths::ConfigPaths::file`], which
/// [`crate::store::InoxConfig::load`] runs on all three of them. A file that is
/// not parsed yet is therefore still checked at startup.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ComponentFiles {
    /// Compositor configuration file.
    pub compositor: PathBuf,
    /// Shell configuration file, modelled by [`crate::shell::ShellConfig`].
    pub shell: PathBuf,
    /// Notification daemon configuration file, modelled by
    /// [`crate::notifications::NotificationsConfig`].
    pub notifications: PathBuf,
}

impl Default for ComponentFiles {
    fn default() -> Self {
        Self {
            compositor: PathBuf::from(COMPOSITOR_FILE),
            shell: PathBuf::from(SHELL_FILE),
            notifications: PathBuf::from(NOTIFICATIONS_FILE),
        }
    }
}

/// The theme the session should use (PRD §9.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ThemeSelection {
    /// Name of the theme: a built-in such as `inox-light`, or a directory below
    /// `themes/`. `None` — the default — selects the built-in theme matching the
    /// effective appearance.
    pub name: Option<String>,
    /// Appearance to use, or [`Appearance::System`] to follow the session.
    pub appearance: Appearance,
}

/// Light, dark, or whatever the session reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Light colours.
    Light,
    /// Dark colours.
    Dark,
    /// Follow the appearance the session reports. When the session reports
    /// `system` as well there is nothing to follow, which
    /// [`ThemeSelection::resolve`] documents as falling back to
    /// [`Appearance::Dark`].
    ///
    /// Only this selection may ask for it: a theme file describes a single
    /// palette and has to pick [`Appearance::Light`] or [`Appearance::Dark`].
    #[default]
    System,
}

/// The language the session runs in.
///
/// A tag is validated while `config.toml` is parsed — a malformed one is a
/// parse error naming `locale.language` rather than a session that quietly
/// shows its strings in the wrong language — and resolved by
/// [`LocaleConfig::resolve`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct LocaleConfig {
    /// Language tag to run in, for example `en-US` or `zh-CN`. `None` — the
    /// default — follows the environment.
    #[serde(deserialize_with = "validate_language")]
    pub language: Option<String>,
}

impl LocaleConfig {
    /// The locale this session runs in.
    ///
    /// A configured tag is negotiated against the catalogs Inox ships; without
    /// one, the environment decides, through `LC_ALL`, `LC_MESSAGES` and
    /// `LANG` in that order. Negotiation is total — a language with no catalog
    /// resolves to [`inox_common::i18n::DEFAULT_LOCALE`] — so a caller can load
    /// its catalogs without adding a fallback of its own.
    ///
    /// A tag that was rejected while parsing cannot reach here; one built by
    /// hand can, and is answered from the environment, for the same reason
    /// [`Locale::from_env`] never fails.
    pub fn resolve(&self) -> Locale {
        let requested = match &self.language {
            Some(tag) => Locale::parse(tag).unwrap_or_else(|_| Locale::from_env()),
            None => Locale::from_env(),
        };

        Locale::negotiate(&requested)
    }
}

/// Reject a `locale.language` that does not name a language.
///
/// A tag that is not well-formed is rejected, and so are the POSIX `C` and
/// `POSIX` sentinels: they mean "no locale was set", and `Locale::parse`
/// would read `POSIX` as the language subtag `posix`.
fn validate_language<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let language = Option::<String>::deserialize(deserializer)?;

    if let Some(tag) = &language {
        // `C` and `POSIX` are not locales: they are what a process that has set
        // no locale at all reports. `Locale::from_env_with` skips them so the
        // session keeps its own language, and a configuration file has the same
        // choice in front of it — omit `[locale]`, or name a real tag — so
        // reading `POSIX` as "the language called posix" and pinning `en-US`
        // would be a surprise rather than a convenience.
        //
        // The codeset and modifier parts are dropped first (`C.UTF-8`,
        // `POSIX@euro`), the same way `Locale::parse` drops them, so those
        // spellings of the sentinel are caught here rather than only the bare
        // one.
        let bare = tag.split(['.', '@']).next().unwrap_or(tag).trim();
        if bare.eq_ignore_ascii_case("C") || bare.eq_ignore_ascii_case("POSIX") {
            return Err(serde::de::Error::custom(
                "locale.language must name a language; omit `[locale]` to follow the environment",
            ));
        }

        if let Err(error) = Locale::parse(tag) {
            return Err(serde::de::Error::custom(format!(
                "locale.language must be a well-formed BCP-47 language tag: {error}"
            )));
        }
    }

    Ok(language)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ConfigError;
    use crate::error::test_support::line_of;
    use std::path::Path;

    /// The file name used in assertions on rendered errors.
    const ROOT_PATH: &str = "/cfg/inox/config.toml";

    fn parse_root(text: &str) -> Result<RootConfig, ConfigError> {
        crate::error::parse(Path::new(ROOT_PATH), text)
    }

    #[test]
    fn an_empty_document_uses_every_default() {
        let root = parse_root("").expect("empty document");

        assert!(root.environment.is_empty());
        assert_eq!(root.components.compositor, PathBuf::from("compositor.toml"));
        assert_eq!(root.components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            root.components.notifications,
            PathBuf::from("notifications.toml")
        );
        assert_eq!(root.theme.name, None);
        assert_eq!(root.theme.appearance, Appearance::System);
        assert_eq!(root.locale.language, None);
    }

    #[test]
    fn the_component_defaults_are_the_file_names_of_the_prd_layout() {
        let components = ComponentFiles::default();

        assert_eq!(components.compositor, PathBuf::from("compositor.toml"));
        assert_eq!(components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            components.notifications,
            PathBuf::from("notifications.toml")
        );
    }

    #[test]
    fn environment_components_and_theme_are_parsed() {
        let root = parse_root(
            "[environment]\n\
             MOZ_ENABLE_WAYLAND = \"1\"\n\
             WINIT_UNIX_BACKEND = \"wayland\"\n\
             \n\
             [components]\n\
             compositor = \"c.toml\"\n\
             shell = \"s.toml\"\n\
             notifications = \"n.toml\"\n\
             \n\
             [theme]\n\
             name = \"mine\"\n\
             appearance = \"light\"\n",
        )
        .expect("document");

        assert_eq!(
            root.environment
                .get("WINIT_UNIX_BACKEND")
                .map(String::as_str),
            Some("wayland")
        );
        assert_eq!(root.environment.len(), 2);
        assert_eq!(root.components.compositor, PathBuf::from("c.toml"));
        assert_eq!(root.components.shell, PathBuf::from("s.toml"));
        assert_eq!(root.components.notifications, PathBuf::from("n.toml"));
        assert_eq!(root.theme.name.as_deref(), Some("mine"));
        assert_eq!(root.theme.appearance, Appearance::Light);
    }

    #[test]
    fn a_partial_section_keeps_the_defaults_of_the_fields_it_leaves_out() {
        let root = parse_root("[theme]\nappearance = \"dark\"\n").expect("document");

        assert_eq!(root.theme.appearance, Appearance::Dark);
        assert_eq!(root.theme.name, None);
        assert_eq!(root.components.compositor, PathBuf::from("compositor.toml"));
        assert_eq!(root.components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            root.components.notifications,
            PathBuf::from("notifications.toml")
        );
        assert!(root.environment.is_empty());
    }

    #[test]
    fn every_appearance_spelling_is_parsed() {
        for (text, expected) in [
            ("light", Appearance::Light),
            ("dark", Appearance::Dark),
            ("system", Appearance::System),
        ] {
            let root =
                parse_root(&format!("[theme]\nappearance = \"{text}\"\n")).expect("document");

            assert_eq!(root.theme.appearance, expected);
        }
    }

    #[test]
    fn a_misspelled_appearance_is_rejected() {
        let error = parse_root("[theme]\nappearance = \"purple\"\n").expect_err("unknown variant");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("theme.appearance"));
                assert_eq!(line, Some(2));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_key_is_rejected_with_its_field_path() {
        let error = parse_root("nonsense = 1\n").expect_err("unknown key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("nonsense"));
                assert_eq!(line, Some(1));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_key_inside_a_section_reports_the_dotted_path() {
        let error = parse_root("[components]\nshell = \"s.toml\"\nnonsense = 1\n")
            .expect_err("unknown key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("components.nonsense"));
                assert_eq!(line, Some(3));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_parse_error_names_the_file_the_position_and_the_field() {
        // The offending key is the first thing on the first line of the document.
        let error = parse_root("nonsense = 1\n").expect_err("unknown key");
        let rendered = error.to_string();

        assert!(
            rendered.contains("/cfg/inox/config.toml:1:1"),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains("(field `nonsense`)"),
            "rendered `{rendered}`"
        );
        assert!(rendered.contains("nonsense"), "rendered `{rendered}`");
    }

    #[test]
    fn a_document_level_error_is_reported_without_a_field_path() {
        // The TOML parser rejects the duplicated key before any field is being
        // deserialized, so there is no field path to report.
        let error = parse_root("[components]\ncompositor = \"a.toml\"\ncompositor = \"b.toml\"\n")
            .expect_err("duplicate key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field, None);
                assert!(
                    line.is_some(),
                    "expected the parser to locate the duplicate key"
                );
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn the_locale_defaults_to_following_the_environment() {
        let root = parse_root("").expect("empty document");

        assert_eq!(root.locale, LocaleConfig::default());
        assert_eq!(root.locale.language, None);
    }

    #[test]
    fn a_configured_locale_is_parsed() {
        let root = parse_root("[locale]\nlanguage = \"zh-CN\"\n").expect("document");

        assert_eq!(root.locale.language.as_deref(), Some("zh-CN"));
        assert_eq!(
            root.locale.resolve(),
            Locale::parse("zh-CN").expect("a supported tag")
        );
    }

    #[test]
    fn a_configured_locale_is_negotiated_against_the_catalogs() {
        // `zh-Hans` is not a catalog Inox ships, but its language is, so it
        // resolves to the `zh-CN` catalog rather than to the default.
        let config = LocaleConfig {
            language: Some("zh-Hans".to_owned()),
        };

        assert_eq!(
            config.resolve(),
            Locale::parse("zh-CN").expect("a supported tag")
        );

        // A language with no catalog at all falls back to the default locale.
        let config = LocaleConfig {
            language: Some("de-DE".to_owned()),
        };

        assert_eq!(config.resolve(), Locale::default_locale());
    }

    #[test]
    fn a_malformed_locale_tag_is_rejected_while_parsing() {
        let document = "[locale]\nlanguage = \"not a tag !\"\n";
        let error = parse_root(document).expect_err("malformed tag");

        let rendered = error.to_string();
        assert!(
            rendered.contains(&format!(
                "/cfg/inox/config.toml:{}:",
                line_of(document, "language")
            )),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains("(field `locale.language`)"),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains("BCP-47"),
            "the message should say what a language tag has to be: `{rendered}`"
        );

        match error {
            ConfigError::Parse { field, message, .. } => {
                assert_eq!(field.as_deref(), Some("locale.language"));
                assert!(message.contains("BCP-47"), "message `{message}`");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn the_posix_sentinels_are_rejected_rather_than_read_as_languages() {
        // `C` and `POSIX` mean "no locale was set", not "the language posix".
        // `Locale::parse` would read the second one as a language subtag and
        // the session would end up pinned to `en-US` instead of following the
        // environment, so the configuration layer refuses both. A codeset or a
        // modifier (`C.UTF-8`, `POSIX@euro`) is the same sentinel with a suffix
        // that `Locale::parse` strips, so it is refused as well.
        for tag in [
            "C",
            "POSIX",
            "c",
            "posix",
            " c ",
            "C.UTF-8",
            "POSIX.UTF-8",
            "POSIX@euro",
        ] {
            let document = format!("[locale]\nlanguage = \"{tag}\"\n");
            let error = parse_root(&document).expect_err("a POSIX sentinel is not a language");

            assert!(
                error.to_string().contains("(field `locale.language`)"),
                "`{tag}`: `{error}`"
            );
            assert!(
                error.to_string().contains("omit `[locale]`"),
                "`{tag}` should be told how to follow the environment: `{error}`"
            );
        }
    }

    #[test]
    fn a_locale_without_a_language_follows_the_environment() {
        // `resolve` reads the process environment, which a test cannot rewrite:
        // `std::env::set_var` is `unsafe` in edition 2024 and this crate forbids
        // `unsafe`. The two steps it runs are therefore checked against an
        // injected environment — the same one `Locale::from_env` reads.
        let language = LocaleConfig::default();

        assert_eq!(language.resolve(), Locale::negotiate(&Locale::from_env()));

        let requested = Locale::from_env_with(|name| match name {
            "LANG" => Some("zh_CN.UTF-8".to_owned()),
            _ => None,
        });
        assert_eq!(
            Locale::negotiate(&requested),
            Locale::parse("zh-CN").expect("a supported tag")
        );

        let requested = Locale::from_env_with(|name| match name {
            "LANG" => Some("de_DE.UTF-8".to_owned()),
            _ => None,
        });
        assert_eq!(Locale::negotiate(&requested), Locale::default_locale());
    }

    #[test]
    fn a_locale_that_was_not_validated_still_resolves() {
        // Parsing rejects this tag; a `LocaleConfig` built by hand can still
        // carry one, and `resolve` answers it from the environment instead of
        // failing.
        let config = LocaleConfig {
            language: Some("not a tag !".to_owned()),
        };

        assert_eq!(config.resolve(), Locale::negotiate(&Locale::from_env()));
    }

    #[test]
    fn an_unknown_key_inside_the_locale_section_is_rejected() {
        let error =
            parse_root("[locale]\nlanguage = \"en-US\"\nnonsense = 1\n").expect_err("unknown key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("locale.nonsense"));
                assert_eq!(line, Some(3));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }
}
