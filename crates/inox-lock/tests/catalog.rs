//! The catalogs are in step, and the code only asks for messages that exist
//! (acceptance Q03).
//!
//! Root AGENTS.md §7.3 makes Project Fluent the only way a user-visible string
//! reaches the screen. This is the test that keeps that rule from decaying into a
//! string that renders as its own identifier — on a lock screen, where the person
//! reading it cannot get to a shell to find out what went wrong. It checks three
//! things, none of which a code review reliably catches:
//!
//! - the two catalogs define the same ids with the same placeholders, so a
//!   translation cannot silently lose a variable;
//! - every `lock-…` id the *code* mentions is defined, so a typo in a call site is
//!   a test failure rather than an identifier on screen;
//! - every id the catalogs define is mentioned by the code, so a message nobody
//!   shows is deleted rather than translated forever.

use std::collections::BTreeSet;

use inox_common::i18n::{DEFAULT_LOCALE, compare_catalogs, message_ids};
use inox_lock::i18n::{CATALOGS, EN_US, ZH_CN};

/// The id prefixes this component owns.
const PREFIXES: [&str; 1] = ["lock-"];

/// The id literals that are *not* messages.
///
/// `lock-…` is otherwise a comfortable name for a protocol object or a widget id,
/// and one of those in the sources would otherwise read as a message nobody
/// defines. Empty today, and kept so that the exemption is a decision someone
/// makes in this file rather than a hole in the pattern.
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
///
/// A source scan rather than a list to maintain: the point is to catch the call
/// site that says `lock-disable-protocol`, and only reading the sources does that.
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
            // A bare prefix is not a message: the tests assert that a message id
            // starts with `lock-`, and that literal is a prefix, not an id.
            if !owns(&literal) || PREFIXES.contains(&literal.as_str()) {
                continue;
            }
            if NOT_MESSAGES.contains(&literal.as_str()) {
                continue;
            }
            // An id may carry a `.attribute` suffix; the catalog defines the
            // message, and the attribute is selected when it is formatted.
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
/// raw strings containing `lock-` — rather than a regular expression the test would
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
                    // The escape is kept: `\n` inside a literal is not part of an
                    // id, and no id contains a backslash.
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
    let found = string_literals(r#"let a = "lock-prompt"; let b = "other";"#);
    assert_eq!(found, ["lock-prompt", "other"]);

    let found = string_literals("let a = 'c'; let b = \"lock-window-title\";");
    assert_eq!(found, ["lock-window-title"]);
}

#[test]
fn the_component_owns_exactly_one_prefix() {
    assert_eq!(PREFIXES, ["lock-"]);

    for id in defined_ids() {
        assert!(
            !id.contains('.'),
            "`{id}` looks like an attribute path, not a message id"
        );
    }
}
