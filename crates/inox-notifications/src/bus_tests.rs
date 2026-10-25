//! The `org.freedesktop.Notifications` service, driven by a real D-Bus client.
//!
//! This is the evidence acceptance D06 asks for, at the level it can be had
//! without a session: a **real** client and a **real** daemon, on a real D-Bus
//! connection.
//!
//! # Why peer-to-peer, and what that does not prove
//!
//! CI has no session bus, and a test that needed one would either be skipped
//! everywhere or be flaky. zbus's `p2p` support gives the same wire protocol
//! over a `UnixStream::pair()` — the messages, the serials, the replies, the
//! errors and the signals are identical — with two differences this file is
//! honest about:
//!
//! - **No bus daemon, so no name ownership.** `org.freedesktop.Notifications` is
//!   a well-known name, and a name is something a bus arbitrates: over a
//!   peer-to-peer connection `request_name` succeeds locally and proves nothing.
//!   What a second daemon owning the name does to startup is therefore a unit
//!   test of the mapping from `zbus::Error::NameTaken` in
//!   [`crate::bus`]; the live case — two daemons on one session
//!   bus, the second exiting non-zero — is what a session with a bus is for. No
//!   test here pretends to cover it.
//! - **No bus, so no `destination` header.** Every call below is addressed to
//!   the connection itself, which is what a peer-to-peer D-Bus call is.
//!
//! # What the daemon in here is
//!
//! Not a stub. [`Harness`] starts the real [`Service`] on one end of the socket
//! pair, and on the other end the real [`State`] — built by `State::new`,
//! updated by [`app::update`], drained by `State::take_outgoing`. The only thing
//! this file replaces is the Qt host: the pump thread reads the inbox by hand and
//! calls `app::update` in a loop rather than running a window around it, which is
//! what makes the daemon a user drives the daemon these tests drive. It lives in
//! the binary because cxx-qt links the bridge's generated C++ into every target of
//! the package, so an integration test in `tests/` cannot link (docs/
//! qml-migration-contract.md §2.1, KDAB/cxx-qt #770).
//!
//! Threads outlive their assertions in two places, and both are deliberate: the
//! service's own threads end when the connection does (the test process ends
//! with them), and the reader in
//! [`a_signal_reaches_the_wire_with_what_the_client_is_waiting_for`] blocks on a
//! connection the test still holds.

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use inox_common::i18n::Locale;
use inox_config::{NotificationsConfig, Theme as Tokens};
use zbus::OwnedVariant;
use zbus::message::{Message, Type};
use zbus::{Connection, MessageStream};

use crate::actions::ActionId;
use crate::app::{self, Inbox, Message as AppMessage, State};
use crate::bus::{Counts, Service};
use crate::contract::{Outgoing, Signal};
use crate::policy::SessionState;
use crate::protocol::{self, CloseReason, HINT_CATEGORY, HINT_URGENCY, Urgency};
use crate::queue::Counters;
use crate::queue::NotificationId;
use crate::view_model::{PopupStack, StackStatus};

/// How long a test waits for something it expects to happen.
const TIMEOUT: Duration = Duration::from_secs(5);

/// How long a call waits for the daemon in the test that is about the timeout
/// rather than about the daemon.
const SHORT_DEADLINE: Duration = Duration::from_millis(300);

/// How often the daemon's pump thread looks for work.
///
/// Far below a frame, because these tests draw nothing: a call is expected to be
/// answered within microseconds of being picked up.
const PUMP: Duration = Duration::from_millis(1);

/// The identifier a call that replaces nothing sends.
const NEW_NOTIFICATION: u32 = 0;

/// One method call's worth of `Notify` arguments, so that a test states only
/// what it is about.
#[derive(Debug, Clone)]
struct Notify {
    app_name: String,
    replaces_id: u32,
    app_icon: String,
    summary: String,
    body: String,
    actions: Vec<String>,
    hints: HashMap<String, OwnedVariant>,
    expire_timeout: i32,
}

impl Default for Notify {
    fn default() -> Self {
        Self {
            app_name: "test-app".to_owned(),
            replaces_id: NEW_NOTIFICATION,
            app_icon: String::new(),
            summary: "a summary".to_owned(),
            body: "a body".to_owned(),
            actions: Vec::new(),
            hints: HashMap::new(),
            expire_timeout: protocol::EXPIRETIMEOUT_SERVER_DEFAULT,
        }
    }
}

impl Notify {
    /// The same call, with the summary replaced.
    #[must_use]
    fn summary(mut self, summary: &str) -> Self {
        self.summary = summary.to_owned();
        self
    }

    /// The same call, carrying a body.
    #[must_use]
    fn body(mut self, body: &str) -> Self {
        self.body = body.to_owned();
        self
    }

    /// The same call, replacing the notification `replaces_id` names.
    #[must_use]
    fn replacing(mut self, replaces_id: u32) -> Self {
        self.replaces_id = replaces_id;
        self
    }

