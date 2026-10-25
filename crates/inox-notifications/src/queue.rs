//! The notifications the daemon owns: identifiers, lifetimes, and the bound.
//!
//! This is the model acceptance D06 is written against. Its rules, in one place:
//!
//! - **Identifiers are the daemon's.** [`NotificationId`] is allocated here,
//!   monotonically, and a notification keeps its identifier for its whole life —
//!   including when a client replaces it with `replaces_id`, which updates the
//!   content in place and restarts the lifetime rather than creating a second
//!   notification.
//! - **Every notification ends exactly once.** It is closed by a call, dismissed
//!   by the user, expired by its own lifetime, dropped or coalesced away by a
//!   full queue — five terminal outcomes and no sixth, with no path that lets a
//!   notification disappear without one of them being counted (see [`Counters`],
//!   which is the observability D06 asks for).
//! - **The queue is bounded.** [`QueueConfig::max_pending`] is a hard limit and
//!   [`QueueConfig::policy`] says what happens at it, so a burst of a hundred
//!   notifications a second cannot grow the daemon's memory without bound.
//! - **A critical notification never expires on its own**, whatever the client
//!   asked for: something the user has to see stays until the user has seen it.
//!
//! Time is always passed in ([`Queue::notify`] takes the moment, [`Queue::expire`]
//! the tick that swept): the model has no clock of its own, which is what makes
//! every lifetime rule here a unit test rather than a wait.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use inox_config::{QueueConfig, QueuePolicy, TimeoutConfig};

use crate::actions::Actions;
use crate::markup::Span;
use crate::protocol::{CloseReason, Expiry, Hints, Urgency};

/// The identifier of a notification.
///
/// Zero is the identifier no notification has: it is what a `Notify` call sends
/// as `replaces_id` to mean "this is a new notification".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NotificationId(u32);

impl NotificationId {
    /// The identifier a `Notify` call uses for "not a replacement".
    pub const NONE: Self = Self(0);

    /// The identifier a raw value names, for a transport that read it off the
    /// bus or a test that needs to name one.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw value, for the wire.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Whether this is [`NotificationId::NONE`], which names no notification.
    pub const fn is_none(self) -> bool {
        self.0 == 0
    }
}

impl Default for NotificationId {
    /// The identifier no notification has, which is what a draft that replaces
    /// nothing names.
    fn default() -> Self {
        Self::NONE
    }
}

impl std::fmt::Display for NotificationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "#{}", self.0)
    }
}

/// How long a notification may live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    /// It never expires on its own; only a close ends it.
    Never,
    /// It expires this long after it was admitted, or after it was replaced.
    After(Duration),
}

impl Lifetime {
    /// The lifetime a notification has, from its urgency, what the client asked
    /// for, and the daemon's configuration.
    ///
    /// The rules, in the order they apply:
    ///
    /// 1. a critical notification never expires on its own, whatever
    ///    `expire_timeout` said;
    /// 2. `expire_timeout = 0` means the same thing;
    /// 3. a positive `expire_timeout` is the lifetime;
    /// 4. otherwise the configured lifetime for the urgency applies, and a
    ///    configured `0` is "never" again.
    pub fn of(urgency: Urgency, expiry: Expiry, timeouts: &TimeoutConfig) -> Self {
        if urgency == Urgency::Critical {
            return Self::Never;
        }

        match expiry {
            Expiry::Never => Self::Never,
            Expiry::After(milliseconds) => {
                Self::After(Duration::from_millis(u64::from(milliseconds)))
            }
            Expiry::ServerDefault => {
                let configured = match urgency {
                    Urgency::Low => timeouts.low_ms,
                    Urgency::Normal | Urgency::Critical => timeouts.normal_ms,
                };

                if configured == 0 {
                    Self::Never
                } else {
                    Self::After(Duration::from_millis(u64::from(configured)))
                }
            }
        }
    }

    /// When a notification admitted at `now` expires, if it does.
    pub fn deadline(self, now: Instant) -> Option<Instant> {
        match self {
            Self::Never => None,
            Self::After(duration) => Some(now + duration),
        }
    }

    /// Whether this is [`Lifetime::Never`].
    pub const fn is_never(self) -> bool {
        matches!(self, Self::Never)
    }
}

/// A notification the daemon owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The identifier this notification answers to, for its whole life.
    pub id: NotificationId,
    /// The application that sent it, as it named itself.
    pub app_name: String,
    /// The icon the application asked for, if it asked for one.
    pub app_icon: Option<String>,
    /// The summary, one line.
    pub summary: String,
    /// The sanitised body ([`crate::markup`]), never raw markup.
    pub body: Vec<Span>,
    /// The actions it offers, with their at-most-once record.
    pub actions: Actions,
    /// The hints it carried.
    pub hints: Hints,
    /// How long it may live.
    pub lifetime: Lifetime,
    /// When it was admitted (or last replaced).
    pub arrived: Instant,
    /// When it expires, if it does.
    pub deadline: Option<Instant>,
}

impl Notification {
    /// The urgency it was sent with.
    pub fn urgency(&self) -> Urgency {
        self.hints.urgency
    }

    /// Whether its lifetime has run out at `now`.
    pub fn is_due(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| deadline <= now)
    }

    /// Whether it never expires on its own.
    pub fn is_permanent(&self) -> bool {
        self.deadline.is_none()
    }

    /// The icon reference a popup could draw: the notification's own icon, or
    /// the `image-path` hint.
    ///
    /// This daemon does not decode either yet; the reference is carried so that
    /// the popup can show that an icon exists, and so that a locked session can
    /// withhold it.
    pub fn icon_reference(&self) -> Option<&str> {
        self.app_icon
            .as_deref()
            .filter(|icon| !icon.trim().is_empty())
            .or_else(|| self.hints.icon_reference())
    }
}

