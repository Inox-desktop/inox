//! The component's message catalogs.
//!
//! the L3 component architecture puts the Fluent machinery in `inox-common::i18n` — loading a catalog
//! and resolving a locale is not UI code — and leaves each component with the
//! one thing that is its own: the catalogs themselves, embedded at compile time
//! and handed over. That is all this module does.
//!
//! Two catalogs ship, `en-US` and `zh-CN`, and they are kept in step by
//! [`MESSAGES`] rather than by review: [`crate::view_model`] may only format ids
//! that are listed there, the list is checked against both catalogs, and
//! `tests/catalog.rs` checks the two catalogs against each other (acceptance
//! Q03). A string added to one catalog and forgotten in the other fails a test
//! rather than showing an id to a user.

use inox_common::i18n::{I18nError, Locale, LocaleError, Localizer};

/// The default locale's catalog, embedded at compile time.
pub const EN_US: &str = include_str!("../i18n/en-US/wallpaper.ftl");

/// The second catalog Inox ships, embedded at compile time.
pub const ZH_CN: &str = include_str!("../i18n/zh-CN/wallpaper.ftl");

/// The language tag [`ZH_CN`] is written for.
pub const CHINESE_TAG: &str = "zh-CN";

/// Every message id this component formats.
///
/// The list exists so that a message id typed into the code cannot silently
/// become a message id that no catalog defines — [`Localizer::message`] returns
/// the id itself in that case, which is diagnosable but ugly on screen, and this
/// is the check that keeps it from happening.
pub const MESSAGES: &[&str] = &[
    "wallpaper-action-nothing",
    "wallpaper-action-retry",
    "wallpaper-config-error",
    "wallpaper-config-read-once",
    "wallpaper-config-remedy",
    "wallpaper-empty-remedy",
    "wallpaper-mode-center",
    "wallpaper-mode-fill",
    "wallpaper-mode-fit",
    "wallpaper-mode-stretch",
    "wallpaper-mode-tile",
    "wallpaper-output-background",
    "wallpaper-output-heading",
    "wallpaper-output-image",
    "wallpaper-output-source",
    "wallpaper-phase-degraded",
    "wallpaper-phase-idle",
    "wallpaper-phase-loading",
    "wallpaper-phase-ready",
    "wallpaper-problem-heading",
    "wallpaper-problem-subject",
    "wallpaper-reason-degenerate",
    "wallpaper-reason-missing",
    "wallpaper-reason-not-a-file-directory",
    "wallpaper-reason-not-a-file-other",
    "wallpaper-reason-too-large",
    "wallpaper-reason-too-long-side",
    "wallpaper-reason-too-many-tiles",
    "wallpaper-reason-undecodable",
    "wallpaper-reason-unreadable",
    "wallpaper-remedy",
    "wallpaper-scaling",
    "wallpaper-status-loading",
    "wallpaper-window-title",
    "wallpaper-windowed-notice",
];

/// Why the component's own catalogs could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    /// A language tag this crate hard-codes is malformed, which can only happen
    /// if the constant is edited wrongly. It is a distinct variant so that it
    /// cannot be mistaken for a missing translation.
    #[error("`{tag}` is not a well-formed language tag: {source}")]
    Locale {
        /// The rejected tag.
        tag: String,
        /// The parser's own explanation.
        #[source]
        source: LocaleError,
    },
    /// A catalog does not parse, or the default one is missing.
    #[error(transparent)]
    Load(#[from] I18nError),
}

/// The catalogs this component ships, keyed by the locale they are written for.
///
/// # Errors
///
/// [`CatalogError::Locale`] when a hard-coded tag is not well formed.
pub fn catalogs() -> Result<Vec<(Locale, &'static [&'static str])>, CatalogError> {
    let chinese = Locale::parse(CHINESE_TAG).map_err(|source| CatalogError::Locale {
        tag: CHINESE_TAG.to_owned(),
        source,
    })?;

    Ok(vec![
        (Locale::default_locale(), &[EN_US][..]),
        (chinese, &[ZH_CN][..]),
    ])
}

