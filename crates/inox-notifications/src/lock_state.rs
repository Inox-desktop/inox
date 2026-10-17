//! The session lock state, as the compositor reports it.
//!
//! Acceptance D06 asks that a locked session hide the sensitive parts of a
//! notification. The lock itself is the compositor's (ADR 0015: the lock state
//! is not a UI client's to keep), and the compositor publishes it over the
//! private `inox-ipc` protocol as the `session_lock_state` event (ADR 0022 §4).
//! This module is the reader of that event and nothing else:
//!
//! ```text
//! compositor ──inox-ipc: session_lock_state──► this watcher ──Message──► app::State
//! ```
//!
//! Three things about it are load-bearing, and all three are about *not* doing
//! more than the compositor said:
//!
//! - **It is not a capability and not a permission.** The event is a statement
//!   by the authority to an observer. Binding it grants nothing, and a client
//!   that never binds it loses nothing; the daemon reads it only to decide
//!   whether to withhold what it draws.
//! - **Silence is not "unlocked".** If there is no compositor, no `inox_ipc`
//!   global, or no version 2 of it, the watcher is [`Tracking::Disabled`] with
//!   the reason as data, and it reports *nothing*: it does not invent an
//!   unlocked session, because a state that was never observed is not a state.
//!   The daemon keeps the last state it was told and keeps running — it is not
//!   the compositor's supervisor, and a session with no lock transport is still
//!   a session.
//! - **A state the daemon was never told is told again.** The first
//!   `session_lock_state` can arrive before the application has built its
//!   subscription, so a state that could not be handed over is kept and retried
//!   rather than dropped; the compositor sends its state on bind and on every
//!   change, and losing the first one would leave a locked session's popups
//!   showing their bodies.
//!
//! The watcher owns its own [`Connection`] and its own [`EventQueue`] on its own
//! thread, and never touches [`crate::app::State`], which is not `Send`. What it
//! can do is [`Inbox::deliver`], which is the same door the bus thread knocks
//! on.
//!
//! **What is not here:** reconnecting. If the compositor goes away the watcher
//! logs it and stops; a compositor that dies ends the session (ADR 0022,
//! failure semantics), and a daemon that reconnected to a *new* compositor
//! would be guessing about a session it knows nothing about.

use std::ops::RangeInclusive;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tracing::{debug, warn};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};

use inox_protocols::ipc::inox_ipc::client::inox_ipc::{Event as IpcEvent, InoxIpc, LockState};

use crate::app::{Inbox, Message};
use crate::policy::SessionState;

/// The thread the lock state is followed on.
const THREAD: &str = "inox-notifications lock state";

/// The global the compositor advertises the private control protocol under.
const IPC_INTERFACE: &str = "inox_ipc";

/// The version of `inox-ipc` that carries `session_lock_state`.
///
/// Version 2 is where ADR 0022 §4 put the event. A compositor that only offers
/// version 1 has no lock state to report, which is a reason to be disabled and
/// not a reason to assume the session is unlocked.
const IPC_VERSIONS: RangeInclusive<u32> = 2..=2;

/// How long a state waits for the application to be reading.
///
/// The compositor sends `session_lock_state` as soon as the global is bound,
/// which is before the application has built its subscription; waiting is
/// bounded, and losing the state would leave a locked session's popups showing
/// their bodies.
const HANDOVER_GRACE: Duration = Duration::from_secs(5);

/// What the watcher is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tracking {
    /// The compositor's lock state is being followed, and every change is
    /// handed to the daemon.
    Attached,
    /// The lock state cannot be observed, and this is why.
    ///
    /// The daemon keeps running and keeps the last state it was told; it does
    /// not exit, and it does not claim a state it was never told.
    Disabled {
        /// Why, in a sentence a log or a diagnostic can carry.
        reason: String,
    },
}

/// A running lock-state watcher.
#[derive(Debug)]
pub struct Watcher {
    tracking: Arc<Mutex<Tracking>>,
    thread: Option<JoinHandle<()>>,
}

