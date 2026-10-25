//! The catalogs are in step, and the code only asks for messages that exist
//! (acceptance Q03).
//!
//! Root AGENTS.md §7.3 makes Project Fluent the only way user-visible text reaches a
//! person. For the greeter host that person is whoever is reading the journal after
//! their machine failed to reach a login screen, so a message id rendered instead of a
//! sentence is exactly the moment the stage cannot explain itself. This test checks
//! the three things a code review does not:
//!
//! - the two catalogs define the same ids with the same placeholders, so a translation
//!   cannot silently lose a variable;
//! - every `host-…` id the *code* mentions is defined, so a typo in a call site is a
//!   test failure rather than an identifier in the journal;
//! - every id the catalogs define is mentioned by the code, so a message nobody shows
//!   is deleted rather than translated forever.
//!
//! It is the same shape as `inox-greet/tests/catalog.rs`: the L3 component architecture accepts a catalog
//! copy per component rather than a shared same-layer crate.

use std::collections::BTreeSet;

use inox_common::i18n::{DEFAULT_LOCALE, compare_catalogs, message_ids};
use inox_host::i18n::{CATALOGS, EN_US, ZH_CN};

/// The id prefixes this component owns.
const PREFIXES: [&str; 1] = ["host-"];

/// The id literals that are *not* messages.
///
/// `host-…` is otherwise a comfortable name for a test's expectation, and one in the
/// sources would read as a message nobody defines. Empty today, and kept so that the
/// exemption is a decision someone makes in this file rather than a hole in the
/// pattern.
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

#[test]
fn the_component_owns_exactly_one_prefix() {
    assert_eq!(PREFIXES, ["host-"]);

    for id in defined_ids() {
        assert!(
            !id.contains('.'),
            "`{id}` looks like an attribute path, not a message id"
        );
    }
}

/// The ids the catalogs define.
fn defined_ids() -> BTreeSet<String> {
    let mut ids = message_ids(EN_US).expect("the default catalog parses");
    ids.retain(|id| owns(id));

    ids
}

/// The ids the crate's own sources mention.
///
/// A source scan rather than a list to maintain: the point is to catch the call site
/// that says `host-socket-timout`, and only reading the sources does that.
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
            // A bare prefix is not a message: the test above asserts that an id starts
            // with `host-`, and that literal is a prefix, not an id.
            if !owns(&literal) || PREFIXES.contains(&literal.as_str()) {
                continue;
            }
            if NOT_MESSAGES.contains(&literal.as_str()) {
                continue;
            }
            // An id may carry a `.attribute` suffix; the catalog defines the message.
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
///
/// A deliberately small scanner — enough for this crate's own source, which has no
/// raw strings containing `host-` — rather than a regular expression the test would
/// then need a dependency for.
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
                    // The escape is kept: `\n` inside a literal is not part of an id,
                    // and no id contains a backslash.
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
    let found = string_literals(r#"let a = "host-usage"; let b = "other";"#);
    assert_eq!(found, ["host-usage", "other"]);

    let found = string_literals("let a = 'c'; let b = \"host-stopping\";");
    assert_eq!(found, ["host-stopping"]);
}
