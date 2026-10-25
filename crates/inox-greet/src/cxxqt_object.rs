//! cxx-qt bridge: the only place QML and Rust meet.
//!
//! `Greeter` is a Rust-backed `QObject` the QML module instantiates. It carries a
//! **single** read-only property — the JSON projection from [`crate::view`] — and a
//! narrow channel back: one invokable that carries a UI action, two that carry key
//! presses, a pump for the answers the boundary's threads deliver, a tick for the
//! model's clock, and the readiness report the headless check waits for.
//! docs/qml-migration-contract.md §3 makes the shape a rule: one model, re-sent
//! whole on every change, rather than a property per field.
//!
//! # What QML can and cannot say
//!
//! - It can say **"this control was activated"**, **"this key was pressed"** and
//!   **"this character was typed"**. It cannot say "authenticate as alice": the
//!   action vocabulary is a fixed list ([`parse_action`]), and every message it can
//!   produce is one the state machine already accepts from a keyboard.
//! - It can say **"this character was typed"** and nothing about what the field now
//!   holds. The password lives in [`inox_common::secret::Secret`], in Rust, moved
//!   into the `auth-v1` request when it is submitted. No property, no
//!   projection field and no QML variable ever carries it back — the interface
//!   receives a mask derived from its *length* and draws that.
//! - It cannot make a login happen. The commands the model returns are queued by
//!   [`crate::runtime::Host`], which speaks `auth-v1` to `inox-auth`; a session only
//!   starts because the boundary said so.
//!
//! Note: doc comments are not placed on items inside the `#[cxx_qt::bridge]` block
//! below — the cxx parser rejects them there. Documentation lives on the Rust type
//! and on the plain-Rust `impl` block outside the macro.

use core::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};

use cxx_qt_lib::QString;
use inox_config::Theme as Tokens;
use serde_json::Value;
use tracing::warn;

use crate::app::{self, KeyPress, Message};
use crate::runtime::{self, Host};
use crate::view;

/// Set once QML has loaded the module and reported that its tree completed.
pub static QML_READY: AtomicBool = AtomicBool::new(false);

/// Whether QML reported a completed tree.
///
/// This is the headless check's evidence that the Rust-backed element was really
/// instantiated by the engine — not merely that a URL was handed to `load`.
pub fn qml_ready() -> bool {
    QML_READY.load(Ordering::SeqCst)
}

/// The Rust backing struct of the QML `Greeter` element.
pub struct GreeterRust {
    /// The whole projection, as JSON. QML parses it into the object every binding
    /// reads.
    pub model_json: QString,
    /// The login screen this element drives, taken from [`crate::runtime`] when the
    /// engine created it.
    ///
    /// `None` only when the element was created before the binary installed a state
    /// (a unit test, or a QML document loaded on its own). Every invokable then does
    /// nothing and says so, because aborting a constructor takes the whole process
    /// with it.
    host: Option<Host>,
}

impl Default for GreeterRust {
    fn default() -> Self {
        match runtime::take() {
            Some(host) => Self {
                model_json: QString::from(host.model_json().as_str()),
                host: Some(host),
            },
            None => {
                warn!(
                    "the QML element was created before the login screen installed its \
                     state; rendering the scaffolding"
                );
                Self {
                    model_json: QString::from(
                        view::scaffolding_json(&Tokens::builtin_dark()).as_str(),
                    ),
                    host: None,
                }
            }
        }
    }
}

/// The QML element and its invokables.
#[cxx_qt::bridge]
pub mod qobject {
    // `type QString` below is a cxx alias with no Rust-side documentation, and the
    // cxx parser rejects `///` comments inside bridge items, so allow `missing_docs`
    // for the bridge as a whole rather than annotating each item.
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
        type Greeter = super::GreeterRust;

        #[qinvokable]
        fn dispatch(self: Pin<&mut Self>, action: QString);

        #[qinvokable]
        fn key(self: Pin<&mut Self>, name: QString);

        #[qinvokable]
        fn text(self: Pin<&mut Self>, value: QString);

