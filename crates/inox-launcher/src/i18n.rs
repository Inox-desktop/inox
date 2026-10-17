//! The launcher's Fluent catalogs.
//!
//! The catalogs are embedded in the binary, so a missing or malformed one is a
//! build-time packaging mistake that `Localizer::load` reports at startup rather
//! than a blank label the user has to notice (root AGENTS.md §7.3, acceptance
//! Q03). `tests/catalog.rs` asserts that the two catalogs define the same ids
//! with the same placeholders.
//!
//! Message ids follow the project's `<component>-<area>-<name>` convention:
//! `launcher-search-placeholder`, `launcher-state-nomatch-title`.

use inox_common::i18n::{I18nError, Locale, Localizer};
use tracing::warn;

/// The `en-US` catalog, which every lookup falls back to.
pub const EN_US: &str = include_str!("../i18n/en-US/launcher.ftl");

/// The `zh-CN` catalog.
pub const ZH_CN: &str = include_str!("../i18n/zh-CN/launcher.ftl");

/// The catalogs this crate ships, as the language tag and its sources.
pub const CATALOGS: [(&str, &[&str]); 2] = [("en-US", &[EN_US]), ("zh-CN", &[ZH_CN])];

/// Loads the launcher's catalogs for `locale`.
///
/// # Errors
///
/// Returns [`I18nError::MissingDefaultCatalog`] if the `en-US` catalog is
/// missing from [`CATALOGS`] and [`I18nError::Syntax`] if any catalog does not
/// parse. A locale with no catalog of its own falls back to `en-US` rather than
/// failing: a packaging gap is not a reason to refuse to start.
pub fn localizer(locale: &Locale) -> Result<Localizer, I18nError> {
    let mut catalogs = Vec::with_capacity(CATALOGS.len());
    for (tag, sources) in CATALOGS {
        match Locale::parse(tag) {
            Ok(locale) => catalogs.push((locale, sources)),
            // Unreachable for the tags above — `catalogs_are_language_tags`
            // keeps it that way — and if it ever happened, the missing default
            // catalog is what `load` reports.
            Err(error) => warn!(%error, tag, "embedded catalog tag is not a language tag"),
        }
    }
    Localizer::load(locale.clone(), &catalogs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_are_language_tags() {
        for (tag, sources) in CATALOGS {
            assert!(
                Locale::parse(tag).is_ok(),
                "{tag} is not a well-formed language tag"
            );
            assert!(!sources.is_empty(), "{tag} has no sources");
        }
    }

    #[test]
    fn the_default_catalog_is_present() {
        assert!(
            CATALOGS
                .iter()
                .any(|(tag, _)| *tag == inox_common::i18n::DEFAULT_LOCALE),
            "the fallback catalog must ship",
        );
    }

    #[test]
    fn a_locale_without_a_catalog_falls_back() {
        let locale = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = localizer(&locale).expect("the catalogs load");
        assert_eq!(localizer.locale().tag(), inox_common::i18n::DEFAULT_LOCALE);
    }

    #[test]
    fn both_catalogs_resolve_the_window_title() {
        for tag in ["en-US", "zh-CN"] {
            let locale = Locale::parse(tag).expect("a well-formed tag");
            let localizer = localizer(&locale).expect("the catalogs load");
            assert!(
                localizer.is_translated("launcher-window-title"),
                "{tag} must translate the window title",
            );
            assert_ne!(localizer.message("launcher-window-title"), "");
        }
    }

    #[test]
    fn placeholders_are_substituted() {
        let locale = Locale::parse("en-US").expect("a well-formed tag");
        let localizer = localizer(&locale).expect("the catalogs load");
        let message = localizer.message_with(
            "launcher-state-nomatch-body",
            &[("query", "firefox".into())],
        );
        assert!(message.contains("firefox"), "{message}");
    }
}
