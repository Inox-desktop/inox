//! The two shipped catalogs must say the same things (acceptance Q03).
//!
//! Three checks, in increasing strength:
//!
//! 1. `compare_catalogs` — the translated catalog defines the same ids with the
//!    same placeholders as the reference one, which is the structural part a
//!    build can verify for every message at once.
//! 2. the reference catalog defines exactly the ids the crate declares in
//!    `i18n::id::ALL`, so neither a message used in code and missing from the
//!    catalog nor a message nothing uses can survive.
//! 3. every id renders to something other than itself in both locales, which
//!    catches a message that exists but resolves to nothing.

use inox_common::i18n::{Locale, LocaleError, Localizer, compare_catalogs, message_ids};
use inox_shell::contract::{CommandResult, ServiceState};
use inox_shell::diagnostic::Diagnostic;
use inox_shell::i18n::{EN_US, ZH_CN, ZH_CN_TAG, id, localizer};
use inox_shell::navigation::{FallbackReason, NavigationOutcome};
use inox_shell::{EntryId, ReturnTarget, SessionGeneration, WindowId};

fn locale(tag: &str) -> Locale {
    Locale::parse(tag).expect("a well-formed language tag")
}

fn shipped(tag: &str) -> Localizer {
    localizer(locale(tag)).expect("the shipped catalogs parse")
}

/// A reason that is itself a diagnostic with a placeholder, so that nesting is
/// exercised rather than assumed.
fn nested_reason() -> Diagnostic {
    Diagnostic::new(id::REGISTRY_UNKNOWN_ENTRY)
        .with("name", "tray")
        .with("known", EntryId::names())
}

fn every_service_state() -> Vec<ServiceState> {
    vec![
        ServiceState::Starting,
        ServiceState::Ready,
        ServiceState::Unavailable {
            actionable: nested_reason(),
        },
        ServiceState::Failed {
            reason: nested_reason(),
        },
    ]
}

fn every_command_result() -> Vec<CommandResult> {
    vec![
        CommandResult::Sent,
        CommandResult::Accepted,
        CommandResult::Applied,
        CommandResult::Rejected {
            reason: nested_reason(),
        },
        CommandResult::TimedOut,
    ]
}

fn every_outcome() -> Vec<NavigationOutcome> {
    vec![
        NavigationOutcome::Opened {
            entry: EntryId::Launcher,
            focus: Some(EntryId::Launcher),
        },
        NavigationOutcome::Opened {
            entry: EntryId::Panel,
            focus: None,
        },
        NavigationOutcome::AlreadyOpen {
            entry: EntryId::Launcher,
        },
        NavigationOutcome::Closed {
            returned_to: Some(ReturnTarget::new(
                WindowId::new(7),
                SessionGeneration::new(1),
            )),
        },
        NavigationOutcome::Closed { returned_to: None },
        NavigationOutcome::ReturnedToFallback {
            reason: FallbackReason::WindowDestroyed {
                window: WindowId::new(7),
            },
        },
        NavigationOutcome::ReturnedToFallback {
            reason: FallbackReason::SessionChanged {
                saved: SessionGeneration::new(1),
                current: SessionGeneration::new(2),
            },
        },
        NavigationOutcome::NothingToClose,
    ]
}

#[test]
fn both_catalogs_define_the_same_messages_with_the_same_placeholders() {
    let diff = compare_catalogs(&[EN_US], &[ZH_CN]).expect("both catalogs parse");

    assert!(diff.is_clean(), "{diff:?}");
}

#[test]
fn the_catalogs_define_exactly_the_ids_the_crate_declares() {
    let declared: Vec<String> = id::ALL.iter().map(|id| (*id).to_owned()).collect();

    for catalog in [EN_US, ZH_CN] {
        let defined = message_ids(&[catalog]).expect("the catalog parses");
        let missing = declared
            .iter()
            .filter(|id| !defined.contains(*id))
            .collect::<Vec<_>>();
        let extra = defined
            .iter()
            .filter(|id| !declared.contains(id))
            .collect::<Vec<_>>();

        assert!(missing.is_empty(), "used but not defined: {missing:?}");
        assert!(extra.is_empty(), "defined but unused: {extra:?}");
    }
}

#[test]
fn the_ids_the_crate_declares_are_unique() {
    let mut sorted = id::ALL.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    assert_eq!(sorted.len(), id::ALL.len(), "an id is declared twice");
}

#[test]
fn every_declared_id_renders_in_both_locales() {
    for tag in ["en-US", ZH_CN_TAG] {
        let localizer = shipped(tag);

        for id in id::ALL {
            let rendered = localizer.message(id);

            assert_ne!(rendered, id, "`{id}` is not translated into {tag}");
            assert!(!rendered.trim().is_empty(), "`{id}` is empty in {tag}");
        }
    }
}

#[test]
fn every_diagnostic_the_crate_produces_renders_in_both_locales() {
    for tag in ["en-US", ZH_CN_TAG] {
        let localizer = shipped(tag);
        let mut diagnostics: Vec<Diagnostic> = Vec::new();

        for state in every_service_state() {
            diagnostics.push(state.describe());
        }
        for result in every_command_result() {
            diagnostics.push(result.describe());
        }
        for outcome in every_outcome() {
            diagnostics.push(outcome.describe());
        }

        assert_eq!(diagnostics.len(), 17);

        for diagnostic in &diagnostics {
            let rendered = diagnostic.render(&localizer);

            assert!(
                id::ALL.contains(&diagnostic.id()),
                "`{}` is not a declared id",
                diagnostic.id()
            );
            assert_ne!(rendered, diagnostic.id(), "`{rendered}` in {tag}");
            assert!(
                !rendered.contains('{') && !rendered.contains('$'),
                "`{}` kept a placeholder in {tag}: {rendered}",
                diagnostic.id()
            );
        }
    }
}

#[test]
fn a_nested_diagnostic_is_translated_too() {
    let chinese = shipped(ZH_CN_TAG);
    let english = shipped("en-US");

    let state = ServiceState::Unavailable {
        actionable: nested_reason(),
    };
    let rendered_chinese = state.describe().render(&chinese);
    let rendered_english = state.describe().render(&english);

    assert!(rendered_chinese.contains("tray"), "{rendered_chinese}");
    assert!(rendered_english.contains("tray"), "{rendered_english}");
    assert_ne!(rendered_chinese, rendered_english);
}

#[test]
fn a_locale_without_a_catalog_falls_back_to_the_default() {
    // German is not a language Inox ships; the localizer still has to render
    // every message rather than refuse to start (PRD §11.2, acceptance Q03).
    let fallback = shipped("de-DE");

    assert_eq!(fallback.locale().tag(), "en-US");
    assert_eq!(
        fallback.message(id::SERVICE_READY),
        shipped("en-US").message(id::SERVICE_READY)
    );
}

#[test]
fn the_translated_locale_is_the_one_asked_for() {
    let chinese = shipped(ZH_CN_TAG);
    let english = shipped("en-US");

    assert_eq!(chinese.locale().tag(), ZH_CN_TAG);
    assert_ne!(
        chinese.message(id::SERVICE_READY),
        english.message(id::SERVICE_READY)
    );
    assert!(locale(ZH_CN_TAG).is_supported());
}

#[test]
fn a_malformed_tag_is_rejected_rather_than_guessed_at() {
    assert!(matches!(
        Locale::parse("not a tag"),
        Err(LocaleError::Malformed { .. })
    ));
}