    /// The same call, offering the flat key/label pairs of `actions`.
    #[must_use]
    fn actions(mut self, actions: &[&str]) -> Self {
        self.actions = actions.iter().map(|action| (*action).to_owned()).collect();
        self
    }

    /// The same call, with one more hint.
    #[must_use]
    fn hint(mut self, key: &str, value: impl Into<OwnedVariant>) -> Self {
        self.hints.insert(key.to_owned(), value.into());
        self
    }

    /// The same call, expiring after `milliseconds`.
    #[must_use]
    fn expiring_in(mut self, milliseconds: i32) -> Self {
        self.expire_timeout = milliseconds;
        self
    }

    /// The eight arguments `Notify` carries, in the order its signature states.
    fn arguments(&self) -> NotifyArguments {
        (
            self.app_name.clone(),
            self.replaces_id,
            self.app_icon.clone(),
            self.summary.clone(),
            self.body.clone(),
            self.actions.clone(),
            self.hints.clone(),
            self.expire_timeout,
        )
    }
}

/// The eight arguments `Notify` carries.
type NotifyArguments = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    HashMap<String, OwnedVariant>,
    i32,
);

/// What the daemon holds, as it reports it about itself.
///
/// Read through the pump thread: [`State`] is not `Send`, and a test process has
/// no second way to ask a daemon that lives on another thread.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    counters: Counters,
    /// How many notifications are held right now.
    held: usize,
    /// What the popup stack would draw.
    model: PopupStack,
    session: SessionState,
    /// Calls that arrived with an identity that was already answered.
    repeated_calls: u64,
    /// Invocations that were refused because the action was already invoked or
    /// the notification was gone.
    rejected_invocations: u64,
    /// What the transport has handled, as the service counts it.
    transport: Counts,
    /// Messages no stream could take, on the way in to the daemon.
    lost: u64,
}

impl Snapshot {
    /// What the popup stack would draw.
    fn status(&self) -> StackStatus {
        self.model.status
    }
}

/// Something the pump thread is asked to do.
enum Command {
    /// Hand a message to the update loop, as the surface or a timer would.
    Inject(AppMessage),
    /// Apply a configuration reload, as the Qt host does when the bridge hands
    /// one over.
    Reload(NotificationsConfig),
    /// Stop answering calls, so that the transport's deadline can be reached
    /// without waiting for a slow machine.
    Pause(bool),
    /// Report what the daemon holds, and answer on the sender.
    Probe(Sender<Snapshot>),
    /// Stop.
    Stop,
}

/// Only one daemon runs at a time in this file.
///
/// Each test drives a real connection with its own threads, and libtest runs a
/// file's tests on as many threads as the machine has cores. A dozen
/// peer-to-peer connections at once is not a shape this daemon ever has — a
/// session runs one notification daemon — and the transport underneath it (zbus
/// on async-io, through the blocking API) is not driven that way in production
/// either: with eighteen connections in one process a blocked call can be left
/// waiting on a wake-up that never comes. The tests take a lock and say so
/// rather than hide it, and the daemon's own concurrency — one connection, two
/// transport threads — is what the rest of the file exercises.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// A connected pair of peer-to-peer connections: the serving end, and a client.
///
/// One side has to be the D-Bus *server* — it is the one that answers the SASL
/// handshake and the client's `Hello` — and the two handshakes have to happen at
/// the same time. Building one end to completion before the other is a deadlock,
/// which is why the serving end is built on its own thread: the daemon under
/// test really does connect, it is only the *bus* that is missing.
fn connected_pair() -> (Connection, Connection) {
    let (server, client) = UnixStream::pair().expect("a socket pair");
    let guid = zbus::Guid::generate();

    let serving = {
        let guid = guid.clone();

        thread::Builder::new()
            .name("the connection the daemon serves".to_owned())
            .spawn(move || {
                zbus::block_on(
                    zbus::connection::Builder::unix_stream(server)
                        .server(guid)
                        .p2p()
                        .build(),
                )
            })
            .expect("the connecting thread starts")
    };

    let client = zbus::block_on(zbus::connection::Builder::unix_stream(client).p2p().build())
        .expect("the client end of the connection");
    let server = serving
        .join()
        .expect("the connecting thread does not panic")
        .expect("the serving end of the connection");

    (server, client)
}

/// A daemon, a client, and the wire between them.
struct Harness {
    client: Connection,
    commands: Sender<Command>,
    signals: Arc<Mutex<Vec<Signal>>>,
    pump: Option<JoinHandle<()>>,
    /// Held for the life of the harness: see [`ONE_AT_A_TIME`].
    _serial: std::sync::MutexGuard<'static, ()>,
}

impl Harness {
    /// Starts the service on one end of a socket pair and the state machine on
    /// the other, with the daemon's own reply deadline.
    fn start() -> Self {
        Self::start_with_deadline(None)
    }

