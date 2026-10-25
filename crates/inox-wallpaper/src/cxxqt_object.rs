//! cxx-qt bridge: the only place QML and Rust meet.
//!
//! `WallpaperPreview` is a Rust-backed `QObject` the QML module instantiates. It
//! exposes exactly one read-only projection — `modelJson`, the document
//! `docs/qml-migration-contract.md` §3 asks for — and three invokables:
//!
//! - `pump()` drains the queues ([`crate::runtime::Runtime::pump`]); QML's timer
//!   calls it, which is how finished decodes and configuration reloads reach the
//!   surface;
//! - `dispatch(action)` carries one JSON action back into the state machine
//!   (`{"type":"reload"}`), so QML never has to know what an action *is*;
//! - `reportReady(title, accessibleItems, imageReady)` is the tree confirming,
//!   once, that it was built against the projection — the evidence the headless
//!   self-check waits for.
//!
//! There is deliberately no property per field: a large view model bridged field
//! by field would become a second UI definition (contract §3).
//!
//! Note: doc comments are not placed on items inside the `#[cxx_qt::bridge]`
//! block below; the cxx parser rejects them there. Documentation lives on the
//! Rust type and on the plain-Rust `impl` block outside the macro.

use core::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt_lib::QString;
use tracing::warn;

use crate::runtime::{self, Runtime};

/// Set once the QML tree has loaded and reported a state that agrees with the
/// projection.
pub static QML_READY: AtomicBool = AtomicBool::new(false);

/// Whether QML reported a tree that matches the projection.
///
/// This is the self-check's evidence that the Rust-backed element was really
/// instantiated by the engine and that both directions of the bridge work — not
/// merely that a URL was handed to `load`.
pub fn qml_ready() -> bool {
    QML_READY.load(Ordering::SeqCst)
}

/// The Rust backing struct of the QML `WallpaperPreview` element.
#[derive(Debug)]
pub struct WallpaperPreviewRust {
    /// The read-only projection QML parses (contract §3).
    pub model_json: QString,
    runtime: Option<Runtime>,
}

impl WallpaperPreviewRust {
    /// The element the QML engine constructs.
    ///
    /// The engine creates it while loading the module, which happens after the
    /// binary installed its runtime on the same thread; taking that runtime is
    /// the only way a `Default`-constructed `QObject` can be given state the
    /// binary already built. When there is none (a test that instantiates the
    /// backing struct, or a module loaded without the binary) the element is
    /// total and renders an empty document instead of panicking in a
    /// constructor.
    fn from_installed() -> Self {
        match runtime::take() {
            Some(runtime) => Self {
                model_json: QString::from(runtime.model_json()),
                runtime: Some(runtime),
            },
            None => {
                warn!("no wallpaper runtime was installed; the preview has no state");

                Self {
                    model_json: QString::from("{}"),
                    runtime: None,
                }
            }
        }
    }
}

impl Default for WallpaperPreviewRust {
    fn default() -> Self {
        Self::from_installed()
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
    // (`report_ready` becomes `reportReady`); it is only allowed on the block.
    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, model_json)]
        type WallpaperPreview = super::WallpaperPreviewRust;

        #[qinvokable]
        fn pump(self: Pin<&mut Self>);

        #[qinvokable]
        fn dispatch(self: Pin<&mut Self>, action: QString);

        #[qinvokable]
        fn report_ready(
            self: Pin<&mut Self>,
            title: QString,
            accessible_items: i32,
            image_ready: bool,
        ) -> bool;
    }
}

/// Behaviour of the `WallpaperPreview` QObject.
impl qobject::WallpaperPreview {
    /// Drain the queues and republish the projection when anything changed.
    pub fn pump(mut self: Pin<&mut Self>) {
        use cxx_qt::CxxQtType;

        let mut projected = None;
        {
            let mut this = self.as_mut().rust_mut();
            if let Some(runtime) = this.runtime.as_mut()
                && runtime.pump()
            {
                projected = Some(QString::from(runtime.model_json()));
            }
        }

        if let Some(json) = projected {
            self.as_mut().set_model_json(json);
        }
    }

    /// Apply one JSON action reported by a control.
    pub fn dispatch(mut self: Pin<&mut Self>, action: QString) {
        use cxx_qt::CxxQtType;

        let action = String::from(&action);
        let mut projected = None;
        {
            let mut this = self.as_mut().rust_mut();
            match this.runtime.as_mut() {
                Some(runtime) => {
                    if runtime.dispatch(&action) {
                        projected = Some(QString::from(runtime.model_json()));
                    }
                }
                None => warn!("the preview has no state machine to dispatch to"),
            }
        }

        if let Some(json) = projected {
            self.as_mut().set_model_json(json);
        }
    }

    /// Accept the QML tree's readiness report, once it agrees with the
    /// projection.
    ///
    /// Returns whether the report was accepted, so the tree can stop asking. A
    /// rejected report is not an error in the tree: while the first frame is
    /// being decoded it is simply too early.
    pub fn report_ready(
        mut self: Pin<&mut Self>,
        title: QString,
        accessible_items: i32,
        image_ready: bool,
    ) -> bool {
        use cxx_qt::CxxQtType;

        let title = String::from(&title);
        let accepted = {
            let mut this = self.as_mut().rust_mut();
            match this.runtime.as_mut() {
                Some(runtime) => runtime.verify_report(&title, accessible_items, image_ready),
                None => false,
            }
        };

        if accepted {
            QML_READY.store(true, Ordering::SeqCst);
        }

        accepted
    }
}
