//! What replaced `iced::Task` and `iced::Subscription`: the pump the Qt host drives.
//!
//! The Elm shape is kept — [`crate::app::update`] still takes one [`Message`] and
//! returns a task — but nothing here is an async runtime. The daemon's sources
//! are explicit and registered by the host, as docs/qml-migration-contract.md §5
//! requires:
//!
//! - **The transports** (`crate::bus`, `crate::lock_state`) deliver into
//!   [`crate::app::Inbox`], a bounded `std::sync::mpsc` channel. Their threads
//!   never touch [`crate::app::State`], and `State` never leaves the Qt thread.
//! - **The configuration bridge** ([`crate::reload`]) hands over prepared reloads
//!   on the same kind of channel.
//! - **The expiry sweep** is a [`TICK`] the pump raises itself; QML drives the
//!   pump from a `Timer`, so an idle daemon with nothing held is not woken at all.
//!
//! [`Runtime::pump`] is therefore the single place where background threads meet
//! the interface: it drains the inbox, applies each message through
//! [`crate::app::update`], hands what the daemon decided to the transport, and
//! recomputes the JSON projection when something changed. Nothing in this module
//! knows that QML exists — the bridge in [`crate::cxxqt_object`] is what turns
//! "the projection changed" into a property assignment on the Qt thread.
//!
//! Two more things live here because the migration needed them and neither is
//! worth a module of its own: [`Task`], the minimal task type `app::update`
//! returns (this daemon has no side effect that needs one), and [`Reply`], a
//! one-shot future the configuration bus awaits while the Qt thread does the work.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::actions::ActionId;
use crate::app::{self, Inbox, Message, State};
use crate::bus::Service;
use crate::queue::NotificationId;
use crate::reload;
use crate::view;

/// How often the daemon sweeps for notifications that have run out of time.
///
/// A quarter of a second is far below the shortest lifetime a popup is given by
/// default (five seconds) and well above the point where the wake-up matters; the
/// sweep only runs while something is held, so an idle daemon is not woken at all.
pub const TICK: Duration = Duration::from_millis(250);

/// One thing the update loop asked for, to be carried out by the host.
///
/// This daemon returns no task from [`crate::app::update`] — every effect leaves
/// through [`State::take_outgoing`] — so the type exists to keep the Elm shape
/// the rest of the L3 components have and to make "the update has nothing more to
/// say" a value rather than a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task<T> {
    messages: VecDeque<T>,
}

impl<T> Task<T> {
    /// The update has nothing else to do.
    pub fn none() -> Self {
        Self {
            messages: VecDeque::new(),
        }
    }

    /// One message the host should feed back into the update loop.
    #[allow(dead_code)]
    pub fn done(message: T) -> Self {
        Self {
            messages: VecDeque::from([message]),
        }
    }

    /// Several tasks, run in order.
    #[allow(dead_code)]
    pub fn batch(tasks: impl IntoIterator<Item = Self>) -> Self {
        Self {
            messages: tasks.into_iter().flat_map(|task| task.messages).collect(),
        }
    }

    /// Rewrites the messages a task carries.
    #[allow(dead_code)]
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Task<U> {
        Task {
            messages: self.messages.into_iter().map(&mut f).collect(),
        }
    }

    /// Whether there is nothing to carry out.
    pub fn is_none(&self) -> bool {
        self.messages.is_empty()
    }

    /// Takes the messages the host should feed back, in order.
    pub fn into_messages(self) -> Vec<T> {
        self.messages.into()
    }
}

/// Why a [`Reply`] never received a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the daemon dropped the completion without answering")]
pub struct Abandoned;

/// The giving half of a one-shot reply channel.
#[derive(Debug)]
pub struct Completer<T> {
    inner: Arc<ReplyInner<T>>,
}

/// The waiting half: a future that resolves once, or reports the daemon dropped it.
#[derive(Debug)]
pub struct Reply<T> {
    inner: Arc<ReplyInner<T>>,
}

/// The shared state of a one-shot reply.
#[derive(Debug)]
struct ReplyInner<T> {
    slot: Mutex<Slot<T>>,
}

/// What a [`ReplyInner`] holds right now.
#[derive(Debug)]
enum Slot<T> {
    /// Nobody has answered yet; the waker of the waiter, once there is one.
    Waiting(Option<Waker>),
    /// The answer arrived.
    Answered(T),
    /// The giving half was dropped.
    Abandoned,
}

