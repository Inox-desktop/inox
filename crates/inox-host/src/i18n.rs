//! The greeter host's Fluent catalogs, and the loader that assembles them.
//!
//! The host is a systemd unit, so most of what it says goes to the journal rather
//! than to a window — but it is still read by a person looking at why their machine
//! did not reach a login screen, which is what makes these strings user-visible text
//! under root AGENTS.md §7.3. They live in `i18n/en-US/messages.ftl` and
//! `i18n/zh-CN/messages.ftl`, are embedded with `include_str!` so a missing catalog
//! is a build failure, and are both parsed at startup so a typo in either locale is
//! reported before this stage starts a compositor.
//!
//! The loading is [`inox_common::i18n::Localizer`]'s job; this module only says
//! which catalogs the host ships. The parity check that keeps the two locales in
//! step (same ids, same placeholders) and the check that every `host-…` id the code
//! names exists — and that nothing in the catalog is unused — run in
//! `tests/catalog.rs` (acceptance Q03).

use inox_common::i18n::{I18nError, Locale, Localizer};

/// The `en-US` catalog, the fallback every other locale resolves against.
pub const EN_US: &[&str] = &[include_str!("../i18n/en-US/messages.ftl")];

/// The `zh-CN` catalog.
pub const ZH_CN: &[&str] = &[include_str!("../i18n/zh-CN/messages.ftl")];

/// The catalogs this component ships, by locale tag, for the parity test.
pub const CATALOGS: [(&str, &[&str]); 2] = [("en-US", EN_US), ("zh-CN", ZH_CN)];

/// The catalogs the host ships, in the shape [`Localizer::load`] wants.
pub fn sources() -> Vec<(Locale, &'static [&'static str])> {
    vec![(Locale::default_locale(), EN_US), (zh_cn(), ZH_CN)]
}

/// Loads the host's catalogs for `requested`.
///
/// A requested locale Inox does not ship falls back to `en-US` with a warning: an
/// operator with an unusual locale should get the sentence that explains the
/// failure, not an error about the sentence.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when either catalog does not parse — a packaging
/// defect that has to stop the host at startup rather than at the moment a child
/// fails.
pub fn load(requested: &Locale) -> Result<Localizer, I18nError> {
    Localizer::load(requested.clone(), &sources())
}

/// The `zh-CN` locale.
///
/// The tag is a compile-time constant, so a parse failure is not a runtime condition;
/// falling back rather than unwrapping keeps the promise that nothing in this crate
/// panics outside its tests (root AGENTS.md §3.3).
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
        assert!(localizer.message("host-usage").contains("inox-host"));
    }

    #[test]
    fn a_shipped_locale_has_its_own_catalog() {
        let localizer = load(&zh_cn()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "zh-CN");
        assert!(localizer.is_translated("host-usage"));
        assert_no_message_id_reaches_the_screen(&localizer);
    }

    #[test]
    fn an_unshipped_locale_falls_back_without_failing() {
        let french = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = load(&french).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
    }

    #[test]
    fn the_refusal_names_both_the_boundary_and_its_process() {
        for (locale, boundary) in [
            (Locale::default_locale(), "authentication boundary"),
            (zh_cn(), "认证边界"),
        ] {
            let localizer = load(&locale).expect("the catalogs parse");
            let message = localizer.message_with(
                "host-boundary-missing",
                &[("socket", "/run/inox/auth.sock".into())],
            );

            // ADR 0037 §决策 3.1 requires the refusal to sit on both facts: what is
            // missing, and which process provides it. An operator reading the journal
            // has to know both without guessing, in their own language.
            assert!(message.contains(boundary), "{}: {message}", locale.tag());
            assert!(message.contains("inox-auth"), "{}: {message}", locale.tag());
            assert!(
                message.contains("/run/inox/auth.sock"),
                "the refusal has to name the socket that was tried: {message}"
            );
            assert!(
                !message.contains("host-"),
                "a message id on screen: {message}"
            );
        }
    }

    #[test]
    fn formatting_fills_every_placeholder() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        let message = localizer.message_with(
            "host-socket-timeout",
            &[
                ("socket", "wayland-inox-greet".into()),
                ("seconds", 10.into()),
            ],
        );
        assert!(message.contains("wayland-inox-greet"), "{message}");
        assert!(message.contains("10"), "{message}");
        assert!(!message.contains('$'), "an unfilled placeholder: {message}");
    }

    /// A `host-…` id rendered on screen means a missing translation, which is a
    /// packaging defect rather than a message.
    fn assert_no_message_id_reaches_the_screen(localizer: &Localizer) {
        for id in [
            "host-usage",
            "host-version",
            "host-boundary-missing",
            "host-compositor-exited",
            "host-socket-timeout",
            "host-compositor-unstartable",
            "host-greeter-unstartable",
            "host-restarts-exhausted",
            "host-stopping",
        ] {
            let message = localizer.message(id);
            assert_ne!(message, id, "`{id}` is not translated");
        }
    }
}
