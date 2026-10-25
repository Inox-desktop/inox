//! The D-Bus service: `org.freedesktop.Notifications` on the session bus.
//! This is the transport the session-lock and notification bus work, rule 5 asks for, migrated by the zbus 6 migration.
//! This is the transport decision 5 asks for, migrated by//! It is a translation layer and nothing else: a method call becomes a [`Request`] the application
//! answers, the answer becomes a D-Bus reply or error, and a [`Signal`] becomes
//! `NotificationClosed` or `ActionInvoked`. Every decision about what a
//! notification *is* — identifiers, lifetimes, the bound, the policy, the
//! sanitised body — stays in [`crate::app`], [`crate::queue`] and
//! [`crate::policy`], which is why the daemon has one state machine rather than
//! two.
//!
//! # How the threads are arranged
//!
//! | thread | what it owns |
//! |---|---|
//! | the application's | [`State`](crate::app::State), the widgets, the Elm loop |
//! | `inox-notifications bus` | the connection, every reply, one call at a time |
//! | `inox-notifications signals` | every `NotificationClosed` and `ActionInvoked` |
//!
//! The two transport threads exist because neither direction may wait on the
//! other. The bus thread blocks reading the next call, so a signal released by
//! an expiry sweep would sit in a queue until some client happened to call —
//! and the signals thread, which only ever drains [`Service::deliver`], is what
//! stops that. Neither thread touches [`State`](crate::app::State): it holds an
//! `Rc<Localizer>` and is not `Send`, and the only way to it is
//! [`crate::app::Inbox`] (calls in) and `take_outgoing` (answers out).
//!
//! The daemon answers **one call at a time**, and the bus thread waits for the
//! matching answer with [`REPLY_DEADLINE`]. That is deliberate: the
//! application's outgoing queue is bounded and drops its oldest entry, so a
//! client that pipelined calls would eventually have its *reply* dropped and
//! wait forever. Serving one at a time means at most one reply is ever in
//! flight, and a reply that is late anyway becomes a D-Bus error the client can
//! act on rather than silence.
//!
//! # What crosses the wire, and what does not
//!
//! - **Call identity is the message serial.** `CallId` is the serial of the
//!   incoming message (`Header::serial_num`), never a counter this daemon
//!   invents, so a redelivered call is recognised by [`crate::contract::Calls`]
//!   and answered from memory instead of acted on twice.
//! - **`CloseNotification` answers with the empty reply the specification
//!   promises**, not with the daemon's internal `ReplyBody::Closed` boolean.
//!   [`crate::protocol::CLOSE_NOTIFICATION`] declares no return value, and
//!   clients such as `GDBusProxy` check a reply against the signature they
//!   expect; a boolean here would be a protocol error for them. Whether anything
//!   was closed is visible to the client through `NotificationClosed`, and to a
//!   test at the contract seam.
//! - **A refused request becomes an error the specification does not name.**
//!   `Notify` has no error of its own in the desktop-notification specification,
//!   so Inox sends its own error type, which lives under
//!   `org.freedesktop.Notifications.Error`: `.InvalidActions` for a call whose
//!   actions array is not a whole number of key/label pairs (or holds more than
//!   [`crate::actions::MAX_ACTIONS`] of them), `.Timeout` when the daemon did
//!   not answer within [`REPLY_DEADLINE`], and `.Unavailable` when the daemon
//!   has no application reading its calls. Refusing is the point: an actions
//!   array that cannot be read is never silently dropped.
//! - **Hints are bounded and lossy, and say so.** A `Notify` may carry any
//!   number of hint pairs; at most [`MAX_HINTS`] of them are carried into the
//!   daemon, the standard keys of [`crate::protocol::HINT_KEYS`] first. A hint
//!   whose D-Bus value has no [`HintValue`] form — a double, a byte array, a
//!   nested variant, an object path — is dropped rather than coerced, and
//!   counted separately from the ones the cap dropped. Both counts are in
//!   [`Counts`].
//! - **`Outgoing::FollowLink` is dropped.** Opening a link is a launch, which
//!   belongs to whatever launches applications on this desktop, and no launcher
//!   or portal exists in this workspace yet. The target is logged at `debug`
//!   and counted; nothing is executed.
//! - **The standard interfaces are served** — `Introspectable.Introspect`,
//!   `Properties.GetAll` (empty: this interface has no properties) and
//!   `Peer.Ping` — because a real client builds a proxy before it calls
//!   `Notify`, and a reflection-based proxy asks. The introspection XML is
//!   generated from [`crate::protocol::METHODS`] and
//!   [`crate::protocol::SIGNALS`], so it cannot drift from the contract the
//!   rest of the crate is written to.
//!
//! # Startup and shutdown
//!
//! [`Service::serve`] connects to the session bus and claims
//! [`crate::protocol::BUS_NAME`]
//! *before* the Qt host starts pumping. A session that already has a
//! notification daemon gets a readable error and a non-zero exit instead of two
//! daemons silently sharing the name. There is no signal handling here: the
//! session stops this daemon with `SIGTERM`, the process ends, and D-Bus
//! releases the name when the connection closes — which is what a well-behaved
//! bus client does, and one fewer thing to get wrong than a hand-rolled
//! shutdown path.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tracing::{debug, info, warn};
use zbus::fdo;
use zbus::message::{Header, Message, Type};
use zbus::{Connection, MessageStream};
use zbus::{OwnedVariant, Variant};

