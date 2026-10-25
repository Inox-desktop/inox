//! The daemon's side of the notification contract.
//! The daemon's side of the notification contract (ADR 0013).
//! ADR 0013 asks every L3 component to model its service boundary rather than
//! assume it: a snapshot with a generation for state that belongs to someone
//! else, and requests whose sent / accepted / applied / rejected outcomes are
//! distinguishable with a diagnosable reason. For the notification daemon the
//! boundary is shaped differently, and this module says why in the types:
//!
//! - **The daemon is the authority, not a client of one.** There is no snapshot
//!   and no generation to resynchronise after: the notification stack *is* the
//!   state, it lives here, and it is not reconstructed from anybody else. What
//!   the daemon owes a caller is the *result* of a call, which is what
//!   [`ReplyBody`] distinguishes — an identifier for a notification that was
//!   admitted, a boolean for a close that named something already gone (normal,
//!   not exceptional), the capability list, the server information, and
//!   [`ReplyBody::Rejected`] with the reason when the request itself was
//!   malformed.
//! - **A repeated call is answered, not repeated.** [`Calls`] remembers the
//!   answer to the last [`CALL_MEMORY`] calls, so a redelivered `Notify` — the
//!   deduplication ADR 0013 decision 3 asks for — returns the identifier it
//!   already produced instead of admitting a second notification. The memory is
//!   bounded and says how many answers it has forgotten, because "bounded" is
//!   only honest if the bound is visible.
//! - **What the daemon says is data too.** [`Reply`], [`Signal`] and
//!   [`Outgoing`] are what a transport carries; nothing here opens a socket, and
//!   there is no bus in this milestone (PRD §13 puts the D-Bus service in M5).
//!   The D-Bus leg is the only thing missing: the mapping from a `Notify`
//!   argument list to a [`crate::queue::Draft`] is implemented and tested, so the
//!   service that lands in M5 has the contract rather than a description of one.

use std::collections::{BTreeMap, VecDeque};

use crate::actions::{ActionError, ActionId, Actions};
use crate::markup::sanitize;
use crate::protocol::{CloseReason, Expiry, HintValue, Hints, ServerInformation};
use crate::queue::{Draft, NotificationId};

/// How many call answers the daemon remembers for deduplication.
pub const CALL_MEMORY: usize = 128;

/// The identity of one call the transport delivered.
///
/// D-Bus correlates a reply with the serial of the message it answers; nothing
/// in this daemon invents one, so this is the number the transport carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CallId(u64);

impl CallId {
    /// The identity a transport read off the wire.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw value, for the transport that carries the reply back.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for CallId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "call {}", self.0)
    }
}

/// A `Notify` call, argument for argument.
///
/// The fields are the names in [`crate::protocol::NOTIFY`], so a transport can
/// fill them in the order the signature states without translating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyRequest {
    /// The application's name for itself.
    pub app_name: String,
    /// The notification this one replaces, or `0` for a new one.
    pub replaces_id: u32,
    /// The icon name or path the application asks for.
    pub app_icon: String,
    /// The summary line.
    pub summary: String,
    /// The body, which may carry the markup subset
    /// [`crate::markup::sanitize`] accepts.
    pub body: String,
    /// The actions, as the flat key/label array the signature carries.
    pub actions: Vec<String>,
    /// The hints, as the variant table the signature carries.
    pub hints: Vec<(String, HintValue)>,
    /// What the client asks for in `expire_timeout`.
    pub expire_timeout: i32,
}

impl Default for NotifyRequest {
    /// A call with nothing in it but the timeout the specification reserves for
    /// "the server decides".
    ///
    /// `expire_timeout` cannot be defaulted to zero: on the wire `0` means
    /// "never expire", which is not what an empty call asks for.
    fn default() -> Self {
        Self {
            app_name: String::new(),
            replaces_id: 0,
            app_icon: String::new(),
            summary: String::new(),
            body: String::new(),
            actions: Vec::new(),
            hints: Vec::new(),
            expire_timeout: crate::protocol::EXPIRETIMEOUT_SERVER_DEFAULT,
        }
    }
}

impl NotifyRequest {
    /// The hints, read out of the table.
    pub fn hints(&self) -> Hints {
        Hints::from_pairs(&self.hints)
    }

