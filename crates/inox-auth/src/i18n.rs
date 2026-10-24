//! The authentication boundary's Fluent catalogs.
//!
//! The helper has no window, but it does have text a person reads: its usage line
//! and the sentences it refuses to start with. Root AGENTS.md §7.3 puts those in
//! Fluent like every other user-visible string in the workspace rather than in a
//! `bail!` literal, so that an operator who reads Chinese gets the same sentence
//! the unit's documentation quotes.
//!
//! Everything the service says while it is *running* is a `tracing` event rather
//! than a catalog message: logs are for whoever runs the machine, and they are
//! already structured, filtered and routed (root AGENTS.md §3.5). The catalogs are
//! for the two moments a person is looking at a terminal: `--help` and a refusal.

use inox_common::i18n::{I18nError, Locale, Localizer};

/// The `en-US` catalog, the fallback every other locale resolves against.
pub const EN_US: &[&str] = &[include_str!("../i18n/en-US/messages.ftl")];

/// The `zh-CN` catalog.
pub const ZH_CN: &[&str] = &[include_str!("../i18n/zh-CN/messages.ftl")];

/// The catalogs this crate ships, by locale tag, for the parity test.
pub const CATALOGS: [(&str, &[&str]); 2] = [("en-US", EN_US), ("zh-CN", ZH_CN)];

/// The catalogs this crate ships, in the shape [`Localizer::load`] wants.
pub fn sources() -> Vec<(Locale, &'static [&'static str])> {
    vec![(Locale::default_locale(), EN_US), (zh_cn(), ZH_CN)]
}

/// Loads the boundary's catalogs for `requested`.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when either catalog does not parse — a packaging
/// defect that must stop the helper before it opens a socket, not a usage line
/// that renders as its own identifier.
pub fn load(requested: &Locale) -> Result<Localizer, I18nError> {
    Localizer::load(requested.clone(), &sources())
}

/// The `zh-CN` locale.
fn zh_cn() -> Locale {
    Locale::parse("zh-CN").unwrap_or_else(|_| Locale::default_locale())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_have_identical_messages_and_placeholders() {
        let diff = inox_common::i18n::compare_catalogs(EN_US, ZH_CN).expect("valid catalogs");

        assert!(diff.is_clean(), "{diff:?}");
    }

    #[test]
    fn the_usage_line_names_the_binary_and_its_modes() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");
        let usage = localizer.message("auth-usage");

        assert!(usage.contains("inox-auth"), "{usage}");
        assert!(usage.contains("--socket"), "{usage}");
        assert!(
            usage.contains("--session-program"),
            "an operator has to be able to find out what the boundary starts: {usage}"
        );
    }

    #[test]
    fn an_unshipped_locale_falls_back_without_failing() {
        let french = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = load(&french).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
    }
}
