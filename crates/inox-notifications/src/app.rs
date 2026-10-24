//! The daemon's application: state, messages, and the Elm update loop.
//!
//! The shape is Iced's and ADR 0003 makes it mandatory: a [`State`], a
//! [`Message`] per thing that can happen, one [`update`] that is the only place
//! state changes, and a [`view()`] that only reads. Nothing in `update` performs
//! I/O — the daemon decides, and hands what it decided to the transport that will
//! carry it, which is [`Outgoing`].
//!
//! **The transport is [`crate::bus`]**, and the seam between it and this module
//! is two channels: [`Inbox`] is how a call (or the session lock state) gets in,
//! and [`State::take_outgoing`] is how an answer or a signal gets out. The
//! transport runs on its own threads and never touches [`State`], which is not
//! `Send`; the tests below drive the same messages the bus delivers, which is
//! what makes the behaviour acceptance D06 asks about — identifiers, close
//! reasons, at-most-once actions, the bound, do not disturb, the locked
//! redaction — testable without a session bus.
//!
//! Three rules are load-bearing and tested here rather than asserted in a
//! comment:
//!
//! - **A notification ends exactly once.** Every path that removes one emits
//!   exactly one `NotificationClosed` with the reason that applies, including
//!   the paths the user never sees (a full queue evicting the oldest).
//! - **An action fires at most once.** The invocation is recorded on the
//!   notification *and* the notification is closed, so a second click finds
//!   nothing to invoke: it is counted as a rejection and no second signal is
//!   emitted.
//! - **Do not disturb is the daemon's, and survives a reload.** The
//!   configuration's `do_not_disturb` is the value the next session starts with;
//!   [`Message::ConfigReloaded`] deliberately does not touch the running one.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use iced::{Element, Subscription};
use inox_common::i18n::Localizer;
use inox_config::{NotificationsConfig, Theme as Tokens};
use tracing::debug;

use crate::actions::ActionId;
use crate::contract::{Calls, Outgoing, Reply, ReplyBody, Request, RequestBody, Signal};
use crate::policy::{Policy, SessionState};
use crate::protocol::{CAPABILITIES, CloseReason, SERVER_INFORMATION};
use crate::queue::{NotificationId, Queue};
use crate::view;
use crate::view_model::{PopupStack, StackInput};

/// How often the daemon sweeps for notifications that have run out of time.
///
/// A quarter of a second is far below the shortest lifetime a popup is given by
/// default (five seconds) and well above the point where the wake-up matters; the
/// subscription is only alive while something is held, so an idle daemon is not
/// woken at all.
pub const TICK: Duration = Duration::from_millis(250);

/// How many things the daemon may have to tell the transport at once.
///
/// The outgoing queue is the seam the M5 bus drains. It is bounded for the same
/// reason the notification queue is: a burst of a hundred notifications a second
/// must not grow anything without bound, even if nobody is reading.
pub const MAX_OUTGOING: usize = 256;

/// Everything the daemon reacts to.
///
/// These are the messages the transport, the timer and the widgets produce;
/// nothing else mutates [`State`].
#[derive(Debug, Clone)]
pub enum Message {
    /// The transport delivered a call.
    Request(Request),
    /// The time to sweep for notifications that have run out of time.
    Tick(Instant),
    /// The user clicked a popup's close button.
    Dismissed {
        /// The notification.
        id: NotificationId,
    },
    /// The user pressed one of a notification's action buttons.
    ActionInvoked {
        /// The notification.
        id: NotificationId,
        /// Which action.
        action: ActionId,
    },
    /// The user clicked the notification itself, which invokes its default
    /// action when it has one.
    BodyClicked {
        /// The notification.
        id: NotificationId,
    },
    /// The user followed a link in a notification's body.
    LinkFollowed {
        /// The notification it was in.
        id: NotificationId,
        /// The target, which the sanitiser already checked.
        href: String,
    },
    /// The panel turned do not disturb on or off.
    DoNotDisturbSet(bool),
    /// The panel flipped do not disturb.
    DoNotDisturbToggled,
    /// The session was locked or unlocked (ADR 0015: the compositor owns this).
    SessionLockChanged(SessionState),
    /// The configuration was reloaded.
    ConfigReloaded(NotificationsConfig),
}

/// How many messages the transports may hand the application at once.
///
/// The application drains this channel once per frame. The depth is what a burst
/// of a hundred calls a second fits into while a frame is being drawn: a
/// transport that finds it full is told so and refuses the call on the wire
/// rather than waiting for an unbounded queue to drain.
pub const INBOX_DEPTH: usize = 256;

/// Why a transport could not hand the application a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Undelivered {
    /// Nothing is reading: the application has not subscribed yet, or it
    /// replaced its subscription and the new stream has not been installed.
    #[error("no stream is reading the daemon's messages")]
    Detached,
    /// The application has not drained [`INBOX_DEPTH`] messages yet.
    #[error("the daemon's message channel is full")]
    Full,
}

/// The transports' end of the daemon's message channel.
///
/// Iced asks for a [`Subscription`]'s stream through a factory function that
/// cannot be handed the running daemon (see [`Inbox::subscription`]), so the two
/// ends meet here instead: [`Inbox::subscription`] is what the application
/// subscribes with, and [`Inbox::deliver`] is what the bus thread and the
/// lock-state watcher push into. Nothing else in the daemon needs to know how
/// the two are wired, and a transport that cannot deliver is told so rather than
/// assuming it was heard.
#[derive(Debug, Clone)]
pub struct Inbox(Arc<InboxInner>);

/// The shared half of an [`Inbox`].
#[derive(Debug, Default)]
struct InboxInner {
    /// Where the running subscription's stream is fed, once it exists.
    sink: Mutex<Option<iced::futures::channel::mpsc::Sender<Message>>>,
    /// Signalled when a stream is installed, so that a transport with a call in
    /// hand can wait for the application instead of refusing it.
    attached: Condvar,
    /// Messages no stream could take, so that "undeliverable" is a number a
    /// test can assert rather than a claim.
    lost: AtomicU64,
}

/// One daemon has one transport channel, and iced identifies a subscription by
/// the hash of what it was built from: hashing the contents would make the
/// identity change when the first message is delivered, and iced would tear the
/// stream down and build a new one.
impl Hash for InboxInner {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u8(0);
    }
}

impl Inbox {
    /// An inbox nobody is reading yet.
    pub fn new() -> Self {
        Self(Arc::new(InboxInner::default()))
    }

    /// The application's end: a subscription that carries what the transports
    /// deliver as ordinary [`Message`]s.
    ///
    /// The stream this builds installs itself as the inbox's sink, so a
    /// transport is never left pushing into a channel nobody reads. A second
    /// stream replaces the first, which is what makes the wiring survive iced
    /// building the subscription again.
    pub fn subscription(&self) -> Subscription<Message> {
        Subscription::run_with(Arc::clone(&self.0), subscribed)
    }

