//! The daemon's Fluent catalogs, and the loader that assembles them.
//!
//! Every user-visible string the daemon itself can show lives in
//! `i18n/en-US/notifications.ftl` and `i18n/zh-CN/notifications.ftl` (root
//! AGENTS.md §7.3, PRD §11.2). They are embedded with `include_str!`, so a
//! missing catalog is a build failure rather than a popup that renders message
//! ids, and both are parsed at startup: a typo in either locale is reported
//! before the session begins instead of when a user switches language.
//!
//! Two kinds of text are deliberately **not** here:
//!
//! - the application name, the summary, the body and the action labels of a
//!   notification: those are data that arrives with the notification, and
//!   translating them is not the daemon's business;
//! - the protocol's own strings, such as the capability names
//!   ([`crate::protocol`]): those are wire values, not prose.
//!
//! The loading itself is [`inox_common::i18n::Localizer`]'s job — this module
//! only says which catalogs the daemon ships and which ids the code names. The
//! parity check that keeps the two locales in step runs in
//! `crate::headless_tests` through [`inox_common::i18n::compare_catalogs`]
//! (acceptance Q03), together with a two-way check that [`MESSAGES`] and the
//! catalogs agree.

use inox_common::i18n::{I18nError, Locale, Localizer};

/// The `en-US` catalog, the fallback every other locale resolves against.
pub const EN_US: &[&str] = &[include_str!("../i18n/en-US/notifications.ftl")];

/// The `zh-CN` catalog.
pub const ZH_CN: &[&str] = &[include_str!("../i18n/zh-CN/notifications.ftl")];

/// Every message id this crate names, so that a renaming cannot leave a popup
/// showing a raw id.
///
/// `crate::headless_tests` asserts both directions: every id here is in the
/// catalogs, and every id in the catalogs is here.
pub const MESSAGES: &[&str] = &[
    "notifications-window-title",
    "notifications-empty",
    "notifications-suppressed",
    "notifications-generic-summary",
    "notifications-close",
    "notifications-action-disabled-policy",
    "notifications-overflow",
    "notifications-urgency-low",
    "notifications-urgency-critical",
    "notifications-age-just-now",
    "notifications-age-minutes",
    "notifications-age-hours",
    "notifications-age-days",
    "notifications-image-placeholder",
];

/// The catalogs this component ships, in the shape [`Localizer::load`] wants.
pub fn sources() -> Vec<(Locale, &'static [&'static str])> {
    vec![(Locale::default_locale(), EN_US), (zh_cn(), ZH_CN)]
}

/// Loads the daemon's catalogs for `requested`.
///
/// A requested locale the daemon does not ship falls back to `en-US` with a
/// warning, so a user who configured a language Inox has not translated yet gets
/// a working daemon rather than an error.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when either catalog does not parse — a
/// packaging defect that must stop the daemon at startup, not a swallowed one.
pub fn load(requested: &Locale) -> Result<Localizer, I18nError> {
    Localizer::load(requested.clone(), &sources())
}

/// The `zh-CN` locale.
///
/// The tag is a compile-time constant, so a parse failure is not a runtime
/// condition. Falling back to the default locale rather than unwrapping keeps
/// the promise that no user-visible flag can make the daemon panic (root
/// AGENTS.md §3.3); the fallback is unreachable for the constant written here.
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
        assert_eq!(localizer.message("notifications-empty"), "No notifications");
    }

    #[test]
    fn a_shipped_locale_has_its_own_catalog() {
        let localizer = load(&zh_cn()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "zh-CN");
        assert_eq!(localizer.message("notifications-close"), "关闭");
        assert!(localizer.is_translated("notifications-close"));
    }

    #[test]
    fn an_unshipped_locale_falls_back_without_failing() {
        let french = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = load(&french).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
        assert_eq!(
            localizer.message("notifications-generic-summary"),
            "A notification arrived"
        );
    }

    #[test]
    fn the_overflow_message_takes_the_number_it_was_given() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        let text = localizer.message_with("notifications-overflow", &[("count", 4usize.into())]);

        assert_eq!(text, "4 more waiting");
    }

    #[test]
    fn the_chinese_catalog_substitutes_the_same_placeholder() {
        let localizer = load(&zh_cn()).expect("the catalogs parse");

        let text = localizer.message_with("notifications-overflow", &[("count", Arg::from(4i32))]);

        assert!(text.contains('4'), "the count is in the sentence: {text}");
        assert_ne!(text, "4 more waiting");
    }

    #[test]
    fn every_message_the_code_names_exists_in_both_locales() {
        for locale in [Locale::default_locale(), zh_cn()] {
            let localizer = load(&locale).expect("the catalogs parse");

            for id in MESSAGES {
                assert!(
                    localizer.is_translated(id),
                    "`{id}` is missing from the `{}` catalog",
                    locale.tag()
                );
                assert!(
                    !localizer.message(id).trim().is_empty(),
                    "`{id}` renders nothing in `{}`",
                    locale.tag()
                );
            }
        }
    }

    #[test]
    fn an_id_the_catalogs_do_not_define_is_returned_as_it_stands() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        assert_eq!(
            localizer.message("notifications-nonsense"),
            "notifications-nonsense"
        );
    }
}