impl Watcher {
    /// What the watcher is doing now.
    pub fn tracking(&self) -> Tracking {
        self.tracking
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Whether the compositor's lock state is being followed.
    pub fn is_attached(&self) -> bool {
        matches!(self.tracking(), Tracking::Attached)
    }

    /// The thread the watcher runs on, when it was able to attach at all.
    pub fn thread(&self) -> Option<&JoinHandle<()>> {
        self.thread.as_ref()
    }
}

/// Starts following the session lock state, reporting it to `inbox`.
///
/// The connection and the bind happen on the calling thread, so that a
/// compositor this daemon cannot read the lock state from is a `warn!` at
/// startup rather than a surprise later. Everything after that — waiting for
/// events, and handing them over — happens on the watcher's own thread.
pub fn watch(inbox: Inbox) -> Watcher {
    match Client::attach(inbox) {
        Ok(client) => {
            let thread = thread::Builder::new()
                .name(THREAD.to_owned())
                .spawn(move || client.follow());

            match thread {
                Ok(thread) => Watcher {
                    tracking: Arc::new(Mutex::new(Tracking::Attached)),
                    thread: Some(thread),
                },
                Err(error) => {
                    Watcher::disabled(format!("starting the lock-state watcher thread: {error}"))
                }
            }
        }
        Err(reason) => Watcher::disabled(reason),
    }
}

impl Watcher {
    /// A watcher that could not attach, and says why.
    fn disabled(reason: String) -> Self {
        warn!(
            %reason,
            "the session lock state cannot be observed, so nothing will tell this daemon to \
             withhold notification bodies; it keeps running and keeps whatever it was last told"
        );

        Self {
            tracking: Arc::new(Mutex::new(Tracking::Disabled { reason })),
            thread: None,
        }
    }
}

/// The connection the lock state is read from.
struct Client {
    /// Kept so the object is not destroyed while it is being read from.
    _ipc: InoxIpc,
    /// The queue `session_lock_state` arrives on.
    queue: EventQueue<Report>,
    /// Where the events are recorded.
    report: Report,
}

impl Client {
    /// Connects to the compositor and binds the private control protocol.
    fn attach(inbox: Inbox) -> Result<Self, String> {
        let connection = Connection::connect_to_env().map_err(|error| {
            format!("there is no compositor to ask (WAYLAND_DISPLAY is not a socket this process can use): {error}")
        })?;

        let (globals, queue) = registry_queue_init::<Report>(&connection)
            .map_err(|error| format!("reading the compositor's globals: {error}"))?;

        let ipc = globals
            .bind::<InoxIpc, Report, ()>(&queue.handle(), IPC_VERSIONS, ())
            .map_err(|error| {
                format!(
                    "the compositor does not advertise `{IPC_INTERFACE}` at version 2, so it \
                     cannot say whether the session is locked: {error}"
                )
            })?;

        Ok(Self {
            _ipc: ipc,
            queue,
            report: Report::new(inbox),
        })
    }

    /// Reads the lock state until the connection ends.
    fn follow(mut self) {
        hand_over(&mut self.report);

        loop {
            match self.queue.blocking_dispatch(&mut self.report) {
                Ok(_) => {}
                Err(error) => {
                    warn!(
                        %error,
                        "the connection to the compositor ended, so the session lock state is \
                         no longer being followed; the daemon keeps the last state it was told \
                         and keeps running"
                    );

                    return;
                }
            }

            hand_over(&mut self.report);
        }
    }
}

/// Hands over whatever the daemon has not been told.
fn hand_over(report: &mut Report) {
    if !report.flush() {
        warn!(
            "the session lock state could not be handed to the daemon; the daemon keeps the last \
             state it was told until the compositor reports a change"
        );
    }
}

/// What the lock-state events are recorded into, and handed on from.
#[derive(Debug)]
struct Report {
    inbox: Inbox,
    /// The state the compositor last reported, whether or not it got through.
    current: Option<SessionState>,
    /// The state the daemon was last told.
    told: Option<SessionState>,
}

impl Report {
    /// A report that has heard nothing yet.
    fn new(inbox: Inbox) -> Self {
        Self {
            inbox,
            current: None,
            told: None,
        }
    }

    /// Records what the compositor said.
    fn record(&mut self, state: SessionState) {
        if self.current == Some(state) {
            return;
        }

        debug!(?state, "the compositor reported the session lock state");
        self.current = Some(state);
    }

