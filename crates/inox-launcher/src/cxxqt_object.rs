//! cxx-qt bridge: the only place QML and Rust meet.
//!
//! `Launcher` is a Rust-backed `QObject` the QML module instantiates. It owns the
//! launcher's [`State`] and exposes the projection computed in [`crate::view`] as
//! one read-only JSON string, plus the handful of control bits the tree needs to
//! act on it:
//!
//! | property | what QML does with it |
//! |----------|-----------------------|
//! | `modelJson` | parses it once per change and binds every colour, size and sentence to it |
//! | `windowWidth` / `windowHeight` | sizes the window to the placement policy |
//! | `focusSerial` | moves the caret into the search field whenever it changes |
//! | `quitRequested` | closes the window (Esc, ) |
//!
//! The reverse direction is one invokable wide on purpose
//! (docs/qml-migration-contract.md §3): `dispatch` takes the JSON action a control
//! sends — `{"type":"set-query","value":"foo"}`, `{"type":"press","key":"enter"}` —
//! and turns it into a [`Message`] the state machine already understands. There
//! is no second UI definition in the bridge, and an action the launcher does not
//! understand is a `warn!`, never a panic.
//!
//! `pump` is the other half: QML calls it from a timer, it drains what the worker
//! threads posted (`crate::runtime`), applies it through `app::update`, and
//! republishes the projection when anything changed.
//!
//! Note: doc comments are not placed on items inside the `#[cxx_qt::bridge]`
//! block below; the cxx parser rejects them there. Documentation lives on the
//! Rust types and on the plain-Rust `impl` blocks outside the macro.

use core::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt_lib::QString;
use serde::Deserialize;
use tracing::{debug, warn};

use crate::app::{self, Message, State};
use crate::runtime::{self, Task};
use crate::view;
use crate::{RuntimeMessage, reload};

/// Set once QML has loaded the module and reported that its tree completed.
pub static QML_READY: AtomicBool = AtomicBool::new(false);

/// Whether QML reported a completed tree.
///
/// This is the self-check's evidence that the Rust-backed element was really
/// instantiated by the engine — not merely that a URL was handed to `load` — and
/// that the semantic tree it built is populated.
pub fn qml_ready() -> bool {
    QML_READY.load(Ordering::SeqCst)
}

/// One action a QML control reports.
///
/// The names are the ones `src/Main.qml` sends. They are an interface, not user
/// content: no string here is ever shown to anyone.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub(crate) enum UiAction {
    /// The search field's text changed.
    SetQuery {
        /// The field's whole text.
        value: String,
    },
    /// The clear control was pressed.
    ClearQuery,
    /// A row was chosen with the pointer.
    Choose {
        /// The desktop file id of the row.
        id: String,
    },
    /// The launch control was pressed.
    Activate,
    /// The window was asked to close.
    Dismiss,
    /// The index was asked for again.
    RetryIndex,
    /// A failed launch was asked for again.
    RetryLaunch,
    /// A key press the QML tree does not otherwise handle.
    Press {
        /// The key's name, as [`app::key_binding`] spells it.
        key: String,
        /// Whether the key is being held down.
        #[serde(default)]
        repeat: bool,
    },
}

impl UiAction {
    /// The message this action is for, or `None` for a key the launcher ignores.
    fn into_message(self) -> Option<Message> {
        match self {
            Self::SetQuery { value } => Some(Message::QueryChanged(value)),
            Self::ClearQuery => Some(Message::QueryCleared),
            Self::Choose { id } => Some(Message::RowChosen(id)),
            Self::Activate => Some(Message::Activated),
            Self::Dismiss => Some(Message::Dismissed),
            Self::RetryIndex => Some(Message::IndexRetried),
            Self::RetryLaunch => Some(Message::LaunchRetried),
            Self::Press { key, repeat } => app::key_binding(&key, repeat),
        }
    }
}

/// Parse one action and map it to the message the state machine understands.
///
/// Pure and public within the crate so the tests can drive the whole QML → state
/// path (typing, Enter, Escape, Up/Down) without a running window.
///
/// # Errors
///
/// Returns the `serde_json` error for a document that is not an action at all; a
/// well-formed action the launcher has no use for is `Ok(None)`.
pub(crate) fn action_message(action: &str) -> Result<Option<Message>, serde_json::Error> {
    serde_json::from_str::<UiAction>(action).map(UiAction::into_message)
}