        #[qinvokable]
        fn pump(self: Pin<&mut Self>);

        #[qinvokable]
        fn tick(self: Pin<&mut Self>);

        #[qinvokable]
        fn report_ready(self: Pin<&mut Self>, title: QString, accessible_items: i32);
    }
}

/// Behaviour of the `Greeter` QObject.
impl qobject::Greeter {
    /// Applies one UI action, as JSON: `{"type":"activate"}`,
    /// `{"type":"select-account","index":1}`.
    ///
    /// A malformed action is logged and dropped. It cannot be an error the person
    /// sees: what arrives here came from this crate's own QML, and a login screen
    /// that refused to work because a button sent something odd would be worse than
    /// one that ignored it.
    pub fn dispatch(self: Pin<&mut Self>, action: QString) {
        let raw = String::from(&action);

        match parse_action(&raw) {
            Some(UiAction::Message(message)) => self.apply(message),
            Some(UiAction::Account(index)) => match self.as_ref().account_name(index) {
                Some(name) => self.apply(Message::AccountChosen(name)),
                None => warn!(
                    index,
                    "the QML tree chose an account the screen does not draw"
                ),
            },
            None => warn!(%raw, "the QML tree sent an action this screen does not know"),
        }
    }

    /// Applies one named key press: `tab`, `shift-tab`, `enter`, `escape`,
    /// `backspace`.
    ///
    /// The names are a fixed vocabulary, matching what [`app::key_binding`] binds
    /// and nothing else. Space is deliberately absent: it is a character somebody may
    /// want in their own password.
    pub fn key(self: Pin<&mut Self>, name: QString) {
        let name = String::from(&name);

        match named_key(&name).and_then(app::key_binding) {
            Some(message) => self.apply(message),
            None => warn!(%name, "the QML tree sent a key this screen does not bind"),
        }
    }

    /// Applies one typed character.
    ///
    /// The character is handed straight to the state machine, which appends it to the
    /// [`inox_common::secret::Secret`] while the ring is on the credential field. It
    /// is not stored anywhere else, and it is never sent back to QML: what the field
    /// draws is the mask the projection derives from the credential's length.
    ///
    /// A string that is not exactly one character is an input-method commit rather
    /// than a keystroke and is dropped, the rule the Iced build had.
    pub fn text(self: Pin<&mut Self>, value: QString) {
        let value = String::from(&value);

        match typed_character(&value).and_then(app::key_binding) {
            Some(message) => self.apply(message),
            None => warn!("the QML tree reported text that is not one character"),
        }
    }

    /// Applies every answer the boundary's threads have delivered.
    pub fn pump(mut self: Pin<&mut Self>) {
        let changed = {
            use cxx_qt::CxxQtType;

            // `rust_mut` hands back a `Pin<&mut GreeterRust>`; the generated type is
            // `Unpin`, so the deref coercion below is the whole conversion.
            let rust: &mut GreeterRust = &mut self.as_mut().rust_mut();

            if rust.host.as_mut().is_some_and(Host::pump) {
                rust.host.as_ref().map(Host::model_json)
            } else {
                None
            }
        };

        if let Some(json) = changed {
            self.as_mut().set_model_json(QString::from(json.as_str()));
        }
    }

    /// Advances the model's clock: the reply deadline, the rolling allowance and the
    /// started face's dwell.
    pub fn tick(mut self: Pin<&mut Self>) {
        // One borrow of the backing struct: the projection is read while the host
        // borrow is still alive, so it has to come out of the same `rust_mut()` call
        // rather than from a second `as_ref()` whose temporary would already be gone.
        let projected = {
            use cxx_qt::CxxQtType;

            // `rust_mut` hands back a `Pin<&mut GreeterRust>`; the generated type is
            // `Unpin`, so the deref coercion below is the whole conversion.
            let rust: &mut GreeterRust = &mut self.as_mut().rust_mut();

            rust.host.as_mut().map(|host| {
                host.tick();
                host.model_json()
            })
        };

        if let Some(json) = projected {
            self.as_mut().set_model_json(QString::from(json.as_str()));
        }
    }