/// A notification as it arrives, before the daemon has given it an identifier.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    /// The application that sent it.
    pub app_name: String,
    /// The icon the application asked for, if any.
    pub app_icon: Option<String>,
    /// The summary.
    pub summary: String,
    /// The sanitised body.
    pub body: Vec<Span>,
    /// The actions it offers.
    pub actions: Actions,
    /// The hints it carried.
    pub hints: Hints,
    /// The notification this one replaces, or [`NotificationId::NONE`].
    pub replaces: NotificationId,
    /// What the client asked for in `expire_timeout`.
    pub expiry: Expiry,
}

/// What came of admitting a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admission {
    /// The identifier the notification has: one it kept, or a new one.
    pub id: NotificationId,
    /// The notification it replaced, which keeps its place in the stack.
    pub replaced: Option<NotificationId>,
    /// The notification that made room for it, if the queue was full.
    pub evicted: Option<Eviction>,
}

/// How a full queue made room for a new notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eviction {
    /// The oldest waiting notification was discarded
    /// ([`QueuePolicy::DropOldest`]).
    Dropped {
        /// The notification that was discarded.
        id: NotificationId,
    },
    /// A waiting notification of the same application was replaced
    /// ([`QueuePolicy::CoalesceByApp`]).
    Coalesced {
        /// The notification that was replaced.
        id: NotificationId,
    },
}

impl Eviction {
    /// The notification that is gone.
    pub const fn id(self) -> NotificationId {
        match self {
            Self::Dropped { id } | Self::Coalesced { id } => id,
        }
    }
}

/// A notification that reached a terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed {
    /// Which notification.
    pub id: NotificationId,
    /// Why, in the terms `NotificationClosed` carries.
    pub reason: CloseReason,
}

/// What the queue has done, for the observability D06 asks for.
///
/// The counters are the answer to "the bounded queue and its drop/coalesce
/// policy must be observable": an eviction is never silent, it is one of
/// [`Counters::dropped`] or [`Counters::coalesced`], and the identifier it
/// happened to is reported through [`Admission::evicted`] and as a
/// `NotificationClosed` signal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Notifications admitted, replacements included.
    pub admitted: u64,
    /// Admitted notifications that replaced a live one and kept its identifier.
    pub replaced: u64,
    /// Notifications that reached the queue as a new identifier.
    pub created: u64,
    /// Notifications a full queue discarded.
    pub dropped: u64,
    /// Notifications a full queue coalesced into a newer one.
    pub coalesced: u64,
    /// Notifications that reached the end of their lifetime.
    pub expired: u64,
    /// Notifications the user dismissed.
    pub dismissed: u64,
    /// Notifications a client closed with `CloseNotification`.
    pub closed_by_call: u64,
}

impl Counters {
    /// Every notification that reached a terminal state, of any kind.
    pub const fn settled(&self) -> u64 {
        self.dropped + self.coalesced + self.expired + self.dismissed + self.closed_by_call
    }
}

/// The notifications the daemon holds.
///
/// In arrival order, oldest first; the newest `visible` of them are the ones a
/// popup stack shows, and the rest are waiting.
#[derive(Debug)]
pub struct Queue {
    queue: QueueConfig,
    timeouts: TimeoutConfig,
    visible: usize,
    next_id: u32,
    items: VecDeque<Notification>,
    counters: Counters,
}

impl Queue {
    /// An empty queue.
    ///
    /// `visible` is how many of the notifications the popup stack shows at once
    /// ([`inox_config::DisplayConfig::max_visible`]); the queue needs it to know
    /// what is waiting, which is what both eviction policies act on.
    pub fn new(queue: QueueConfig, timeouts: TimeoutConfig, visible: usize) -> Self {
        Self {
            queue,
            timeouts,
            visible,
            next_id: 1,
            items: VecDeque::new(),
            counters: Counters::default(),
        }
    }

    /// How many notifications are held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the daemon holds nothing.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// What the queue has done so far.
    pub fn counters(&self) -> Counters {
        self.counters
    }

    /// How many notifications are shown at once.
    pub fn visible(&self) -> usize {
        self.visible
    }

    /// How many notifications are waiting behind the visible ones.
    pub fn waiting(&self) -> usize {
        self.items.len().saturating_sub(self.visible)
    }

    /// The notification called `id`, if it is still held.
    pub fn get(&self, id: NotificationId) -> Option<&Notification> {
        self.items.iter().find(|item| item.id == id)
    }