    /// What the client asked for in `expire_timeout`.
    pub fn expiry(&self) -> Expiry {
        Expiry::from_wire(self.expire_timeout)
    }

    /// The actions the call offers.
    ///
    /// # Errors
    ///
    /// [`ActionError`] when the array is not a whole number of pairs, when a key
    /// is empty or repeated, or when there are more than
    /// [`crate::actions::MAX_ACTIONS`] of them — the request is rejected with
    /// that reason rather than quietly losing the excess.
    pub fn actions(&self) -> Result<Actions, ActionError> {
        Actions::from_flat(&self.actions)
    }

    /// The notification this call describes, ready for the queue.
    ///
    /// The body is sanitised here and nowhere else, so a [`Draft`] never carries
    /// raw markup: whatever reaches a popup has already been through
    /// [`sanitize`].
    ///
    /// # Errors
    ///
    /// As [`NotifyRequest::actions`].
    pub fn draft(&self) -> Result<Draft, ActionError> {
        Ok(Draft {
            app_name: self.app_name.clone(),
            app_icon: Some(self.app_icon.clone()).filter(|icon| !icon.trim().is_empty()),
            summary: self.summary.clone(),
            body: sanitize(&self.body),
            actions: self.actions()?,
            hints: self.hints(),
            replaces: NotificationId::new(self.replaces_id),
            expiry: self.expiry(),
        })
    }
}

/// What one call asks the daemon to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestBody {
    /// `Notify`.
    Notify(NotifyRequest),
    /// `CloseNotification`.
    Close(NotificationId),
    /// `GetCapabilities`.
    GetCapabilities,
    /// `GetServerInformation`.
    GetServerInformation,
}

/// A call the transport delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The identity the reply is correlated with.
    pub call: CallId,
    /// What it asks for.
    pub body: RequestBody,
}

impl Request {
    /// A call the transport delivered.
    pub fn new(call: CallId, body: RequestBody) -> Self {
        Self { call, body }
    }
}

/// What the daemon answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyBody {
    /// The notification was admitted, under this identifier.
    Notified {
        /// The identifier the client gets back.
        id: NotificationId,
    },
    /// `CloseNotification` was handled.
    ///
    /// `false` means the identifier named nothing the daemon held, which is the
    /// normal outcome for a notification that already expired — reported rather
    /// than swallowed.
    Closed {
        /// Whether something was closed.
        closed: bool,
    },
    /// `GetCapabilities` was handled.
    Capabilities {
        /// The capabilities Inox claims ([`crate::protocol::CAPABILITIES`]).
        capabilities: Vec<&'static str>,
    },
    /// `GetServerInformation` was handled.
    ServerInformation {
        /// What the daemon reports about itself.
        information: ServerInformation,
    },
    /// The request was refused, with the reason a caller can act on.
    Rejected {
        /// Why it was refused.
        reason: ActionError,
    },
}

/// A reply to one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The call it answers.
    pub call: CallId,
    /// What it says.
    pub body: ReplyBody,
}

/// Something that happened to a notification, for the applications waiting on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// The notification is gone, and this is why (`NotificationClosed`).
    NotificationClosed {
        /// Which notification.
        id: NotificationId,
        /// Why it is gone.
        reason: CloseReason,
    },
    /// One of the notification's actions was invoked (`ActionInvoked`).
    ActionInvoked {
        /// Which notification.
        id: NotificationId,
        /// Which action, as the client's own key.
        action: ActionId,
    },
}

impl Signal {
    /// The `NotificationClosed` signal for `id`.
    pub fn closed(id: NotificationId, reason: CloseReason) -> Self {
        Self::NotificationClosed { id, reason }
    }

    /// The `ActionInvoked` signal for `id`.
    pub fn invoked(id: NotificationId, action: ActionId) -> Self {
        Self::ActionInvoked { id, action }
    }

    /// The notification this signal is about.
    pub const fn id(&self) -> NotificationId {
        match self {
            Self::NotificationClosed { id, .. } | Self::ActionInvoked { id, .. } => *id,
        }
    }
}

