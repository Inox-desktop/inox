//! The boundary's catalogs are in step, and the code only asks for messages that
//! exist.
//!
//! The same three checks `inox-greet/tests/catalog.rs` runs (root AGENTS.md §7.3):
//! the two locales define the same ids with the same placeholders, every `auth-…`
//! id the code mentions is defined, and every id the catalogs define is mentioned
//! by the code. A message that renders as its own identifier in front of an
//! operator who cannot start the display manager is exactly as bad as one in front
//! of someone trying to log in.

use std::collections::BTreeSet;

use inox_auth::i18n::{CATALOGS, EN_US, ZH_CN};
use inox_common::i18n::{DEFAULT_LOCALE, compare_catalogs, message_ids};

/// The id prefixes this crate owns.
const PREFIXES: [&str; 1] = ["auth-"];

/// The id literals that are *not* messages.
///
/// `auth.sock` and `auth-v1` are names of things, not of sentences, and both are
/// referenced from the sources in ways a prefix scan would otherwise read as a
/// message nobody defines. Listing them here makes that an explicit decision rather
/// than a hole in the pattern.
const NOT_MESSAGES: [&str; 0] = [];

#[test]
fn the_two_catalogs_define_the_same_messages() {
    let diff = compare_catalogs(EN_US, ZH_CN).expect("both catalogs parse");

    assert!(
        diff.is_clean(),
        "en-US and zh-CN disagree: missing {:?}, extra {:?}, placeholders {:?}",
        diff.missing_ids,
        diff.extra_ids,
        diff.placeholder_mismatches,
    );
}

#[test]
fn the_default_locale_ships_a_catalog() {
    assert!(
        CATALOGS
            .iter()
            .any(|(tag, _)| *tag == DEFAULT_LOCALE && !tag.is_empty()),
        "the fallback locale must have a catalog",
    );
}

#[test]
fn every_message_the_code_asks_for_is_defined() {
    let missing: Vec<String> = referenced_ids()
        .difference(&defined_ids())
        .cloned()
        .collect();

    assert!(
        missing.is_empty(),
        "these ids are used in the code but missing from the catalogs: {missing:?}",
    );
}

#[test]
fn every_message_the_catalogs_define_is_used() {
    let unused: Vec<String> = defined_ids()
        .difference(&referenced_ids())
        .cloned()
        .collect();

    assert!(
        unused.is_empty(),
        "these catalog entries are shown by nothing: {unused:?}",
    );
}

/// The ids the catalogs define.
fn defined_ids() -> BTreeSet<String> {
    let mut ids = message_ids(EN_US).expect("the default catalog parses");
    ids.retain(|id| owns(id));

    ids
}

/// The ids the crate's own sources mention.
fn referenced_ids() -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    let sources = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
        .expect("the crate has a src directory");

    for entry in sources {
        let path = entry.expect("a readable directory entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("a readable source file");
        for literal in string_literals(&source) {
            if !owns(&literal) || PREFIXES.contains(&literal.as_str()) {
                continue;
            }
            if NOT_MESSAGES.contains(&literal.as_str()) {
                continue;
            }
            let id = literal.split('.').next().unwrap_or(&literal).to_owned();
            ids.insert(id);
        }
    }

    ids
}

/// Whether an id belongs to this component.
fn owns(id: &str) -> bool {
    PREFIXES.iter().any(|prefix| id.starts_with(prefix))
}

/// The contents of every double-quoted string literal in `source`.
fn string_literals(source: &str) -> Vec<String> {
    let mut literals = Vec::new();
    let mut current: Option<String> = None;
    let mut escaped = false;

    for character in source.chars() {
        match (&mut current, character) {
            (None, '"') => current = Some(String::new()),
            (Some(literal), '"') if !escaped => {
                literals.push(std::mem::take(literal));
                current = None;
            }
            (Some(literal), other) => {
                if escaped {
                    literal.push('\\');
                }
                literal.push(other);
            }
            (None, _) => {}
        }
        escaped = character == '\\' && !escaped;
    }

    literals
}

#[test]
fn the_source_scanner_finds_what_it_is_looking_for() {
    let found = string_literals(r#"let a = "auth-usage"; let b = "other";"#);
    assert_eq!(found, ["auth-usage", "other"]);
}

#[test]
fn the_component_owns_exactly_one_prefix() {
    assert_eq!(PREFIXES, ["auth-"]);

    for id in defined_ids() {
        assert!(
            !id.contains('.'),
            "`{id}` looks like an attribute path, not a message id"
        );
    }
}