    /// The same, with a shorter deadline than the daemon's own.
    fn start_with_deadline(deadline: Option<Duration>) -> Self {
        let serial = ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let (server, client) = connected_pair();

        let inbox = Inbox::new();
        let service = Service::serve_on(server, inbox.clone())
            .expect("the service starts on a peer-to-peer connection");
        let service = match deadline {
            Some(deadline) => service.with_reply_deadline(deadline),
            None => service,
        };

        let (commands, command_queue) = mpsc::channel();
        let signals = Arc::new(Mutex::new(Vec::new()));

        let pump = {
            let inbox = inbox.clone();
            let signals = Arc::clone(&signals);

            thread::Builder::new()
                .name("the daemon under test".to_owned())
                .spawn(move || pump(inbox, service, command_queue, signals))
                .expect("the daemon thread starts")
        };

        Self {
            client,
            commands,
            signals,
            pump: Some(pump),
            _serial: serial,
        }
    }

    /// Sends one method call and returns what came back.
    ///
    /// The `Serialize` bound is zbus's own re-export of serde: this crate does
    /// not depend on serde itself, and a body it can put on the wire is exactly
    /// what a body has to be.
    fn call<B>(&self, member: &str, body: &B) -> zbus::Result<Message>
    where
        B: zbus::export::serde::Serialize + zbus::DynamicType,
    {
        zbus::block_on(self.client.call_method(
            None::<()>,
            protocol::OBJECT_PATH,
            Some(protocol::INTERFACE),
            member,
            body,
        ))
    }

    /// Sends a `Notify` call and returns the identifier it was answered with.
    fn notify(&self, notify: &Notify) -> u32 {
        let reply = self
            .call("Notify", &notify.arguments())
            .unwrap_or_else(|error| panic!("`Notify` failed: {error}"));

        reply
            .body()
            .deserialize::<u32>()
            .expect("`Notify` answers with an identifier")
    }

    /// Sends a `Notify` call whose actions array cannot be read, and returns the
    /// error the service answered with.
    fn notify_badly(&self, actions: &[&str]) -> zbus::Error {
        let notify = Notify::default().actions(actions);

        self.call("Notify", &notify.arguments())
            .expect_err("an unusable actions array is refused")
    }

    /// Sends a `CloseNotification` call.
    fn close(&self, id: u32) -> zbus::Result<Message> {
        self.call("CloseNotification", &id)
    }

    /// Asks what the daemon holds.
    fn snapshot(&self) -> Snapshot {
        let (reply, answered) = mpsc::channel();

        self.commands
            .send(Command::Probe(reply))
            .expect("the daemon is running");

        answered
            .recv_timeout(TIMEOUT)
            .expect("the daemon answered the probe")
    }

    /// Applies a configuration reload, as the Qt host does.
    fn reload(&self, config: NotificationsConfig) {
        self.commands
            .send(Command::Reload(config))
            .expect("the daemon is running");
    }

    /// Hands the update loop a message, as the surface or a timer would.
    fn inject(&self, message: AppMessage) {
        self.commands
            .send(Command::Inject(message))
            .expect("the daemon is running");
    }

    /// Stops the daemon answering calls, or lets it answer again.
    fn pause(&self, paused: bool) {
        self.commands
            .send(Command::Pause(paused))
            .expect("the daemon is running");
    }