    /// Hands a message to the application.
    ///
    /// # Errors
    ///
    /// [`Undelivered::Detached`] when nothing is reading and
    /// [`Undelivered::Full`] when the application has not drained the channel;
    /// both are counted in [`Inbox::lost`]. A transport that cannot deliver must
    /// say so to whoever is waiting — the bus answers such a call with a D-Bus
    /// error rather than leaving the client with no reply at all.
    pub fn deliver(&self, message: Message) -> Result<(), Undelivered> {
        self.deliver_within(message, Duration::ZERO)
    }

    /// Hands a message to the application, waiting up to `grace` for a stream
    /// to appear.
    ///
    /// The daemon owns its bus name before iced has built the subscription that
    /// reads it, so a call that arrives during startup would otherwise be
    /// refused for a reason the client can do nothing about — an application
    /// started at the same time as the session loses its first notification.
    /// Waiting for a reader is bounded and ends either way: a daemon with no
    /// application at all is still told `Detached`, and says so on the wire.
    ///
    /// # Errors
    ///
    /// As [`Inbox::deliver`].
    pub fn deliver_within(&self, message: Message, grace: Duration) -> Result<(), Undelivered> {
        let deadline = Instant::now() + grace;
        let mut sink = self
            .0
            .sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let outcome = loop {
            match sink.as_mut() {
                Some(stream) => match stream.try_send(message) {
                    Ok(()) => break Ok(()),
                    Err(error) if error.is_full() => break Err(Undelivered::Full),
                    // The message went with the failed send; the caller is told
                    // rather than it being silently dropped on the floor.
                    Err(_) => break Err(Undelivered::Detached),
                },
                None => {
                    let remaining = deadline.saturating_duration_since(Instant::now());

                    if remaining.is_zero() {
                        break Err(Undelivered::Detached);
                    }

                    let (waited, _) = self
                        .0
                        .attached
                        .wait_timeout(sink, remaining)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());

                    sink = waited;
                }
            }
        };

        if outcome.is_err() {
            self.0.lost.fetch_add(1, Ordering::Relaxed);
        }

        outcome
    }

    /// How many messages no stream could take.
    pub fn lost(&self) -> u64 {
        self.0.lost.load(Ordering::Relaxed)
    }

    /// The stream this inbox feeds, installing it as the one reader.
    ///
    /// [`Inbox::subscription`] is this stream wrapped so that iced runs it;
    /// taking it directly is for a host that drives its own loop — the
    /// transport's tests do, and a future non-iced host would. There is one
    /// reader at a time: taking the stream again replaces the previous one,
    /// which then ends.
    pub fn stream(&self) -> iced::futures::channel::mpsc::Receiver<Message> {
        subscribed(&self.0)
    }
}

impl InboxInner {
    /// Replaces the stream the application reads with `sink`.
    ///
    /// The sender that was there is dropped, which closes the channel the
    /// previous stream was reading: an inbox never has two live readers, and a
    /// replaced stream ends instead of being left waiting on messages that will
    /// never arrive.
    fn install(&self, sink: iced::futures::channel::mpsc::Sender<Message>) {
        let mut slot = self
            .sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        *slot = Some(sink);
        drop(slot);
        // A transport may be holding a call for an application that has just
        // arrived; it is woken rather than left to its deadline.
        self.attached.notify_all();
    }
}

impl Default for Inbox {
    fn default() -> Self {
        Self::new()
    }
}

/// The stream an [`Inbox`] subscription runs.
///
/// It installs itself and then does nothing but hold the channel open: the
/// messages travel straight from the transports' `deliver` call into the
/// application, so there is no second queue between them and no thread to
/// stop.
fn subscribed(inbox: &Arc<InboxInner>) -> iced::futures::channel::mpsc::Receiver<Message> {
    let (sender, receiver) = iced::futures::channel::mpsc::channel(INBOX_DEPTH);
    inbox.install(sender);

    receiver
}

/// Everything the daemon knows.
///
/// The localizer is shared rather than owned because Iced asks for the initial
/// state through a factory it may call more than once, and a catalog cannot be
/// cloned — it is parsed once and read for the life of the process. A plain
/// [`Rc`] is enough: the daemon is one thread, and `FluentBundle` is neither
/// `Send` nor `Sync`.
pub struct State {
    localizer: Rc<Localizer>,
    tokens: Tokens,
    config: NotificationsConfig,
    queue: Queue,
    policy: Policy,
    session: SessionState,
    replies: Calls,
    outgoing: VecDeque<Outgoing>,
    repeated_calls: u64,
    rejected_invocations: u64,
    dropped_outgoing: u64,
}

impl State {
    /// Applies preferences without replacing live notifications or lock tracking.
    /// An explicitly changed do-not-disturb preference overrides the session toggle.
    pub fn apply_configuration(
        &mut self,
        config: NotificationsConfig,
        tokens: Tokens,
        localizer: Rc<Localizer>,
    ) {
        if self.config.policy.do_not_disturb != config.policy.do_not_disturb {
            self.policy.set_do_not_disturb(config.policy.do_not_disturb);
        }
        self.reload(config);
        self.tokens = tokens;
        self.localizer = localizer;
    }

    /// A daemon holding nothing.
    ///
    /// The queue's bound and lifetimes, and the policy, come from `config`; do
    /// not disturb starts at the configured value and is the daemon's from then
    /// on.
    pub fn new(config: NotificationsConfig, tokens: Tokens, localizer: Rc<Localizer>) -> Self {
        let queue = Queue::new(
            config.queue,
            config.timeouts,
            usize::from(config.display.max_visible),
        );
        let policy = Policy::from_config(&config.policy);

        Self {
            localizer,
            tokens,
            config,
            queue,
            policy,
            session: SessionState::Unlocked,
            replies: Calls::new(),
            outgoing: VecDeque::new(),
            repeated_calls: 0,
            rejected_invocations: 0,
            dropped_outgoing: 0,
        }
    }

    /// The localizer the daemon renders text with.
    pub fn localizer(&self) -> &Localizer {
        &self.localizer
    }

    /// The theme tokens the daemon draws with.
    pub fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    /// The configuration in force.
    pub fn config(&self) -> &NotificationsConfig {
        &self.config
    }

    /// The notifications held.
    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// The policy in force, do not disturb included.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Whether the session is locked.
    pub fn session(&self) -> SessionState {
        self.session
    }

    /// How many calls arrived with an identity that has already been answered.
    ///
    /// The deduplication ADR 0013 decision 3 asks for, counted so that a caller
    /// can see it happened rather than infer it.
    pub fn repeated_calls(&self) -> u64 {
        self.repeated_calls
    }

    /// How many invocations were refused: an action that was already invoked, or
    /// one that belongs to a notification that is already gone.
    pub fn rejected_invocations(&self) -> u64 {
        self.rejected_invocations
    }

