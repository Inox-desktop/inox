//! cxx-qt bridge: the only place QML and Rust meet.
//!
//! The QML module instantiates one `Notifications` element. It exposes exactly
//! two read-only properties — `modelJson`, the whole projection computed in
//! [`crate::view`], and nothing else — plus three invokables:
//!
//! - `pump()`: drains what the transports delivered, runs the update loop and
//!   refreshes `modelJson` when something changed. QML drives it from a `Timer`,
//!   which is what replaces `iced::Subscription` (docs/qml-migration-contract.md
//!   §5). It runs on the Qt thread, so background threads still never touch the
//!   interface: they hand [`crate::app::Inbox`] a message and return.
//! - `dispatch(json)`: one action QML reported, as `{"type":…}`. Parsed into
//!   [`crate::runtime::UiAction`], then the daemon's own `Message`; a malformed
//!   action is logged and dropped (never a panic).
//! - `reportReady(title, accessibleItems, popupCards)`: the self-check's evidence
//!   that the engine instantiated this element, that the properties crossed the
//!   bridge intact, and that the semantic tree a screen reader walks is populated
//!   (root AGENTS.md §7.4).
//!
//! The daemon itself — state, transports, projection — lives on this thread
//! behind [`install`]: [`crate::runtime::Runtime`] is not `Send` (it holds the
//! `Rc<Localizer>` the catalogs are read through), so it cannot be a `QObject`
//! field safely reached from anywhere else. QML constructs the element on the Qt
//! thread, and the thread-local is what that constructor reads.
//!
//! Note: doc comments are not placed on items inside the `#[cxx_qt::bridge]`
//! block below; the cxx parser rejects them there. Documentation lives on the
//! Rust types and on the plain-Rust `impl` blocks outside the macro.

use core::pin::Pin;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt_lib::QString;
use serde_json::Value;
use tracing::warn;

use crate::runtime::Runtime;

/// Set once QML reported that its tree matches the projection.
pub static QML_READY: AtomicBool = AtomicBool::new(false);

/// Whether QML confirmed its tree.
///
/// This is the self-check's evidence that the Rust-backed element was really
/// instantiated by the engine — not merely that a URL was handed to `load`.
pub fn qml_ready() -> bool {
    QML_READY.load(Ordering::SeqCst)
}

thread_local! {
    /// The daemon, on the thread that owns the interface.
    static INSTALLED: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Makes `runtime` the daemon the QML element drives.
///
/// Returns `false` when one was already installed: the binary installs exactly
/// one, and a second call is a programming error rather than something to
/// overwrite silently.
pub fn install(runtime: Runtime) -> bool {
    INSTALLED.with(|slot| {
        let mut slot = slot.borrow_mut();

        if slot.is_some() {
            return false;
        }

        *slot = Some(runtime);

        true
    })
}

/// Runs `f` against the installed daemon, if there is one.
fn with_runtime<R>(f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    INSTALLED.with(|slot| slot.borrow_mut().as_mut().map(f))
}

/// The Rust backing struct of the QML `Notifications` element.
#[derive(Debug)]
pub struct NotificationsRust {
    /// The whole interface, as JSON, already resolved from the tokens and the
    /// catalogs. QML parses it and draws it; it never edits it.
    pub model_json: QString,
}

impl NotificationsRust {
    /// The element as the installed daemon projects it.
    ///
    /// Before the daemon is installed (a unit test that drives the backing struct
    /// directly, or an engine that instantiated the element first) this is the
    /// empty projection, which is valid JSON: a surface whose model cannot be
    /// parsed cannot render at all, and a constructor must not panic.
    pub fn projected() -> Self {
        let model_json = with_runtime(|runtime| QString::from(runtime.model_json()))
            .unwrap_or_else(|| QString::from(&crate::view::fallback_json()));

        Self { model_json }
    }
}

impl Default for NotificationsRust {
    fn default() -> Self {
        Self::projected()
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
        type Notifications = super::NotificationsRust;

        #[qinvokable]
        fn pump(self: Pin<&mut Self>);

        #[qinvokable]
        fn dispatch(self: Pin<&mut Self>, action: &QString);

        #[qinvokable]
        fn report_ready(
            self: Pin<&mut Self>,
            title: QString,
            accessible_items: i32,
            popup_cards: i32,
        );
    }
}

/// Behaviour of the `Notifications` QObject.
impl qobject::Notifications {
    /// Drains the transports, runs the update loop, and refreshes the projection.
    ///
    /// Called from QML's timer at the frame interval. The single property is
    /// only reassigned when the projection actually differs, so an idle daemon
    /// does not make Qt re-parse JSON on every tick.
    pub fn pump(mut self: Pin<&mut Self>) {
        let changed =
            with_runtime(|runtime| runtime.pump().then(|| QString::from(runtime.model_json())))
                .flatten();

        if let Some(model_json) = changed {
            self.as_mut().set_model_json(model_json);
        }
    }

    /// Applies one action QML reported.
    pub fn dispatch(mut self: Pin<&mut Self>, action: &QString) {
        let raw = action.to_string();
        let outcome = with_runtime(|runtime| {
            let applied = runtime.dispatch(&raw);
            let changed = runtime.pump().then(|| QString::from(runtime.model_json()));

            (applied, changed)
        });

        match outcome {
            Some((Ok(()), Some(model_json))) => self.as_mut().set_model_json(model_json),
            Some((Ok(()), None)) => {}
            Some((Err(error), _)) => warn!(%error, "refusing an action the surface reported"),
            None => warn!("no daemon is installed; the action is dropped"),
        }
    }

    /// Confirms from QML that the tree it built is the one the projection asks for.
    ///
    /// The title has to arrive intact, every popup the projection carries has to
    /// have been instantiated, and the semantic tree has to expose at least the
    /// stack and one node per popup. Anything less and the self-check fails
    /// instead of passing a load that produced no usable surface.
    pub fn report_ready(
        self: Pin<&mut Self>,
        title: QString,
        accessible_items: i32,
        popup_cards: i32,
    ) {
        let projected = self.model_json().to_string();
        let Ok(value) = serde_json::from_str::<Value>(&projected) else {
            warn!("the projection QML reported on is not JSON");

            return;
        };

        let expected_title = value["window"]["title"].as_str().unwrap_or_default();
        let expected_cards = value["stack"]["popups"]
            .as_array()
            .map_or(0, |popups| popups.len());

        if title.to_string() != expected_title {
            warn!(
                reported = %title,
                expected = expected_title,
                "the QML tree reported a title that does not match the projection"
            );

            return;
        }

        if usize::try_from(popup_cards).unwrap_or_default() != expected_cards {
            warn!(
                reported = popup_cards,
                expected = expected_cards,
                "the QML tree did not instantiate every popup the projection carries"
            );

            return;
        }

        let minimum = 1 + popup_cards;

        if accessible_items < minimum {
            warn!(
                reported = accessible_items,
                minimum, "the QML semantic tree is not populated (root AGENTS.md §7.4)"
            );

            return;
        }

        QML_READY.store(true, Ordering::SeqCst);
    }
}
