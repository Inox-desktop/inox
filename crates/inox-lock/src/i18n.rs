//! The lock screen's Fluent catalogs, and the loader that assembles them.
//!
//! Every user-visible string the lock screen can show lives in
//! `i18n/en-US/lock.ftl` and `i18n/zh-CN/lock.ftl` (root AGENTS.md §7.3, PRD
//! §11.2). They are embedded with `include_str!`, so a missing catalog is a build
//! failure rather than a screen that renders message ids, and both are parsed at
//! startup: a typo in either locale is reported before a session begins instead of
//! when a user switches language.
//!
//! The loading itself is [`inox_common::i18n::Localizer`]'s job — this module only
//! says which catalogs the lock screen ships. The parity check that keeps the two
//! locales in step (same ids, same placeholders) runs in `tests/catalog.rs`
//! through [`inox_common::i18n::compare_catalogs`] (acceptance Q03).
//!
//! One catalog is load-bearing in a way the others are not: the message that says
//! *why* the session lock cannot start is the only thing `main.rs` has to say, and
//! it has to be said in a language the user reads. That is why the binary loads
//! the catalogs before it reports anything (ADR 0015, ADR 0018).

use inox_common::i18n::{I18nError, Locale, Localizer};

/// The `en-US` catalog, the fallback every other locale resolves against.
pub const EN_US: &[&str] = &[include_str!("../i18n/en-US/lock.ftl")];

/// The `zh-CN` catalog.
pub const ZH_CN: &[&str] = &[include_str!("../i18n/zh-CN/lock.ftl")];

/// The catalogs this component ships, by locale tag, for the parity test.
pub const CATALOGS: [(&str, &[&str]); 2] = [("en-US", EN_US), ("zh-CN", ZH_CN)];

/// The catalogs this component ships, in the shape [`Localizer::load`] wants.
pub fn sources() -> Vec<(Locale, &'static [&'static str])> {
    vec![(Locale::default_locale(), EN_US), (zh_cn(), ZH_CN)]
}

/// Loads the lock screen's catalogs for `requested`.
///
/// A requested locale that Inox does not ship falls back to `en-US` with a
/// warning, so a user who configured a language there is no translation for gets a
/// working lock screen rather than an error — and, because a lock screen is the
/// one screen a user cannot do without, that fallback is the reason this function
/// has no failure path beyond a malformed catalog.
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
    use inox_common::i18n::Arg;

    #[test]
    fn the_default_locale_is_the_fallback() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
        assert_eq!(localizer.message("lock-prompt"), "Enter your password");
    }

    #[test]
    fn a_shipped_locale_has_its_own_catalog() {
        let localizer = load(&zh_cn()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "zh-CN");
        assert_eq!(localizer.message("lock-prompt"), "请输入密码");
        assert!(localizer.is_translated("lock-prompt"));
    }

    #[test]
    fn an_unshipped_locale_falls_back_without_failing() {
        let french = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = load(&french).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
        assert_eq!(localizer.message("lock-prompt"), "Enter your password");
    }

    #[test]
    fn the_message_that_explains_a_blocked_lock_says_which_block_i_t_is() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        let reason = localizer.message("lock-start-blocked-reason");
        let message =
            localizer.message_with("lock-start-blocked", &[("reason", Arg::from(reason))]);

        assert!(message.contains("ext-session-lock-v1"), "{message}");
        assert!(message.contains("human security review"), "{message}");
        assert!(
            !message.contains("lock-start-blocked"),
            "a message id on screen is a missing translation: {message}"
        );

        let chinese = load(&zh_cn()).expect("the catalogs parse");
        let reason = chinese.message("lock-start-blocked-reason");
        let message = chinese.message_with("lock-start-blocked", &[("reason", Arg::from(reason))]);
        assert!(message.contains("ext-session-lock-v1"), "{message}");
        assert!(!message.contains("lock-start-blocked"), "{message}");
    }

    #[test]
    fn every_message_the_lock_screen_asks_for_exists() {
        // The ids the code names, so a renamed catalog entry fails here rather
        // than rendering as a raw id on a screen a user is looking at while
        // unable to reach their session.
        let ids = [
            "lock-window-title",
            "lock-prompt",
            "lock-field-label",
            "lock-field-placeholder",
            "lock-hint",
            "lock-submit",
            "lock-verifying",
            "lock-attempts-waiting",
            "lock-failed",
            "lock-failure-rejected",
            "lock-failure-timed-out",
            "lock-failure-unavailable",
            "lock-failure-cancelled",
            "lock-disabled-waiting",
            "lock-disabled-verifying",
            "lock-disabled-unlocked",
            "lock-disabled-protocol",
            "lock-disabled-boundary",
            "lock-disabled-no-ui",
            "lock-held-title",
            "lock-held-detail",
            "lock-unavailable",
            "lock-unavailable-retry",
            "lock-protocol-missing-global",
            "lock-protocol-registry-unavailable",
            "lock-boundary-disconnected",
            "lock-boundary-timeout",
            "lock-suspend-refused-lock-failed",
            "lock-suspend-refused-timeout",
            "lock-suspend-refused-no-protocol",
            "lock-start-blocked",
            "lock-start-blocked-reason",
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