    /// How many things the daemon had to tell the transport and could not,
    /// because it has not drained the queue.
    pub fn dropped_outgoing(&self) -> u64 {
        self.dropped_outgoing
    }

    /// Takes what is waiting for a transport.
    ///
    /// Whatever is returned is the caller's to carry; an empty queue is the
    /// normal state of a daemon nobody has sent anything to.
    pub fn take_outgoing(&mut self) -> Vec<Outgoing> {
        self.outgoing.drain(..).collect()
    }

    /// Projects the state into what the view draws.
    ///
    /// The instant is read here, once per projection, because the age of each
    /// notification is part of what the view draws and the projection is the only
    /// place allowed to decide what the view draws (ADR 0013).
    pub fn model(&self) -> PopupStack {
        StackInput {
            queue: &self.queue,
            display: &self.config.display,
            policy: &self.policy,
            session: self.session,
            now: Instant::now(),
        }
        .project(&self.localizer)
    }

    /// Queues something for the transport.
    fn push(&mut self, outgoing: Outgoing) {
        if self.outgoing.len() >= MAX_OUTGOING {
            self.outgoing.pop_front();
            self.dropped_outgoing += 1;
            debug!(
                dropped = self.dropped_outgoing,
                "the outgoing queue is full: the oldest thing the transport had \
                 not taken is gone, and counted"
            );
        }

        self.outgoing.push_back(outgoing);
    }

    /// Queues a signal.
    fn signal(&mut self, signal: Signal) {
        self.push(Outgoing::Signal(signal));
    }

    /// Closes a notification, telling its client why.
    fn close(&mut self, id: NotificationId, reason: CloseReason) {
        if let Some(closed) = self.queue.close(id, reason) {
            self.signal(Signal::closed(closed.id, closed.reason));
        }
    }

    /// Answers a call the transport delivered.
    fn answer(&mut self, request: Request) {
        if let Some(answered) = self.replies.answered(request.call) {
            self.repeated_calls += 1;
            debug!(
                call = request.call.get(),
                "this call was already answered; answering the same way rather \
                 than acting twice"
            );

            let body = answered.clone();
            self.push(Outgoing::Reply(Reply {
                call: request.call,
                body,
            }));

            return;
        }

        let body = match request.body {
            RequestBody::Notify(notify) => match notify.draft() {
                Ok(draft) => {
                    let admission = self.queue.notify(draft, Instant::now());

                    if let Some(evicted) = admission.evicted {
                        // A notification that was dropped or coalesced away is
                        // gone, and its client is entitled to know why. The
                        // specification defines no reason for "the queue had no
                        // room", so it is `Undefined`; the counters say which of
                        // the two it was.
                        debug!(
                            evicted = evicted.id().get(),
                            ?evicted,
                            "the queue was full and made room"
                        );
                        self.signal(Signal::closed(evicted.id(), CloseReason::Undefined));
                    }

                    ReplyBody::Notified { id: admission.id }
                }
                Err(reason) => {
                    debug!(%reason, "refusing a notification with unusable actions");
                    ReplyBody::Rejected { reason }
                }
            },
            RequestBody::Close(id) => {
                let closed = self.queue.close(id, CloseReason::ClosedByCall);

                if let Some(closed) = closed {
                    self.signal(Signal::closed(closed.id, closed.reason));
                }

                ReplyBody::Closed {
                    closed: closed.is_some(),
                }
            }
            RequestBody::GetCapabilities => ReplyBody::Capabilities {
                capabilities: CAPABILITIES.to_vec(),
            },
            RequestBody::GetServerInformation => ReplyBody::ServerInformation {
                information: SERVER_INFORMATION,
            },
        };

        self.replies.record(request.call, body.clone());
        self.push(Outgoing::Reply(Reply {
            call: request.call,
            body,
        }));
    }

    /// Invokes one of a notification's actions, at most once.
    fn invoke(&mut self, id: NotificationId, action: ActionId) {
        let outcome = self
            .queue
            .get_mut(id)
            .map(|notification| notification.actions.invoke(&action))
            .transpose();

        match outcome {
            Ok(Some(invoked)) => {
                self.signal(Signal::invoked(id, invoked.id.clone()));
                // The specification closes a notification when one of its
                // actions is invoked, and that is also what makes the second
                // click a rejection rather than a repeat.
                self.close(id, CloseReason::Dismissed);
            }
            Err(error) => {
                self.rejected_invocations += 1;
                debug!(id = id.get(), %error, "refusing an invocation");
            }
            Ok(None) => {
                self.rejected_invocations += 1;
                debug!(
                    id = id.get(),
                    action = action.as_str(),
                    "an invocation for a notification that is gone: refused \
                     rather than repeated"
                );
            }
        }
    }

    /// Reloads the configuration.
    fn reload(&mut self, config: NotificationsConfig) {
        let previous = self.config.clone();
        self.config = config;

        // The policy's do-not-disturb is the daemon's own running state and is
        // deliberately not touched; everything else follows the file.
        self.policy.apply_config(&self.config.policy);

        let closed = self.queue.reconfigure(
            self.config.queue,
            self.config.timeouts,
            usize::from(self.config.display.max_visible),
        );

        for closed in &closed {
            debug!(
                id = closed.id.get(),
                "a lower queue bound closed a notification"
            );
            self.signal(Signal::closed(closed.id, closed.reason));
        }

        debug!(
            visible = self.config.display.max_visible,
            max_pending = self.config.queue.max_pending,
            show_actions = self.config.policy.show_actions,
            was_visible = previous.display.max_visible,
            "the configuration was reloaded"
        );
    }
}

/// The Elm update: the only place [`State`] changes.
pub fn update(state: &mut State, message: Message) -> iced::Task<Message> {
    match message {
        Message::Request(request) => state.answer(request),
        Message::Tick(now) => {
            for closed in state.queue.expire(now) {
                state.signal(Signal::closed(closed.id, closed.reason));
            }
        }
        Message::Dismissed { id } => state.close(id, CloseReason::Dismissed),
        Message::ActionInvoked { id, action } => state.invoke(id, action),
        Message::BodyClicked { id } => {
            let default = state
                .queue
                .get(id)
                .and_then(|notification| notification.actions.default_action())
                .map(|action| action.id.clone());

            match default {
                Some(action) => state.invoke(id, action),
                // A notification with no default action is dismissed when the
                // user clicks it, which is what the specification says the
                // click means once there is nothing to invoke.
                None => state.close(id, CloseReason::Dismissed),
            }
        }
        Message::LinkFollowed { id, href } => {
            debug!(id = id.get(), %href, "a link was followed");
            state.push(Outgoing::FollowLink { id, href });
        }
        Message::DoNotDisturbSet(on) => state.policy.set_do_not_disturb(on),
        Message::DoNotDisturbToggled => {
            let on = state.policy.toggle_do_not_disturb();
            debug!(do_not_disturb = on, "do not disturb was toggled");
        }
        Message::SessionLockChanged(session) => state.session = session,
        Message::ConfigReloaded(config) => state.reload(config),
    }

    iced::Task::none()
}