    /// The signals the daemon decided to send, in the order it decided them.
    fn signals(&self) -> Vec<Signal> {
        self.signals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The `NotificationClosed` signals so far, as identifier and reason.
    fn closures(&self) -> Vec<(NotificationId, CloseReason)> {
        self.signals()
            .into_iter()
            .filter_map(|signal| match signal {
                Signal::NotificationClosed { id, reason } => Some((id, reason)),
                Signal::ActionInvoked { .. } => None,
            })
            .collect()
    }

    /// The `ActionInvoked` signals so far, as identifier and action.
    fn invocations(&self) -> Vec<(NotificationId, ActionId)> {
        self.signals()
            .into_iter()
            .filter_map(|signal| match signal {
                Signal::ActionInvoked { id, action } => Some((id, action)),
                Signal::NotificationClosed { .. } => None,
            })
            .collect()
    }

    /// Waits until `condition` holds for a snapshot, or gives up.
    ///
    /// The transport emits signals on its own thread, so a count that is still
    /// catching up is not a failure: what a test waits for is that it *gets*
    /// there.
    fn wait_for(&self, what: &str, condition: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let deadline = Instant::now() + TIMEOUT;

        loop {
            let snapshot = self.snapshot();

            if condition(&snapshot) {
                return snapshot;
            }

            assert!(
                Instant::now() < deadline,
                "the daemon never reached the state this test waits for: {what}"
            );

            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits until the transport has emitted `count` signals.
    fn wait_for_signals(&self, count: u64) -> Snapshot {
        self.wait_for("the transport emitted what the daemon queued", |snapshot| {
            snapshot.transport.signals == count
        })
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);

        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
    }
}

/// The daemon: a real state machine, updated in a loop, drained into the service.
fn pump(
    inbox: Inbox,
    service: Service,
    commands: Receiver<Command>,
    signals: Arc<Mutex<Vec<Signal>>>,
) {
    let mut state = State::new(
        NotificationsConfig::default(),
        Tokens::builtin_dark(),
        Rc::new(crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse")),
    );

    let requests = inbox.stream();
    let mut paused = false;
    let mut running = true;

    while running {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Inject(message) => {
                    // The update's task is nothing in this milestone: the
                    // daemon's side effects leave through `take_outgoing`.
                    let _ = app::update(&mut state, message);
                    drain(&mut state, &service, &signals);
                }
                Command::Reload(config) => {
                    // The host applies a reload through `State::apply_configuration`
                    // with the tokens and the catalogues the file names.
                    let tokens = state.tokens().clone();
                    let localizer = Rc::new(
                        crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse"),
                    );
                    state.apply_configuration(config, tokens, localizer);
                    drain(&mut state, &service, &signals);
                }
                Command::Pause(now_paused) => paused = now_paused,
                Command::Probe(reply) => {
                    let _ = reply.send(snapshot(&state, &service, &inbox));
                }
                Command::Stop => running = false,
            }
        }

        if !paused {
            // One message at a time, draining after each: this is what a frame
            // of the real application does, and draining in batches would let
            // the transport's own bound drop a reply a client is waiting for.
            match requests.try_recv() {
                Ok(message) => {
                    let _ = app::update(&mut state, message);
                    drain(&mut state, &service, &signals);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => running = false,
            }
        }

        thread::sleep(PUMP);
    }
}

/// Hands everything the daemon decided to the transport, keeping the signals for
/// the test to read.
fn drain(state: &mut State, service: &Service, signals: &Mutex<Vec<Signal>>) {
    for outgoing in state.take_outgoing() {
        if let Outgoing::Signal(signal) = &outgoing {
            signals
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(signal.clone());
        }

        service.deliver(outgoing);
    }
}

/// What the daemon reports about itself.
fn snapshot(state: &State, service: &Service, inbox: &Inbox) -> Snapshot {
    Snapshot {
        counters: state.queue().counters(),
        held: state.queue().len(),
        model: state.model(),
        session: state.session(),
        repeated_calls: state.repeated_calls(),
        rejected_invocations: state.rejected_invocations(),
        transport: service.counts(),
        lost: inbox.lost(),
    }
}

// --- the four methods -------------------------------------------------------

#[test]
fn notify_is_answered_with_the_identifier_the_queue_allocated() {
    let harness = Harness::start();

    let id = harness.notify(&Notify::default().summary("a build finished"));

    assert_eq!(
        id, 1,
        "the first notification a session admits is number one"
    );
    let snapshot = harness.snapshot();
    assert_eq!(snapshot.counters.admitted, 1);
    assert_eq!(snapshot.counters.created, 1);
    assert_eq!(snapshot.held, 1);
    assert_eq!(snapshot.transport.calls, 1);
    assert_eq!(snapshot.transport.answered, 1);
    assert_eq!(snapshot.transport.rejected, 0);
    assert_eq!(snapshot.lost, 0);
}

#[test]
fn a_replacements_identifier_is_reused_and_the_client_is_not_told_it_closed() {
    let harness = Harness::start();

    let first = harness.notify(&Notify::default().summary("first"));
    let second = harness.notify(&Notify::default().summary("second").replacing(first));

    assert_eq!(second, first, "a replacement keeps the identifier it named");

    let snapshot = harness.snapshot();
    assert_eq!(snapshot.counters.admitted, 2);
    assert_eq!(snapshot.counters.created, 1);
    assert_eq!(snapshot.counters.replaced, 1);
    assert_eq!(
        snapshot.counters.settled(),
        0,
        "a replacement is not a termination"
    );
    assert!(
        harness.closures().is_empty(),
        "so no `NotificationClosed` is emitted for it: {:?}",
        harness.closures()
    );
}

#[test]
fn closing_a_notification_is_answered_and_told_to_the_client_once() {
    let harness = Harness::start();
    let id = harness.notify(&Notify::default());

    let reply = harness
        .close(id)
        .expect("closing a held notification succeeds");

    assert_eq!(
        reply.body().signature().to_string(),
        "",
        "the specification gives `CloseNotification` no return value, so a \
         boolean here would be a reply a real client's proxy rejects"
    );
    assert_eq!(
        harness.closures(),
        [(NotificationId::new(id), CloseReason::ClosedByCall)],
        "exactly one closure, with the reason the specification has for it"
    );
    assert_eq!(harness.snapshot().counters.closed_by_call, 1);
    harness.wait_for_signals(1);
}

#[test]
fn closing_something_that_is_gone_closes_nothing_and_says_so() {
    let harness = Harness::start();

    let reply = harness
        .close(4_242)
        .expect("a close of an unknown id is not an error");

    assert_eq!(reply.body().signature().to_string(), "");

    let snapshot = harness.snapshot();
    assert_eq!(
        snapshot.counters.closed_by_call, 0,
        "the daemon's answer to that call is `closed: false`, which the \
         contract carries and the wire does not"
    );
    assert!(harness.closures().is_empty());
    assert_eq!(snapshot.transport.answered, 1);
}

#[test]
fn the_capabilities_and_the_server_information_are_the_protocol_modules() {
    let harness = Harness::start();

    let capabilities = harness
        .call("GetCapabilities", &())
        .expect("`GetCapabilities` is served")
        .body()
        .deserialize::<Vec<String>>()
        .expect("`GetCapabilities` answers with a string array");

    assert_eq!(
        capabilities,
        protocol::CAPABILITIES,
        "the wire carries exactly the capabilities the crate declares"
    );

    let (name, vendor, version, spec_version) = harness
        .call("GetServerInformation", &())
        .expect("`GetServerInformation` is served")
        .body()
        .deserialize::<(String, String, String, String)>()
        .expect("`GetServerInformation` answers with four strings");

    assert_eq!(name, protocol::SERVER_INFORMATION.name);
    assert_eq!(vendor, protocol::SERVER_INFORMATION.vendor);
    assert_eq!(version, protocol::SERVER_INFORMATION.version);
    assert_eq!(spec_version, protocol::SERVER_INFORMATION.spec_version);
    assert_eq!(spec_version, protocol::SPEC_VERSION);
}

#[test]
fn an_unusable_actions_array_is_refused_with_the_error_this_daemon_documents() {
    let harness = Harness::start();

    let error = harness.notify_badly(&["only-a-key"]);

    let zbus::Error::MethodError(name, description, _) = error else {
        panic!("an unusable actions array must be an error reply, not a reply: {error:?}");
    };

    assert_eq!(
        name.as_str(),
        "org.freedesktop.Notifications.Error.InvalidActions",
        "the name is Inox's own: the specification defines no error for `Notify`"
    );
    assert!(
        description
            .as_deref()
            .is_some_and(|text| text.contains("not a whole number")),
        "the client is told what was wrong: {description:?}"
    );

    let snapshot = harness.snapshot();
    assert_eq!(snapshot.counters.admitted, 0, "nothing was admitted");
    assert_eq!(snapshot.held, 0, "so the queue is empty");
    assert_eq!(snapshot.transport.rejected, 1);
    assert!(
        harness.closures().is_empty(),
        "a refused call is not a notification that closed"
    );
}

#[test]
fn an_unknown_method_is_refused_with_the_error_the_specification_names() {
    let harness = Harness::start();

    let error = harness
        .call("NoSuchMethod", &())
        .expect_err("an unknown method is refused");

    assert!(
        matches!(error, zbus::Error::FDO(ref error) if matches!(error.as_ref(), zbus::fdo::Error::UnknownMethod(_))),
        "an unknown method must preserve the standard D-Bus error: {error:?}"
    );
    assert_eq!(harness.snapshot().transport.unknown, 1);
}

#[test]
fn the_interfaces_a_proxy_asks_about_before_it_calls_are_served() {
    let harness = Harness::start();

    let xml = zbus::block_on(harness.client.call_method(
        None::<()>,
        protocol::OBJECT_PATH,
        Some("org.freedesktop.DBus.Introspectable"),
        "Introspect",
        &(),
    ))
    .expect("introspection is served")
    .body()
    .deserialize::<String>()
    .expect("introspection answers with a string");

    assert!(xml.contains(&format!("<interface name=\"{}\">", protocol::INTERFACE)));
    assert!(xml.contains("<method name=\"Notify\">"));
    assert!(xml.contains("<signal name=\"NotificationClosed\">"));

    let properties = zbus::block_on(harness.client.call_method(
        None::<()>,
        protocol::OBJECT_PATH,
        Some("org.freedesktop.DBus.Properties"),
        "GetAll",
        &(protocol::INTERFACE,),
    ))
    .expect("a proxy's property load succeeds")
    .body()
    .deserialize::<HashMap<String, OwnedVariant>>()
    .expect("`GetAll` answers with a variant table");

    assert!(properties.is_empty(), "this interface has no properties");

    zbus::block_on(harness.client.call_method(
        None::<()>,
        protocol::OBJECT_PATH,
        Some("org.freedesktop.DBus.Peer"),
        "Ping",
        &(),
    ))
    .expect("a peer ping is answered");
}

// --- hints, lifetimes, actions and the shape of a session -------------------

#[test]
fn hints_the_daemon_can_read_survive_and_the_ones_it_cannot_are_counted() {
    let harness = Harness::start();

    // The two standard hints are sent the way the specification types them (a
    // byte and a string); the two `x-*` ones are legal D-Bus values that no
    // `HintValue` can carry.
    let mut notify = Notify::default()
        .hint(HINT_URGENCY, Urgency::Critical.raw())
        .hint(HINT_CATEGORY, zbus::Str::from("device".to_owned()))
        .hint("x-a-double", 1.5_f64)
        .hint("x-a-64-bit", 2_i64);

    // Padding the table past the transport's cap: the standard keys are what
    // this daemon reads, and they are kept ahead of everything else.
    for index in 0..(crate::bus::MAX_HINTS * 2) {
        notify = notify.hint(&format!("x-padding-{index:03}"), 7_u32);
    }

    let id = harness.notify(&notify);
    assert_eq!(id, 1);

    let snapshot = harness.snapshot();
    assert_eq!(snapshot.counters.created, 1, "the notification arrived");
    assert_eq!(
        snapshot.transport.hints_unsupported, 2,
        "a double and a 64-bit integer have no `HintValue` form, and neither is \
         guessed at"
    );
    assert!(
        snapshot.transport.hints_dropped > 0,
        "more hints than the cap were dropped and counted"
    );

    // The urgency the standard key carried is what the daemon acts on: a
    // critical notification outlives every sweep.
    harness.inject(AppMessage::Tick(
        Instant::now() + Duration::from_secs(86_400),
    ));

    // Give the sweep its turn before asking: what is asserted is that the
    // urgency the standard hint carried outlived it, not that the daemon had
    // already looked.
    thread::sleep(Duration::from_millis(50));

    let after = harness.snapshot();
    assert_eq!(after.held, 1, "a critical notification is not swept away");
    assert_eq!(after.counters.expired, 0);
}

#[test]
fn a_notification_that_runs_out_of_time_closes_itself_with_the_expiry_reason() {
    let harness = Harness::start();

    let id = harness.notify(&Notify::default().expiring_in(1_000));

    // The daemon's own sweep is a message like any other; this hands it the
    // instant the notification's lifetime ends, which is what a tick two seconds
    // from now would be.
    harness.inject(AppMessage::Tick(Instant::now() + Duration::from_secs(2)));

    let snapshot = harness.wait_for("the expiry was swept", |snapshot| {
        snapshot.counters.expired == 1
    });

    assert_eq!(snapshot.counters.settled(), 1);
    assert_eq!(snapshot.counters.dropped, 0);
    assert_eq!(snapshot.held, 0);
    assert_eq!(
        harness.closures(),
        [(NotificationId::new(id), CloseReason::Expired)],
        "reason 1 is what the specification calls an expiry"
    );
    assert_eq!(CloseReason::Expired.code(), 1);
    harness.wait_for_signals(1);
}

#[test]
fn an_action_is_invoked_once_and_closes_the_popup_once() {
    let harness = Harness::start();
    let id = harness.notify(&Notify::default().actions(&["default", "", "open", "Open"]));

    harness.inject(AppMessage::ActionInvoked {
        id: NotificationId::new(id),
        action: ActionId::new("open"),
    });

    let snapshot = harness.wait_for("the action was invoked and the popup closed", |snapshot| {
        snapshot.counters.dismissed == 1
    });

    assert_eq!(
        harness.invocations(),
        [(NotificationId::new(id), ActionId::new("open"))],
        "the client is told to act exactly once"
    );
    assert_eq!(
        harness.closures(),
        [(NotificationId::new(id), CloseReason::Dismissed)],
        "and the popup is closed exactly once"
    );

    // A second press of the same button finds nothing to invoke.
    harness.inject(AppMessage::ActionInvoked {
        id: NotificationId::new(id),
        action: ActionId::new("open"),
    });

    let again = harness.wait_for("the repeat was refused", |snapshot| {
        snapshot.rejected_invocations == 1
    });

    assert_eq!(again.counters.dismissed, snapshot.counters.dismissed);
    assert_eq!(harness.invocations().len(), 1);
    assert_eq!(harness.closures().len(), 1);
    harness.wait_for_signals(2);
}

#[test]
fn do_not_disturb_suppresses_the_popup_and_does_not_suppress_the_notification() {
    let harness = Harness::start();

    harness.inject(AppMessage::DoNotDisturbSet(true));
    let id = harness.notify(&Notify::default().summary("held while muted"));

    let snapshot = harness.wait_for("the stack reports itself muted", |snapshot| {
        snapshot.status() == StackStatus::Suppressed
    });

    assert!(id > 0);
    assert_eq!(
        snapshot.counters.created, 1,
        "the notification was delivered, counted and held"
    );
    assert_eq!(
        snapshot.held, 1,
        "muting is presentation: nothing was dropped"
    );
    assert!(harness.closures().is_empty(), "so nothing closed");

    // And the stack it draws has no focus to steal: this daemon's popups are
    // never a keyboard destination, whatever the policy says.
    assert!(
        snapshot.model.popups.is_empty(),
        "a suppressed stack draws no popups to focus"
    );
}

#[test]
fn there_is_no_body_anywhere_in_the_model_while_the_session_is_locked() {
    let harness = Harness::start();

    harness.inject(AppMessage::SessionLockChanged(SessionState::Locked));
    harness.notify(
        &Notify::default()
            .summary("a password was generated")
            .body("hunter2"),
    );

    let snapshot = harness.wait_for("the daemon is locked", |snapshot| {
        snapshot.session == SessionState::Locked
    });

    // The redaction is a type, not a check the view has to remember: the popup
    // model carries no body, no application name and no icon, and this is asked
    // of the same state machine the window draws from.
    assert_eq!(snapshot.model.status, StackStatus::Showing);
    assert_eq!(snapshot.model.popups.len(), 1, "the popup is there");

    for popup in &snapshot.model.popups {
        assert!(
            popup.content.is_withheld(),
            "the popup withholds its content"
        );
        assert!(
            popup.content.body().is_none() && popup.content.app_name().is_none(),
            "and has no field the body could be read out of"
        );
    }
}

#[test]
fn the_call_serial_is_what_stops_a_redelivered_call_from_acting_twice() {
    use std::num::NonZeroU32;

    let harness = Harness::start();
    let notify = Notify::default().summary("delivered twice");
    let serial = NonZeroU32::new(4_242).expect("a non-zero serial");

    // A call the transport redelivers keeps its serial: that is the only thing
    // that says "this is the same call", and it is what `contract::Calls`
    // remembers.
    for _ in 0..2 {
        let message = Message::method_call(protocol::OBJECT_PATH, "Notify")
            .interface(protocol::INTERFACE)
            .serial(serial)
            .build(&notify.arguments())
            .expect("a well-formed call");

        zbus::block_on(harness.client.send(&message)).expect("the call is sent");
    }

    let snapshot = harness.wait_for("both deliveries were answered", |snapshot| {
        snapshot.repeated_calls == 1
    });

    assert_eq!(
        snapshot.counters.created, 1,
        "the same call is not a second notification"
    );
    assert_eq!(snapshot.counters.admitted, 1);
    assert_eq!(snapshot.held, 1);
    assert_eq!(
        snapshot.transport.answered, 2,
        "both deliveries were answered"
    );
    assert_eq!(snapshot.transport.timed_out, 0);
}

// --- the burst D06 injects --------------------------------------------------

#[test]
fn a_hundred_calls_a_second_for_ten_seconds_is_bounded_and_loses_nothing() {
    let harness = Harness::start();

    // The bound is small so that eviction is reached early; the burst is the
    // one acceptance D06 injects: a hundred a second, for ten seconds.
    let mut config = NotificationsConfig::default();
    config.queue.max_pending = 32;
    harness.reload(config);

    let mut answered = Vec::new();

    for index in 0..1_000u32 {
        let started = Instant::now();
        let notify = Notify::default().summary(&format!("burst {index}"));

        let reply = harness
            .call("Notify", &notify.arguments())
            .unwrap_or_else(|error| panic!("call {index} was not answered: {error}"));

        answered.push(
            reply
                .body()
                .deserialize::<u32>()
                .expect("`Notify` answers with an identifier"),
        );

        // A hundred a second, which is one call every ten milliseconds.
        if let Some(rest) = Duration::from_millis(10).checked_sub(started.elapsed()) {
            thread::sleep(rest);
        }
    }

    assert_eq!(answered.len(), 1_000);
    assert!(
        answered.iter().all(|id| *id > 0),
        "every call was answered with an identifier"
    );

    let snapshot = harness.wait_for("the queue reached its bound", |snapshot| {
        snapshot.counters.dropped > 0
    });

    assert_eq!(snapshot.transport.calls, 1_000);
    assert_eq!(snapshot.transport.timed_out, 0);
    assert_eq!(snapshot.transport.unavailable, 0);
    assert_eq!(
        snapshot.lost, 0,
        "the daemon never refused an input: it stayed responsive under the burst"
    );

    // The invariant the queue module states, over the whole burst: every
    // notification that was created is either held or has reached exactly one
    // terminal state.
    assert_eq!(snapshot.counters.created, 1_000);
    assert_eq!(
        snapshot.counters.created,
        snapshot.counters.settled() + snapshot.held as u64,
        "nothing was lost in the burst: {snapshot:?}"
    );
    assert!(snapshot.held <= 32, "the bound held");
    assert_eq!(snapshot.counters.settled(), snapshot.counters.dropped);
    assert!(snapshot.counters.dropped > 0, "the bound was reached");

    // Every terminal state told its client, and reached the wire.
    harness.wait_for_signals(snapshot.counters.settled());
    assert_eq!(
        harness.closures().len() as u64,
        snapshot.counters.settled(),
        "every notification the queue dropped told its client why"
    );

    let capabilities = harness
        .call("GetCapabilities", &())
        .expect("the daemon is still answering")
        .body()
        .deserialize::<Vec<String>>()
        .expect("a string array");

    assert_eq!(capabilities, protocol::CAPABILITIES);
}

// --- what the transport does when the daemon cannot answer -------------------

#[test]
fn a_call_the_daemon_does_not_answer_in_time_is_refused_with_an_error() {
    let harness = Harness::start_with_deadline(Some(SHORT_DEADLINE));

    // The daemon is held back on purpose, so the timeout is a property of the
    // transport rather than a guess about how slow the machine is.
    harness.pause(true);

    let error = harness
        .call("GetCapabilities", &())
        .expect_err("a call nobody answers must not be left hanging");

    let zbus::Error::MethodError(name, _, _) = error else {
        panic!("a timeout must be an error reply: {error:?}");
    };

    assert_eq!(name.as_str(), "org.freedesktop.Notifications.Error.Timeout");

    harness.pause(false);

    let reply = harness
        .call("GetCapabilities", &())
        .expect("the daemon answers again once it is listening");

    assert_eq!(
        reply
            .body()
            .deserialize::<Vec<String>>()
            .expect("a string array"),
        protocol::CAPABILITIES,
        "and the answer the held-back call left behind is not mistaken for this one"
    );

    assert_eq!(harness.snapshot().transport.timed_out, 1);
}

#[test]
fn a_call_nobody_is_reading_is_refused_rather_than_left_unanswered() {
    let _serial = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let (server, client) = connected_pair();

    // A service whose inbox nothing reads: the daemon has not started yet, or
    // has stopped, and the transport says so instead of waiting.
    let service = Service::serve_on(server, Inbox::new()).expect("the service starts");

    let error = zbus::block_on(client.call_method(
        None::<()>,
        protocol::OBJECT_PATH,
        Some(protocol::INTERFACE),
        "GetCapabilities",
        &(),
    ))
    .expect_err("nobody is reading, so the call cannot be answered");

    let zbus::Error::MethodError(name, _, _) = error else {
        panic!("an unanswerable call must be an error reply: {error:?}");
    };

    assert_eq!(
        name.as_str(),
        "org.freedesktop.Notifications.Error.Unavailable"
    );
    assert_eq!(service.counts().unavailable, 1);
}

// --- the two signals, on the wire -------------------------------------------

#[test]
fn a_signal_reaches_the_wire_with_what_the_client_is_waiting_for() {
    let harness = Harness::start();
    let id = harness.notify(&Notify::default().actions(&["open", "Open"]));

    // The signals are read off the connection by their own thread: a test that
    // blocked on the socket would hang instead of failing.
    let received = watch_signals(&harness.client);

    harness.inject(AppMessage::ActionInvoked {
        id: NotificationId::new(id),
        action: ActionId::new("open"),
    });

    let invoked = received
        .recv_timeout(TIMEOUT)
        .expect("`ActionInvoked` arrives");

    assert_eq!(
        invoked,
        OnTheWire::Invoked {
            id,
            action: "open".to_owned()
        },
        "the identifier and the client's own action key"
    );

    let closed = received
        .recv_timeout(TIMEOUT)
        .expect("`NotificationClosed` follows it");

    assert_eq!(
        closed,
        OnTheWire::Closed {
            id,
            reason: CloseReason::Dismissed.code()
        },
        "the identifier and the reason the specification names"
    );

    // A second press is not a second pair of signals.
    harness.inject(AppMessage::ActionInvoked {
        id: NotificationId::new(id),
        action: ActionId::new("open"),
    });

    harness.wait_for("the repeat was refused", |snapshot| {
        snapshot.rejected_invocations == 1
    });

    assert_eq!(
        received.recv_timeout(Duration::from_millis(250)),
        Err(RecvTimeoutError::Timeout),
        "the client must not be told to act twice"
    );
}

/// What a client sees on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OnTheWire {
    /// `NotificationClosed`: which notification, and the reason code.
    Closed {
        /// The identifier.
        id: u32,
        /// The reason code the specification defines.
        reason: u32,
    },
    /// `ActionInvoked`: which notification, and which action key.
    Invoked {
        /// The identifier.
        id: u32,
        /// The client's own key for the action.
        action: String,
    },
}

/// Reads every notification signal the connection carries, from its own thread.
///
/// The thread blocks on the connection, which the test holds for as long as it
/// asserts, so it parks when the test ends; the test process ends with it.
fn watch_signals(client: &Connection) -> Receiver<OnTheWire> {
    let rule = zbus::MatchRule::builder()
        .msg_type(Type::Signal)
        .interface(protocol::INTERFACE)
        .build()
        .expect("a valid signal match rule");

    let mut messages = zbus::block_on(MessageStream::for_match_rule(
        rule,
        client,
        std::num::NonZeroUsize::new(64),
    ))
    .expect("a subscription for them");

    let (sender, receiver) = mpsc::channel();

    thread::Builder::new()
        .name("the client's signal reader".to_owned())
        .spawn(move || {
            while let Some(message) = crate::bus::next_message(&mut messages) {
                let Ok(message) = message else {
                    return;
                };

                let member = message
                    .header()
                    .member()
                    .map(|member| member.as_str().to_owned());

                let observed = match member.as_deref() {
                    Some("NotificationClosed") => message
                        .body()
                        .deserialize::<(u32, u32)>()
                        .ok()
                        .map(|(id, reason)| OnTheWire::Closed { id, reason }),
                    Some("ActionInvoked") => message
                        .body()
                        .deserialize::<(u32, String)>()
                        .ok()
                        .map(|(id, action)| OnTheWire::Invoked { id, action }),
                    _ => None,
                };

                let Some(observed) = observed else {
                    continue;
                };

                if sender.send(observed).is_err() {
                    return;
                }
            }
        })
        .expect("the reader thread starts");

    receiver
}