    /// The notification called `id`, for a caller that has to change it.
    ///
    /// Used for one thing: recording an action invocation
    /// ([`crate::actions::Actions::invoke`]).
    pub fn get_mut(&mut self, id: NotificationId) -> Option<&mut Notification> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    /// Every notification, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Notification> {
        self.items.iter()
    }

    /// The newest `count` notifications, newest first: the popup stack.
    pub fn newest(&self, count: usize) -> Vec<&Notification> {
        self.items.iter().rev().take(count).collect()
    }

    /// Admits a notification.
    ///
    /// A draft whose `replaces` names a live notification updates it in place:
    /// same identifier, same place in the stack, lifetime restarted. Anything
    /// else is a new notification, and a full queue makes room for it according
    /// to [`QueueConfig::policy`].
    pub fn notify(&mut self, draft: Draft, now: Instant) -> Admission {
        self.counters.admitted += 1;

        let lifetime = Lifetime::of(draft.hints.urgency, draft.expiry, &self.timeouts);
        let deadline = lifetime.deadline(now);

        if !draft.replaces.is_none()
            && let Some(index) = self.items.iter().position(|item| item.id == draft.replaces)
        {
            let id = self.items[index].id;
            self.items[index] = Self::build(id, draft, lifetime, deadline, now);
            self.counters.replaced += 1;

            return Admission {
                id,
                replaced: Some(id),
                evicted: None,
            };
        }

        let id = self.allocate();
        self.counters.created += 1;

        let evicted = self.make_room(draft.app_name.as_str());
        self.items
            .push_back(Self::build(id, draft, lifetime, deadline, now));

        Admission {
            id,
            replaced: None,
            evicted,
        }
    }

    /// Closes a notification, with the reason the caller has for it.
    ///
    /// Returns `None` when the identifier names nothing the queue holds — a
    /// normal outcome, not an error: a client may close a notification that
    /// expired a moment ago.
    pub fn close(&mut self, id: NotificationId, reason: CloseReason) -> Option<Closed> {
        let index = self.items.iter().position(|item| item.id == id)?;
        let removed = self.items.remove(index)?;

        match reason {
            CloseReason::Expired => self.counters.expired += 1,
            CloseReason::Dismissed => self.counters.dismissed += 1,
            CloseReason::ClosedByCall => self.counters.closed_by_call += 1,
            CloseReason::Undefined => {}
        }

        Some(Closed {
            id: removed.id,
            reason,
        })
    }

    /// Closes every notification whose lifetime has run out at `now`.
    pub fn expire(&mut self, now: Instant) -> Vec<Closed> {
        let due: Vec<NotificationId> = self
            .items
            .iter()
            .filter(|item| item.is_due(now))
            .map(|item| item.id)
            .collect();

        due.into_iter()
            .filter_map(|id| self.close(id, CloseReason::Expired))
            .collect()
    }

    /// Applies a reloaded configuration.
    ///
    /// Capacity applies at once: a queue that has been told to hold less drops
    /// the oldest notifications until it fits, and reports each one so the
    /// daemon can tell its client. Lifetimes do **not** change for
    /// notifications already held — they were admitted under the lifetime they
    /// were given, and re-timing a notification a user is reading is a worse
    /// surprise than letting it run out as promised.
    pub fn reconfigure(
        &mut self,
        queue: QueueConfig,
        timeouts: TimeoutConfig,
        visible: usize,
    ) -> Vec<Closed> {
        self.queue = queue;
        self.timeouts = timeouts;
        self.visible = visible;

        let mut closed = Vec::new();

        while self.items.len() > self.bound() {
            let Some(removed) = self.items.pop_front() else {
                break;
            };

            self.counters.dropped += 1;
            closed.push(Closed {
                id: removed.id,
                reason: CloseReason::Undefined,
            });
        }

        closed
    }

    /// The queue's bound, as the queue enforces it.
    ///
    /// A configured bound of zero is read as one: `Notify` has to answer with
    /// an identifier, so a daemon that admitted nothing could not do its job —
    /// and the configuration schema rejects a zero anyway
    /// ([`inox_config::QueueConfig`]), which makes this the second line of
    /// defence rather than the first.
    fn bound(&self) -> usize {
        usize::from(self.queue.max_pending).max(1)
    }

    /// Makes room for a notification from `app`, if the queue is full.
    fn make_room(&mut self, app: &str) -> Option<Eviction> {
        if self.items.len() < self.bound() {
            return None;
        }

        if self.queue.policy == QueuePolicy::CoalesceByApp
            && let Some(index) = self.coalescible(app)
        {
            let removed = self.items.remove(index)?;
            self.counters.coalesced += 1;

            return Some(Eviction::Coalesced { id: removed.id });
        }

        let removed = self.items.pop_front()?;
        self.counters.dropped += 1;

        Some(Eviction::Dropped { id: removed.id })
    }

    /// The oldest *waiting* notification of `app`, if it has one.
    ///
    /// Only what has not been shown yet may be coalesced away: a notification
    /// the user is reading is not something a later one may take off the screen.
    fn coalescible(&self, app: &str) -> Option<usize> {
        let waiting = self.items.len().saturating_sub(self.visible);

        self.items
            .iter()
            .take(waiting)
            .position(|item| item.app_name == app)
    }

    /// The next free identifier.
    ///
    /// Monotonic until the `u32` identifier space is exhausted, which is the
    /// widest the specification offers; the loop then steps over the identifiers
    /// still in use, so an identifier is never reused while the notification
    /// that has it is alive. The loop is bounded by the queue's own length: at
    /// most that many values can be taken, and the space has billions of them.
    fn allocate(&mut self) -> NotificationId {
        let mut raw = if self.next_id == 0 { 1 } else { self.next_id };

        while self.items.iter().any(|item| item.id == NotificationId(raw)) {
            raw = Self::step(raw);
        }

        self.next_id = Self::step(raw);

        NotificationId(raw)
    }

    /// The next raw identifier, wrapping inside the range identifiers may use.
    fn step(raw: u32) -> u32 {
        if raw == u32::MAX { 1 } else { raw + 1 }
    }

    /// Assembles a notification from a draft.
    fn build(
        id: NotificationId,
        draft: Draft,
        lifetime: Lifetime,
        deadline: Option<Instant>,
        now: Instant,
    ) -> Notification {
        Notification {
            id,
            app_name: draft.app_name,
            app_icon: draft.app_icon,
            summary: draft.summary,
            body: draft.body,
            actions: draft.actions,
            hints: draft.hints,
            lifetime,
            arrived: now,
            deadline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup::sanitize;
    use proptest::prelude::*;

    fn queue(config: QueueConfig, visible: usize) -> Queue {
        Queue::new(config, TimeoutConfig::default(), visible)
    }

    fn draft(summary: &str) -> Draft {
        Draft {
            app_name: "app".to_owned(),
            summary: summary.to_owned(),
            ..Draft::default()
        }
    }

    fn from_app(app: &str, summary: &str) -> Draft {
        Draft {
            app_name: app.to_owned(),
            summary: summary.to_owned(),
            ..Draft::default()
        }
    }

    fn critical(summary: &str) -> Draft {
        Draft {
            hints: Hints {
                urgency: Urgency::Critical,
                ..Hints::default()
            },
            ..draft(summary)
        }
    }

    fn arriving(expiry: Expiry, summary: &str) -> Draft {
        Draft {
            expiry,
            ..draft(summary)
        }
    }

    // --- identifiers -------------------------------------------------------

    #[test]
    fn identifiers_start_at_one_and_are_never_zero() {
        let mut queue = queue(QueueConfig::default(), 3);

        let first = queue.notify(draft("one"), Instant::now()).id;
        let second = queue.notify(draft("two"), Instant::now()).id;

        assert_eq!(first, NotificationId::new(1));
        assert_eq!(second, NotificationId::new(2));
        assert!(!first.is_none());
        assert!(NotificationId::NONE.is_none());
        assert_eq!(NotificationId::NONE.get(), 0);
        assert_eq!(first.to_string(), "#1");
    }

    #[test]
    fn a_closed_identifier_is_not_handed_out_again() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();
        let first = queue.notify(draft("one"), now).id;

        assert!(queue.close(first, CloseReason::Dismissed).is_some());
        let second = queue.notify(draft("two"), now).id;

        assert_ne!(first, second);
    }

    #[test]
    fn the_identifier_space_wrapping_skips_what_is_still_alive() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        // Two notifications are held, so both have to be stepped over: one sits
        // just below the end of the space, the other at the top of it.
        queue.next_id = u32::MAX;
        let last = queue.notify(draft("last"), now).id;

        let first = queue.notify(draft("first"), now).id;
        let second = queue.notify(draft("second"), now).id;

        assert_eq!(last, NotificationId::new(u32::MAX));
        assert_eq!(first, NotificationId::new(1), "the space wraps");
        assert_eq!(second, NotificationId::new(2));
        assert_ne!(last, first);
        assert_ne!(first, second);
    }

    #[test]
    fn an_identifier_in_use_is_skipped_even_at_the_wrap() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        queue.next_id = 1;
        let first = queue.notify(draft("one"), now).id;
        // Pretend the counter wrapped back onto an identifier that is taken.
        queue.next_id = 1;
        let second = queue.notify(draft("two"), now).id;

        assert_eq!(first, NotificationId::new(1));
        assert_eq!(second, NotificationId::new(2));
    }

    // --- replacement -------------------------------------------------------

    #[test]
    fn replacing_updates_in_place_and_keeps_the_identifier_and_the_place() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        let older = queue.notify(draft("older"), now).id;
        let replaced = queue.notify(draft("replace me"), now).id;
        let newest = queue.notify(draft("newest"), now).id;

        let admission = queue.notify(
            Draft {
                replaces: replaced,
                ..draft("replaced")
            },
            now,
        );

        assert_eq!(admission.id, replaced, "the identifier survives");
        assert_eq!(admission.replaced, Some(replaced));
        assert_eq!(admission.evicted, None);
        assert_eq!(
            queue
                .iter()
                .map(|item| item.summary.as_str())
                .collect::<Vec<_>>(),
            ["older", "replaced", "newest"],
            "the place in the stack is the one it had, not the newest"
        );
        assert_eq!(queue.len(), 3, "a replacement is not a second notification");
        assert_eq!(queue.get(older).map(|item| item.id), Some(older));
        assert_eq!(queue.get(newest).map(|item| item.id), Some(newest));
        assert_eq!(queue.counters().replaced, 1);
        assert_eq!(queue.counters().created, 3);
        assert_eq!(queue.counters().admitted, 4);
    }

    #[test]
    fn replacing_restarts_the_lifetime() {
        let mut queue = queue(QueueConfig::default(), 3);
        let start = Instant::now();

        let id = queue
            .notify(arriving(Expiry::After(1_000), "one"), start)
            .id;
        let later = start + Duration::from_millis(900);

        let replaced = queue.notify(
            Draft {
                replaces: id,
                ..arriving(Expiry::After(1_000), "one again")
            },
            later,
        );

        assert_eq!(replaced.id, id);
        assert_eq!(
            queue.get(id).and_then(|item| item.deadline),
            Some(later + Duration::from_millis(1_000)),
            "the clock starts again from the replacement"
        );
        assert!(
            queue.expire(later + Duration::from_millis(999)).is_empty(),
            "it is not due before its new deadline"
        );
        assert_eq!(queue.expire(later + Duration::from_millis(1_000)).len(), 1);
    }

    #[test]
    fn replacing_something_that_is_gone_makes_a_new_notification() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        let admission = queue.notify(
            Draft {
                replaces: NotificationId::new(404),
                ..draft("new")
            },
            now,
        );

        assert_eq!(admission.replaced, None, "there was nothing to replace");
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.counters().created, 1);
        assert_eq!(queue.counters().replaced, 0);
    }

    #[test]
    fn a_replacement_replaces_the_actions_too() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        let id = queue
            .notify(
                Draft {
                    actions: Actions::from_flat(&["open".to_owned(), "Open".to_owned()])
                        .expect("actions"),
                    ..draft("one")
                },
                now,
            )
            .id;

        let replaced = queue.notify(
            Draft {
                replaces: id,
                ..draft("one again")
            },
            now,
        );

        assert_eq!(replaced.id, id);
        assert!(
            queue.get(id).is_some_and(|item| item.actions.is_empty()),
            "the replaced notification's actions are the new ones, not the old"
        );
    }

    // --- lifetimes ---------------------------------------------------------

    #[test]
    fn a_critical_notification_never_expires_on_its_own() {
        let timeouts = TimeoutConfig::default();

        for expiry in [Expiry::ServerDefault, Expiry::Never, Expiry::After(1_000)] {
            assert_eq!(
                Lifetime::of(Urgency::Critical, expiry, &timeouts),
                Lifetime::Never,
                "{expiry:?} must not time out a critical notification"
            );
        }
    }

    #[test]
    fn a_critical_notification_survives_every_sweep() {
        let mut queue = queue(QueueConfig::default(), 3);
        let start = Instant::now();

        let id = queue.notify(critical("see me"), start).id;

        assert!(
            queue.get(id).is_some_and(Notification::is_permanent),
            "the default critical lifetime is `never`"
        );
        assert!(
            queue.expire(start + Duration::from_secs(86_400)).is_empty(),
            "a day later it is still there"
        );
        assert!(queue.get(id).is_some());
    }

    #[test]
    fn a_timeout_of_zero_means_never() {
        let mut queue = queue(QueueConfig::default(), 3);
        let start = Instant::now();

        let id = queue.notify(arriving(Expiry::Never, "stay"), start).id;

        assert!(queue.get(id).is_some_and(Notification::is_permanent));
        assert!(
            queue.expire(start + Duration::from_secs(3_600)).is_empty(),
            "nothing about a client's own timeout expires it"
        );
    }

    #[test]
    fn a_positive_timeout_is_the_lifetime() {
        let mut queue = queue(QueueConfig::default(), 3);
        let start = Instant::now();

        let id = queue
            .notify(arriving(Expiry::After(2_000), "briefly"), start)
            .id;

        assert!(
            queue
                .expire(start + Duration::from_millis(1_999))
                .is_empty()
        );
        assert_eq!(
            queue.expire(start + Duration::from_millis(2_000)),
            [Closed {
                id,
                reason: CloseReason::Expired
            }]
        );
        assert_eq!(queue.counters().expired, 1);
        assert!(queue.get(id).is_none());
    }

    #[test]
    fn the_server_default_comes_from_the_configured_lifetime_of_the_urgency() {
        let timeouts = TimeoutConfig {
            low_ms: 1_000,
            normal_ms: 2_000,
            critical_ms: 0,
        };

        assert_eq!(
            Lifetime::of(Urgency::Low, Expiry::ServerDefault, &timeouts),
            Lifetime::After(Duration::from_millis(1_000))
        );
        assert_eq!(
            Lifetime::of(Urgency::Normal, Expiry::ServerDefault, &timeouts),
            Lifetime::After(Duration::from_millis(2_000))
        );
    }

    #[test]
    fn a_configured_lifetime_of_zero_means_never() {
        let timeouts = TimeoutConfig {
            low_ms: 0,
            normal_ms: 0,
            critical_ms: 0,
        };

        assert_eq!(
            Lifetime::of(Urgency::Low, Expiry::ServerDefault, &timeouts),
            Lifetime::Never
        );
        assert_eq!(
            Lifetime::of(Urgency::Normal, Expiry::ServerDefault, &timeouts),
            Lifetime::Never
        );
        assert!(Lifetime::Never.is_never());
        assert!(!Lifetime::After(Duration::from_millis(1)).is_never());
    }

    #[test]
    fn a_sweep_closes_everything_that_is_due_and_nothing_else() {
        let mut queue = queue(QueueConfig::default(), 3);
        let start = Instant::now();

        let soon = queue.notify(arriving(Expiry::After(100), "soon"), start).id;
        let later = queue
            .notify(arriving(Expiry::After(500), "later"), start)
            .id;
        let never = queue.notify(arriving(Expiry::Never, "never"), start).id;

        let closed = queue.expire(start + Duration::from_millis(200));

        assert_eq!(
            closed,
            [Closed {
                id: soon,
                reason: CloseReason::Expired
            }]
        );
        assert!(queue.get(later).is_some());
        assert!(queue.get(never).is_some());
    }

    // --- closing -----------------------------------------------------------

    #[test]
    fn closing_reports_the_reason_and_never_twice() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();
        let id = queue.notify(draft("one"), now).id;

        assert_eq!(
            queue.close(id, CloseReason::ClosedByCall),
            Some(Closed {
                id,
                reason: CloseReason::ClosedByCall
            })
        );
        assert_eq!(queue.close(id, CloseReason::ClosedByCall), None);
        assert_eq!(queue.counters().closed_by_call, 1);
        assert!(queue.is_empty());
    }

    #[test]
    fn a_dismissal_is_counted_apart_from_a_close_call() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        let dismissed = queue.notify(draft("one"), now).id;
        let closed = queue.notify(draft("two"), now).id;

        queue.close(dismissed, CloseReason::Dismissed);
        queue.close(closed, CloseReason::ClosedByCall);

        let counters = queue.counters();
        assert_eq!(counters.dismissed, 1);
        assert_eq!(counters.closed_by_call, 1);
        assert_eq!(counters.expired, 0);
        assert_eq!(counters.settled(), 2);
    }

    #[test]
    fn closing_an_identifier_that_is_not_held_is_not_an_error() {
        let mut queue = queue(QueueConfig::default(), 3);

        assert!(
            queue
                .close(NotificationId::new(7), CloseReason::Dismissed)
                .is_none()
        );
        assert!(
            queue
                .close(NotificationId::NONE, CloseReason::Dismissed)
                .is_none()
        );
        assert_eq!(queue.counters().dismissed, 0);
    }

    // --- capacity ----------------------------------------------------------

    #[test]
    fn a_full_queue_drops_the_oldest_waiting_notification() {
        let config = QueueConfig {
            max_pending: 3,
            ..QueueConfig::default()
        };
        let mut queue = queue(config, 1);
        let now = Instant::now();

        let first = queue.notify(draft("one"), now).id;
        queue.notify(draft("two"), now);
        queue.notify(draft("three"), now);

        let admission = queue.notify(draft("four"), now);

        assert_eq!(admission.evicted, Some(Eviction::Dropped { id: first }));
        assert_eq!(queue.len(), 3, "the bound is a bound");
        assert_eq!(
            queue
                .iter()
                .map(|item| item.summary.as_str())
                .collect::<Vec<_>>(),
            ["two", "three", "four"]
        );
        assert_eq!(queue.counters().dropped, 1);
        assert_eq!(queue.counters().coalesced, 0);
    }

    #[test]
    fn drop_oldest_never_discards_what_is_on_screen_while_something_waits() {
        let config = QueueConfig {
            max_pending: 3,
            ..QueueConfig::default()
        };
        let mut queue = queue(config, 1);
        let now = Instant::now();

        queue.notify(draft("one"), now);
        let visible = queue.notify(draft("two"), now).id;
        queue.notify(draft("three"), now);

        // "one" is the oldest waiting notification; "two" is on screen.
        let admission = queue.notify(draft("four"), now);

        assert_eq!(
            admission.evicted,
            Some(Eviction::Dropped {
                id: NotificationId::new(1)
            })
        );
        assert!(
            queue.get(visible).is_some(),
            "a notification the user can see is not dropped behind their back"
        );
    }

    #[test]
    fn coalescing_replaces_the_waiting_notification_of_the_same_application() {
        let config = QueueConfig {
            max_pending: 3,
            policy: QueuePolicy::CoalesceByApp,
        };
        let mut queue = queue(config, 1);
        let now = Instant::now();

        let first = queue.notify(from_app("chat", "one"), now).id;
        let shown = queue.notify(from_app("other", "two"), now).id;
        let waiting = queue.notify(from_app("chat", "three"), now).id;

        let admission = queue.notify(from_app("chat", "four"), now);

        assert_eq!(
            admission.evicted,
            Some(Eviction::Coalesced { id: first }),
            "the waiting notification of the same application goes first"
        );
        assert!(queue.get(shown).is_some());
        assert!(queue.get(waiting).is_some());
        assert_eq!(queue.counters().coalesced, 1);
        assert_eq!(queue.counters().dropped, 0);
        assert_eq!(queue.len(), 3);
    }

    #[test]
    fn coalescing_falls_back_to_dropping_when_the_application_has_nothing_waiting() {
        let config = QueueConfig {
            max_pending: 2,
            policy: QueuePolicy::CoalesceByApp,
        };
        let mut queue = queue(config, 1);
        let now = Instant::now();

        // The one on screen is the chat notification, so the waiting one is
        // another application's: there is nothing of chat's to coalesce away,
        // and the oldest is dropped instead.
        let waiting = queue.notify(from_app("other", "one"), now).id;
        let shown = queue.notify(from_app("chat", "two"), now).id;

        let admission = queue.notify(from_app("chat", "three"), now);

        assert_eq!(
            admission.evicted,
            Some(Eviction::Dropped { id: waiting }),
            "nothing of the sending application's was waiting, so the oldest went"
        );
        assert_eq!(queue.counters().coalesced, 0);
        assert_eq!(queue.counters().dropped, 1);
        assert!(queue.get(shown).is_some());
    }

    #[test]
    fn a_queue_that_is_not_full_evicts_nothing() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        for index in 0..32 {
            let admission = queue.notify(draft(&format!("{index}")), now);
            assert_eq!(admission.evicted, None);
        }

        assert_eq!(queue.len(), 32);
        assert_eq!(queue.counters().dropped, 0);
    }

    #[test]
    fn a_bound_of_zero_is_read_as_one() {
        // The schema rejects a zero, so this cannot come from a configuration
        // file; a bound that admitted nothing would still leave `Notify` with
        // no identifier to answer with, so the queue never admits nothing.
        let mut queue = queue(
            QueueConfig {
                max_pending: 0,
                ..QueueConfig::default()
            },
            1,
        );
        let now = Instant::now();

        let first = queue.notify(draft("one"), now).id;
        queue.notify(draft("two"), now);

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.counters().dropped, 1);
        assert!(queue.get(first).is_none());
    }

    #[test]
    fn the_bound_is_exactly_the_configured_one() {
        let mut queue = queue(
            QueueConfig {
                max_pending: 1,
                ..QueueConfig::default()
            },
            1,
        );
        let now = Instant::now();

        queue.notify(draft("one"), now);
        queue.notify(draft("two"), now);

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.counters().dropped, 1);
    }

    // --- waiting and the visible window ------------------------------------

    #[test]
    fn the_waiting_count_is_what_the_stack_cannot_show() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        assert_eq!(queue.waiting(), 0);

        for index in 0..5 {
            queue.notify(draft(&format!("{index}")), now);
        }

        assert_eq!(queue.visible(), 3);
        assert_eq!(queue.waiting(), 2);
        assert_eq!(queue.newest(3).len(), 3);
        assert_eq!(
            queue
                .newest(3)
                .iter()
                .map(|item| item.summary.as_str())
                .collect::<Vec<_>>(),
            ["4", "3", "2"],
            "the newest are the ones on screen, newest first"
        );
    }

    #[test]
    fn asking_for_more_than_are_held_gives_what_there_is() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();
        queue.notify(draft("one"), now);

        assert_eq!(queue.newest(10).len(), 1);
    }

    // --- reconfiguration ---------------------------------------------------

    #[test]
    fn a_reload_that_lowers_the_bound_drops_the_oldest_first() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        for index in 0..5 {
            queue.notify(draft(&format!("{index}")), now);
        }

        let closed = queue.reconfigure(
            QueueConfig {
                max_pending: 2,
                ..QueueConfig::default()
            },
            TimeoutConfig::default(),
            3,
        );

        assert_eq!(
            closed,
            [
                Closed {
                    id: NotificationId::new(1),
                    reason: CloseReason::Undefined
                },
                Closed {
                    id: NotificationId::new(2),
                    reason: CloseReason::Undefined
                },
                Closed {
                    id: NotificationId::new(3),
                    reason: CloseReason::Undefined
                },
            ]
        );
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.counters().dropped, 3);
        assert_eq!(
            queue
                .iter()
                .map(|item| item.summary.as_str())
                .collect::<Vec<_>>(),
            ["3", "4"]
        );
    }

    #[test]
    fn a_reload_does_not_re_time_a_notification_already_held() {
        let mut queue = queue(QueueConfig::default(), 3);
        let start = Instant::now();

        let id = queue
            .notify(arriving(Expiry::ServerDefault, "one"), start)
            .id;
        let before = queue.get(id).and_then(|item| item.deadline);

        queue.reconfigure(
            QueueConfig::default(),
            TimeoutConfig {
                low_ms: 1,
                normal_ms: 1,
                critical_ms: 0,
            },
            3,
        );

        assert_eq!(
            queue.get(id).and_then(|item| item.deadline),
            before,
            "the lifetime it was admitted with is the lifetime it keeps"
        );
    }

    #[test]
    fn a_reload_changes_the_policy_for_what_comes_next() {
        let mut queue = queue(
            QueueConfig {
                max_pending: 2,
                ..QueueConfig::default()
            },
            1,
        );
        let now = Instant::now();

        queue.reconfigure(
            QueueConfig {
                max_pending: 2,
                policy: QueuePolicy::CoalesceByApp,
            },
            TimeoutConfig::default(),
            1,
        );

        queue.notify(from_app("chat", "one"), now);
        queue.notify(from_app("other", "two"), now);
        let admission = queue.notify(from_app("chat", "three"), now);

        assert_eq!(
            admission.evicted,
            Some(Eviction::Coalesced {
                id: NotificationId::new(1)
            })
        );
    }

    #[test]
    fn a_reload_that_raises_the_bound_drops_nothing() {
        let mut queue = queue(
            QueueConfig {
                max_pending: 2,
                ..QueueConfig::default()
            },
            1,
        );
        let now = Instant::now();
        queue.notify(draft("one"), now);

        let closed = queue.reconfigure(
            QueueConfig {
                max_pending: 8,
                ..QueueConfig::default()
            },
            TimeoutConfig::default(),
            1,
        );

        assert!(closed.is_empty());
        assert_eq!(queue.len(), 1);
    }

    // --- hints and content carried through ---------------------------------

    #[test]
    fn a_notification_carries_what_it_arrived_with() {
        let mut queue = queue(QueueConfig::default(), 3);
        let now = Instant::now();

        let id = queue
            .notify(
                Draft {
                    app_name: "Firefox".to_owned(),
                    app_icon: Some("firefox".to_owned()),
                    summary: "A page loaded".to_owned(),
                    body: sanitize("with <b>markup</b>"),
                    hints: Hints {
                        urgency: Urgency::Low,
                        category: Some("device".to_owned()),
                        ..Hints::default()
                    },
                    ..Draft::default()
                },
                now,
            )
            .id;

        let notification = queue.get(id).expect("it is held");

        assert_eq!(notification.app_name, "Firefox");
        assert_eq!(notification.summary, "A page loaded");
        assert_eq!(notification.urgency(), Urgency::Low);
        assert_eq!(notification.icon_reference(), Some("firefox"));
        assert_eq!(crate::markup::plain_text(&notification.body), "with markup");
        assert_eq!(notification.arrived, now);
    }

    #[test]
    fn the_image_path_hint_stands_in_for_a_missing_icon() {
        let mut queue = queue(QueueConfig::default(), 3);

        let id = queue
            .notify(
                Draft {
                    hints: Hints {
                        image_path: Some("/usr/share/icons/a.png".to_owned()),
                        ..Hints::default()
                    },
                    ..draft("one")
                },
                Instant::now(),
            )
            .id;

        assert_eq!(
            queue.get(id).and_then(Notification::icon_reference),
            Some("/usr/share/icons/a.png")
        );

        let id = queue
            .notify(
                Draft {
                    app_icon: Some("   ".to_owned()),
                    ..draft("two")
                },
                Instant::now(),
            )
            .id;

        assert_eq!(
            queue.get(id).and_then(Notification::icon_reference),
            None,
            "a blank icon name is not an icon"
        );
    }

    // --- the burst acceptance D06 injects ----------------------------------

    #[test]
    fn a_hundred_notifications_a_second_for_ten_seconds_stay_bounded() {
        let config = QueueConfig {
            max_pending: 32,
            ..QueueConfig::default()
        };
        let mut queue = queue(config, 3);
        let start = Instant::now();

        for index in 0..1_000u32 {
            let now = start + Duration::from_millis(u64::from(index) * 10);
            queue.notify(
                Draft {
                    app_name: format!("app{}", index % 7),
                    ..arriving(Expiry::After(8_000), "burst")
                },
                now,
            );
            queue.expire(now);

            assert!(
                queue.len() <= 32,
                "the queue exceeded its bound on notification {index}"
            );
        }

        let counters = queue.counters();

        assert_eq!(counters.admitted, 1_000);
        assert_eq!(counters.created, 1_000);
        assert_eq!(
            counters.created,
            queue.len() as u64 + counters.settled(),
            "every notification is either held or settled, and counts once"
        );
        assert_eq!(
            counters.dropped + counters.coalesced,
            1_000 - 32,
            "every notification beyond the bound was evicted, one way or the other"
        );
        assert_eq!(
            counters.expired, 0,
            "the whole queue is younger than the lifetime it was given, so the \
             bound — not the timeout — is what happened under the burst"
        );
    }

    // --- properties --------------------------------------------------------

    /// One operation in a random sequence, as the properties drive them.
    #[derive(Debug, Clone)]
    enum Op {
        /// Send a notification, possibly replacing a live one.
        Notify {
            /// Which application sends it.
            app: u8,
            /// How urgent it says it is.
            urgency: u8,
            /// Whether it names a live notification to replace.
            replace: bool,
            /// How long the client asks it to live, in milliseconds.
            millis: u32,
        },
        /// Close whatever the sweep finds due.
        Expire,
        /// Close the oldest live notification.
        CloseOldest,
    }

    fn operations() -> impl Strategy<Value = Vec<Op>> {
        proptest::collection::vec(
            prop_oneof![
                6 => (0u8..4, 0u8..3, any::<bool>(), 0u32..4_000).prop_map(
                    |(app, urgency, replace, millis)| Op::Notify { app, urgency, replace, millis }
                ),
                2 => Just(Op::Expire),
                1 => Just(Op::CloseOldest),
            ],
            0..200,
        )
    }

    /// A queue configured for the properties: a small bound, so eviction is
    /// reached often enough to be exercised.
    fn property_queue() -> Queue {
        Queue::new(
            QueueConfig {
                max_pending: 4,
                ..QueueConfig::default()
            },
            TimeoutConfig::default(),
            2,
        )
    }

    /// Drives `ops` and reports what the queue did.
    ///
    /// Returns, for every operation, the identifiers that reached a terminal
    /// state, so a test can check that none is lost and none is settled twice.
    fn drive(ops: &[Op]) -> (Queue, Vec<NotificationId>, Vec<Closed>) {
        let mut queue = property_queue();
        let mut start = Instant::now();
        let mut admitted = Vec::new();
        let mut closed = Vec::new();

        for op in ops {
            start += Duration::from_millis(10);

            match op {
                Op::Notify {
                    app,
                    urgency,
                    replace,
                    millis,
                } => {
                    let replaces = if *replace {
                        admitted
                            .last()
                            .copied()
                            .filter(|id: &NotificationId| queue.get(*id).is_some())
                            .unwrap_or(NotificationId::NONE)
                    } else {
                        NotificationId::NONE
                    };

                    let admission = queue.notify(
                        Draft {
                            app_name: format!("app{app}"),
                            hints: Hints {
                                urgency: Urgency::from_raw(*urgency),
                                ..Hints::default()
                            },
                            replaces,
                            expiry: Expiry::After(*millis),
                            ..draft("op")
                        },
                        start,
                    );

                    admitted.push(admission.id);
                    if let Some(evicted) = admission.evicted {
                        closed.push(Closed {
                            id: evicted.id(),
                            reason: CloseReason::Undefined,
                        });
                    }
                }
                Op::Expire => closed.extend(queue.expire(start)),
                Op::CloseOldest => {
                    let oldest = queue.iter().map(|item| item.id).next();

                    if let Some(id) = oldest {
                        closed.extend(queue.close(id, CloseReason::Dismissed));
                    }
                }
            }
        }

        (queue, admitted, closed)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn the_queue_never_exceeds_its_bound(ops in operations()) {
            let (queue, _, _) = drive(&ops);

            prop_assert!(queue.len() <= 4, "{} held", queue.len());
            prop_assert!(queue.counters().created == queue.len() as u64 + queue.counters().settled());
        }

        #[test]
        fn every_notification_reaches_exactly_one_terminal_state(ops in operations()) {
            let (queue, admitted, closed) = drive(&ops);

            for id in &admitted {
                let settlements = closed.iter().filter(|entry| entry.id == *id).count();
                prop_assert!(
                    settlements <= 1,
                    "{} was settled {} times", id, settlements
                );

                if queue.get(*id).is_none() {
                    prop_assert!(
                        settlements == 1,
                        "{} is neither held nor settled, so it was lost", id
                    );
                } else {
                    prop_assert!(settlements == 0, "{} is held and settled", id);
                }
            }
        }

        #[test]
        fn a_live_notification_never_shares_its_identifier(ops in operations()) {
            let (queue, _, _) = drive(&ops);
            let ids: Vec<NotificationId> = queue.iter().map(|item| item.id).collect();
            let mut unique = ids.clone();

            unique.sort_unstable();
            unique.dedup();
            prop_assert_eq!(unique.len(), ids.len());
            prop_assert!(ids.iter().all(|id| !id.is_none()));
        }

        #[test]
        fn a_replacement_keeps_the_identifier_it_named(ops in operations()) {
            let mut queue = property_queue();
            let mut now = Instant::now();
            let mut live: Vec<NotificationId> = Vec::new();

            for op in &ops {
                now += Duration::from_millis(10);

                let Op::Notify { app, urgency, replace, millis } = op else {
                    queue.expire(now);
                    continue;
                };

                let replaces = if *replace {
                    live.iter().rev().copied().find(|id| queue.get(*id).is_some())
                        .unwrap_or(NotificationId::NONE)
                } else {
                    NotificationId::NONE
                };

                let admission = queue.notify(
                    Draft {
                        app_name: format!("app{app}"),
                        hints: Hints {
                            urgency: Urgency::from_raw(*urgency),
                            ..Hints::default()
                        },
                        replaces,
                        expiry: Expiry::After(*millis),
                        ..draft("op")
                    },
                    now,
                );

                if !replaces.is_none() && queue.get(replaces).is_some() {
                    prop_assert_eq!(admission.id, replaces);
                }
                if !live.contains(&admission.id) {
                    live.push(admission.id);
                }
            }
        }

        #[test]
        fn a_critical_notification_is_never_expired_by_a_sweep(millis in 0u32..10_000) {
            let mut queue = property_queue();
            let start = Instant::now();

            let id = queue.notify(
                Draft {
                    hints: Hints { urgency: Urgency::Critical, ..Hints::default() },
                    expiry: Expiry::After(millis),
                    ..draft("critical")
                },
                start,
            ).id;

            queue.expire(start + Duration::from_secs(86_400));

            prop_assert!(queue.get(id).is_some());
        }
    }
}
