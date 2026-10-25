//! cxx-qt bridge: the only place QML and Rust meet.
//!
//! `SettingsApp` is a Rust-backed `QObject` the QML module instantiates. It holds
//! exactly one thing the tree renders — the JSON projection — plus the two
//! directions the window needs:
//!
//! - **Rust → QML**: `modelJson`, re-read whenever the tree pumps and the
//!   projection changed. Everything the tree draws (colours, sizes, sentences,
//!   the status line, the trial dialog) comes out of that one string, so the
//!   boundary never turns into a second UI definition. This is the trade
//!   `docs/qml-migration-contract.md` §3 records: one model, resent whole, with
//!   the incremental version deferred until there is data that asks for it.
//! - **QML → Rust**: `dispatch`, one JSON action of the closed set in `view`'s
//!   `UiAction` (spelled without an intra-doc link: this file is also compiled
//!   into `inoxctl`, which has no `view` module). A malformed action is logged
//!   and dropped; it can never panic a window.
//!
//! The bridge knows nothing about the settings domain: it asks
//! [`crate::ui_host`] for the projection and hands actions back to it. That is
//! what lets the package's second binary (`inoxctl`) compile this module to
//! resolve the cxx-qt initializers without compiling the window — see
//! `AGENTS.md`, "the second binary".
//!
//! Note: doc comments are not placed on items inside the `#[cxx_qt::bridge]`
//! blocks below; the cxx parser rejects them there. Documentation lives on the
//! Rust types and on the plain-Rust `impl` block outside the macro.

use core::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt_lib::QString;
use tracing::{debug, warn};

use crate::ui_host::{self, Applied};

/// An empty projection, for the case where no window is installed.
///
/// Only the control binary and a directly-constructed unit test can see this:
/// the window installs its runtime before the QML engine loads, so the tree is
/// never handed an empty model in a real session.
const EMPTY_MODEL: &str = "{}";

/// Set once QML has loaded the module and reported that its tree completed.
pub static QML_READY: AtomicBool = AtomicBool::new(false);

/// Whether QML reported a completed tree.
///
/// This is the headless check's evidence that the Rust-backed element was really
/// instantiated by the engine — not merely that a URL was handed to `load`.
pub fn qml_ready() -> bool {
    QML_READY.load(Ordering::SeqCst)
}

/// The Rust backing struct of the QML `SettingsApp` element.
///
/// Every field named in a `#[qproperty]` below is a property QML can read; the
/// rest is bookkeeping.
#[derive(Debug)]
pub struct SettingsAppRust {
    /// The projection, as JSON: everything the tree draws.
    pub model_json: QString,
    /// Whether the tree reported that it had loaded and read the projection.
    pub ready: bool,
}

impl Default for SettingsAppRust {
    /// The element as the QML engine constructs it.
    ///
    /// The window installs its runtime before the engine loads, so the first
    /// projection is already available here; with no runtime installed (the
    /// control binary, a direct unit test) the element answers with an empty
    /// model instead of panicking in a constructor.
    fn default() -> Self {
        Self {
            model_json: QString::from(&Self::projection()),
            ready: false,
        }
    }
}

impl SettingsAppRust {
    /// The projection the installed window carries, as JSON.
    fn projection() -> String {
        ui_host::with(|host| host.model_json()).unwrap_or_else(|| EMPTY_MODEL.to_owned())
    }
}

/// The QML element and its invokables.
#[cxx_qt::bridge]
pub mod qobject {
    // `type QString` below is a cxx alias with no Rust-side documentation, and
    // the cxx parser rejects `///` comments inside bridge items, so allow
    // `missing_docs` for the bridge as a whole.
    #[allow(missing_docs)]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    // `auto_cxx_name` makes snake_case Rust items visible to QML in camelCase
    // (`model_json` becomes `modelJson`); it is only allowed on the block.
    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, model_json)]
        #[qproperty(bool, ready)]
        type SettingsApp = super::SettingsAppRust;

        #[qinvokable]
        fn pump(self: Pin<&mut Self>);

        #[qinvokable]
        fn dispatch(self: Pin<&mut Self>, action: QString);

        #[qinvokable]
        fn report_ready(self: Pin<&mut Self>, title: QString, pages: i32, accessible_items: i32);

        #[qinvokable]
        fn quit(self: Pin<&mut Self>);
    }
}

/// Behaviour of the `SettingsApp` QObject.
impl qobject::SettingsApp {
    /// Drains work that finished off the window's thread, and re-reads the
    /// projection when there is a reason to.
    ///
    /// The QML tree calls this from its own timer (16 ms), which is what makes
    /// a save that settled on a worker thread reach the status line.
    pub fn pump(self: Pin<&mut Self>) {
        let changed = ui_host::with(|host| host.pump()).unwrap_or(false);

        if changed {
            self.refresh();
        }
    }

    /// Applies one JSON action the tree sent.
    pub fn dispatch(mut self: Pin<&mut Self>, action: QString) {
        let action = String::from(&action);

        match ui_host::with(|host| host.apply(&action)) {
            Some(Applied::Changed) => self.as_mut().refresh(),
            Some(Applied::Unsupported) => {}
            Some(Applied::Unknown) => {
                warn!("the QML tree sent an action the window does not know");
            }
            None => {
                debug!("no window is installed; the action is dropped");
            }
        }
    }

    /// Confirms from QML that the tree completed, and that what it read matches
    /// the projection it was given.
    ///
    /// The readiness flag is only set when the title, the number of navigation
    /// entries and the size of the semantic tree all agree, so a headless load
    /// that produced an empty or partial window fails instead of passing on
    /// "a URL was loaded" (root `AGENTS.md` §7.4).
    pub fn report_ready(
        mut self: Pin<&mut Self>,
        title: QString,
        pages: i32,
        accessible_items: i32,
    ) {
        let Some(expected) = ui_host::with(|host| host.readiness()) else {
            warn!("QML reported ready, but no window is installed");
            return;
        };

        if String::from(&title) != expected.title {
            warn!(
                reported = %String::from(&title),
                expected = %expected.title,
                "the QML tree reported a title that does not match the projection"
            );
            return;
        }

        if pages != expected.pages {
            warn!(
                reported = pages,
                expected = expected.pages,
                "the QML tree drew a different number of pages than the projection has"
            );
            return;
        }

        if accessible_items < expected.accessible_items {
            warn!(
                reported = accessible_items,
                expected = expected.accessible_items,
                "the QML semantic tree is smaller than the projection requires"
            );
            return;
        }

        self.as_mut().set_ready(true);
        QML_READY.store(true, Ordering::SeqCst);
        debug!(pages, accessible_items, "the QML tree reported ready");
    }

    /// The window asked to close.
    pub fn quit(self: Pin<&mut Self>) {
        ui_host::with(|host| host.quit());
    }

    /// Re-reads the projection and publishes it.
    ///
    /// The whole model is resent: it is the trade this migration accepts, and it
    /// keeps the boundary from growing a second description of the window.
    fn refresh(mut self: Pin<&mut Self>) {
        let model = SettingsAppRust::projection();
        // The getter borrows the property, so the value is cloned before the
        // property is written again.
        let current = String::from(self.as_ref().model_json());

        if model == current {
            return;
        }

        self.as_mut().set_model_json(QString::from(&model));
    }
}
