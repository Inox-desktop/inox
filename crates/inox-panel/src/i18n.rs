//! The panel's Fluent catalogs, and the loader that assembles them.
//!
//! Every user-visible string the panel can show lives in
//! `i18n/en-US/panel.ftl` and `i18n/zh-CN/panel.ftl` (root AGENTS.md §7.3, PRD
//! §11.2). They are embedded with `include_str!`, so a missing catalog is a
//! build failure rather than a panel that renders message ids, and both are
//! parsed at startup: a typo in either locale is reported before the session
//! begins instead of when a user switches language.
//!
//! The loading itself is [`inox_common::i18n::Localizer`]'s job — this module
//! only says which catalogs the panel ships. The parity check that keeps the two
//! locales in step (same ids, same placeholders) runs in `tests/catalog.rs`
//! through [`inox_common::i18n::compare_catalogs`] (acceptance Q03).

use inox_common::i18n::{I18nError, Locale, Localizer};

/// The `en-US` catalog, the fallback every other locale resolves against.
pub const EN_US: &[&str] = &[include_str!("../i18n/en-US/panel.ftl")];

/// The `zh-CN` catalog.
pub const ZH_CN: &[&str] = &[include_str!("../i18n/zh-CN/panel.ftl")];

/// The catalogs this component ships, in the shape [`Localizer::load`] wants.
pub fn sources() -> Vec<(Locale, &'static [&'static str])> {
    vec![(Locale::default_locale(), EN_US), (zh_cn(), ZH_CN)]
}

/// Loads the panel's catalogs for `requested`.
///
/// A requested locale the panel does not ship falls back to `en-US` with a
/// warning, so a user who configured a language Inox has not translated yet gets
/// a working panel rather than an error.
///
/// # Errors
///
/// Returns [`I18nError::Syntax`] when either catalog does not parse — a
/// packaging defect that must stop the panel at startup, not a blank label later.
pub fn load(requested: &Locale) -> Result<Localizer, I18nError> {
    Localizer::load(requested.clone(), &sources())
}

/// The `zh-CN` locale.
///
/// The tag is a compile-time constant, so a parse failure is not a runtime
/// condition. Falling back to the default locale rather than unwrapping keeps
/// the promise that no user-visible flag can make the panel panic (root
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
        assert_eq!(localizer.message("panel-state-empty"), "No windows open");
    }

    #[test]
    fn a_shipped_locale_has_its_own_catalog() {
        let localizer = load(&zh_cn()).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "zh-CN");
        assert_eq!(localizer.message("panel-state-empty"), "没有打开的窗口");
        assert!(localizer.is_translated("panel-state-empty"));
    }

    #[test]
    fn an_unshipped_locale_falls_back_without_failing() {
        let french = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = load(&french).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
        assert_eq!(localizer.message("panel-state-empty"), "No windows open");
    }

    #[test]
    fn placeholders_are_substituted_from_arguments() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        let text = localizer.message_with(
            "panel-task-tooltip",
            &[("app", "Terminal".into()), ("title", "~/src".into())],
        );

        assert_eq!(text, "Terminal: ~/src");
    }

    #[test]
    fn every_message_the_panel_asks_for_exists() {
        // The ids the code names, so a renamed catalog entry fails here rather
        // than rendering as a raw id on the panel.
        let ids = [
            "panel-window-title",
            "panel-state-starting",
            "panel-state-empty",
            "panel-service-unavailable",
            "panel-service-retry",
            "panel-task-unknown-app",
            "panel-task-disabled-service",
            "panel-task-tooltip",
            "panel-task-tooltip-empty",
            "panel-tray-unknown-item",
            "panel-tray-tooltip",
            "panel-tray-status-passive",
            "panel-tray-status-active",
            "panel-tray-status-attention",
            "panel-clock-tooltip",
            "panel-clock-am",
            "panel-clock-pm",
            "panel-clock-zone-utc",
            "panel-clock-zone-offset",
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

    #[test]
    fn names_are_numbered_like_chrono_numbers_them() {
        let localizer = load(&Locale::default_locale()).expect("the catalogs parse");

        assert_eq!(localizer.message("panel-clock-weekday-abbr-1"), "Mon");
        assert_eq!(localizer.message("panel-clock-weekday-full-7"), "Sunday");
        assert_eq!(localizer.message("panel-clock-month-abbr-12"), "Dec");
        assert_eq!(localizer.message("panel-clock-month-full-1"), "January");

        let chinese = load(&zh_cn()).expect("the catalogs parse");
        assert_eq!(chinese.message("panel-clock-weekday-full-1"), "星期一");
        assert_eq!(chinese.message("panel-clock-month-full-12"), "12 月");
        assert_eq!(
            chinese.message_with(
                "panel-clock-zone-offset",
                &[
                    ("sign", Arg::from("+")),
                    ("hours", "08".into()),
                    ("minutes", "00".into())
                ]
            ),
            "UTC+08:00"
        );
    }
}