use crate::actions::ActionError;
use crate::app::{Inbox, Message as AppMessage};
use crate::contract::{
    CallId, NotifyRequest, Outgoing, Reply, ReplyBody, Request, RequestBody, Signal,
};
use crate::protocol::{self, HINT_IMAGE_PATH_DEPRECATED, HintValue};
use crate::queue::NotificationId;

/// How many hint pairs one `Notify` call may carry into the daemon.
///
/// The specification's own hints are eight keys, and real clients send a handful
/// at most; a client that sends more than this is either far ahead of this
/// daemon or pushing at it, and either way the pairs beyond the cap are counted
/// rather than carried. The standard keys are kept first, so padding the table
/// with `x-*` keys cannot starve the hints this daemon actually reads.
pub const MAX_HINTS: usize = 64;

/// How long the bus waits for the daemon to answer one call before it answers
/// the client with an error.
///
/// A frame of the daemon's own loop is measured in microseconds, so seconds are
/// four orders of magnitude of headroom: a call that reaches this is not slow,
/// it is unanswered.
pub const REPLY_DEADLINE: Duration = Duration::from_secs(5);

/// The thread the connection and every reply are handled on.
const BUS_THREAD: &str = "inox-notifications bus";

/// The thread the two signals are emitted from.
const SIGNAL_THREAD: &str = "inox-notifications signals";

/// Why the service could not be started.
#[derive(Debug, thiserror::Error)]
pub enum BusError {
    /// There is no session bus to connect to.
    ///
    /// Boxed because `zbus::Error` is large — a start-up error is read once and
    /// never passed around, so the indirection costs nothing that matters and
    /// keeps `Result<_, BusError>` small.
    #[error("connecting to the session bus: {0}")]
    Connect(#[source] Box<zbus::Error>),
    /// Another daemon already owns the notification bus name.
    ///
    /// Not a warning and not a takeover: two daemons sharing the name would
    /// split a client's calls between them, and a session has one notification
    /// stack.
    #[error(
        "`{name}` is already owned on the session bus: another notification daemon is \
         running, and this one refuses to coexist with it"
    )]
    NameTaken {
        /// The name that was taken.
        name: &'static str,
    },
    /// The bus refused the name for some other reason.
    #[error("claiming `{name}` on the session bus: {source}")]
    Claim {
        /// The name that was refused.
        name: &'static str,
        /// Why.
        #[source]
        source: Box<zbus::Error>,
    },
    /// The transport threads could not be started.
    #[error("starting the notification transport thread: {0}")]
    Spawn(#[source] std::io::Error),
}

/// The errors this service returns that the specification does not name.
///
/// The desktop-notification specification defines no error for `Notify`, so
/// these are Inox's own choice, under Inox's own prefix. They are error *names*
/// a client can branch on, not sentences: a description travels with each one,
/// but nothing that reads a name has to parse prose.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.Notifications.Error")]
enum ServiceError {
    /// A call whose actions array is not what its signature promises.
    InvalidActions(String),
    /// The daemon did not answer within [`REPLY_DEADLINE`].
    Timeout(String),
    /// No application is reading the daemon's calls.
    Unavailable(String),
}

/// What the transport has handled so far, as a snapshot.
///
/// Every count here is something acceptance D06 asks to be *observable* rather
/// than asserted: hints the cap dropped and hints no [`HintValue`] can carry are
/// separate numbers, and a refusal that reached a client is one of `rejected`,
/// `timed_out`, `unavailable`, `unknown` or `malformed` depending on why.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Method calls the connection delivered.
    pub calls: u64,
    /// Calls the daemon answered and the service replied to.
    pub answered: u64,
    /// Calls the daemon refused: `ReplyBody::Rejected`.
    pub rejected: u64,
    /// Calls the daemon did not answer within [`REPLY_DEADLINE`].
    pub timed_out: u64,
    /// Calls no application could take.
    pub unavailable: u64,
    /// Calls for a path, interface or member this service does not serve.
    pub unknown: u64,
    /// Calls whose arguments did not match the signature they arrived under.
    pub malformed: u64,
    /// Signals emitted on the bus.
    pub signals: u64,
    /// Signals the bus refused to carry.
    pub failed_signals: u64,
    /// Links followed in a body, dropped because nothing here launches.
    pub follow_links: u64,
    /// Hint pairs the cap of [`MAX_HINTS`] left out.
    pub hints_dropped: u64,
    /// Hint values with no [`HintValue`] form, dropped rather than coerced.
    pub hints_unsupported: u64,
    /// Things the application queued for the transport that no thread took.
    pub outgoing_lost: u64,
}

/// The running tallies behind [`Counts`].
#[derive(Debug, Default)]
struct Stats {
    calls: AtomicU64,
    answered: AtomicU64,
    rejected: AtomicU64,
    timed_out: AtomicU64,
    unavailable: AtomicU64,
    unknown: AtomicU64,
    malformed: AtomicU64,
    signals: AtomicU64,
    failed_signals: AtomicU64,
    follow_links: AtomicU64,
    hints_dropped: AtomicU64,
    hints_unsupported: AtomicU64,
    outgoing_lost: AtomicU64,
}