/// The Rust backing struct of the QML `Launcher` element.
///
/// Every field named in a `#[qproperty]` below is a property QML can read; the
/// rest is the model behind them.
pub struct LauncherRust {
    /// The projection, as the JSON document QML parses.
    pub model_json: QString,
    /// Whether the launcher has been asked to close.
    pub quit_requested: bool,
    /// Window width in logical pixels, from the placement policy.
    pub window_width: i32,
    /// Window height in logical pixels, from the placement policy.
    pub window_height: i32,
    /// How many times the caret was asked for the search field.
    pub focus_serial: i32,
    /// The state machine, when the binary has booted one.
    state: Option<State>,
    /// Messages from the worker threads, drained by `pump`.
    queue: runtime::Queue<RuntimeMessage>,
    /// The SIGHUP / D-Bus reload source, when it could be opened.
    host: Option<reload::Host>,
}

/// The values the QObject properties are set to.
///
/// Computing them first and assigning second is what keeps `pump` from reading
/// and writing the same fields: the projection borrows the state, and the
/// setters need the object mutably.
struct Published {
    model_json: String,
    quit_requested: bool,
    window_width: i32,
    window_height: i32,
    focus_serial: i32,
}

impl Published {
    /// The projection of `state`.
    fn of(state: &State) -> Self {
        Self {
            model_json: view::to_json(state),
            quit_requested: state.close_requested(),
            window_width: clamp(state.placement.size.width),
            window_height: clamp(state.placement.size.height),
            focus_serial: i32::try_from(state.focus_serial).unwrap_or(i32::MAX),
        }
    }
}

/// A logical pixel count as the `i32` the QML properties carry.
fn clamp(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

impl LauncherRust {
    /// The object behind a booted launcher.
    pub(crate) fn from_boot(boot: crate::Boot) -> Self {
        let published = Published::of(&boot.state);
        Self {
            model_json: QString::from(&published.model_json),
            quit_requested: published.quit_requested,
            window_width: published.window_width,
            window_height: published.window_height,
            focus_serial: published.focus_serial,
            state: Some(boot.state),
            queue: boot.queue,
            host: boot.host,
        }
    }

    /// The object a QML engine can always construct.
    ///
    /// The engine instantiates the element from the module, which can happen
    /// before the binary installed a booted state (or in a test that builds the
    /// module without one). Falling back to an empty projection keeps that path
    /// total — a blank launcher window rather than a constructor that aborts the
    /// process — and every binding QML has still resolves.
    pub(crate) fn fallback() -> Self {
        Self {
            model_json: QString::from(&view::empty_projection().to_string()),
            quit_requested: false,
            window_width: 0,
            window_height: 0,
            focus_serial: 0,
            state: None,
            queue: runtime::Queue::new(),
            host: None,
        }
    }

    /// The state machine, when there is one.
    pub(crate) fn state(&self) -> Option<&State> {
        self.state.as_ref()
    }

    /// Apply one message and run the task it returned.
    fn apply(&mut self, message: RuntimeMessage) {
        let emitter = self.queue.emitter();
        let Some(state) = self.state.as_mut() else {
            debug!("a message arrived before the launcher was booted");
            return;
        };
        let task: Task<RuntimeMessage> = match message {
            RuntimeMessage::App(message) => app::update(state, message).map(RuntimeMessage::App),
            RuntimeMessage::Reload(request) => crate::reload_state(state, *request),
        };
        task.run(emitter);
    }

    /// Drain the queue and apply everything in it.
    ///
    /// Returns whether anything changed, which is what decides if the projection
    /// has to be re-sent: the timer that calls this runs on every frame, and
    /// re-publishing an unchanged document would re-parse it in QML and cost more
    /// than the check.
    fn pump_once(&mut self) -> bool {
        if let Some(host) = self.host.as_ref() {
            for request in host.requests() {
                self.queue.push(RuntimeMessage::Reload(Box::new(request)));
            }
        }

        let messages = self.queue.drain();
        if messages.is_empty() {
            return false;
        }
        for message in messages {
            self.apply(message);
        }
        true
    }
}

impl Default for LauncherRust {
    fn default() -> Self {
        match crate::take_boot() {
            Some(boot) => Self::from_boot(boot),
            None => Self::fallback(),
        }
    }
}

/// The QML element and its invokables.
#[cxx_qt::bridge]
pub mod qobject {
    // The `type QString` below is a cxx alias with no Rust-side documentation,
    // and the cxx parser rejects `///` comments inside bridge items, so allow
    // `missing_docs` for the bridge as a whole.
    #[allow(missing_docs)]
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    // `auto_cxx_name` makes snake_case Rust items visible to QML in camelCase
    // (`report_ready` becomes `reportReady`, `model_json` becomes `modelJson`);
    // it is only allowed on the block.
    #[auto_cxx_name]
    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, model_json)]
        #[qproperty(bool, quit_requested)]
        #[qproperty(i32, window_width)]
        #[qproperty(i32, window_height)]
        #[qproperty(i32, focus_serial)]
        type Launcher = super::LauncherRust;

        #[qinvokable]
        fn dispatch(self: Pin<&mut Self>, action: QString);

        #[qinvokable]
        fn pump(self: Pin<&mut Self>);

        #[qinvokable]
        fn report_ready(self: Pin<&mut Self>, title: QString, accessible_items: i32);
    }
}