/// A one-shot reply channel.
///
/// The configuration bus calls a method and the Qt thread answers it, so the
/// method has to await something. `futures-channel` is not a dependency of this
/// crate (it left with `iced`), and pulling an async runtime in for one oneshot
/// would be the opposite of what docs/qml-migration-contract.md §5 asks for, so
/// the three states and one `Waker` are written out here.
pub fn reply_channel<T>() -> (Completer<T>, Reply<T>) {
    let inner = Arc::new(ReplyInner {
        slot: Mutex::new(Slot::Waiting(None)),
    });

    (
        Completer {
            inner: Arc::clone(&inner),
        },
        Reply { inner },
    )
}

impl<T> Completer<T> {
    /// Answers the waiting future.
    ///
    /// # Errors
    ///
    /// Returns the value back when nobody is waiting any more: the caller asked,
    /// went away, and there is nothing left to tell.
    pub fn complete(self, value: T) -> Result<(), T> {
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        match &mut *slot {
            // Nobody is waiting: the value goes back to the caller rather than
            // into a slot nothing will ever read.
            Slot::Answered(_) | Slot::Abandoned => Err(value),
            Slot::Waiting(_) => {
                let waker = match std::mem::replace(&mut *slot, Slot::Answered(value)) {
                    Slot::Waiting(waker) => waker,
                    Slot::Answered(_) | Slot::Abandoned => None,
                };
                drop(slot);

                if let Some(waker) = waker {
                    waker.wake();
                }

                Ok(())
            }
        }
    }
}

impl<T> Drop for Completer<T> {
    fn drop(&mut self) {
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Only a waiter is turned into `Abandoned`: an answer that has already
        // been written (`Slot::Answered`) must survive the giving half going
        // away, or every `complete` would be undone by the completer's own drop
        // right after it returned.
        let waker = match &mut *slot {
            Slot::Waiting(waker) => waker.take(),
            Slot::Answered(_) | Slot::Abandoned => return,
        };

        *slot = Slot::Abandoned;
        drop(slot);

        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<T> Future for Reply<T> {
    type Output = Result<T, Abandoned>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self
            .inner
            .slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        match &mut *slot {
            Slot::Answered(_) => match std::mem::replace(&mut *slot, Slot::Abandoned) {
                Slot::Answered(value) => Poll::Ready(Ok(value)),
                Slot::Waiting(_) | Slot::Abandoned => Poll::Ready(Err(Abandoned)),
            },
            Slot::Abandoned => Poll::Ready(Err(Abandoned)),
            Slot::Waiting(waker) => {
                *waker = Some(context.waker().clone());

                Poll::Pending
            }
        }
    }
}

/// An action QML reported, as the JSON the `dispatch` invokable carries.
///
/// The bridge parses one JSON object into this and this into an
/// [`crate::app::Message`]; nothing else interprets the strings QML sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    /// The close button of a notification.
    Close {
        /// The notification.
        id: u32,
    },
    /// One of a notification's action buttons.
    Action {
        /// The notification.
        id: u32,
        /// The client's own key for the action.
        key: String,
    },
    /// The notification itself, which invokes its default action.
    Invoke {
        /// The notification.
        id: u32,
    },
    /// A link in a notification's body.
    Link {
        /// The notification it was in.
        id: u32,
        /// The target the sanitiser accepted.
        href: String,
    },
    /// The panel flipped do not disturb.
    ToggleDoNotDisturb,
    /// The panel set do not disturb.
    SetDoNotDisturb {
        /// Whether it is now on.
        on: bool,
    },
}

/// Why an action QML sent could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    /// The string was not a JSON object.
    #[error("the action is not a JSON object")]
    Malformed,
    /// The object carried no `type`, or one this crate does not know.
    #[error("the action names no type this daemon knows")]
    UnknownType,
    /// The object named a type but not a field it needs.
    #[error("the action is missing the `{field}` field")]
    Missing {
        /// The field that is missing.
        field: &'static str,
    },
    /// The object carried an identifier no notification can have.
    #[error("the `{field}` field is not a notification identifier")]
    InvalidId {
        /// The field that is out of range.
        field: &'static str,
    },
}

