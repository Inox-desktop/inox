//! Everything the settings window proves about itself, inside the binary.
//!
//! These modules are `#[cfg(test)]` children of the crate root for the same
//! reason the crate is bin-only: cxx-qt links the bridge's generated C++ archive
//! into every target of the package, and an integration test in `tests/` is a
//! separate target that would fail to link (`undefined reference to
//! 'cxx_qt_init_*'`, KDAB/cxx-qt #770). `cargo test -p inox-settings` runs them
//! as the binary's test harness.
//!
//! | module | evidence |
//! |---|---|
//! | [`support`] | the window over a real configuration directory |
//! | [`workflow`] | the D04 user journeys: apply, cancel, conflict, restart, reload, display trials |
//! | [`complete_settings`] | every editable field, its limits and its TOML round trip |
//! | [`catalog`] | `en-US` and `zh-CN` in step, and every id the code names defined (Q03) |
//! | [`projection`] | what the window draws, as values: tokens, sentences, states, focus (Q02, D04, D05, Q01) |
//! | [`qml`] | the QML module loading offscreen and reporting that it read the projection |
//!
//! The rendered-pixel evidence the iced version carried in `tests/render.rs` has
//! no counterpart after the migration — Qt draws the window, not this crate — so
//! its assertions were rewritten as projection assertions (the same fact, read
//! without a display) plus the offscreen load test, which is the part only a real
//! Qt can answer.

mod catalog;
mod complete_settings;
mod projection;
mod qml;
mod support;
mod workflow;

use crate::apply::Component;
use crate::pages::PageId;
use crate::view_model::{Action, TrialChoice};

/// Every message id the code asks for, from every module that names one.
///
/// The page and field ids come from the model that needs them, so a field added
/// to [`crate::draft::Field`] brings its labels with it and [`catalog`] fails
/// until both catalogs define them.
///
/// Kept here rather than in `i18n` because it is a test's checklist, not
/// something the window asks for at runtime — and because `i18n` is compiled into
/// the control binary, which has no use for the window's field registry.
pub fn code_message_ids() -> Vec<&'static str> {
    let mut ids = crate::view_model::message_ids();
    ids.extend([
        "settings-ctl-help",
        "settings-ctl-applied",
        "settings-ctl-absent",
        "settings-ctl-failed",
        "settings-ctl-incomplete",
    ]);

    for page in PageId::ALL {
        ids.extend(page.message_ids());
    }

    for component in Component::ALL {
        ids.push(component.label_id());
    }

    for action in [
        Action::Apply,
        Action::Cancel,
        Action::ResetAll,
        Action::Reload,
    ] {
        ids.push(action.label_id());
    }

    for choice in TrialChoice::ALL {
        ids.push(choice.label_id());
    }

    ids
}