/// Draws the popup stack.
pub fn view(state: &State) -> Element<'_, Message> {
    view::stack(state.model(), &state.tokens, &state.config.display)
}

/// The title of the daemon's own surface.
pub fn title(state: &State) -> String {
    state.localizer.message("notifications-window-title")
}

/// The daemon's subscriptions: the expiry sweep, while anything is held.
///
/// An empty daemon subscribes to nothing at all, which is what keeps an idle
/// session from waking four times a second for nothing.
pub fn subscription(state: &State) -> Subscription<Message> {
    if state.queue.is_empty() {
        return Subscription::none();
    }

    Subscription::run(ticks)
}

/// A stream that produces one [`Message::Tick`] every [`TICK`], for as long as
/// the subscription is alive.
///
/// The thread ends when the runtime drops the receiver, which is what stops the
/// timer when the daemon exits or the stack empties.
fn ticks() -> iced::futures::channel::mpsc::Receiver<Message> {
    let (mut sender, receiver) = iced::futures::channel::mpsc::channel(1);

    std::thread::spawn(move || {
        loop {
            match sender.try_send(Message::Tick(Instant::now())) {
                Ok(()) => {}
                // The runtime has not read the last tick yet; skipping one is
                // better than blocking the timer.
                Err(error) if error.is_full() => {}
                Err(_) => break,
            }

            std::thread::sleep(TICK);
        }
    });

    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CallId, NotifyRequest};
    use crate::markup::plain_text;
    use crate::protocol::{Expiry, HINT_URGENCY, HintValue, Hints, Urgency};
    use crate::view_model::StackStatus;
    use proptest::prelude::*;

    fn daemon() -> State {
        let locale = inox_common::i18n::Locale::default_locale();

        State::new(
            NotificationsConfig::default(),
            Tokens::builtin_dark(),
            Rc::new(crate::i18n::load(&locale).expect("the catalogs parse")),
        )
    }

    fn send(state: &mut State, message: Message) {
        let _ = update(state, message);
    }

    fn notify(call: u64, summary: &str) -> Message {
        Message::Request(Request::new(
            CallId::new(call),
            RequestBody::Notify(NotifyRequest {
                app_name: "app".to_owned(),
                summary: summary.to_owned(),
                ..NotifyRequest::default()
            }),
        ))
    }

    /// Sends a notification and returns the identifier the daemon answered with.
    fn admitted(state: &mut State, call: u64, summary: &str) -> NotificationId {
        admitted_with_outgoing(state, call, summary).0
    }

    /// The same, keeping what the daemon had to say besides the answer.
    fn admitted_with_outgoing(
        state: &mut State,
        call: u64,
        summary: &str,
    ) -> (NotificationId, Vec<Outgoing>) {
        send(state, notify(call, summary));
        let outgoing = state.take_outgoing();
        let id = outgoing
            .iter()
            .find_map(|entry| match entry {
                Outgoing::Reply(Reply {
                    body: ReplyBody::Notified { id },
                    ..
                }) => Some(*id),
                _ => None,
            })
            .expect("a Notify call is answered with an identifier");

        (id, outgoing)
    }

    // --- the four methods ---------------------------------------------------

    #[test]
    fn live_configuration_preserves_notifications_and_applies_explicit_dnd_changes() {
        let mut state = daemon();
        send(&mut state, notify(1, "keep me"));
        let before = state.queue.counters();
        let mut config = state.config.clone();
        config.policy.do_not_disturb = true;
        let tokens = Tokens::builtin_light();
        state.apply_configuration(config, tokens.clone(), Rc::clone(&state.localizer));
        assert!(state.policy().do_not_disturb());
        assert_eq!(state.queue.counters(), before);
        assert_eq!(state.tokens(), &tokens);
        let mut config = state.config.clone();
        config.policy.do_not_disturb = false;
        state.apply_configuration(config, tokens, Rc::clone(&state.localizer));
        assert!(!state.policy().do_not_disturb());
        assert_eq!(state.queue.counters(), before);
    }

    #[test]
    fn a_notify_call_is_answered_with_the_identifier_it_was_given() {
        let mut state = daemon();
        let id = admitted(&mut state, 1, "a build finished");

        assert_eq!(id, NotificationId::new(1));
        assert_eq!(state.queue().len(), 1);
        assert_eq!(
            state.queue().get(id).map(|item| item.summary.as_str()),
            Some("a build finished")
        );
    }

    #[test]
    fn the_same_call_twice_answers_the_same_way_and_acts_once() {
        let mut state = daemon();
        let id = admitted(&mut state, 1, "one");

        send(&mut state, notify(1, "one"));

        let answers: Vec<ReplyBody> = state
            .take_outgoing()
            .into_iter()
            .filter_map(|outgoing| match outgoing {
                Outgoing::Reply(reply) => Some(reply.body),
                Outgoing::Signal(_) | Outgoing::FollowLink { .. } => None,
            })
            .collect();

        assert_eq!(answers.len(), 1);
        assert_eq!(answers[0], ReplyBody::Notified { id });
        assert_eq!(
            state.queue().len(),
            1,
            "a repeat is not a second notification"
        );
        assert_eq!(state.repeated_calls(), 1);
    }

    #[test]
    fn a_different_call_creates_a_second_notification() {
        let mut state = daemon();
        let first = admitted(&mut state, 1, "one");
        let second = admitted(&mut state, 2, "two");

        assert_ne!(first, second);
        assert_eq!(state.queue().len(), 2);
        assert_eq!(state.repeated_calls(), 0);
    }

    #[test]
    fn a_notification_with_unusable_actions_is_rejected_with_a_reason() {
        let mut state = daemon();
        send(
            &mut state,
            Message::Request(Request::new(
                CallId::new(1),
                RequestBody::Notify(NotifyRequest {
                    actions: vec!["only-a-key".to_owned()],
                    ..NotifyRequest::default()
                }),
            )),
        );

        let reply = state
            .take_outgoing()
            .into_iter()
            .find_map(|outgoing| match outgoing {
                Outgoing::Reply(reply) => Some(reply.body),
                Outgoing::Signal(_) | Outgoing::FollowLink { .. } => None,
            })
            .expect("the call is answered");

        assert!(matches!(reply, ReplyBody::Rejected { .. }));
        assert_eq!(state.queue().len(), 0, "nothing was admitted");
    }

    #[test]
    fn closing_a_notification_answers_and_tells_its_client() {
        let mut state = daemon();
        let id = admitted(&mut state, 1, "one");

        send(
            &mut state,
            Message::Request(Request::new(CallId::new(2), RequestBody::Close(id))),
        );

        let outgoing = state.take_outgoing();

        assert!(outgoing.contains(&Outgoing::Reply(Reply {
            call: CallId::new(2),
            body: ReplyBody::Closed { closed: true },
        })));
        assert!(outgoing.contains(&Outgoing::Signal(Signal::closed(
            id,
            CloseReason::ClosedByCall
        ))));
        assert!(state.queue().is_empty());
    }

    #[test]
    fn closing_something_that_is_gone_answers_that_nothing_was_closed() {
        let mut state = daemon();

        send(
            &mut state,
            Message::Request(Request::new(
                CallId::new(1),
                RequestBody::Close(NotificationId::new(99)),
            )),
        );

        assert!(state.take_outgoing().contains(&Outgoing::Reply(Reply {
            call: CallId::new(1),
            body: ReplyBody::Closed { closed: false },
        })));
    }

    #[test]
    fn the_capabilities_and_the_server_information_are_answered() {
        let mut state = daemon();

        send(
            &mut state,
            Message::Request(Request::new(CallId::new(1), RequestBody::GetCapabilities)),
        );
        send(
            &mut state,
            Message::Request(Request::new(
                CallId::new(2),
                RequestBody::GetServerInformation,
            )),
        );

        let answers: Vec<ReplyBody> = state
            .take_outgoing()
            .into_iter()
            .filter_map(|outgoing| match outgoing {
                Outgoing::Reply(reply) => Some(reply.body),
                Outgoing::Signal(_) | Outgoing::FollowLink { .. } => None,
            })
            .collect();

        assert_eq!(
            answers[0],
            ReplyBody::Capabilities {
                capabilities: CAPABILITIES.to_vec()
            }
        );
        assert_eq!(
            answers[1],
            ReplyBody::ServerInformation {
                information: SERVER_INFORMATION
            }
        );
    }

    // --- closing, dismissal, expiry ----------------------------------------

    #[test]
    fn dismissing_a_popup_closes_it_with_the_reason_the_specification_has_for_it() {
        let mut state = daemon();
        let id = admitted(&mut state, 1, "one");

        send(&mut state, Message::Dismissed { id });

        assert_eq!(
            state.take_outgoing(),
            [Outgoing::Signal(Signal::closed(id, CloseReason::Dismissed))]
        );
        assert!(state.queue().is_empty());
    }

    #[test]
    fn a_sweep_closes_what_has_run_out_of_time_and_says_so() {
        let mut state = daemon();
        let id = admitted(&mut state, 1, "one");
        // The default low lifetime is five seconds; the notification is sent as
        // low urgency, so it expires.
        let _ = id;

        let far = Instant::now() + Duration::from_secs(3_600);
        send(&mut state, Message::Tick(far));

        assert_eq!(
            state.take_outgoing(),
            [Outgoing::Signal(Signal::closed(
                NotificationId::new(1),
                CloseReason::Expired
            ))]
        );
        assert!(state.queue().is_empty());
    }

    #[test]
    fn a_critical_notification_survives_every_sweep() {
        let mut state = daemon();
        send(
            &mut state,
            Message::Request(Request::new(
                CallId::new(1),
                RequestBody::Notify(NotifyRequest {
                    hints: vec![(HINT_URGENCY.to_owned(), HintValue::Byte(2))],
                    summary: "urgent".to_owned(),
                    ..NotifyRequest::default()
                }),
            )),
        );
        let _ = state.take_outgoing();

        send(
            &mut state,
            Message::Tick(Instant::now() + Duration::from_secs(86_400)),
        );

        assert!(state.take_outgoing().is_empty());
        assert_eq!(state.queue().len(), 1);
    }

    #[test]
    fn dismissing_something_that_is_gone_changes_nothing() {
        let mut state = daemon();

        send(
            &mut state,
            Message::Dismissed {
                id: NotificationId::new(42),
            },
        );

        assert!(state.take_outgoing().is_empty());
    }

    // --- actions ------------------------------------------------------------

    /// A daemon holding one notification that offers a default and a button.
    fn with_actions() -> (State, NotificationId) {
        let mut state = daemon();

        send(
            &mut state,
            Message::Request(Request::new(
                CallId::new(1),
                RequestBody::Notify(NotifyRequest {
                    actions: vec![
                        "default".to_owned(),
                        String::new(),
                        "open".to_owned(),
                        "Open".to_owned(),
                    ],
                    summary: "with actions".to_owned(),
                    ..NotifyRequest::default()
                }),
            )),
        );
        let _ = state.take_outgoing();

        (state, NotificationId::new(1))
    }

    #[test]
    fn an_action_fires_once_and_closes_the_notification() {
        let (mut state, id) = with_actions();

        send(
            &mut state,
            Message::ActionInvoked {
                id,
                action: ActionId::new("open"),
            },
        );

        assert_eq!(
            state.take_outgoing(),
            [
                Outgoing::Signal(Signal::invoked(id, ActionId::new("open"))),
                Outgoing::Signal(Signal::closed(id, CloseReason::Dismissed)),
            ],
            "one ActionInvoked, then the close the specification requires"
        );
        assert!(state.queue().is_empty());
    }

    #[test]
    fn a_second_press_produces_no_second_signal() {
        let (mut state, id) = with_actions();

        send(
            &mut state,
            Message::ActionInvoked {
                id,
                action: ActionId::new("open"),
            },
        );
        let _ = state.take_outgoing();

        send(
            &mut state,
            Message::ActionInvoked {
                id,
                action: ActionId::new("open"),
            },
        );

        assert!(
            state.take_outgoing().is_empty(),
            "the client must not be told to act twice"
        );
        assert_eq!(state.rejected_invocations(), 1);
    }

    #[test]
    fn an_action_the_notification_does_not_offer_is_refused() {
        let (mut state, id) = with_actions();

        send(
            &mut state,
            Message::ActionInvoked {
                id,
                action: ActionId::new("nonsense"),
            },
        );

        assert!(state.take_outgoing().is_empty());
        assert_eq!(state.rejected_invocations(), 1);
        assert_eq!(state.queue().len(), 1, "the notification is still there");
    }

    #[test]
    fn clicking_the_body_invokes_the_default_action() {
        let (mut state, id) = with_actions();

        send(&mut state, Message::BodyClicked { id });

        assert!(
            state
                .take_outgoing()
                .contains(&Outgoing::Signal(Signal::invoked(
                    id,
                    ActionId::new("default")
                )))
        );
        assert!(state.queue().is_empty());
    }

    #[test]
    fn clicking_a_body_with_no_default_action_dismisses_it() {
        let mut state = daemon();
        let id = admitted(&mut state, 1, "no actions");

        send(&mut state, Message::BodyClicked { id });

        assert_eq!(
            state.take_outgoing(),
            [Outgoing::Signal(Signal::closed(id, CloseReason::Dismissed))]
        );
    }

    #[test]
    fn following_a_link_is_queued_for_the_transport() {
        let mut state = daemon();
        let id = admitted(&mut state, 1, "one");
        let _ = state.take_outgoing();

        send(
            &mut state,
            Message::LinkFollowed {
                id,
                href: "https://example.com".to_owned(),
            },
        );

        assert_eq!(
            state.take_outgoing(),
            [Outgoing::FollowLink {
                id,
                href: "https://example.com".to_owned()
            }]
        );
    }

    // --- do not disturb and the locked session ------------------------------

    #[test]
    fn do_not_disturb_suppresses_the_stack_without_dropping_anything() {
        let mut state = daemon();
        admitted(&mut state, 1, "one");
        admitted(&mut state, 2, "two");
        let _ = state.take_outgoing();

        send(&mut state, Message::DoNotDisturbToggled);

        assert!(state.policy().do_not_disturb());
        assert_eq!(state.model().status, StackStatus::Suppressed);
        assert!(state.model().popups.is_empty());
        assert_eq!(state.queue().len(), 2, "delivery is not suppressed");

        send(&mut state, Message::DoNotDisturbSet(false));
        assert_eq!(state.model().status, StackStatus::Showing);
        assert_eq!(state.model().popups.len(), 2);
    }

    #[test]
    fn a_reload_keeps_the_running_do_not_disturb() {
        let mut state = daemon();
        send(&mut state, Message::DoNotDisturbSet(true));

        let mut config = NotificationsConfig::default();
        config.policy.do_not_disturb = false;
        config.policy.show_actions = false;
        send(&mut state, Message::ConfigReloaded(config));

        assert!(
            state.policy().do_not_disturb(),
            "the file is the next session's starting value"
        );
        assert!(!state.policy().show_actions());
        assert!(!state.config().policy.show_actions);
    }

    #[test]
    fn a_lower_bound_on_reload_closes_the_oldest_and_says_so() {
        let mut state = daemon();

        for call in 1..=5 {
            admitted(&mut state, call, "held");
        }
        let _ = state.take_outgoing();

        let mut config = NotificationsConfig::default();
        config.queue.max_pending = 2;
        send(&mut state, Message::ConfigReloaded(config));

        assert_eq!(state.queue().len(), 2);
        let signals: Vec<Outgoing> = state.take_outgoing();
        assert_eq!(
            signals.len(),
            3,
            "one NotificationClosed per notification the reload closed"
        );
        assert_eq!(state.queue().counters().dropped, 3);
    }

    #[test]
    fn locking_the_session_redacts_what_the_model_holds() {
        let mut state = daemon();
        admitted(&mut state, 1, "a secret summary");

        send(
            &mut state,
            Message::SessionLockChanged(SessionState::Locked),
        );

        let model = state.model();
        assert!(model.popups[0].content.is_withheld());
        assert_eq!(model.popups[0].content.summary(), "A notification arrived");
        assert!(model.popups[0].content.body().is_none());
        assert!(!format!("{model:?}").contains("a secret summary"));

        send(
            &mut state,
            Message::SessionLockChanged(SessionState::Unlocked),
        );
        assert_eq!(
            state.model().popups[0].content.summary(),
            "a secret summary"
        );
    }

    // --- the queue's bound, through the daemon ------------------------------

    #[test]
    fn a_full_queue_evicts_and_reports_the_eviction() {
        let mut state = daemon();
        let mut config = NotificationsConfig::default();
        config.queue.max_pending = 2;
        send(&mut state, Message::ConfigReloaded(config));
        let _ = state.take_outgoing();

        admitted(&mut state, 1, "one");
        admitted(&mut state, 2, "two");
        let (_, outgoing) = admitted_with_outgoing(&mut state, 3, "three");

        assert!(
            outgoing.contains(&Outgoing::Signal(Signal::closed(
                NotificationId::new(1),
                CloseReason::Undefined
            ))),
            "the evicted notification's client is told: {outgoing:?}"
        );
        assert_eq!(state.queue().len(), 2);
        assert_eq!(state.queue().counters().dropped, 1);
    }

    #[test]
    fn the_overflow_sentence_counts_what_the_queue_calls_waiting() {
        let mut state = daemon();

        for call in 1..=5 {
            admitted(&mut state, call, "held");
        }

        // Two places know how many notifications are not on screen — the queue
        // (which is what eviction acts on) and the projection (which is what
        // knows how many popups it drew). They must agree, or the queue would be
        // evicting under a different rule than the one the user is told about.
        assert_eq!(state.queue().waiting(), 2);
        assert_eq!(state.model().hidden(), 2);
        assert_eq!(
            state.model().overflow.map(|overflow| overflow.message),
            Some("2 more waiting".to_owned())
        );
    }

    #[test]
    fn a_replacement_keeps_its_identifier_and_tells_no_one_it_was_closed() {
        let mut state = daemon();
        let id = admitted(&mut state, 1, "the first version");
        let _ = state.take_outgoing();

        send(
            &mut state,
            Message::Request(Request::new(
                CallId::new(2),
                RequestBody::Notify(NotifyRequest {
                    replaces_id: id.get(),
                    summary: "the second version".to_owned(),
                    ..NotifyRequest::default()
                }),
            )),
        );

        let outgoing = state.take_outgoing();

        assert!(outgoing.contains(&Outgoing::Reply(Reply {
            call: CallId::new(2),
            body: ReplyBody::Notified { id },
        })));
        assert!(
            outgoing
                .iter()
                .all(|entry| !matches!(entry, Outgoing::Signal(Signal::NotificationClosed { .. }))),
            "the notification was not closed, it was replaced: {outgoing:?}"
        );
        assert_eq!(state.queue().len(), 1);
        assert_eq!(
            state.queue().get(id).map(|item| item.summary.as_str()),
            Some("the second version")
        );
    }

    #[test]
    fn a_hundred_notifications_a_second_for_ten_seconds_through_the_daemon() {
        let mut state = daemon();

        // The bound is small so that eviction is reached early; the burst is the
        // one D06 injects: a hundred a second for ten seconds.
        let mut config = NotificationsConfig::default();
        config.queue.max_pending = 32;
        send(&mut state, Message::ConfigReloaded(config));
        let _ = state.take_outgoing();

        for index in 0..1_000u32 {
            send(
                &mut state,
                Message::Request(Request::new(
                    CallId::new(u64::from(index)),
                    RequestBody::Notify(NotifyRequest {
                        app_name: format!("app{}", index % 7),
                        summary: format!("burst {index}"),
                        ..NotifyRequest::default()
                    }),
                )),
            );
        }

        let counters = state.queue().counters();

        assert_eq!(counters.admitted, 1_000);
        assert!(
            state.queue().len() <= 32,
            "the queue held {} notifications",
            state.queue().len()
        );
        assert_eq!(
            counters.created,
            state.queue().len() as u64 + counters.settled(),
            "nothing was lost in the burst: every notification is held or settled"
        );
        assert!(counters.dropped > 0, "the bound was reached");

        // Nothing drained the transport, so the outgoing queue hit its own bound
        // and said so instead of growing without end.
        assert!(state.outgoing.len() <= MAX_OUTGOING);
        assert!(state.dropped_outgoing > 0);
        assert_eq!(state.take_outgoing().len(), MAX_OUTGOING);
    }

    #[test]
    fn the_outgoing_queue_is_bounded_and_counts_what_it_drops() {
        let mut state = daemon();

        for call in 0..(MAX_OUTGOING as u64 + 10) {
            send(
                &mut state,
                Message::Request(Request::new(
                    CallId::new(call),
                    RequestBody::Close(NotificationId::new(1)),
                )),
            );
        }

        assert_eq!(state.take_outgoing().len(), MAX_OUTGOING);
        assert_eq!(state.dropped_outgoing(), 10);
    }

    // --- the transport channel ----------------------------------------------

    #[test]
    fn a_message_a_transport_delivers_reaches_the_update_loop() {
        let inbox = Inbox::new();
        let mut stream = inbox.stream();

        inbox
            .deliver(Message::DoNotDisturbSet(true))
            .expect("the inbox is being read");

        let delivered = stream.try_recv().expect("the stream is alive");
        assert!(
            matches!(delivered, Message::DoNotDisturbSet(true)),
            "the message arrived unchanged: {delivered:?}"
        );
        assert_eq!(inbox.lost(), 0);
    }

    #[test]
    fn the_subscription_is_what_makes_the_inbox_readable() {
        let inbox = Inbox::new();

        assert_eq!(
            inbox.deliver(Message::DoNotDisturbToggled),
            Err(Undelivered::Detached),
            "nobody is reading yet"
        );
        assert_eq!(inbox.lost(), 1);

        let _stream = inbox.stream();
        assert!(inbox.deliver(Message::DoNotDisturbToggled).is_ok());

        // Smoke: the subscription itself is buildable without a running
        // runtime, which is all `main.rs` needs from it at boot.
        let _subscription = inbox.subscription();
    }

    #[test]
    fn a_replaced_stream_ends_rather_than_keeping_a_dead_channel_open() {
        let inbox = Inbox::new();
        let mut first = inbox.stream();
        let mut second = inbox.stream();

        inbox
            .deliver(Message::DoNotDisturbToggled)
            .expect("the second stream is reading");

        let ended = first
            .try_recv()
            .expect_err("the replaced stream has nothing to read");
        assert!(
            ended.is_closed(),
            "the replaced stream is closed, so iced is not left running one: {ended:?}"
        );
        assert!(matches!(
            second.try_recv(),
            Ok(Message::DoNotDisturbToggled)
        ));
    }

    #[test]
    fn a_message_waits_for_a_reader_that_is_still_starting() {
        let inbox = Inbox::new();
        let (reader_started, waiting) = std::sync::mpsc::channel();

        let reader = {
            let inbox = inbox.clone();

            std::thread::spawn(move || {
                waiting.recv().expect("the delivery is under way");
                std::thread::sleep(Duration::from_millis(30));
                inbox.stream()
            })
        };

        reader_started.send(()).expect("the reader thread is there");

        let outcome = inbox.deliver_within(Message::DoNotDisturbSet(true), Duration::from_secs(2));
        let mut stream = reader.join().expect("the reader thread does not panic");

        assert_eq!(
            outcome,
            Ok(()),
            "the message waited for the reader instead of being refused"
        );
        assert!(matches!(
            stream.try_recv(),
            Ok(Message::DoNotDisturbSet(true))
        ));
        assert_eq!(inbox.lost(), 0);
    }

    #[test]
    fn a_message_nobody_ever_reads_is_refused_after_the_wait() {
        let inbox = Inbox::new();
        let started = Instant::now();

        let outcome = inbox.deliver_within(Message::DoNotDisturbToggled, Duration::from_millis(50));

        assert_eq!(outcome, Err(Undelivered::Detached));
        assert!(
            started.elapsed() >= Duration::from_millis(50),
            "the wait is the grace, not less"
        );
        assert_eq!(inbox.lost(), 1);
    }

    #[test]
    fn a_full_channel_is_reported_rather_than_grown() {
        let inbox = Inbox::new();
        let (sender, _receiver) = iced::futures::channel::mpsc::channel(1);
        inbox.0.install(sender);

        let mut delivered = 0;
        let mut refused = 0;

        for _ in 0..16 {
            match inbox.deliver(Message::DoNotDisturbSet(true)) {
                Ok(()) => delivered += 1,
                Err(error) => {
                    assert_eq!(error, Undelivered::Full, "the channel was open, not gone");
                    refused += 1;
                }
            }
        }

        assert!(delivered >= 1, "the first messages did go through");
        assert!(
            refused > 0,
            "a channel of depth one must refuse rather than grow without bound"
        );
        assert_eq!(
            inbox.lost(),
            refused,
            "every refusal is counted, so a transport can tell that it happened"
        );
    }

    // --- the projection and the frame ---------------------------------------

    #[test]
    fn the_view_and_the_title_read_the_state() {
        let mut state = daemon();
        admitted(&mut state, 1, "one");

        let _element = view(&state);
        assert_eq!(title(&state), "Inox Notifications");
        assert_eq!(state.localizer().locale().tag(), "en-US");
        assert!(state.tokens().colors.background.a > 0);
        assert_eq!(state.session(), SessionState::Unlocked);
        assert_eq!(state.config().display.max_visible, 3);
    }

    #[test]
    fn the_expiry_lifetime_uses_the_configuration_of_the_daemon() {
        let mut state = daemon();
        send(
            &mut state,
            Message::Request(Request::new(
                CallId::new(1),
                RequestBody::Notify(NotifyRequest {
                    summary: "briefly".to_owned(),
                    expire_timeout: Expiry::After(1_000).to_wire(),
                    hints: vec![(HINT_URGENCY.to_owned(), HintValue::Byte(0))],
                    ..NotifyRequest::default()
                }),
            )),
        );
        let _ = state.take_outgoing();

        let notification = state.queue().get(NotificationId::new(1)).expect("held");

        assert_eq!(notification.hints.urgency, Urgency::Low);
        assert!(!notification.is_permanent());
        assert_eq!(
            notification.body,
            crate::markup::sanitize(""),
            "a notification with no body has no spans"
        );
        assert!(plain_text(&notification.body).is_empty());
        assert_eq!(
            notification.hints,
            Hints::from_pairs(&[(HINT_URGENCY.to_owned(), HintValue::Byte(0))]),
            "the hints are what the call's table carried"
        );
        assert_eq!(
            notification.hints.urgency,
            Urgency::Low,
            "the urgency came from the hint the call carried"
        );
    }

    // --- properties ---------------------------------------------------------

    /// One operation in a random sequence, as the properties drive them.
    #[derive(Debug, Clone)]
    enum Op {
        /// Deliver a `Notify` call.
        Notify {
            /// Which call identity it arrives under.
            call: u8,
            /// Which application sends it.
            app: u8,
            /// How urgent it says it is.
            urgency: u8,
            /// How long the client asks it to live, in milliseconds (0 = never).
            millis: u32,
            /// Whether it offers an action.
            action: bool,
        },
        /// Deliver a `CloseNotification` call for the newest notification.
        Close,
        /// Click an action button on the newest notification.
        Press,
        /// Dismiss the newest notification.
        Dismiss,
        /// Sweep for expiries.
        Sweep,
    }

    fn operations() -> impl Strategy<Value = Vec<Op>> {
        proptest::collection::vec(
            prop_oneof![
                6 => (0u8..12, 0u8..3, 0u8..3, 0u32..3_000, any::<bool>()).prop_map(
                    |(call, app, urgency, millis, action)| Op::Notify {
                        call,
                        app,
                        urgency,
                        millis,
                        action
                    }
                ),
                1 => Just(Op::Close),
                2 => Just(Op::Press),
                1 => Just(Op::Dismiss),
                1 => Just(Op::Sweep),
            ],
            0..120,
        )
    }

    /// What one driven sequence produced.
    struct Driven {
        /// The daemon.
        state: State,
        /// Every identifier a `Notify` call was answered with.
        admitted: Vec<NotificationId>,
        /// The identifiers each call identity was answered with, so that a
        /// repeated call can be checked to answer the same way.
        answered: std::collections::BTreeMap<u64, std::collections::BTreeSet<NotificationId>>,
        /// Every `NotificationClosed` signal, in the order it was emitted.
        closed: Vec<(NotificationId, CloseReason)>,
        /// Every `ActionInvoked` signal.
        invoked: Vec<NotificationId>,
    }

    fn drive(ops: &[Op]) -> Driven {
        let mut state = daemon();
        let mut now = Instant::now();
        let mut admitted = Vec::new();
        let mut answered: std::collections::BTreeMap<
            u64,
            std::collections::BTreeSet<NotificationId>,
        > = std::collections::BTreeMap::new();
        let mut closed = Vec::new();
        let mut invoked = Vec::new();
        let mut last_call: Option<u64> = None;

        // The queue is small on purpose, so that eviction is reached often.
        let mut config = NotificationsConfig::default();
        config.queue.max_pending = 3;
        config.display.max_visible = 2;
        send(&mut state, Message::ConfigReloaded(config));

        for (index, op) in ops.iter().enumerate() {
            now += Duration::from_millis(50);

            match op {
                Op::Notify {
                    call,
                    app,
                    urgency,
                    millis,
                    action,
                } => {
                    let mut request = NotifyRequest {
                        app_name: format!("app{app}"),
                        summary: format!("notification {index}"),
                        hints: vec![(HINT_URGENCY.to_owned(), HintValue::Byte(*urgency))],
                        expire_timeout: if *millis == 0 {
                            Expiry::Never.to_wire()
                        } else {
                            Expiry::After(*millis).to_wire()
                        },
                        ..NotifyRequest::default()
                    };
                    if *action {
                        request.actions = vec!["open".to_owned(), "Open".to_owned()];
                    }

                    last_call = Some(u64::from(*call));
                    send(
                        &mut state,
                        Message::Request(Request::new(
                            CallId::new(u64::from(*call)),
                            RequestBody::Notify(request),
                        )),
                    );
                }
                Op::Close => {
                    // The identifier is taken out of the borrow first: the send
                    // below needs the daemon mutably.
                    let oldest = state.queue().iter().next().map(|item| item.id);

                    if let Some(id) = oldest {
                        send(
                            &mut state,
                            Message::Request(Request::new(
                                CallId::new(1_000 + index as u64),
                                RequestBody::Close(id),
                            )),
                        );
                    }
                }
                Op::Press => {
                    let newest = state.queue().newest(1).first().map(|item| item.id);

                    if let Some(id) = newest {
                        send(
                            &mut state,
                            Message::ActionInvoked {
                                id,
                                action: ActionId::new("open"),
                            },
                        );
                    }
                }
                Op::Dismiss => {
                    let newest = state.queue().newest(1).first().map(|item| item.id);

                    if let Some(id) = newest {
                        send(&mut state, Message::Dismissed { id });
                    }
                }
                Op::Sweep => send(&mut state, Message::Tick(now)),
            }

            for outgoing in state.take_outgoing() {
                match outgoing {
                    Outgoing::Reply(Reply {
                        body: ReplyBody::Notified { id },
                        ..
                    }) => {
                        admitted.push(id);
                        if let Some(call) = last_call {
                            answered.entry(call).or_default().insert(id);
                        }
                    }
                    Outgoing::Signal(Signal::NotificationClosed { id, reason }) => {
                        closed.push((id, reason));
                    }
                    Outgoing::Signal(Signal::ActionInvoked { id, .. }) => invoked.push(id),
                    Outgoing::Reply(_) | Outgoing::FollowLink { .. } => {}
                }
            }
        }

        Driven {
            state,
            admitted,
            answered,
            closed,
            invoked,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn every_admitted_notification_ends_exactly_once(ops in operations()) {
            let driven = drive(&ops);

            for id in &driven.admitted {
                let terminations = driven
                    .closed
                    .iter()
                    .filter(|(closed, _)| closed == id)
                    .count();

                prop_assert!(terminations <= 1, "{} was closed twice", id);
                if driven.state.queue().get(*id).is_none() {
                    prop_assert!(
                        terminations == 1,
                        "{} is gone and no client was told",
                        id
                    );
                } else {
                    prop_assert!(terminations == 0, "{} is held and closed", id);
                }
            }
        }

        #[test]
        fn no_notification_has_an_action_invoked_twice(ops in operations()) {
            let driven = drive(&ops);
            let mut invoked = driven.invoked.clone();

            invoked.sort_unstable();
            let mut unique = invoked.clone();
            unique.dedup();

            prop_assert_eq!(unique.len(), invoked.len(), "an action fired twice");
        }

        #[test]
        fn the_bound_holds_through_any_sequence(ops in operations()) {
            let driven = drive(&ops);

            prop_assert!(driven.state.queue().len() <= 3);
            prop_assert_eq!(
                driven.state.queue().counters().created,
                driven.state.queue().len() as u64 + driven.state.queue().counters().settled()
            );
        }

        #[test]
        fn a_call_identity_never_produces_a_second_notification(ops in operations()) {
            let driven = drive(&ops);

            for (call, ids) in &driven.answered {
                prop_assert!(
                    ids.len() <= 1,
                    "call {} was answered with {} different identifiers",
                    call,
                    ids.len()
                );
            }
        }

        #[test]
        fn a_repeated_call_is_answered_as_before_and_counted(ops in operations()) {
            let driven = drive(&ops);

            let answered_twice = driven
                .answered
                .values()
                .filter(|ids| !ids.is_empty())
                .count() as u64;

            prop_assert!(
                driven.state.repeated_calls() <= driven.admitted.len() as u64,
                "a repeat is always one of the calls that was answered"
            );
            prop_assert!(answered_twice <= driven.admitted.len() as u64 + 1);
        }
    }
}