impl UiAction {
    /// Reads one action out of the JSON string QML sends.
    ///
    /// # Errors
    ///
    /// [`ActionError`] when the string is not an object of a known type; the
    /// bridge logs that and does nothing else (a malformed action must never take
    /// the daemon down).
    pub fn parse(json: &str) -> Result<Self, ActionError> {
        let value: Value = serde_json::from_str(json).map_err(|_| ActionError::Malformed)?;
        let object = value.as_object().ok_or(ActionError::Malformed)?;
        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ActionError::UnknownType)?;

        match kind {
            "close" => Ok(Self::Close {
                id: notification_id(object, "id")?,
            }),
            "action" => Ok(Self::Action {
                id: notification_id(object, "id")?,
                key: text(object, "key")?,
            }),
            "invoke" => Ok(Self::Invoke {
                id: notification_id(object, "id")?,
            }),
            "link" => Ok(Self::Link {
                id: notification_id(object, "id")?,
                href: text(object, "href")?,
            }),
            "do-not-disturb-toggle" => Ok(Self::ToggleDoNotDisturb),
            "do-not-disturb" => Ok(Self::SetDoNotDisturb {
                on: object.get("on").and_then(Value::as_bool).unwrap_or(false),
            }),
            _ => Err(ActionError::UnknownType),
        }
    }

    /// The message this action becomes.
    pub fn message(self) -> Message {
        match self {
            Self::Close { id } => Message::Dismissed {
                id: NotificationId::new(id),
            },
            Self::Action { id, key } => Message::ActionInvoked {
                id: NotificationId::new(id),
                action: ActionId::new(&key),
            },
            Self::Invoke { id } => Message::BodyClicked {
                id: NotificationId::new(id),
            },
            Self::Link { id, href } => Message::LinkFollowed {
                id: NotificationId::new(id),
                href,
            },
            Self::ToggleDoNotDisturb => Message::DoNotDisturbToggled,
            Self::SetDoNotDisturb { on } => Message::DoNotDisturbSet(on),
        }
    }
}

