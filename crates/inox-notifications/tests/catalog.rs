//! The `en-US` and `zh-CN` catalogs, checked against each other (Q03).
//!
//! A translation is only useful if it is complete: an id that exists in one
//! locale and not the other renders as the default language at best and as a
//! message id at worst, and a placeholder that was renamed in one file breaks
//! only for the users of that language. `compare_catalogs` checks both
//! structurally, and this file also checks the other direction — that the code
//! and the catalogs name the same set of messages, so neither a renamed id nor
//! a string nobody asks for can survive a review.

use inox_common::i18n::{DEFAULT_LOCALE, Locale, Localizer, compare_catalogs, message_ids};
use inox_notifications::i18n::{EN_US, MESSAGES, ZH_CN};

/// The tag of the second locale the daemon ships.
const TRANSLATED: &str = "zh-CN";

fn locale(tag: &str) -> Locale {
    Locale::parse(tag).expect("a well-formed language tag")
}

fn localizer(tag: &str) -> Localizer {
    Localizer::load(locale(tag), &inox_notifications::i18n::sources()).expect("the catalogs parse")
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
    let ids = message_ids(EN_US).expect("the default catalog parses");
    assert!(!ids.is_empty(), "the daemon ships messages");

    let chinese = localizer(TRANSLATED);

    for id in ids {
        assert!(
            chinese.is_translated(&id),
            "`{id}` is only defined for `{DEFAULT_LOCALE}`"
        );
    }
}

#[test]
fn the_code_and_the_catalogs_name_the_same_messages() {
    let defined = message_ids(EN_US).expect("the default catalog parses");
    let named: std::collections::BTreeSet<String> =
        MESSAGES.iter().map(|id| (*id).to_owned()).collect();

    assert_eq!(
        defined, named,
        "a message the code names and the catalogs do not, or the other way \
         round, is a message id on screen or a string nobody can reach"
    );
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
            id.starts_with("notifications-"),
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
fn a_locale_the_daemon_does_not_ship_still_gets_a_daemon() {
    let french = localizer("fr-FR");

    assert_eq!(french.locale().tag(), DEFAULT_LOCALE);
    assert_eq!(
        french.message("notifications-empty"),
        "No notifications",
        "a missing translation falls back to the default language (Q03)"
    );
}

#[test]
fn no_message_is_left_empty() {
    for tag in [DEFAULT_LOCALE, TRANSLATED] {
        let localizer = localizer(tag);
        let sources = if tag == DEFAULT_LOCALE { EN_US } else { ZH_CN };
        let ids = message_ids(sources).expect("the catalog parses");

        for id in ids {
            assert!(
                !localizer.message(&id).trim().is_empty(),
                "`{id}` renders nothing in `{tag}`"
            );
        }
    }
}

#[test]
fn the_overflow_sentence_carries_the_number_in_both_locales() {
    for tag in [DEFAULT_LOCALE, TRANSLATED] {
        let localizer = localizer(tag);
        let sentence = localizer.message_with("notifications-overflow", &[("count", 7i32.into())]);

        assert!(
            sentence.contains('7'),
            "the count is in the `{tag}` sentence: {sentence}"
        );
        assert!(
            !sentence.contains('$'),
            "the placeholder was substituted, not left standing: {sentence}"
        );
    }
}

#[test]
fn the_two_locales_say_the_muted_line_differently() {
    let english = localizer(DEFAULT_LOCALE).message("notifications-suppressed");
    let chinese = localizer(TRANSLATED).message("notifications-suppressed");

    assert_ne!(english, chinese, "the second locale is a translation");
    assert!(!english.is_empty());
    assert!(!chinese.is_empty());
}