/// Behaviour of the `Launcher` QObject.
impl qobject::Launcher {
    /// Turn the JSON action one control sent into a message and apply it.
    pub fn dispatch(mut self: Pin<&mut Self>, action: QString) {
        use cxx_qt::CxxQtType;

        let raw = String::from(&action);
        match action_message(&raw) {
            Ok(Some(message)) => {
                let rust = self.as_mut().rust_mut();
                rust.queue.push(RuntimeMessage::App(message));
            }
            Ok(None) => debug!(action = %raw, "an action the launcher has nothing to do for"),
            Err(error) => {
                warn!(%error, action = %raw, "ignoring an action QML sent that is not a launcher action");
            }
        }
        self.pump();
    }

    /// Apply everything the worker threads posted, and republish on a change.
    pub fn pump(mut self: Pin<&mut Self>) {
        use cxx_qt::CxxQtType;

        if !self.as_mut().rust_mut().pump_once() {
            return;
        }
        let published = {
            let pinned = self.as_ref();
            let rust = pinned.rust();
            rust.state().map(Published::of)
        };
        let Some(published) = published else {
            return;
        };
        self.as_mut()
            .set_model_json(QString::from(&published.model_json));
        self.as_mut().set_quit_requested(published.quit_requested);
        self.as_mut().set_window_width(published.window_width);
        self.as_mut().set_window_height(published.window_height);
        self.as_mut().set_focus_serial(published.focus_serial);
    }

    /// Confirm from QML that the tree completed.
    ///
    /// The readiness flag is only set when the title and the number of accessible
    /// items agree with the projection, so the headless test fails when a control
    /// loses its `Accessible.role` (root AGENTS.md §7.4) instead of passing on a
    /// load that produced no usable surface.
    pub fn report_ready(self: Pin<&mut Self>, title: QString, accessible_items: i32) {
        use cxx_qt::CxxQtType;

        let expected = {
            let pinned = self.as_ref();
            let rust = pinned.rust();
            rust.state().map(app::title)
        };
        let Some(expected) = expected else {
            warn!("the QML tree reported ready before the launcher was booted");
            return;
        };
        let reported = String::from(&title);
        if reported != expected {
            warn!(
                reported = %reported,
                expected = %expected,
                "the QML tree reported a title that does not match the projection"
            );
            return;
        }
        if accessible_items < view::MINIMUM_ACCESSIBLE_ITEMS {
            warn!(
                reported = accessible_items,
                expected = view::MINIMUM_ACCESSIBLE_ITEMS,
                "the QML semantic tree is missing controls"
            );
            return;
        }
        debug!(accessible_items, "the QML tree reported ready");
        QML_READY.store(true, Ordering::SeqCst);
    }
}