/// One notification identifier in an action object.
///
/// The identifier is the queue's own (`u32`); a number QML could not have got
/// from the projection is refused rather than truncated into a different
/// notification.
fn notification_id(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<u32, ActionError> {
    let raw = object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(ActionError::Missing { field })?;

    u32::try_from(raw).map_err(|_| ActionError::InvalidId { field })
}

/// One string field of an action object.
fn text(
    object: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<String, ActionError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(ActionError::Missing { field })
}

/// The daemon as the Qt thread drives it.
///
/// One per process. Everything that is not the projection lives behind this
/// value, and the Qt object in [`crate::cxxqt_object`] owns exactly one of it.
pub struct Runtime {
    state: State,
    messages: std::sync::mpsc::Receiver<Message>,
    service: Option<Service>,
    reload: Option<reload::Host>,
    last_tick: Instant,
    projection: String,
    dirty: bool,
}

impl Runtime {
    /// The daemon, with no transport attached yet.
    ///
    /// The transports are attached by the host once they exist ([`Runtime::attach`]
    /// and [`Runtime::attach_reload`]); a daemon without them is what the
    /// `--self-check` diagnostic runs, so that the QML surface can be verified on a
    /// machine with no session bus at all.
    pub fn new(state: State, inbox: &Inbox) -> Self {
        let projection = projection_of(&state);

        Self {
            state,
            messages: inbox.stream(),
            service: None,
            reload: None,
            last_tick: Instant::now(),
            projection,
            dirty: false,
        }
    }

    /// Attaches the D-Bus transport the daemon answers calls through.
    pub fn attach(&mut self, service: Service) {
        self.service = Some(service);
    }

    /// Attaches the configuration bridge (SIGHUP and `org.inox.Configuration1`).
    pub fn attach_reload(&mut self, host: reload::Host) {
        self.reload = Some(host);
    }

    /// The state, for the host's own reporting.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The current projection, as the JSON string the bridge hands to QML.
    pub fn model_json(&self) -> &str {
        &self.projection
    }

    /// Applies one action QML reported.
    ///
    /// # Errors
    ///
    /// [`ActionError`] when the string is not an action this daemon knows; the
    /// caller logs it and carries on.
    pub fn dispatch(&mut self, action: &str) -> Result<(), ActionError> {
        let action = UiAction::parse(action)?;
        self.apply(action.message());

        Ok(())
    }

    /// Drains what arrived, sweeps what expired, and answers whether the surface
    /// has to be redrawn.
    ///
    /// The transports are drained one message at a time, each followed by the
    /// delivery of what it decided: batching would let the transport's own bound
    /// drop a reply a client is waiting for (crates/inox-notifications/AGENTS.md,
    /// "一次只服务一个调用").
    pub fn pump(&mut self) -> bool {
        for _ in 0..app::INBOX_DEPTH {
            match self.messages.try_recv() {
                Ok(message) => self.apply(message),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                // Every transport has gone away; nothing else will arrive.
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }

        self.poll_reloads();
        self.sweep();

        if self.dirty {
            self.dirty = false;
            let projection = projection_of(&self.state);

            if projection != self.projection {
                self.projection = projection;

                return true;
            }
        }

        false
    }

    /// Applies one message, then hands what it decided to the transport.
    fn apply(&mut self, message: Message) {
        let mut pending = VecDeque::from([message]);

        while let Some(message) = pending.pop_front() {
            let task = app::update(&mut self.state, message);
            pending.extend(task.into_messages());
            self.deliver();
        }

        self.dirty = true;
    }

    /// Hands everything the daemon decided to the transport.
    fn deliver(&mut self) {
        let outgoing = self.state.take_outgoing();

        if let Some(service) = &self.service {
            for outgoing in outgoing {
                service.deliver(outgoing);
            }
        }
    }

    /// Applies every configuration reload the bridge has prepared.
    fn poll_reloads(&mut self) {
        while let Some(request) = self.reload.as_ref().and_then(reload::Host::poll) {
            let result = apply_reload(&mut self.state, &request);
            request.finish(result);

            if self.state.config().display.max_visible > 0 {
                // The window is sized from the display configuration, which the
                // projection carries: a reload that changes it resizes the
                // surface through the new model rather than through a message.
                self.dirty = true;
            }
        }
    }

    /// Raises the expiry sweep while anything is held.
    fn sweep(&mut self) {
        if self.state.queue().is_empty() {
            return;
        }

        if self.last_tick.elapsed() < TICK {
            return;
        }

        self.last_tick = Instant::now();
        self.apply(Message::Tick(self.last_tick));
    }
}

/// Applies a prepared reload to the running daemon.
///
/// A reload that would change the placement is refused: live desktop placement
/// needs a layer-shell host, which this component does not have, and
/// pretending otherwise would move the stack somewhere the user did not ask for.
fn apply_reload(state: &mut State, request: &reload::Request) -> Result<(), String> {
    if !request.is_current() {
        return Err("configuration reload expired".to_owned());
    }

    if state.config().display.position != request.config.notifications.display.position {
        return Err("display.position: live desktop placement requires a layer-shell host".into());
    }

    let localizer = std::rc::Rc::new(
        crate::i18n::load(&request.config.locale).map_err(|error| error.to_string())?,
    );
    state.apply_configuration(
        request.config.notifications.clone(),
        request.config.theme.clone(),
        localizer,
    );

    Ok(())
}

/// The projection of `state`, as the JSON string QML parses.
fn projection_of(state: &State) -> String {
    view::project(
        &state.model(),
        state.tokens(),
        &state.config().display,
        &app::title(state),
    )
    .to_string()
}

/// Whether an action string is one this daemon knows, without applying it.
#[cfg(test)]
fn is_known_action(action: &str) -> bool {
    UiAction::parse(action).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;
    use inox_config::{NotificationsConfig, Theme as Tokens};
    use std::rc::Rc;

    fn state() -> State {
        State::new(
            NotificationsConfig::default(),
            Tokens::builtin_dark(),
            Rc::new(crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse")),
        )
    }

    fn runtime() -> (Runtime, Inbox) {
        let inbox = Inbox::new();
        let runtime = Runtime::new(state(), &inbox);

        (runtime, inbox)
    }

    #[test]
    fn a_fresh_projection_is_valid_json_with_the_expected_shape() {
        let (runtime, _inbox) = runtime();
        let value: Value =
            serde_json::from_str(runtime.model_json()).expect("the projection is JSON");

        assert_eq!(value["stack"]["status"], "empty");
        assert_eq!(value["window"]["title"], "Inox Notifications");
        assert!(value["palette"]["background"].as_str().is_some());
    }

    #[test]
    fn a_message_a_transport_delivers_reaches_the_projection() {
        let (mut runtime, inbox) = runtime();
        let id = admit(&mut runtime, &inbox, "a build finished");

        let value: Value =
            serde_json::from_str(runtime.model_json()).expect("the projection is JSON");

        assert_eq!(value["stack"]["popups"][0]["summary"], "a build finished");
        assert_eq!(
            value["stack"]["popups"][0]["id"].as_u64(),
            Some(u64::from(id))
        );
        assert_eq!(value["stack"]["status"], "showing");
    }

    /// Delivers one `Notify` call through the inbox and returns the identifier.
    fn admit(runtime: &mut Runtime, inbox: &Inbox, summary: &str) -> u32 {
        let request = crate::contract::Request::new(
            crate::contract::CallId::new(1),
            crate::contract::RequestBody::Notify(crate::contract::NotifyRequest {
                summary: summary.to_owned(),
                ..crate::contract::NotifyRequest::default()
            }),
        );

        inbox
            .deliver(Message::Request(request))
            .expect("the daemon is reading");
        assert!(runtime.pump(), "the projection changed");

        let value: Value =
            serde_json::from_str(runtime.model_json()).expect("the projection is JSON");

        u32::try_from(
            value["stack"]["popups"][0]["id"]
                .as_u64()
                .expect("a notification identifier"),
        )
        .expect("the identifier fits")
    }

    #[test]
    fn pumping_nothing_changes_nothing() {
        let (mut runtime, _inbox) = runtime();

        assert!(!runtime.pump(), "an idle daemon does not redraw");
    }

    #[test]
    fn a_dispatch_closes_a_notification_and_redraws() {
        let (mut runtime, inbox) = runtime();
        let id = admit(&mut runtime, &inbox, "a build finished");

        runtime
            .dispatch(&format!("{{\"type\":\"close\",\"id\":{id}}}"))
            .expect("a known action");

        assert!(
            runtime.pump(),
            "closing a popup changes what the surface draws"
        );

        let value: Value =
            serde_json::from_str(runtime.model_json()).expect("the projection is JSON");

        assert_eq!(value["stack"]["status"], "empty");
        assert_eq!(
            runtime.state().queue().counters().closed_by_call,
            0,
            "a dismissal is not a `CloseNotification` call"
        );
        assert_eq!(runtime.state().queue().counters().dismissed, 1);
    }

    #[test]
    fn an_action_press_is_at_most_once_even_from_qml() {
        let (mut runtime, inbox) = runtime();

        let request = crate::contract::Request::new(
            crate::contract::CallId::new(1),
            crate::contract::RequestBody::Notify(crate::contract::NotifyRequest {
                summary: "with actions".to_owned(),
                actions: vec!["open".to_owned(), "Open".to_owned()],
                ..crate::contract::NotifyRequest::default()
            }),
        );
        inbox
            .deliver(Message::Request(request))
            .expect("the daemon is reading");
        let _ = runtime.pump();

        let action = "{\"type\":\"action\",\"id\":1,\"key\":\"open\"}";
        runtime.dispatch(action).expect("a known action");
        runtime.dispatch(action).expect("the same action again");
        let _ = runtime.pump();

        assert_eq!(
            runtime.state().rejected_invocations(),
            1,
            "the second press is refused, not repeated"
        );
    }

    #[test]
    fn a_malformed_action_is_refused_and_changes_nothing() {
        let (mut runtime, _inbox) = runtime();

        for action in [
            "",
            "not json",
            "[]",
            "{}",
            "{\"type\":\"nonsense\"}",
            "{\"type\":\"close\"}",
            "{\"type\":\"action\",\"id\":1}",
        ] {
            assert!(
                runtime.dispatch(action).is_err(),
                "`{action}` must be refused"
            );
            assert!(!runtime.pump(), "and must not redraw anything");
        }

        assert!(runtime.state().queue().is_empty());
    }

    #[test]
    fn every_action_qml_sends_maps_to_the_message_the_state_machine_knows() {
        let cases = [
            ("{\"type\":\"close\",\"id\":7}", "close"),
            ("{\"type\":\"invoke\",\"id\":7}", "invoke"),
            (
                "{\"type\":\"link\",\"id\":7,\"href\":\"https://example.com\"}",
                "link",
            ),
            ("{\"type\":\"action\",\"id\":7,\"key\":\"open\"}", "action"),
            ("{\"type\":\"do-not-disturb-toggle\"}", "toggle"),
            ("{\"type\":\"do-not-disturb\",\"on\":true}", "set"),
        ];

        for (json, expected) in cases {
            let message = UiAction::parse(json).expect("a known action").message();

            match (message, expected) {
                (Message::Dismissed { id }, "close") => {
                    assert_eq!(id, NotificationId::new(7));
                }
                (Message::BodyClicked { id }, "invoke") => {
                    assert_eq!(id, NotificationId::new(7));
                }
                (Message::LinkFollowed { id, href }, "link") => {
                    assert_eq!(id, NotificationId::new(7));
                    assert_eq!(href, "https://example.com");
                }
                (Message::ActionInvoked { id, action }, "action") => {
                    assert_eq!(id, NotificationId::new(7));
                    assert_eq!(action, ActionId::new("open"));
                }
                (Message::DoNotDisturbToggled, "toggle") => {}
                (Message::DoNotDisturbSet(on), "set") => assert!(on),
                (other, expected) => panic!("`{expected}` became {other:?}"),
            }
        }

        assert!(is_known_action("{\"type\":\"close\",\"id\":1}"));
        assert!(!is_known_action("{\"type\":\"close\"}"));
    }

    #[test]
    fn a_task_carries_messages_and_none_carries_none() {
        let none: Task<Message> = Task::none();
        assert!(none.is_none());
        assert!(none.into_messages().is_empty());

        let done = Task::done(Message::DoNotDisturbToggled);
        assert!(!done.is_none());
        assert!(matches!(
            done.into_messages().as_slice(),
            [Message::DoNotDisturbToggled]
        ));

        let batch = Task::batch([Task::done(1u8), Task::none(), Task::done(2)]);
        assert_eq!(batch.into_messages(), [1, 2]);

        let mapped = Task::done(1u8).map(|value| value + 1);
        assert_eq!(mapped.into_messages(), [2]);
    }

    #[test]
    fn a_reply_is_answered_once() {
        let (completer, reply) = reply_channel::<u32>();

        completer.complete(7).expect("the future is waiting");

        let mut reply = reply;
        assert_eq!(poll_once(&mut reply), Poll::Ready(Ok(7)));
    }

    #[test]
    fn a_reply_nobody_answers_reports_the_daemon_went_away() {
        let (completer, reply) = reply_channel::<u32>();
        drop(completer);

        let mut reply = reply;
        assert_eq!(poll_once(&mut reply), Poll::Ready(Err(Abandoned)));
    }

    #[test]
    fn a_reply_completed_before_the_waiter_polls_is_still_delivered() {
        let (completer, reply) = reply_channel::<u32>();
        completer.complete(11).expect("the future is waiting");

        let mut reply = reply;
        assert_eq!(poll_once(&mut reply), Poll::Ready(Ok(11)));
    }

    /// Polls a future once with a no-op waker.
    ///
    /// Enough for the reply tests: a reply that is already answered resolves on
    /// the first poll, one whose giver has gone reports that, and the pending case
    /// is asserted to be pending rather than waited on. No executor is involved
    /// (and none is in the crate); `Waker::noop` is the standard library's own
    /// no-op waker, so this needs no `unsafe`.
    fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
        let waker = std::task::Waker::noop();
        let mut context = Context::from_waker(waker);

        Pin::new(future).poll(&mut context)
    }

    #[test]
    fn a_reply_parks_its_waiter_until_it_is_answered() {
        let (completer, mut reply) = reply_channel::<u32>();

        assert!(poll_once(&mut reply).is_pending());

        completer.complete(5).expect("the waiter is there");

        assert_eq!(poll_once(&mut reply), Poll::Ready(Ok(5)));
    }

    #[test]
    fn a_reply_answers_once_and_then_closes() {
        let (completer, mut reply) = reply_channel::<u32>();
        completer.complete(5).expect("the waiter is there");

        assert_eq!(poll_once(&mut reply), Poll::Ready(Ok(5)));
        assert_eq!(poll_once(&mut reply), Poll::Ready(Err(Abandoned)));
    }
}
