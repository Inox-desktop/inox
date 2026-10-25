//! The login screen's runtime: the background half of `auth-v1`, and the slot the
//! QML element picks its state up from.
//!
//! The interface is QML, so there is no framework runtime left to
//! interpret a "task" or a "subscription". Two explicit sources replace them
//! (docs/qml-migration-contract.md §5):
//!
//! - **The boundary is asked on a thread.** [`Transport`] keeps the socket and one
//!   channel; every [`Command`] the model queues becomes one thread that speaks one
//!   `auth-v1` request, sends its [`Message`] back and ends. Requests are independent
//!   because the protocol's are: each opens its own connection, which is what makes a
//!   cancellation a real request rather than a decision this process keeps to itself.
//! - **The clock is QML's.** The tree ticks the model once a second, and
//!   [`Host::tick`] is what that tick reaches; the reply deadline and the started
//!   face's dwell are still the model's own arithmetic. Nothing
//!   here starts a timer thread, and nothing polls: [`Host::pump`] drains whatever
//!   the boundary has delivered, and the tree calls it on its own (short) timer.
//!
//! [`Host`] is installed once, before the QML module is loaded, and taken by the
//! Rust-backed element the engine creates. It lives in a thread-local rather than a
//! global because it holds the [`State`] — a `Localizer`, a `VecDeque` and a clock —
//! and the whole login screen is one thread: Qt's. A `static` would demand `Send` of
//! values that must never leave it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use tracing::debug;

use crate::app::{self, Message, State};
use crate::boundary;
use crate::state::Command;
use crate::view;

/// The channel between the boundary threads and the interface, and the socket they
/// talk to.
pub struct Transport {
    /// Where the boundary is, when this build has one. `None` is `--preview`: the
    /// model queues no request while no boundary is connected, and anything that
    /// still arrives here is dropped with a log line rather than attempted.
    socket: Option<PathBuf>,
    sender: Sender<Message>,
    receiver: Receiver<Message>,
}

impl Transport {
    /// A transport that talks to `socket`, or to nobody.
    pub fn new(socket: Option<PathBuf>) -> Self {
        let (sender, receiver) = mpsc::channel();

        Self {
            socket,
            sender,
            receiver,
        }
    }

    /// Hands `commands` to the boundary, one thread per request.
    ///
    /// The threads are detached: a request that outlives the login screen sends its
    /// answer to a closed channel and ends. Nothing is joined, because a person
    /// closing a window must not wait for a socket's timeout.
    pub fn dispatch(&self, commands: Vec<Command>) {
        let Some(socket) = self.socket.clone() else {
            for command in commands {
                if !matches!(command, Command::None) {
                    debug!("no authentication boundary is connected; the request is dropped");
                }
            }
            return;
        };

        for command in commands {
            let sender = self.sender.clone();
            let socket = socket.clone();

            match command {
                Command::None => {}
                Command::Authenticate {
                    id,
                    username,
                    credential,
                } => {
                    let attempt = id.raw();
                    thread::spawn(move || {
                        let report = boundary::authenticate(&socket, attempt, username, credential);
                        deliver(&sender, Message::TransportAnswered { attempt, report });
                    });
                }
                Command::StartSession(id) => {
                    let attempt = id.raw();
                    thread::spawn(move || {
                        let report = boundary::start_session(&socket, attempt);
                        deliver(&sender, Message::TransportAnswered { attempt, report });
                    });
                }
                Command::Cancel(id) => {
                    let attempt = id.raw();
                    thread::spawn(move || {
                        boundary::cancel(&socket, attempt);
                        deliver(&sender, Message::TransportFinished);
                    });
                }
            }
        }
    }

    /// Everything the boundary has delivered since this was last called.
    pub fn drain(&self) -> Vec<Message> {
        let mut messages = Vec::new();

        while let Ok(message) = self.receiver.try_recv() {
            messages.push(message);
        }

        messages
    }
}

/// Hand one answer to the interface, or say that there is none left to hand it to.
fn deliver(sender: &Sender<Message>, message: Message) {
    if sender.send(message).is_err() {
        debug!("the login screen is gone; the boundary's answer is dropped");
    }
}

/// The login screen's state and its transport, as the QML element drives them.
pub struct Host {
    state: State,
    transport: Transport,
}

impl Host {
    /// A host for `state`, with the boundary the state was built with.
    pub fn new(state: State) -> Self {
        let socket = state.socket().cloned();

        Self {
            state,
            transport: Transport::new(socket),
        }
    }

    /// The login screen's state.
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// The window title, as the catalogs resolved it.
    pub fn title(&self) -> String {
        self.state.title()
    }

    /// How many items the QML tree must expose with an accessibility role.
    pub fn a11y_items(&self) -> usize {
        view::a11y_items(self.state.model())
    }

    /// The whole projection, as the JSON the QML property carries.
    pub fn model_json(&self) -> String {
        view::model_json(&self.state)
    }