    /// Hands the daemon what it has not been told.
    ///
    /// Returns whether there is nothing left to hand over. A state that could
    /// not be delivered is *not* recorded as told, so it is handed over again
    /// rather than lost.
    fn flush(&mut self) -> bool {
        let Some(state) = self.current else {
            return true;
        };

        if self.told == Some(state) {
            return true;
        }

        match self
            .inbox
            .deliver_within(Message::SessionLockChanged(state), HANDOVER_GRACE)
        {
            Ok(()) => {
                self.told = Some(state);
                true
            }
            Err(error) => {
                debug!(
                    %error,
                    ?state,
                    "the session lock state could not be handed to the daemon yet; it is kept \
                     and tried again"
                );

                false
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Report {
    fn event(
        _report: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
        // The globals are read once, before the loop: this watcher binds one
        // interface and has nothing to do with a global appearing or going away.
    }
}

impl Dispatch<InoxIpc, ()> for Report {
    fn event(
        report: &mut Self,
        _ipc: &InoxIpc,
        event: IpcEvent,
        _data: &(),
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
        let IpcEvent::SessionLockState { state: value } = event else {
            return;
        };

        match session_state(value) {
            Some(state) => report.record(state),
            None => warn!(
                "the compositor reported a session lock state this version of the daemon does \
                 not know; it is ignored rather than read as unlocked"
            ),
        }
    }
}

/// The [`SessionState`] an `inox-ipc` lock state value names.
///
/// A discriminant this version does not know is `None`, never
/// [`SessionState::Unlocked`]: reading an unknown state as "unlocked" is exactly
/// the mistake that would show the body of a notification on a locked screen.
fn session_state(value: WEnum<LockState>) -> Option<SessionState> {
    match value.into_result() {
        Ok(LockState::Unlocked) => Some(SessionState::Unlocked),
        Ok(LockState::Locked) => Some(SessionState::Locked),
        // `LockState` is `#[non_exhaustive]`: a state this version of the
        // protocol adds later reads as "not a state this daemon knows", which
        // keeps it away from `Unlocked` rather than guessing.
        Err(_) | Ok(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Message;

    fn report() -> (Inbox, Report) {
        let inbox = Inbox::new();
        let report = Report::new(inbox.clone());

        (inbox, report)
    }

    #[test]
    fn the_two_states_the_protocol_defines_read_as_themselves() {
        assert_eq!(
            session_state(WEnum::Value(LockState::Unlocked)),
            Some(SessionState::Unlocked)
        );
        assert_eq!(
            session_state(WEnum::Value(LockState::Locked)),
            Some(SessionState::Locked)
        );
    }

    #[test]
    fn a_state_this_version_does_not_know_is_not_read_as_unlocked() {
        assert_eq!(session_state(WEnum::Unknown(2)), None);
        assert_eq!(session_state(WEnum::Unknown(u32::MAX)), None);
        assert_ne!(
            session_state(WEnum::Unknown(9)),
            Some(SessionState::Unlocked),
            "an unknown state must never be the state that shows bodies"
        );
    }

    #[test]
    fn a_state_is_handed_over_once_and_not_again() {
        let (inbox, mut report) = report();
        let mut stream = inbox.stream();

        report.record(SessionState::Locked);
        assert!(report.flush(), "the state went out");
        assert_eq!(
            report.current,
            Some(SessionState::Locked),
            "and it is still the state the compositor last reported"
        );

        assert!(
            matches!(
                stream.try_recv().expect("the stream is alive"),
                Message::SessionLockChanged(SessionState::Locked)
            ),
            "the daemon was told the session is locked"
        );

        assert!(report.flush(), "flushing again has nothing to do");
        assert!(
            stream.try_recv().is_err(),
            "a repeated event is not a second message"
        );
    }

    #[test]
    fn a_state_that_could_not_be_handed_over_is_handed_over_again() {
        let (inbox, mut report) = report();

        report.record(SessionState::Locked);
        assert!(
            !report.flush(),
            "nothing is reading yet, so nothing was handed over"
        );
        assert_eq!(
            report.told, None,
            "a state that did not get through is not recorded as told"
        );

        let mut stream = inbox.stream();
        assert!(report.flush(), "the retry succeeds");
        assert_eq!(report.told, Some(SessionState::Locked));
        assert!(matches!(
            stream.try_recv().expect("the stream is alive"),
            Message::SessionLockChanged(SessionState::Locked)
        ));
    }

    #[test]
    fn a_repeated_report_of_the_same_state_is_not_a_second_message() {
        let (inbox, mut report) = report();
        let mut stream = inbox.stream();

        report.record(SessionState::Locked);
        assert!(report.flush());
        assert!(report.flush());

        assert!(matches!(
            stream.try_recv().expect("the stream is alive"),
            Message::SessionLockChanged(SessionState::Locked)
        ));

        report.record(SessionState::Unlocked);
        assert!(report.flush());
        assert!(
            matches!(
                stream.try_recv().expect("the stream is alive"),
                Message::SessionLockChanged(SessionState::Unlocked)
            ),
            "the change is what is handed over, not the repetition"
        );
    }

    #[test]
    fn a_watcher_that_cannot_attach_says_so_rather_than_claiming_a_state() {
        let watcher = Watcher::disabled("there is no compositor".to_owned());

        assert!(!watcher.is_attached());
        assert_eq!(
            watcher.tracking(),
            Tracking::Disabled {
                reason: "there is no compositor".to_owned()
            }
        );
        assert!(watcher.thread().is_none(), "nothing was started");
    }

    #[test]
    fn nothing_is_recorded_before_the_compositor_says_anything() {
        let (_inbox, mut report) = report();

        assert_eq!(report.current, None, "silence is not a state");
        assert!(report.flush(), "and there is nothing to hand over");
    }
}