impl Stats {
    /// Adds one to a tally.
    fn bump(field: &AtomicU64) -> u64 {
        field.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Reads the tallies as one value.
    fn report(&self) -> Counts {
        let read = |field: &AtomicU64| field.load(Ordering::Relaxed);

        Counts {
            calls: read(&self.calls),
            answered: read(&self.answered),
            rejected: read(&self.rejected),
            timed_out: read(&self.timed_out),
            unavailable: read(&self.unavailable),
            unknown: read(&self.unknown),
            malformed: read(&self.malformed),
            signals: read(&self.signals),
            failed_signals: read(&self.failed_signals),
            follow_links: read(&self.follow_links),
            hints_dropped: read(&self.hints_dropped),
            hints_unsupported: read(&self.hints_unsupported),
            outgoing_lost: read(&self.outgoing_lost),
        }
    }
}

/// The running notification service.
///
/// Held by `main` for the life of the daemon. Dropping it stops the signals
/// thread (its senders go with it); the bus thread ends when the connection does.
#[derive(Debug)]
pub struct Service {
    replies: Sender<Reply>,
    signals: Sender<Signal>,
    deadline: Arc<AtomicU64>,
    stats: Arc<Stats>,
    bus: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl Service {
    /// Serves `org.freedesktop.Notifications` on the session bus.
    ///
    /// # Errors
    ///
    /// [`BusError::Connect`] when there is no session bus, and
    /// [`BusError::NameTaken`] when another daemon already owns the name — the
    /// daemon must not start "successfully" and then never be called.
    pub fn serve(inbox: Inbox) -> Result<Self, BusError> {
        let connection = zbus::block_on(Connection::session())
            .map_err(|error| BusError::Connect(Box::new(error)))?;
        claim(&connection)?;

        info!(
            bus = protocol::BUS_NAME,
            path = protocol::OBJECT_PATH,
            "serving the notification interface on the session bus"
        );

        Self::serve_on(connection, inbox)
    }

    /// Serves the interface on a connection the caller built.
    ///
    /// No name is claimed: a peer-to-peer connection has no bus to own one on
    /// and no other daemon to collide with. This is also what the transport's
    /// tests use, so the tests need no session bus.
    ///
    /// # Errors
    ///
    /// [`BusError::Spawn`] when a transport thread cannot be started.
    pub fn serve_on(connection: Connection, inbox: Inbox) -> Result<Self, BusError> {
        let (replies, reply_queue) = std::sync::mpsc::channel();
        let (signals, signal_queue) = std::sync::mpsc::channel();
        let deadline = Arc::new(AtomicU64::new(millis(REPLY_DEADLINE)));
        let stats = Arc::new(Stats::default());

        let bus = {
            let connection = connection.clone();
            let inbox = inbox.clone();
            let stats = Arc::clone(&stats);
            let deadline = Arc::clone(&deadline);

            thread::Builder::new()
                .name(BUS_THREAD.to_owned())
                .spawn(move || bus_loop(connection, inbox, reply_queue, deadline, stats))
                .map_err(BusError::Spawn)?
        };

        let writer = {
            let stats = Arc::clone(&stats);

            thread::Builder::new()
                .name(SIGNAL_THREAD.to_owned())
                .spawn(move || signal_loop(connection, signal_queue, stats))
                .map_err(BusError::Spawn)?
        };

        Ok(Self {
            replies,
            signals,
            deadline,
            stats,
            bus,
            writer,
        })
    }

    /// Shortens how long a call waits for the daemon's answer.
    ///
    /// The deadline is a constant of the daemon, and the reason this exists is
    /// that the timeout path is otherwise only reachable by waiting all of it
    /// out: a test can set a deadline of a few hundred milliseconds, hold the
    /// daemon's answer back on purpose, and see the D-Bus error a client gets
    /// when it happens.
    #[must_use]
    pub fn with_reply_deadline(self, deadline: Duration) -> Self {
        self.deadline.store(millis(deadline), Ordering::Relaxed);
        self
    }

    /// Carries out one thing the daemon decided.
    ///
    /// The application calls this for whatever [`crate::app::State::take_outgoing`]
    /// returned; `main.rs` is the only caller.
    pub fn deliver(&self, outgoing: Outgoing) {
        match outgoing {
            Outgoing::Reply(reply) => {
                if self.replies.send(reply).is_err() {
                    Stats::bump(&self.stats.outgoing_lost);
                }
            }
            Outgoing::Signal(signal) => {
                if self.signals.send(signal).is_err() {
                    Stats::bump(&self.stats.outgoing_lost);
                }
            }
            Outgoing::FollowLink { id, href } => {
                // Not an error and not a notification: following a link is a
                // launch, and no component in this workspace launches yet.
                Stats::bump(&self.stats.follow_links);
                debug!(
                    id = id.get(),
                    %href,
                    "a link was followed: opening it is a launch, which belongs to a launcher \
                     or portal that does not exist yet, so it is dropped"
                );
            }
        }
    }

    /// What the transport has handled so far.
    pub fn counts(&self) -> Counts {
        self.stats.report()
    }

    /// The thread the connection is served on, for a caller that wants to join
    /// it.
    ///
    /// Nothing joins it in the daemon: the thread ends when the connection does,
    /// which is when the process is going away anyway.
    pub fn bus_thread(&self) -> &JoinHandle<()> {
        &self.bus
    }

    /// The thread signals are emitted from.
    pub fn signal_thread(&self) -> &JoinHandle<()> {
        &self.writer
    }
}

/// Claims the notification bus name.
///
/// # Errors
///
/// [`BusError::NameTaken`] when another daemon owns it, [`BusError::Claim`] for
/// anything else the bus says.
pub fn claim(connection: &Connection) -> Result<(), BusError> {
    zbus::block_on(connection.request_name(protocol::BUS_NAME)).map_err(name_error)
}

/// Reads the bus's refusal to hand over a name as one of ours.
fn name_error(error: zbus::Error) -> BusError {
    match error {
        zbus::Error::NameTaken => BusError::NameTaken {
            name: protocol::BUS_NAME,
        },
        error => BusError::Claim {
            name: protocol::BUS_NAME,
            source: Box::new(error),
        },
    }
}

/// A duration as whole milliseconds, which is what the deadline is stored as.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// The bus thread: read a call, hand it to the daemon, answer it, repeat.
fn bus_loop(
    connection: Connection,
    inbox: Inbox,
    replies: Receiver<Reply>,
    deadline: Arc<AtomicU64>,
    stats: Arc<Stats>,
) {
    let mut messages = MessageStream::from(&connection);

    while let Some(message) = next_message(&mut messages) {
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                warn!(%error, "reading from the session bus failed");
                break;
            }
        };