    /// Confirms from QML that the tree completed, that the projected title arrived
    /// intact and that the semantic tree is populated.
    ///
    /// The readiness flag is only set when all three agree, so the headless check
    /// fails when a control loses its `Accessible.role` (root AGENTS.md §7.4) instead
    /// of passing on a load that produced no usable surface.
    pub fn report_ready(self: Pin<&mut Self>, title: QString, accessible_items: i32) {
        use cxx_qt::CxxQtType;

        let expected = self
            .rust()
            .host
            .as_ref()
            .map(|host| (host.title(), host.a11y_items()));

        let Some((expected_title, expected_items)) = expected else {
            warn!("the QML tree reported readiness, but no login screen was installed");
            return;
        };

        if String::from(&title) != expected_title {
            warn!("the QML tree reported a title that does not match the projection");
            return;
        }

        let expected_items = i32::try_from(expected_items).unwrap_or(i32::MAX);

        if accessible_items != expected_items {
            warn!(
                reported = accessible_items,
                expected = expected_items,
                "the QML semantic tree does not match the projected accessible items"
            );
            return;
        }

        QML_READY.store(true, Ordering::SeqCst);
    }

    /// The login name of the account at `index` of the list the screen draws.
    ///
    /// The row reports its position rather than the name, so nothing QML sends is
    /// interpolated into a request: the name comes from the projection's own
    /// accounts, which came from the account database.
    fn account_name(self: Pin<&Self>, index: usize) -> Option<String> {
        use cxx_qt::CxxQtType;

        let rust = self.rust();

        rust.host
            .as_ref()
            .and_then(|host| host.state().model().accounts.get(index))
            .map(|account| account.name.clone())
    }

    /// Applies one message and re-sends the projection it produced.
    fn apply(mut self: Pin<&mut Self>, message: Message) {
        use cxx_qt::CxxQtType;

        let projected = {
            // `rust_mut` hands back a `Pin<&mut GreeterRust>`; the generated type is
            // `Unpin`, so the deref coercion below is the whole conversion.
            let rust: &mut GreeterRust = &mut self.as_mut().rust_mut();

            if let Some(host) = rust.host.as_mut() {
                host.apply(message);
            }

            rust.host.as_ref().map(Host::model_json)
        };

        if let Some(json) = projected {
            self.as_mut().set_model_json(QString::from(json.as_str()));
        }
    }
}

/// What a UI action asks the login screen to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    /// A message the keyboard could have produced as well.
    Message(Message),
    /// The account at this index of the list the screen draws.
    ///
    /// An index rather than a name: the name comes from the projection's own
    /// accounts, so nothing QML sends is ever interpolated into a request, and an
    /// account name the database happens to carry cannot be turned into syntax.
    Account(usize),
}

/// The action a UI action string names, or `None` if it names something else.
///
/// This is the whole vocabulary QML has for asking the state machine to do something
/// (docs/qml-migration-contract.md §3). Everything here is also reachable from the
/// keyboard, which is the point: a pointer cannot operate a control a keyboard could
/// not, and no action can submit an identity the model would not accept.
fn parse_action(raw: &str) -> Option<UiAction> {
    let action: Value = serde_json::from_str(raw).ok()?;
    let kind = action.get("type")?.as_str()?;

    match kind {
        "activate" => Some(UiAction::Message(Message::Activate)),
        "submit" => Some(UiAction::Message(Message::Submitted)),
        "cancel" => Some(UiAction::Message(Message::Cancelled)),
        "switch-user" => Some(UiAction::Message(Message::SwitchUser)),
        "focus-next" => Some(UiAction::Message(Message::FocusNext)),
        "focus-previous" => Some(UiAction::Message(Message::FocusPrevious)),
        "select-account" => action
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
            .map(UiAction::Account),
        _ => None,
    }
}

