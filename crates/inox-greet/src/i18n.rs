//! Embedded catalogs for the component's diagnostics and interface.
use inox_common::i18n::{I18nError, Locale, Localizer};

/// Default language catalog.
pub const EN_US: &str = include_str!("../i18n/en-US/messages.ftl");
/// Simplified Chinese catalog.
pub const ZH_CN: &str = include_str!("../i18n/zh-CN/messages.ftl");

/// Load both catalogs and negotiate the requested language.
pub fn load(locale: Locale) -> Result<Localizer, I18nError> {
    let zh = Locale::parse("zh-CN").unwrap_or_else(|_| Locale::default_locale());
    Localizer::load(
        locale,
        &[(Locale::default_locale(), &[EN_US]), (zh, &[ZH_CN])],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogs_have_identical_messages_and_placeholders() {
        let diff = inox_common::i18n::compare_catalogs(&[EN_US], &[ZH_CN]).expect("valid catalogs");
        assert!(diff.is_clean(), "{diff:?}");
        for tag in ["en-US", "zh-CN"] {
            let localizer = load(Locale::parse(tag).expect("locale")).expect("catalog");
            for id in ["usage", "self-check", "blocked", "title", "user", "login"] {
                assert_ne!(
                    localizer.message(id),
                    id,
                    "message `{id}` must exist in both catalogs"
                );
            }
        }
    }
}