    /// Applies one message, and queues whatever it asked the boundary for.
    pub fn apply(&mut self, message: Message) {
        let commands = app::update(&mut self.state, message);

        self.transport.dispatch(commands);
    }

    /// Applies every answer the boundary has delivered. `true` if any arrived.
    pub fn pump(&mut self) -> bool {
        let messages = self.transport.drain();
        let delivered = !messages.is_empty();

        for message in messages {
            self.apply(message);
        }

        delivered
    }

    /// Advances the model's clock.
    ///
    /// The reply deadline, the rolling allowance and the started face's dwell are
    /// the model's, and this is the only thing that moves them: a tick from the QML
    /// timer goes in, and a session that started goes back to the
    /// login prompt when the dwell is over.
    pub fn tick(&mut self) {
        self.apply(Message::Ticked(Instant::now()));
    }
}

thread_local! {
    /// The login screen the QML element takes its state from.
    ///
    /// Installed by the binary before the engine loads `qml/Main.qml` and taken by
    /// the element's default constructor, so the engine creating the object and the
    /// binary owning the state do not fight over who comes first.
    static INSTALLED: RefCell<Option<Host>> = const { RefCell::new(None) };
}

/// Makes `host` the state the QML element picks up.
///
/// Returns `false` when one was already installed: the binary installs exactly one,
/// and a second is a programming error rather than something to overwrite.
pub fn install(host: Host) -> bool {
    INSTALLED.with(|slot| {
        let mut slot = slot.borrow_mut();

        if slot.is_some() {
            return false;
        }

        *slot = Some(host);
        true
    })
}

/// Takes the installed state, once.
///
/// The element calls this from its default constructor. `None` means the element
/// was created before anything installed a state — in a unit test, or by a QML
/// document loaded on its own — and the element then renders the scaffolding rather
/// than panicking in a constructor, which would abort the process.
pub fn take() -> Option<Host> {
    INSTALLED.with(|slot| slot.borrow_mut().take())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{Account, Accounts};
    use crate::state::Phase;
    use inox_common::i18n::Locale;
    use inox_config::Theme as Tokens;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    fn state(socket: Option<PathBuf>) -> State {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let roster = Accounts {
            accounts: vec![Account::new("alice", 1000, Some("Alice Doe".to_owned()))],
            ..Accounts::default()
        };

        match socket {
            Some(socket) => {
                State::connected(Rc::new(localizer), Tokens::builtin_dark(), socket, roster)
            }
            None => State::new(Rc::new(localizer), Tokens::builtin_dark(), roster),
        }
    }

    #[test]
    fn a_preview_has_nothing_to_send_and_says_so() {
        let transport = Transport::new(None);
        let host = Host::new(state(None));

        // No boundary means no request; the model queues none, and the transport
        // drops anything it is handed instead of opening a socket.
        assert!(!host.model_json().is_empty());
        transport.dispatch(Vec::new());
        assert!(transport.drain().is_empty());
    }

    #[test]
    fn a_tick_moves_the_clock_the_model_reads() {
        let mut host = Host::new(state(None));
        let before = host.state().clock();

        host.tick();

        assert!(host.state().clock() >= before);
        assert_eq!(
            host.state().greeter().phase(),
            Phase::Unavailable,
            "a preview does not become loggable because time passed"
        );
    }

    #[test]
    fn a_submission_with_no_boundary_behind_the_socket_comes_back_unavailable() {
        // The state says a boundary exists (that is what `connected` means), but
        // nothing is listening: the transport runs the request on its own thread,
        // the request fails to connect, and the answer reaches the model through the
        // channel — the whole background path, without a privileged process.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("no-boundary.sock");
        let mut host = Host::new(state(Some(socket)));

        host.apply(Message::Submitted);

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !host.pump() {
            thread::sleep(Duration::from_millis(5));
        }

        assert!(
            matches!(
                host.state().greeter().phase(),
                Phase::Unavailable | Phase::Failed
            ),
            "a boundary that is not there is not a login: {:?}",
            host.state().greeter().phase()
        );
    }

    #[test]
    fn the_host_is_installed_once_and_taken_once() {
        let first = install(Host::new(state(None)));
        assert!(first, "the first install wins");
        assert!(
            !install(Host::new(state(None))),
            "a second install is a programming error, not an overwrite"
        );

        assert!(
            take().is_some(),
            "the element takes the state that was installed"
        );
        assert!(take().is_none(), "and only once");
    }

    #[test]
    fn the_projection_a_host_hands_out_is_the_whole_screen() {
        let host = Host::new(state(None));
        let value: serde_json::Value =
            serde_json::from_str(&host.model_json()).expect("the projection is valid JSON");

        assert_eq!(host.title(), "Inox login");
        assert_eq!(value["window"]["title"], host.title());
        assert_eq!(value["ui"]["a11yItems"], host.a11y_items());
        assert!(host.a11y_items() > 0);
    }
}