        if message.header().message_type() != Type::MethodCall {
            continue;
        }

        answer(&connection, &message, &inbox, &replies, &deadline, &stats);
    }

    warn!(
        "the session bus connection ended: the notification service is no longer \
         listening, and no client will be answered"
    );
}

/// The next message the connection carries, or `None` once it is gone.
///
/// zbus 6's `MessageStream` is a `Stream`, and reading it incrementally used to
/// come from `futures-util`'s `StreamExt` — which this crate no longer depends on
/// now that `iced` is gone. `zbus::export::futures_core` is the same trait zbus
/// itself implements, and `poll_fn` over it is the whole of what `.next()`
/// elsewhere would do, so no async runtime and no extra dependency is needed.
pub(crate) fn next_message(stream: &mut MessageStream) -> Option<zbus::Result<Message>> {
    use std::future::poll_fn;
    use std::pin::Pin;

    use zbus::export::futures_core::Stream as _;

    zbus::block_on(poll_fn(|context| Pin::new(&mut *stream).poll_next(context)))
}

/// Answers one method call, from the wire to the wire.
fn answer(
    connection: &Connection,
    message: &Message,
    inbox: &Inbox,
    replies: &Receiver<Reply>,
    deadline: &AtomicU64,
    stats: &Stats,
) {
    let header = message.header();
    let call = CallId::new(u64::from(header.primary().serial_num().get()));
    Stats::bump(&stats.calls);

    let request = match translate(message, &header, stats) {
        Ok(Translated::Call(body)) => body,
        Ok(Translated::Direct(direct)) => {
            if let Err(error) = reply_direct(connection, &header, &direct) {
                warn!(call = call.get(), %error, "replying to a call failed");
            }

            return;
        }
        Err(error) => {
            let _ = reply_error(connection, &header, error);
            return;
        }
    };

    // The whole budget for this call: what is left after waiting for the
    // application is what is left for its answer.
    let budget = Duration::from_millis(deadline.load(Ordering::Relaxed));
    let started = Instant::now();

    if let Err(error) =
        inbox.deliver_within(AppMessage::Request(Request::new(call, request)), budget)
    {
        Stats::bump(&stats.unavailable);
        debug!(
            call = call.get(),
            %error,
            "the daemon has no stream reading its calls"
        );
        let _ = reply_error(
            connection,
            &header,
            ServiceError::Unavailable(
                "the notification daemon is not reading its calls".to_owned(),
            ),
        );

        return;
    }

    match await_reply(replies, call, budget.saturating_sub(started.elapsed())) {
        Ok(body) => {
            if body_is_rejection(&body) {
                Stats::bump(&stats.rejected);
            } else {
                Stats::bump(&stats.answered);
            }

            if let Err(error) = reply_with(connection, &header, body) {
                warn!(call = call.get(), %error, "replying to a call failed");
            }
        }
        Err(Failed::Timeout) => {
            Stats::bump(&stats.timed_out);
            debug!(
                call = call.get(),
                "the daemon did not answer in time; the client is told so rather \
                 than left waiting"
            );
            let _ = reply_error(
                connection,
                &header,
                ServiceError::Timeout(format!(
                    "the notification daemon did not answer within {} s",
                    deadline.load(Ordering::Relaxed) / 1_000
                )),
            );
        }
        Err(Failed::Detached) => {
            warn!(
                "the daemon's answer channel is gone; the call cannot be answered \
                 and the service is stopping"
            );
        }
    }
}

/// Whether a reply body is the daemon refusing the call.
fn body_is_rejection(body: &ReplyBody) -> bool {
    matches!(body, ReplyBody::Rejected { .. })
}

/// Why a call could not be answered by the daemon.
enum Failed {
    /// Nothing arrived within the deadline.
    Timeout,
    /// The application is gone.
    Detached,
}

