//! The settings window's Fluent catalogs, and the loader that assembles them.
//!
//! Every user-visible string the window can show lives in
//! `i18n/en-US/settings.ftl` and `i18n/zh-CN/settings.ftl` (root `AGENTS.md`
//! §7.3, PRD §11.2). They are embedded with `include_str!`, so a missing catalog
//! is a build failure rather than a window that renders message ids, and both
//! are parsed at startup: a typo in either locale is reported before the window
//! opens instead of when a user switches language.
//!
//! The loading itself is [`inox_common::i18n::Localizer`]'s job — this module
//! only says which catalogs this component ships. The parity check that keeps
//! the two locales in step runs twice: in `tests/catalog.rs` through
//! [`inox_common::i18n::compare_catalogs`], and here, for the case where only
//! the unit tests are run (acceptance Q03).

use inox_common::i18n::{I18nError, Locale, Localizer};

/// The `en-US` catalog, the fallback every other locale resolves against.
pub const EN_US: &[&str] = &[include_str!("../i18n/en-US/settings.ftl")];

/// The `zh-CN` catalog.
pub const ZH_CN: &[&str] = &[include_str!("../i18n/zh-CN/settings.ftl")];

/// The catalogs this component ships, in the shape [`Localizer::load`] wants.
pub fn sources() -> Vec<(Locale, &'static [&'static str])> {
    vec![(Locale::default_locale(), EN_US), (zh_cn(), ZH_CN)]
}

/// Loads the window's catalogs for `requested`.
///
/// A requested locale the window does not ship falls back to `en-US` with a
/// warning, so a user who configured a language Inox has not translated yet gets
/// a working window rather than an error.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when either catalog does not parse — a
/// packaging defect that must stop the window at startup, not a blank label
/// later.
pub fn load(requested: &Locale) -> Result<Localizer, I18nError> {
    Localizer::load(requested.clone(), &sources())
}

/// Every message id the window can ask for, from every module that names one.
///
/// The page and field ids come from the model that needs them, so a field added
/// to [`crate::draft::Field`] brings its labels with it and `tests/catalog.rs`
/// fails until both catalogs define them.
pub fn message_ids() -> Vec<&'static str> {
    let mut ids = crate::view_model::message_ids();
    ids.extend([
        "settings-ctl-help",
        "settings-ctl-applied",
        "settings-ctl-absent",
        "settings-ctl-failed",
        "settings-ctl-incomplete",
    ]);
    ids
}

/// The `zh-CN` locale.
///
/// The tag is a compile-time constant, so a parse failure is not a runtime
/// condition. Falling back to the default locale rather than unwrapping keeps
/// the promise that no user-visible flag can make the window panic (root
/// `AGENTS.md` §3.3); the fallback is unreachable for the constant written here.
fn zh_cn() -> Locale {
    Locale::parse("zh-CN").unwrap_or_else(|_| Locale::default_locale())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::Component;
    use crate::pages::PageId;

    #[test]
    fn the_default_locale_is_the_fallback() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
        assert_eq!(localizer.message("settings-action-apply"), "Apply");
    }

    #[test]
    fn a_shipped_locale_has_its_own_catalog() {
        let localizer = load(&zh_cn()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "zh-CN");
        assert_eq!(localizer.message("settings-action-apply"), "应用");
        assert!(localizer.is_translated("settings-action-apply"));
    }

    #[test]
    fn an_unshipped_locale_falls_back_without_failing() {
        let french = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = load(&french).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
        assert_eq!(localizer.message("settings-action-apply"), "Apply");
    }

    #[test]
    fn placeholders_are_substituted_from_arguments() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        let text = localizer.message_with(
            "settings-number-with-unit",
            &[("value", "48".into()), ("unit", "px".into())],
        );

        assert_eq!(text, "48 px");
    }

    #[test]
    fn every_message_the_window_can_show_exists_in_both_catalogs() {
        for locale in [Locale::default_locale(), zh_cn()] {
            let localizer = load(&locale).expect("the catalogs parse");

            for id in message_ids() {
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
    fn every_page_and_component_is_named() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        for page in PageId::ALL {
            assert!(localizer.is_translated(page.label_id()), "{page:?}");
        }

        for component in Component::ALL {
            assert!(
                localizer.is_translated(component.label_id()),
                "{component:?}"
            );
        }
    }

    #[test]
    fn the_two_catalogs_are_in_step() {
        let difference =
            inox_common::i18n::compare_catalogs(EN_US, ZH_CN).expect("both catalogs parse");

        assert!(difference.is_clean(), "{difference:?}");
    }
}
