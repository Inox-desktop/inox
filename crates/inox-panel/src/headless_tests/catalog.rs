//! The `en-US` and `zh-CN` catalogs, checked against each other (Q03).
//!
//! A translation is only useful if it is complete: an id that exists in one
//! locale and not the other renders as the default language at best and as a
//! message id at worst, and a placeholder that was renamed in one file breaks
//! only for the users of that language. `compare_catalogs` checks both
//! structurally, and this file is where the panel asserts it.

use crate::i18n::{EN_US, ZH_CN};
use inox_common::i18n::{DEFAULT_LOCALE, Locale, Localizer, compare_catalogs, message_ids};

/// The tag of the second locale the panel ships.
const TRANSLATED: &str = "zh-CN";

fn locale(tag: &str) -> Locale {
    Locale::parse(tag).expect("a well-formed language tag")
}

#[test]
fn both_locales_define_the_same_messages_with_the_same_placeholders() {
    let difference = compare_catalogs(EN_US, ZH_CN).expect("both catalogs parse");

    assert!(
        difference.is_clean(),
        "`{TRANSLATED}` is out of step with `{DEFAULT_LOCALE}`: {difference:?}"
    );
}

#[test]
fn every_message_is_translated_rather_than_falling_back() {
    let localizer =
        Localizer::load(locale(TRANSLATED), &crate::i18n::sources()).expect("the catalogs parse");
    let ids = message_ids(EN_US).expect("the default catalog parses");

    assert!(!ids.is_empty(), "the panel ships messages");

    for id in ids {
        assert!(
            localizer.is_translated(&id),
            "`{id}` is only defined for `{DEFAULT_LOCALE}`"
        );
    }
}

#[test]
fn the_default_locale_is_english() {
    assert_eq!(Locale::default_locale().tag(), DEFAULT_LOCALE);
    assert_eq!(DEFAULT_LOCALE, "en-US");
}

#[test]
fn ids_follow_the_component_convention() {
    let ids = message_ids(EN_US).expect("the default catalog parses");

    for id in &ids {
        assert!(
            id.starts_with("panel-"),
            "`{id}` is not named after the component"
        );
        assert!(
            !id.contains('.'),
            "`{id}` carries a dot, which only an attribute may use"
        );
        assert!(
            id.chars().all(|character| character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'),
            "`{id}` uses characters Fluent identifiers do not allow"
        );
    }
}

#[test]
fn a_locale_the_panel_does_not_ship_still_gets_a_panel() {
    let localizer =
        Localizer::load(locale("fr-FR"), &crate::i18n::sources()).expect("the catalogs parse");

    assert_eq!(localizer.locale().tag(), DEFAULT_LOCALE);
    assert_eq!(
        localizer.message("panel-state-empty"),
        "No windows open",
        "a missing translation falls back to the default language (Q03)"
    );
}

#[test]
fn no_message_is_left_empty() {
    for (tag, sources) in [(DEFAULT_LOCALE, EN_US), (TRANSLATED, ZH_CN)] {
        let localizer =
            Localizer::load(locale(tag), &crate::i18n::sources()).expect("the catalogs parse");
        let ids = message_ids(sources).expect("the catalog parses");

        for id in ids {
            assert!(
                !localizer.message(&id).trim().is_empty(),
                "`{id}` renders nothing in `{tag}`"
            );
        }
    }
}
