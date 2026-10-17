//! The two catalogs, checked against each other (acceptance Q03).
//!
//! A component that ships a translation has two ways to break it: a message in
//! one catalog and not the other, or the same message with different
//! placeholders. Both fail here rather than at run time, where the symptom would
//! be an id on screen or a blank one.

use inox_common::i18n::{Locale, compare_catalogs, message_ids};
use inox_wallpaper::i18n::{self, CHINESE_TAG};

/// The default-locale catalog, as embedded in the binary.
const EN_US: &str = include_str!("../i18n/en-US/wallpaper.ftl");

/// The second catalog, as embedded in the binary.
const ZH_CN: &str = include_str!("../i18n/zh-CN/wallpaper.ftl");

#[test]
fn the_two_catalogs_define_the_same_messages_with_the_same_placeholders() {
    let diff = compare_catalogs(&[EN_US], &[ZH_CN]).expect("both catalogs parse");

    assert!(
        diff.is_clean(),
        "the catalogs differ: missing {:?}, extra {:?}, placeholders {:?}",
        diff.missing_ids,
        diff.extra_ids,
        diff.placeholder_mismatches
    );
}

#[test]
fn the_list_the_code_formats_from_is_exactly_what_the_catalogs_define() {
    let mut defined: Vec<String> = message_ids(&[EN_US])
        .expect("the default catalog parses")
        .into_iter()
        .collect();
    defined.sort();

    let mut listed: Vec<String> = i18n::MESSAGES.iter().map(|id| (*id).to_owned()).collect();
    listed.sort();

    assert_eq!(
        listed, defined,
        "every id the component formats must exist, and no catalog entry may be dead"
    );
}

#[test]
fn both_locales_load_and_name_the_window() {
    for tag in ["en-US", "zh-CN", "de-DE", "zh-Hans"] {
        let locale = Locale::parse(tag).expect("a well-formed tag");
        let localizer = i18n::localizer(locale).expect("the shipped catalogs");
        let title = localizer.message("wallpaper-window-title");

        assert!(!title.is_empty());
        assert_ne!(title, "wallpaper-window-title", "`{tag}` renders a title");
    }
}

#[test]
fn a_missing_translation_falls_back_to_the_default_language() {
    // `zh-CN` translates everything today; the fallback is what protects a
    // catalog that lags behind, so it is exercised with an id only the default
    // catalog could have — by asking the localizer for one of its own messages
    // in a locale that has no catalog at all.
    let localizer = i18n::localizer(Locale::parse("fr-FR").expect("a well-formed tag"))
        .expect("the shipped catalogs");

    assert_eq!(localizer.locale(), &Locale::default_locale());
    assert_eq!(
        localizer.message("wallpaper-window-title"),
        "Inox wallpaper"
    );
}

#[test]
fn the_chinese_catalog_is_actually_chinese() {
    let localizer = i18n::localizer(Locale::parse(CHINESE_TAG).expect("a well-formed tag"))
        .expect("the shipped catalogs");
    let title = localizer.message("wallpaper-window-title");

    assert!(
        !title.is_ascii(),
        "`{title}` should not be the English string"
    );
}

#[test]
fn a_message_with_placeholders_names_them_in_both_locales() {
    let english = i18n::localizer(Locale::default_locale()).expect("a localizer");
    let chinese = i18n::localizer(Locale::parse(CHINESE_TAG).expect("a tag")).expect("a localizer");

    for id in [
        "wallpaper-output-heading",
        "wallpaper-output-image",
        "wallpaper-reason-too-large",
        "wallpaper-reason-too-many-tiles",
    ] {
        let placeholders: Vec<(String, String)> = match id {
            "wallpaper-output-heading" => vec![("output".to_owned(), "HDMI-A-1".to_owned())],
            "wallpaper-output-image" => vec![
                ("width".to_owned(), "640".to_owned()),
                ("height".to_owned(), "480".to_owned()),
                ("path".to_owned(), "/w.png".to_owned()),
            ],
            "wallpaper-reason-too-large" => vec![
                ("pixels".to_owned(), "400000000".to_owned()),
                ("limit".to_owned(), "33177600".to_owned()),
            ],
            _ => vec![
                ("width".to_owned(), "1".to_owned()),
                ("height".to_owned(), "1".to_owned()),
                ("tiles".to_owned(), "921600".to_owned()),
                ("limit".to_owned(), "1024".to_owned()),
            ],
        };
        let args: Vec<(&str, inox_common::i18n::Arg)> = placeholders
            .iter()
            .map(|(name, value)| (name.as_str(), inox_common::i18n::Arg::from(value.as_str())))
            .collect();

        for localizer in [&english, &chinese] {
            let rendered = localizer.message_with(id, &args);

            assert!(
                !rendered.contains('$'),
                "`{id}` rendered `{rendered}` with a placeholder left in it"
            );
            for (_, value) in &placeholders {
                assert!(
                    rendered.contains(value),
                    "`{id}` rendered `{rendered}` without `{value}`"
                );
            }
        }
    }
}