/// Something the daemon has to tell the outside world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// An answer to a call.
    Reply(Reply),
    /// A signal an application is waiting for.
    Signal(Signal),
    /// The user followed a link in a notification body.
    ///
    /// Opening it is not this daemon's to do — it is a launch, which belongs
    /// with the rest of the desktop's launching (an `xdg-open`, or a portal when
    /// one exists). What the daemon owes the transport is the target, which has
    /// already been checked by [`crate::markup`].
    FollowLink {
        /// The notification the link was in.
        id: NotificationId,
        /// The target.
        href: String,
    },
}

/// The answers the daemon remembers, so that a repeated call is answered rather
/// than acted on twice.
///
/// Bounded on purpose: a session runs for days, and a map that only grows is a
/// leak. The oldest answer is forgotten first, and [`Calls::forgotten`] counts
/// how many have been, so the bound is observable rather than assumed.
#[derive(Debug)]
pub struct Calls {
    capacity: usize,
    replies: BTreeMap<u64, ReplyBody>,
    order: VecDeque<u64>,
    forgotten: u64,
}

impl Calls {
    /// A memory of [`CALL_MEMORY`] answers.
    pub fn new() -> Self {
        Self::with_capacity(CALL_MEMORY)
    }

    /// A memory of `capacity` answers.
    ///
    /// A capacity of zero remembers nothing, which is a legal configuration: the
    /// daemon then answers every call as if it were new.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            replies: BTreeMap::new(),
            order: VecDeque::new(),
            forgotten: 0,
        }
    }

    /// How many answers are remembered.
    pub fn len(&self) -> usize {
        self.replies.len()
    }

    /// Whether nothing is remembered.
    pub fn is_empty(&self) -> bool {
        self.replies.is_empty()
    }

    /// How many answers have been forgotten to stay within the bound.
    pub fn forgotten(&self) -> u64 {
        self.forgotten
    }

    /// The answer this call already has, if it has one.
    pub fn answered(&self, call: CallId) -> Option<&ReplyBody> {
        self.replies.get(&call.get())
    }

    /// Remembers the answer to `call`.
    ///
    /// Returns `false` when an answer for that call was already there, which is
    /// the caller's cue that it has just handled a repeated call.
    pub fn record(&mut self, call: CallId, body: ReplyBody) -> bool {
        if self.capacity == 0 {
            self.forgotten += 1;
            return false;
        }

        if self.replies.contains_key(&call.get()) {
            return false;
        }

        if self.replies.len() >= self.capacity
            && let Some(oldest) = self.order.pop_front()
        {
            self.replies.remove(&oldest);
            self.forgotten += 1;
        }

        self.order.push_back(call.get());
        self.replies.insert(call.get(), body);

        true
    }
}