/// The key press a QML key name asks for, or `None` if it is not one this screen
/// binds.
fn named_key(name: &str) -> Option<KeyPress> {
    match name {
        "tab" => Some(KeyPress::Tab),
        "shift-tab" => Some(KeyPress::BackTab),
        "enter" => Some(KeyPress::Enter),
        "escape" => Some(KeyPress::Escape),
        "backspace" => Some(KeyPress::Backspace),
        _ => None,
    }
}

/// The character a QML `text` event carries, if it carries exactly one.
fn typed_character(value: &str) -> Option<KeyPress> {
    let mut characters = value.chars();

    match (characters.next(), characters.next()) {
        (Some(character), None) => Some(KeyPress::Character(character)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ui_action_vocabulary_is_what_the_state_machine_accepts() {
        assert_eq!(
            parse_action(r#"{"type":"activate"}"#),
            Some(UiAction::Message(Message::Activate))
        );
        assert_eq!(
            parse_action(r#"{"type":"submit"}"#),
            Some(UiAction::Message(Message::Submitted))
        );
        assert_eq!(
            parse_action(r#"{"type":"cancel"}"#),
            Some(UiAction::Message(Message::Cancelled))
        );
        assert_eq!(
            parse_action(r#"{"type":"switch-user"}"#),
            Some(UiAction::Message(Message::SwitchUser))
        );
        assert_eq!(
            parse_action(r#"{"type":"focus-next"}"#),
            Some(UiAction::Message(Message::FocusNext))
        );
        assert_eq!(
            parse_action(r#"{"type":"focus-previous"}"#),
            Some(UiAction::Message(Message::FocusPrevious))
        );
        assert_eq!(
            parse_action(r#"{"type":"select-account","index":1}"#),
            Some(UiAction::Account(1))
        );
    }

    #[test]
    fn an_action_this_screen_does_not_know_changes_nothing() {
        for raw in [
            "",
            "not json",
            "{}",
            r#"{"type":"authenticate","username":"alice","password":"hunter2"}"#,
            r#"{"type":"select-account"}"#,
            r#"{"type":"select-account","index":-1}"#,
            r#"{"type":"select-account","index":"alice"}"#,
            r#"{"type":"select-account","name":"alice"}"#,
        ] {
            assert_eq!(parse_action(raw), None, "`{raw}` was accepted");
        }
    }

    #[test]
    fn the_key_vocabulary_is_the_one_the_bindings_have() {
        assert_eq!(named_key("tab"), Some(KeyPress::Tab));
        assert_eq!(named_key("shift-tab"), Some(KeyPress::BackTab));
        assert_eq!(named_key("enter"), Some(KeyPress::Enter));
        assert_eq!(named_key("escape"), Some(KeyPress::Escape));
        assert_eq!(named_key("backspace"), Some(KeyPress::Backspace));

        // Space is a character, not a shortcut: a binding on it would eat a
        // character somebody may type into their own password.
        assert_eq!(named_key("space"), None);
        assert_eq!(named_key("delete"), None);
        assert_eq!(
            app::key_binding(KeyPress::Character(' ')),
            Some(Message::Credential(' '))
        );
    }

    #[test]
    fn one_typed_character_is_a_keystroke_and_more_is_an_input_method_commit() {
        assert_eq!(typed_character("a"), Some(KeyPress::Character('a')));
        assert_eq!(typed_character("在"), Some(KeyPress::Character('在')));
        assert_eq!(typed_character(" "), Some(KeyPress::Character(' ')));
        assert_eq!(typed_character(""), None);
        assert_eq!(typed_character("ab"), None, "a commit is not a keystroke");
    }

    #[test]
    fn the_element_starts_from_the_scaffolding_when_nothing_is_installed() {
        // `Default` must be total: the engine constructs the object, and a panic in
        // a constructor would take the whole process with it.
        let rust = GreeterRust::default();
        assert!(rust.host.is_none());

        let value: Value =
            serde_json::from_str(&String::from(&rust.model_json)).expect("valid JSON");
        assert_eq!(value["ui"]["choosing"], false);
        assert_eq!(value["window"]["width"], view::WINDOW_WIDTH);
        assert!(!qml_ready(), "nothing reported readiness in this test");
    }
}