/// Waits for the daemon's answer to `call`.
///
/// An answer for an earlier call is discarded rather than mistaken for this
/// one: it can only be a reply the transport's own bound dropped, and answering
/// a client with somebody else's identifier would be worse than the timeout the
/// earlier call already got.
fn await_reply(
    replies: &Receiver<Reply>,
    call: CallId,
    budget: Duration,
) -> Result<ReplyBody, Failed> {
    let deadline = Instant::now() + budget;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());

        if remaining.is_zero() {
            return Err(Failed::Timeout);
        }

        match replies.recv_timeout(remaining) {
            Ok(reply) if reply.call == call => return Ok(reply.body),
            Ok(reply) => debug!(
                late = reply.call.get(),
                waiting = call.get(),
                "an answer for an earlier call arrived late; discarding it"
            ),
            Err(RecvTimeoutError::Timeout) => return Err(Failed::Timeout),
            Err(RecvTimeoutError::Disconnected) => return Err(Failed::Detached),
        }
    }
}

/// The signals thread: emit what the daemon decided, in the order it decided it.
fn signal_loop(connection: Connection, signals: Receiver<Signal>, stats: Arc<Stats>) {
    while let Ok(signal) = signals.recv() {
        let emitted = zbus::block_on(async {
            match signal {
                Signal::NotificationClosed { id, reason } => {
                    connection
                        .emit_signal(
                            None::<()>,
                            protocol::OBJECT_PATH,
                            protocol::INTERFACE,
                            protocol::NOTIFICATION_CLOSED.name,
                            &(id.get(), reason.code()),
                        )
                        .await
                }
                Signal::ActionInvoked { id, action } => {
                    connection
                        .emit_signal(
                            None::<()>,
                            protocol::OBJECT_PATH,
                            protocol::INTERFACE,
                            protocol::ACTION_INVOKED.name,
                            &(id.get(), action.as_str()),
                        )
                        .await
                }
            }
        });

        match emitted {
            Ok(()) => {
                Stats::bump(&stats.signals);
            }
            Err(error) => {
                Stats::bump(&stats.failed_signals);
                warn!(%error, "emitting a signal failed");
            }
        }
    }
}

/// What one incoming message turned out to be.
enum Translated {
    /// A call of the notification interface, for the daemon to answer.
    Call(RequestBody),
    /// A call the service answers itself, because the daemon has no part in it.
    Direct(Direct),
}

/// The calls that are the service's own.
enum Direct {
    /// `Introspectable.Introspect`: the interface, as XML.
    Introspection,
    /// `Properties.GetAll`: no properties, so an empty table.
    Properties,
    /// `Peer.Ping`: nothing to say.
    Ping,
}

/// Turns one incoming message into what has to happen to it.
///
/// Everything this service does not serve is refused here, with the error the
/// D-Bus specification names for it, so that a client is never left without a
/// reply.
fn translate(
    message: &Message,
    header: &Header<'_>,
    stats: &Stats,
) -> Result<Translated, fdo::Error> {
    let path = header.path().map(|path| path.as_str().to_owned());
    if path.as_deref() != Some(protocol::OBJECT_PATH) {
        Stats::bump(&stats.unknown);
        return Err(fdo::Error::UnknownObject(format!(
            "this service is at `{}`",
            protocol::OBJECT_PATH
        )));
    }

    let interface = header
        .interface()
        .map(|interface| interface.as_str().to_owned())
        .unwrap_or_default();
    let member = header
        .member()
        .map(|member| member.as_str().to_owned())
        .unwrap_or_default();

    match interface.as_str() {
        protocol::INTERFACE => read_call(message, &member, stats).map(Translated::Call),
        INTROSPECTABLE if member == INTROSPECT => Ok(Translated::Direct(Direct::Introspection)),
        PROPERTIES if member == GET_ALL => Ok(Translated::Direct(Direct::Properties)),
        PEER if member == PING => Ok(Translated::Direct(Direct::Ping)),
        INTROSPECTABLE | PROPERTIES | PEER => {
            Stats::bump(&stats.unknown);
            Err(fdo::Error::UnknownMethod(format!(
                "`{interface}.{member}` is not served"
            )))
        }
        _ => {
            Stats::bump(&stats.unknown);
            Err(fdo::Error::UnknownInterface(format!(
                "`{interface}` is not served at `{}`",
                protocol::OBJECT_PATH
            )))
        }
    }
}

/// Reads one of the four notification methods.
fn read_call(message: &Message, member: &str, stats: &Stats) -> Result<RequestBody, fdo::Error> {
    match member {
        "Notify" => read_notify(message, stats).map(RequestBody::Notify),
        "CloseNotification" => {
            let id = message.body().deserialize::<u32>().map_err(|error| {
                Stats::bump(&stats.malformed);
                fdo::Error::InvalidArgs(format!(
                    "`CloseNotification` takes one identifier: {error}"
                ))
            })?;

            Ok(RequestBody::Close(NotificationId::new(id)))
        }
        "GetCapabilities" => Ok(RequestBody::GetCapabilities),
        "GetServerInformation" => Ok(RequestBody::GetServerInformation),
        other => {
            Stats::bump(&stats.unknown);
            Err(fdo::Error::UnknownMethod(format!(
                "`{other}` is not a method of `{}`",
                protocol::INTERFACE
            )))
        }
    }
}