impl Default for Calls {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CAPABILITIES, HINT_URGENCY, SERVER_INFORMATION, Urgency};
    use proptest::prelude::*;
    use std::time::Instant;

    fn notify(body: &str) -> NotifyRequest {
        NotifyRequest {
            app_name: "app".to_owned(),
            summary: "summary".to_owned(),
            body: body.to_owned(),
            ..NotifyRequest::default()
        }
    }

    #[test]
    fn a_draft_sanitises_the_body_before_the_queue_sees_it() {
        let draft = notify("hello <b>world</b> <script>alert(1)</script>")
            .draft()
            .expect("a well-formed request");

        assert_eq!(
            crate::markup::plain_text(&draft.body),
            "hello world <script>alert(1)</script>"
        );
        assert!(
            draft.body.iter().any(|span| !span.emphasis().is_plain()),
            "the accepted markup survived"
        );
    }

    #[test]
    fn a_blank_icon_is_no_icon() {
        assert_eq!(notify("x").draft().expect("request").app_icon, None);

        let draft = NotifyRequest {
            app_icon: "  ".to_owned(),
            ..notify("x")
        }
        .draft()
        .expect("request");

        assert_eq!(draft.app_icon, None);

        let draft = NotifyRequest {
            app_icon: "firefox".to_owned(),
            ..notify("x")
        }
        .draft()
        .expect("request");

        assert_eq!(draft.app_icon.as_deref(), Some("firefox"));
    }

    #[test]
    fn an_identifier_of_zero_means_a_new_notification() {
        let draft = notify("x").draft().expect("request");

        assert!(draft.replaces.is_none());

        let draft = NotifyRequest {
            replaces_id: 12,
            ..notify("x")
        }
        .draft()
        .expect("request");

        assert_eq!(draft.replaces, NotificationId::new(12));
    }

    #[test]
    fn the_hints_and_the_timeout_reach_the_draft() {
        let draft = NotifyRequest {
            hints: vec![(HINT_URGENCY.to_owned(), HintValue::Byte(0))],
            expire_timeout: 2_500,
            ..notify("x")
        }
        .draft()
        .expect("request");

        assert_eq!(draft.hints.urgency, Urgency::Low);
        assert_eq!(draft.expiry, Expiry::After(2_500));

        let draft = NotifyRequest {
            expire_timeout: -1,
            ..notify("x")
        }
        .draft()
        .expect("request");

        assert_eq!(draft.expiry, Expiry::ServerDefault);
        assert_eq!(draft.hints, Hints::default());
    }

    #[test]
    fn malformed_actions_reject_the_request_with_a_reason() {
        let request = NotifyRequest {
            actions: vec!["only-a-key".to_owned()],
            ..notify("x")
        };

        assert_eq!(
            request.draft(),
            Err(ActionError::MalformedPairs { count: 1 })
        );
        assert_eq!(
            request.actions(),
            Err(ActionError::MalformedPairs { count: 1 })
        );
    }

    #[test]
    fn well_formed_actions_reach_the_draft() {
        let draft = NotifyRequest {
            actions: vec![
                "default".to_owned(),
                String::new(),
                "open".to_owned(),
                "Open".to_owned(),
            ],
            ..notify("x")
        }
        .draft()
        .expect("request");

        assert_eq!(draft.actions.len(), 2);
        assert!(draft.actions.default_action().is_some());
    }

    #[test]
    fn the_capability_list_is_the_one_the_protocol_module_declares() {
        let reply = ReplyBody::Capabilities {
            capabilities: CAPABILITIES.to_vec(),
        };

        let ReplyBody::Capabilities { capabilities } = reply else {
            unreachable!("built just above")
        };

        assert!(capabilities.contains(&"body-markup"));
    }

    #[test]
    fn the_server_information_is_the_one_the_protocol_module_declares() {
        let reply = ReplyBody::ServerInformation {
            information: SERVER_INFORMATION,
        };

        let ReplyBody::ServerInformation { information } = reply else {
            unreachable!("built just above")
        };

        assert_eq!(information.name, "Inox");
        assert_eq!(information.spec_version, "1.2");
    }

    #[test]
    fn a_signal_knows_which_notification_it_is_about() {
        let id = NotificationId::new(4);

        assert_eq!(Signal::closed(id, CloseReason::Expired).id(), id);
        assert_eq!(Signal::invoked(id, ActionId::new("open")).id(), id);
        assert_eq!(
            Signal::closed(id, CloseReason::ClosedByCall),
            Signal::NotificationClosed {
                id,
                reason: CloseReason::ClosedByCall
            }
        );
    }

    // --- call correlation and deduplication --------------------------------

    #[test]
    fn an_answer_that_was_recorded_is_handed_back() {
        let mut calls = Calls::new();
        let call = CallId::new(7);

        assert!(calls.answered(call).is_none());
        assert!(calls.record(
            call,
            ReplyBody::Notified {
                id: NotificationId::new(1)
            }
        ));
        assert_eq!(
            calls.answered(call),
            Some(&ReplyBody::Notified {
                id: NotificationId::new(1)
            })
        );
        assert_eq!(calls.len(), 1);
    }

    #[test]
    fn recording_the_same_call_twice_does_not_grow_the_memory() {
        let mut calls = Calls::new();
        let call = CallId::new(7);
        let answer = ReplyBody::Closed { closed: true };

        assert!(calls.record(call, answer.clone()));
        assert!(!calls.record(call, answer));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls.forgotten(), 0);
    }

    #[test]
    fn the_oldest_answer_is_forgotten_first() {
        let mut calls = Calls::with_capacity(2);

        for raw in 0..3u64 {
            calls.record(CallId::new(raw), ReplyBody::Closed { closed: true });
        }

        assert!(calls.answered(CallId::new(0)).is_none(), "the oldest went");
        assert!(calls.answered(CallId::new(1)).is_some());
        assert!(calls.answered(CallId::new(2)).is_some());
        assert_eq!(calls.len(), 2);
        assert_eq!(calls.forgotten(), 1);
    }

    #[test]
    fn a_memory_of_nothing_remembers_nothing() {
        let mut calls = Calls::with_capacity(0);
        let call = CallId::new(1);

        assert!(!calls.record(call, ReplyBody::Closed { closed: true }));
        assert!(calls.answered(call).is_none());
        assert!(calls.is_empty());
        assert_eq!(calls.forgotten(), 1);
    }

    #[test]
    fn the_default_memory_is_the_documented_size() {
        let calls = Calls::new();

        assert_eq!(calls.len(), 0);
        assert_eq!(CALL_MEMORY, 128);
        assert_eq!(CallId::new(3).get(), 3);
        assert_eq!(CallId::new(3).to_string(), "call 3");
    }

    // --- the requests a transport will deliver ------------------------------

    #[test]
    fn every_request_can_be_built_and_answered() {
        let requests = [
            Request::new(CallId::new(1), RequestBody::Notify(notify("x"))),
            Request::new(CallId::new(2), RequestBody::Close(NotificationId::new(3))),
            Request::new(CallId::new(3), RequestBody::GetCapabilities),
            Request::new(CallId::new(4), RequestBody::GetServerInformation),
        ];

        for request in requests {
            let reply = Reply {
                call: request.call,
                body: ReplyBody::Closed { closed: false },
            };

            assert_eq!(reply.call, request.call);
        }
    }

    // --- the definition of "the daemon answered the same way twice" --------

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn the_memory_keeps_the_most_recent_calls_and_forgets_the_rest(
            calls in proptest::collection::vec(0u64..40, 0..120),
            capacity in 1usize..8,
        ) {
            let mut memory = Calls::with_capacity(capacity);
            let mut recorded: Vec<u64> = Vec::new();

            for raw in &calls {
                let call = CallId::new(*raw);
                let was_known = memory.answered(call).is_some();
                let recorded_now = memory.record(call, ReplyBody::Closed { closed: true });

                prop_assert_eq!(
                    recorded_now,
                    !was_known,
                    "an answer is recorded exactly when it was not already known"
                );
                if !recorded.contains(raw) {
                    recorded.push(*raw);
                }
            }

            prop_assert!(memory.len() <= capacity);
            prop_assert!(memory.len() <= recorded.len());

            let expected = recorded.len().min(capacity);
            prop_assert_eq!(memory.len(), expected, "the bound, not the history, decides");
        }

        #[test]
        fn a_call_that_was_never_recorded_has_no_answer(raw in 0u64..1_000, calls in proptest::collection::vec(1_000u64..2_000, 0..40)) {
            let mut memory = Calls::with_capacity(64);
            for call in &calls {
                memory.record(CallId::new(*call), ReplyBody::Closed { closed: true });
            }

            prop_assert!(memory.answered(CallId::new(raw)).is_none());
        }

        #[test]
        fn a_draft_never_carries_markup_that_could_be_executed(body in "\\PC{0,200}") {
            let draft = notify(&body).draft().expect("no actions, so nothing to reject");

            for span in &draft.body {
                if let crate::markup::Span::Link { href, .. } = span {
                    let lowered = href.to_ascii_lowercase();
                    prop_assert!(!lowered.starts_with("javascript:"));
                    prop_assert!(!lowered.starts_with("data:"));
                }
            }
        }
    }

    /// The queue is what a `Notify` ultimately reaches; this is a smoke test
    /// that the pieces fit together the way the daemon uses them.
    #[test]
    fn a_request_reaches_the_queue_as_a_notification() {
        let mut queue = crate::queue::Queue::new(
            inox_config::QueueConfig::default(),
            inox_config::TimeoutConfig::default(),
            3,
        );

        let draft = NotifyRequest {
            actions: vec!["open".to_owned(), "Open".to_owned()],
            ..notify("the body")
        }
        .draft()
        .expect("request");

        let admission = queue.notify(draft, Instant::now());
        let notification = queue.get(admission.id).expect("it is held");

        assert_eq!(notification.summary, "summary");
        assert_eq!(notification.actions.len(), 1);
        assert_eq!(crate::markup::plain_text(&notification.body), "the body");
    }
}