/// A localizer for the locale the session runs in.
///
/// The requested locale is negotiated by [`Localizer::load`], so any language —
/// including one with no catalog at all — lands on a catalog Inox ships.
///
/// # Errors
///
/// [`CatalogError`] when a catalog does not parse. A malformed `.ftl` is a
/// packaging defect and stops the component at startup rather than blanking a
/// label later.
pub fn localizer(requested: Locale) -> Result<Localizer, CatalogError> {
    Ok(Localizer::load(requested, &catalogs()?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::{Arg, message_ids};

    fn ids_of(source: &str) -> Vec<String> {
        let mut ids: Vec<String> = message_ids(&[source])
            .expect("a catalog that parses")
            .into_iter()
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn both_catalogs_are_embedded_and_parse() {
        assert!(!EN_US.is_empty());
        assert!(!ZH_CN.is_empty());
        assert!(localizer(Locale::default_locale()).is_ok());
    }

    #[test]
    fn the_message_list_is_exactly_what_the_catalogs_define() {
        let mut listed: Vec<String> = MESSAGES.iter().map(|id| (*id).to_owned()).collect();
        listed.sort();

        assert_eq!(listed, ids_of(EN_US), "the list and en-US must agree");
        assert_eq!(listed, ids_of(ZH_CN), "the list and zh-CN must agree");
    }

    #[test]
    fn every_message_is_translated_in_both_catalogs() {
        for id in MESSAGES {
            assert!(
                localizer(Locale::default_locale())
                    .expect("a localizer")
                    .is_translated(id),
                "`{id}` is missing from the default catalog"
            );
        }

        let chinese =
            localizer(Locale::parse(CHINESE_TAG).expect("a well-formed tag")).expect("a localizer");

        assert_eq!(chinese.locale().language(), "zh");
        for id in MESSAGES {
            assert!(
                chinese.is_translated(id),
                "`{id}` is missing from the {CHINESE_TAG} catalog"
            );
        }
    }

    #[test]
    fn a_message_id_no_catalog_defines_comes_back_as_itself() {
        // The failure mode the MESSAGES list exists to prevent: diagnosable,
        // which is why a missing message is not an error at run time.
        let localizer = localizer(Locale::default_locale()).expect("a localizer");

        assert_eq!(
            localizer.message("wallpaper-nonsense"),
            "wallpaper-nonsense"
        );
    }

    #[test]
    fn a_language_with_no_catalog_falls_back_to_the_default() {
        let localizer =
            localizer(Locale::parse("de-DE").expect("a well-formed tag")).expect("a localizer");

        assert_eq!(localizer.locale(), &Locale::default_locale());
        assert_eq!(
            localizer.message("wallpaper-window-title"),
            "Inox wallpaper"
        );
    }

    #[test]
    fn the_locale_negotiation_reaches_the_chinese_catalog() {
        let requested = Locale::parse("zh-Hans").expect("a well-formed tag");
        let localizer = localizer(requested).expect("a localizer");

        assert_eq!(localizer.locale().language(), "zh");
        assert_ne!(
            localizer.message("wallpaper-window-title"),
            "Inox wallpaper"
        );
    }

    #[test]
    fn a_message_carries_its_arguments() {
        let localizer = localizer(Locale::default_locale()).expect("a localizer");
        let rendered = localizer.message_with(
            "wallpaper-output-heading",
            &[("output", Arg::from("HDMI-A-1"))],
        );

        assert_eq!(rendered, "Output HDMI-A-1");
    }

    #[test]
    fn the_catalogs_describe_the_locales_they_are_named_for() {
        let catalogs = catalogs().expect("the shipped catalogs");

        assert_eq!(catalogs.len(), 2);
        assert_eq!(catalogs[0].0, Locale::default_locale());
        assert_eq!(catalogs[1].0.language(), "zh");
        assert_eq!(catalogs.iter().map(|(_, s)| s.len()).sum::<usize>(), 2);
    }
}