/// The eight arguments `Notify` carries, in the order its signature states.
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

/// Reads a `Notify` call into the request the daemon answers.
fn read_notify(message: &Message, stats: &Stats) -> Result<NotifyRequest, fdo::Error> {
    let (app_name, replaces_id, app_icon, summary, body, actions, hints, expire_timeout) = message
        .body()
        .deserialize::<NotifyArguments>()
        .map_err(|error| {
            Stats::bump(&stats.malformed);
            fdo::Error::InvalidArgs(format!(
                "`Notify` does not carry the eight arguments of `{}`: {error}",
                protocol::NOTIFY.signature
            ))
        })?;

    Ok(NotifyRequest {
        app_name,
        replaces_id,
        app_icon,
        summary,
        body,
        actions,
        hints: read_hints(hints, stats),
        expire_timeout,
    })
}

/// Reads a hint table into the pairs this daemon can carry.
///
/// Two things are dropped here, and both are counted rather than guessed at:
/// whatever exceeds [`MAX_HINTS`], and any value whose D-Bus type has no
/// [`HintValue`]. The standard keys of [`crate::protocol::HINT_KEYS`] are kept
/// ahead of everything else, in a fixed order, so the result does not depend on
/// how a hash table happened to iterate.
fn read_hints(hints: HashMap<String, OwnedVariant>, stats: &Stats) -> Vec<(String, HintValue)> {
    let mut standard = Vec::new();
    let mut other = Vec::new();

    for (key, value) in hints {
        match hint_value(&value) {
            Some(value) if is_standard(&key) => standard.push((key, value)),
            Some(value) => other.push((key, value)),
            None => {
                Stats::bump(&stats.hints_unsupported);
            }
        }
    }

    standard.sort_by(|left, right| left.0.cmp(&right.0));
    other.sort_by(|left, right| left.0.cmp(&right.0));

    let room = MAX_HINTS.saturating_sub(standard.len());
    stats
        .hints_dropped
        .fetch_add(other.len().saturating_sub(room) as u64, Ordering::Relaxed);

    standard.extend(other.into_iter().take(room));
    standard
}

/// Whether a hint key is one the specification defines.
fn is_standard(key: &str) -> bool {
    protocol::HINT_KEYS.contains(&key) || key == HINT_IMAGE_PATH_DEPRECATED
}

/// The [`HintValue`] a D-Bus value reads as, if it reads as one at all.
///
/// The five encodings [`HintValue`] carries are the five that cross. Everything
/// else — a double, a 64-bit integer, an object path, an array, a nested
/// variant, a file descriptor — is `None`, because coercing it would be this
/// daemon inventing a value the client did not send, and a count of dropped
/// hints is a better answer than a wrong one.
fn hint_value(value: &OwnedVariant) -> Option<HintValue> {
    match &**value {
        Variant::U8(raw) => Some(HintValue::Byte(*raw)),
        Variant::Bool(flag) => Some(HintValue::Boolean(*flag)),
        Variant::I32(raw) => Some(HintValue::Int(*raw)),
        Variant::U32(raw) => Some(HintValue::Unsigned(*raw)),
        Variant::Str(text) => Some(HintValue::Text(text.as_str().to_owned())),
        Variant::I16(_)
        | Variant::U16(_)
        | Variant::I64(_)
        | Variant::U64(_)
        | Variant::F64(_)
        | Variant::Signature(_)
        | Variant::ObjectPath(_)
        | Variant::Variant(_)
        | Variant::Array(_)
        | Variant::Dict(_)
        | Variant::Structure(_)
        | Variant::Fd(_) => None,
    }
}

/// Answers a call with what the daemon decided.
fn reply_with(connection: &Connection, header: &Header<'_>, body: ReplyBody) -> zbus::Result<()> {
    zbus::block_on(async {
        match body {
            ReplyBody::Notified { id } => connection.reply(header, &id.get()).await,
            // The specification gives `CloseNotification` no return value, so the
            // daemon's `closed` boolean is not on the wire: see the module
            // documentation.
            ReplyBody::Closed { .. } => connection.reply(header, &()).await,
            ReplyBody::Capabilities { capabilities } => {
                connection.reply(header, &capabilities).await
            }
            ReplyBody::ServerInformation { information } => {
                connection
                    .reply(
                        header,
                        &(
                            information.name,
                            information.vendor,
                            information.version,
                            information.spec_version,
                        ),
                    )
                    .await
            }
            ReplyBody::Rejected { reason } => {
                let name = rejection(&reason);
                connection.reply_dbus_error(header, name).await
            }
        }
    })
}

/// The D-Bus error an unusable request becomes.
fn rejection(error: &ActionError) -> ServiceError {
    ServiceError::InvalidActions(error.to_string())
}

/// Answers a call with one of the errors this service sends.
fn reply_error(
    connection: &Connection,
    header: &Header<'_>,
    error: impl zbus::DBusError,
) -> zbus::Result<()> {
    zbus::block_on(async { connection.reply_dbus_error(header, error).await })
}

/// The interface a reflection-based proxy asks about first.
const INTROSPECTABLE: &str = "org.freedesktop.DBus.Introspectable";

