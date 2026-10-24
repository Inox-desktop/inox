//! The login screen's Fluent catalogs, and the loader that assembles them.
//!
//! Every user-visible string the login screen can show lives in
//! `i18n/en-US/messages.ftl` and `i18n/zh-CN/messages.ftl` (root AGENTS.md §7.3,
//! PRD §11.2). They are embedded with `include_str!`, so a missing catalog is a
//! build failure rather than a window that renders message ids, and both are
//! parsed at startup: a typo in either locale is reported before a session begins
//! instead of when a user switches language.
//!
//! The loading itself is [`inox_common::i18n::Localizer`]'s job — this module only
//! says which catalogs the login screen ships. The parity check that keeps the two
//! locales in step (same ids, same placeholders) runs in `tests/catalog.rs`
//! through [`inox_common::i18n::compare_catalogs`] (acceptance Q03).
//!
//! The binary loads the catalogs before it reports anything, and that matters
//! more here than elsewhere: the default invocation of `inox-greet` refuses to
//! log anyone in, and the sentence that says why is the only thing it has to say.

use inox_common::i18n::{I18nError, Locale, Localizer};

/// The `en-US` catalog, the fallback every other locale resolves against.
pub const EN_US: &[&str] = &[include_str!("../i18n/en-US/messages.ftl")];

/// The `zh-CN` catalog.
pub const ZH_CN: &[&str] = &[include_str!("../i18n/zh-CN/messages.ftl")];

/// The catalogs this component ships, by locale tag, for the parity test.
pub const CATALOGS: [(&str, &[&str]); 2] = [("en-US", EN_US), ("zh-CN", ZH_CN)];

/// The catalogs this component ships, in the shape [`Localizer::load`] wants.
pub fn sources() -> Vec<(Locale, &'static [&'static str])> {
    vec![(Locale::default_locale(), EN_US), (zh_cn(), ZH_CN)]
}

/// Loads the login screen's catalogs for `requested`.
///
/// A requested locale that Inox does not ship falls back to `en-US` with a
/// warning, so a user who configured a language there is no translation for gets
/// a usable login screen rather than an error.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when either catalog does not parse — a packaging
/// defect that must stop the component at startup, not a blank label later.
pub fn load(requested: &Locale) -> Result<Localizer, I18nError> {
    Localizer::load(requested.clone(), &sources())
}

/// The `zh-CN` locale.
///
/// The tag is a compile-time constant, so a parse failure is not a runtime
/// condition. Falling back to the default locale rather than unwrapping keeps the
/// promise that no user-visible flag can make this crate panic (root AGENTS.md
/// §3.3); the fallback is unreachable for the constant written here.
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
    fn the_default_locale_is_the_fallback() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
        assert_eq!(localizer.message("greet-window-title"), "Inox login");
        assert_ne!(localizer.message("greet-usage"), "greet-usage");
    }

    #[test]
    fn a_shipped_locale_has_its_own_catalog() {
        let localizer = load(&zh_cn()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "zh-CN");
        assert_eq!(localizer.message("greet-window-title"), "Inox 登录");
        assert!(localizer.is_translated("greet-window-title"));
    }

    #[test]
    fn an_unshipped_locale_falls_back_without_failing() {
        let french = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = load(&french).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
        assert_eq!(localizer.message("greet-window-title"), "Inox login");
    }

    #[test]
    fn the_message_that_explains_a_blocked_login_says_what_is_missing() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        let message = localizer.message("greet-blocked");

        // The block is the *authentication boundary*, not the display manager: the
        // screen cannot check a login because the process that can is not there, and
        // a message that blamed something else would send whoever reads it to the
        // wrong place (ADR 0029).
        assert!(message.contains("authentication boundary"), "{message}");
        assert!(message.contains("not available"), "{message}");
        assert!(
            !message.contains("greet-blocked"),
            "a message id on screen is a missing translation: {message}"
        );
    }

    #[test]
    fn every_message_the_login_screen_asks_for_exists() {
        // The ids the code names, so a renamed catalog entry fails here rather
        // than rendering as a raw id in a window the person cannot log in from.
        let ids = [
            "greet-usage",
            "greet-window-title",
            "greet-credential-label",
            "greet-credential-placeholder",
            "greet-credential-too-long",
            "greet-hint",
            "greet-hint-clear",
            "greet-submit",
            "greet-clear",
            "greet-switch-user",
            "greet-back",
            "greet-choose-account",
            "greet-session-label",
            "greet-session-name",
            "greet-banner",
            "greet-banner-live",
            "greet-blocked",
            "greet-unavailable",
            "greet-checking",
            "greet-starting",
            "greet-started",
            "greet-failed",
            "greet-identity-too-long",
            "greet-disabled-unavailable",
            "greet-disabled-verifying",
            "greet-disabled-started",
            "greet-disabled-invalid",
            "greet-disabled-rate-limited",
            "greet-disabled-no-accounts",
            "greet-disabled-accounts-unreadable",
            "greet-disabled-account-unselected",
        ];

        for locale in [Locale::default_locale(), zh_cn()] {
            let localizer = load(&locale).expect("the catalogs parse");
            for id in ids {
                assert!(
                    localizer.is_translated(id),
                    "`{id}` is missing from the `{}` catalog",
                    locale.tag()
                );
            }
        }
    }
}
