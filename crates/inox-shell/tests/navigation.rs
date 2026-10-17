//! The navigation rules as D01 states them.
//!
//! The state machine is unit-tested next to its implementation; this file is
//! the acceptance scenario written the way the criterion reads, so that the
//! behaviour a reviewer checks against ADR 0013 is visible in one place.

use inox_shell::navigation::{
    Entry, EntryFocus, EntryId, FallbackReason, NavigationOutcome, NavigationStack, ReturnTarget,
    SessionGeneration, WindowId,
};
use inox_shell::{Diagnostic, i18n};

const GENERATION: SessionGeneration = SessionGeneration::new(1);

fn window(value: u32) -> WindowId {
    WindowId::new(value)
}

fn target(value: u32) -> ReturnTarget {
    ReturnTarget::new(window(value), GENERATION)
}

/// D01: opening the launcher from a focused window, and leaving with `Esc`.
#[test]
fn esc_returns_focus_to_the_window_the_entry_was_opened_over() {
    let mut stack = NavigationStack::new(GENERATION);
    let editor = target(3);

    // The launcher is opened from the editor, and takes the keyboard.
    assert_eq!(
        stack.open(
            Entry::new(EntryId::Launcher, EntryFocus::Keyboard),
            Some(editor)
        ),
        NavigationOutcome::Opened {
            entry: EntryId::Launcher,
            focus: Some(EntryId::Launcher),
        }
    );

    // `Esc` closes exactly one layer and says where focus goes back to.
    assert_eq!(
        stack.close_top(),
        NavigationOutcome::Closed {
            returned_to: Some(editor)
        }
    );

    // A second `Esc` has nothing to close, and changes nothing.
    assert_eq!(stack.close_top(), NavigationOutcome::NothingToClose);
    assert_eq!(stack.depth(), 0);
}

/// D01: an ordinary panel display must not steal focus from the application.
#[test]
fn showing_an_entry_does_not_steal_the_keyboard() {
    let mut stack = NavigationStack::new(GENERATION);

    let outcome = stack.open(
        Entry::new(EntryId::Panel, EntryFocus::Display),
        Some(target(3)),
    );

    assert_eq!(
        outcome,
        NavigationOutcome::Opened {
            entry: EntryId::Panel,
            focus: None,
        }
    );
    assert_eq!(stack.focus(), None, "the focused window keeps the keyboard");
}

/// D01: a click displays the panel without taking focus, and the keyboard
/// shortcut then navigates into that same panel — one layer, exactly one focus.
#[test]
fn a_keyboard_shortcut_navigates_into_an_entry_that_is_already_displayed() {
    let mut stack = NavigationStack::new(GENERATION);
    let editor = target(3);

    // The click displays the panel; the editor keeps the keyboard.
    assert_eq!(
        stack.open(
            Entry::new(EntryId::Panel, EntryFocus::Display),
            Some(editor)
        ),
        NavigationOutcome::Opened {
            entry: EntryId::Panel,
            focus: None,
        }
    );

    // The shortcut navigates into the same panel: still one layer, and the
    // keyboard is now with the panel and nowhere else.
    assert_eq!(
        stack.open(
            Entry::new(EntryId::Panel, EntryFocus::Keyboard),
            Some(editor)
        ),
        NavigationOutcome::Opened {
            entry: EntryId::Panel,
            focus: Some(EntryId::Panel),
        }
    );
    assert_eq!(stack.depth(), 1, "one layer per entry");
    assert_eq!(stack.focus(), Some(EntryId::Panel));

    // `Esc` closes the panel and returns focus to the editor it was opened
    // over, which is the target the click saved.
    assert_eq!(
        stack.close_top(),
        NavigationOutcome::Closed {
            returned_to: Some(editor)
        }
    );
    assert_eq!(stack.focus(), None);
}

/// ADR 0013: a window that died while its layer was open is never focused again.
#[test]
fn a_destroyed_window_is_never_focused_again() {
    let mut stack = NavigationStack::new(GENERATION);
    stack.open(
        Entry::new(EntryId::Launcher, EntryFocus::Keyboard),
        Some(target(3)),
    );

    assert_eq!(stack.target_destroyed(window(3)), 1);

    assert_eq!(
        stack.close_top(),
        NavigationOutcome::ReturnedToFallback {
            reason: FallbackReason::WindowDestroyed { window: window(3) }
        }
    );
}

/// ADR 0013 decision 1: a session that was rebuilt invalidates the windows the
/// layers recorded, even though their ids may be reused.
#[test]
fn a_target_from_a_previous_session_is_not_reused() {
    let mut stack = NavigationStack::new(GENERATION);
    stack.open(
        Entry::new(EntryId::Launcher, EntryFocus::Keyboard),
        Some(target(3)),
    );

    let rebuilt = SessionGeneration::new(2);
    assert_eq!(stack.set_generation(rebuilt), 1);

    assert_eq!(
        stack.close_top(),
        NavigationOutcome::ReturnedToFallback {
            reason: FallbackReason::SessionChanged {
                saved: GENERATION,
                current: rebuilt,
            }
        }
    );
}

/// Pressing the launcher's shortcut twice leaves one launcher, not two.
#[test]
fn opening_the_same_entry_twice_leaves_one_layer() {
    let mut stack = NavigationStack::new(GENERATION);
    stack.open(
        Entry::new(EntryId::Launcher, EntryFocus::Keyboard),
        Some(target(3)),
    );

    assert_eq!(
        stack.open(
            Entry::new(EntryId::Launcher, EntryFocus::Keyboard),
            Some(target(3))
        ),
        NavigationOutcome::AlreadyOpen {
            entry: EntryId::Launcher
        }
    );
    assert_eq!(stack.depth(), 1);
    assert_eq!(stack.focus(), Some(EntryId::Launcher));
}

/// Every outcome the coordinator can log is a real message in both catalogs
/// (Q03: no user-visible string is assembled in code).
#[test]
fn the_coordinator_can_explain_every_outcome_it_reports() {
    let localizer = i18n::localizer(inox_common::i18n::Locale::default_locale())
        .expect("the shipped catalogs parse");
    let outcomes = [
        NavigationOutcome::NothingToClose,
        NavigationOutcome::Closed {
            returned_to: Some(target(3)),
        },
        NavigationOutcome::ReturnedToFallback {
            reason: FallbackReason::WindowDestroyed { window: window(3) },
        },
    ];

    for outcome in &outcomes {
        let diagnostic: &Diagnostic = &outcome.describe();
        let rendered = diagnostic.render(&localizer);

        assert_ne!(rendered, diagnostic.id());
        assert!(!rendered.contains('{'));
    }
}