/// Its one method.
const INTROSPECT: &str = "Introspect";

/// The interface a proxy loads properties through.
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// The one method of it this service answers.
const GET_ALL: &str = "GetAll";

/// The interface every D-Bus peer may be asked about.
const PEER: &str = "org.freedesktop.DBus.Peer";

/// The liveness check of it.
const PING: &str = "Ping";

/// Answers a call the daemon has no part in.
///
/// A client that builds a proxy before it calls `Notify` asks these first, and
/// there is no state and no policy in any of the answers — which is exactly why
/// they are answered here instead of travelling through the application and
/// waiting for a frame to be drawn.
fn reply_direct(connection: &Connection, header: &Header<'_>, direct: &Direct) -> zbus::Result<()> {
    zbus::block_on(async {
        match direct {
            Direct::Introspection => connection.reply(header, &introspection_xml()).await,
            // An empty table is the whole truth: `org.freedesktop.Notifications` has
            // no properties, and a proxy that asked for them must not be told the
            // call failed.
            Direct::Properties => {
                connection
                    .reply(header, &HashMap::<String, OwnedVariant>::new())
                    .await
            }
            Direct::Ping => connection.reply(header, &()).await,
        }
    })
}

/// The introspection XML this service answers with.
///
/// Written from [`crate::protocol::METHODS`] and
/// [`crate::protocol::SIGNALS`] rather than by hand: the four methods, their
/// argument names and their signatures have exactly one definition in this
/// crate, and a proxy that reflects sees the same interface a caller who reads
/// [`crate::protocol`] does.
pub fn introspection_xml() -> String {
    let mut xml = String::from(
        "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\" \
         \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n<node>\n  <interface name=\"",
    );
    xml.push_str(protocol::INTERFACE);
    xml.push_str("\">\n");

    for method in protocol::METHODS {
        xml.push_str("    <method name=\"");
        xml.push_str(method.name);
        xml.push_str("\">\n");

        for argument in method.arguments {
            xml.push_str("      <arg name=\"");
            xml.push_str(argument.name);
            xml.push_str("\" type=\"");
            xml.push_str(argument.signature);
            xml.push_str("\" direction=\"in\"/>\n");
        }

        for argument in method.returns {
            xml.push_str("      <arg name=\"");
            xml.push_str(argument.name);
            xml.push_str("\" type=\"");
            xml.push_str(argument.signature);
            xml.push_str("\" direction=\"out\"/>\n");
        }

        xml.push_str("    </method>\n");
    }

    for signal in protocol::SIGNALS {
        xml.push_str("    <signal name=\"");
        xml.push_str(signal.name);
        xml.push_str("\">\n");

        for argument in signal.arguments {
            xml.push_str("      <arg name=\"");
            xml.push_str(argument.name);
            xml.push_str("\" type=\"");
            xml.push_str(argument.signature);
            xml.push_str("\"/>\n");
        }

        xml.push_str("    </signal>\n");
    }

    xml.push_str("  </interface>\n</node>\n");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Hints;
    use crate::protocol::{HINT_CATEGORY, HINT_URGENCY, Urgency};

    fn owned(value: HintValue) -> OwnedVariant {
        match value {
            HintValue::Byte(raw) => OwnedVariant::from(raw),
            HintValue::Boolean(flag) => OwnedVariant::from(flag),
            HintValue::Int(raw) => OwnedVariant::from(raw),
            HintValue::Unsigned(raw) => OwnedVariant::from(raw),
            HintValue::Text(text) => OwnedVariant::from(zbus::Str::from(text)),
        }
    }

    #[test]
    fn a_name_another_daemon_owns_is_a_refusal_and_not_a_takeover() {
        let error = name_error(zbus::Error::NameTaken);

        assert!(matches!(
            error,
            BusError::NameTaken {
                name: "org.freedesktop.Notifications"
            }
        ));
        assert!(
            error.to_string().contains("another notification daemon"),
            "the message has to be actionable: {error}"
        );
    }

    #[test]
    fn any_other_refusal_keeps_the_bus_error_it_came_from() {
        let error = name_error(zbus::Error::Failure("the bus said no".to_owned()));

        assert!(matches!(error, BusError::Claim { .. }));
        assert!(error.to_string().contains("the bus said no"));
    }

    #[test]
    fn the_deadline_is_stored_as_whole_milliseconds() {
        assert_eq!(millis(REPLY_DEADLINE), 5_000);
        assert_eq!(millis(Duration::from_millis(1)), 1);
        assert_eq!(millis(Duration::ZERO), 0);
        assert_eq!(millis(Duration::from_secs(86_400 * 365)), 31_536_000_000);
    }

    #[test]
    fn the_hints_this_daemon_reads_survive_every_encoding_it_carries() {
        let stats = Stats::default();
        let mut pairs = HashMap::new();
        pairs.insert(HINT_URGENCY.to_owned(), owned(HintValue::Byte(2)));
        pairs.insert(
            HINT_CATEGORY.to_owned(),
            owned(HintValue::Text("im".to_owned())),
        );
        pairs.insert("x-inox-boolean".to_owned(), owned(HintValue::Boolean(true)));

        let hints = Hints::from_pairs(&read_hints(pairs, &stats));

        assert_eq!(hints.urgency, Urgency::Critical);
        assert_eq!(hints.category.as_deref(), Some("im"));
        assert_eq!(hints.ignored, 1, "the unknown key is counted, not dropped");
        assert_eq!(stats.report().hints_dropped, 0);
        assert_eq!(stats.report().hints_unsupported, 0);
    }

    #[test]
    fn a_value_with_no_hint_form_is_dropped_and_counted() {
        let stats = Stats::default();
        let mut pairs = HashMap::new();
        // A double and a 64-bit integer: both are legal D-Bus hint values and
        // neither has a `HintValue` form, so neither is guessed at.
        pairs.insert(HINT_URGENCY.to_owned(), OwnedVariant::from(1.5_f64));
        pairs.insert(HINT_CATEGORY.to_owned(), OwnedVariant::from(2_i64));

        let hints = Hints::from_pairs(&read_hints(pairs, &stats));

        assert_eq!(hints, Hints::default(), "nothing was guessed from them");
        assert_eq!(stats.report().hints_unsupported, 2);
        assert_eq!(stats.report().hints_dropped, 0);
    }

    #[test]
    fn more_hints_than_the_cap_keeps_the_standard_ones_and_counts_the_rest() {
        let stats = Stats::default();
        let mut pairs = HashMap::new();
        pairs.insert(HINT_URGENCY.to_owned(), owned(HintValue::Byte(2)));

        for index in 0..(MAX_HINTS * 2) {
            pairs.insert(
                format!("x-padding-{index:03}"),
                owned(HintValue::Text("x".to_owned())),
            );
        }

        let read = read_hints(pairs, &stats);
        let hints = Hints::from_pairs(&read);

        assert_eq!(read.len(), MAX_HINTS, "the cap is the bound");
        assert_eq!(
            hints.urgency,
            Urgency::Critical,
            "padding the table cannot starve the hints this daemon reads"
        );
        assert_eq!(
            stats.report().hints_dropped,
            (MAX_HINTS * 2 - (MAX_HINTS - 1)) as u64
        );
    }

    #[test]
    fn the_cap_is_read_in_a_fixed_order_whatever_the_table_iterates_like() {
        let mut first = HashMap::new();
        let mut second = HashMap::new();

        for index in 0..(MAX_HINTS * 2) {
            let key = format!("x-padding-{index:03}");
            let value = owned(HintValue::Unsigned(index as u32));
            first.insert(key.clone(), value.clone());
            second.insert(key, value);
        }

        let stats = Stats::default();
        let mut left = read_hints(first, &stats);
        let mut right = read_hints(second, &stats);

        assert_eq!(left, right, "the same call reads the same way twice");
        left.sort_by(|a, b| a.0.cmp(&b.0));
        right.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(left, right);
    }

    #[test]
    fn every_refusal_the_daemon_can_make_becomes_a_named_error() {
        let error = rejection(&ActionError::MalformedPairs { count: 3 });

        assert_eq!(
            zbus::DBusError::name(&error).as_str(),
            "org.freedesktop.Notifications.Error.InvalidActions"
        );
        assert!(
            zbus::DBusError::description(&error).is_some_and(|text| text.contains("3 strings")),
            "the client is told what was wrong with the array"
        );

        assert_eq!(
            zbus::DBusError::name(&ServiceError::Timeout("late".to_owned())).as_str(),
            "org.freedesktop.Notifications.Error.Timeout"
        );
        assert_eq!(
            zbus::DBusError::name(&ServiceError::Unavailable("nobody".to_owned())).as_str(),
            "org.freedesktop.Notifications.Error.Unavailable"
        );
    }

    #[test]
    fn the_introspection_xml_names_the_contract_and_nothing_else() {
        let xml = introspection_xml();

        assert!(xml.starts_with("<!DOCTYPE node PUBLIC"));
        assert!(xml.contains(&format!("<interface name=\"{}\">", protocol::INTERFACE)));
        assert!(xml.ends_with("</node>\n"));

        for method in protocol::METHODS {
            assert!(
                xml.contains(&format!("<method name=\"{}\">", method.name)),
                "`{}` is missing from the introspection answer",
                method.name
            );

            for argument in method.arguments {
                assert!(xml.contains(&format!(
                    "<arg name=\"{}\" type=\"{}\" direction=\"in\"/>",
                    argument.name, argument.signature
                )));
            }

            for argument in method.returns {
                assert!(xml.contains(&format!(
                    "<arg name=\"{}\" type=\"{}\" direction=\"out\"/>",
                    argument.name, argument.signature
                )));
            }
        }

        for signal in protocol::SIGNALS {
            assert!(xml.contains(&format!("<signal name=\"{}\">", signal.name)));
        }

        assert!(
            !xml.contains("<property"),
            "this interface has no properties"
        );
    }

    #[test]
    fn an_answer_for_an_earlier_call_is_not_mistaken_for_this_one() {
        let (sender, receiver) = std::sync::mpsc::channel();

        sender
            .send(Reply {
                call: CallId::new(1),
                body: ReplyBody::Closed { closed: true },
            })
            .expect("the channel is open");

        assert!(
            matches!(
                await_reply(&receiver, CallId::new(2), REPLY_DEADLINE),
                Err(Failed::Timeout)
            ),
            "another call's answer is not this call's answer"
        );
    }
}
