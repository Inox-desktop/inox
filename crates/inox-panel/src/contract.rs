//! The panel's side of the window-service contract.
//! The panel's side of the window-service contract (ADR 0013).
//! The panel does not guess window state, and it does not treat a click as an
//! accomplished action. ADR 0013 decision 1 gives it a snapshot plus events, and
//! decision 2 asks it to distinguish what happened to a request it sent:
//!
//! ```text
//!   Connecting ──snapshot──▶ Ready { generation }
//!        ▲                        │
//!        │                        │ service lost
//!     retry                        ▼
//!        └──────────────  Unavailable { reason }
//!
//!   a report for another session also returns to Connecting, with the view of
//!   the session that ended dropped and a request for the missing snapshot
//!
//!   Sent ──▶ Accepted ──▶ Applied
//!     └────▶ Rejected { reason }
//!     └────▶ TimedOut
//! ```
//!
//! `Ready` is entered by a snapshot and by nothing else: a readiness report
//! names the session the service is on but carries no windows, so it either
//! confirms the snapshot in hand or sends the panel back to `Connecting` — never
//! to `Ready` on the strength of a report alone.
//!
//! Two of those states exist because of what the panel must *not* do. A service
//! that is down is a readable message and a retry, never a spinner that never
//! ends (D05). And an action the user already asked for is not asked for twice:
//! [`PendingRequests`] is the deduplication ADR 0013 decision 4 requires, keyed by
//! the window the request is about, so a second click while the first is in
//! flight produces no second request.
//!
//! The transport that carries these requests is not implemented in this
//! milestone; [`crate::app::OutgoingCommand`] is the queue it will drain.

use std::collections::BTreeMap;

use crate::tasks::{SessionGeneration, WindowKey};

/// Identity of one request the panel sent to a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(u64);

impl RequestId {
    /// Identifies a request the transport carries a reply for.
    ///
    /// The panel mints its own identifiers; the transport echoes the one it was
    /// given, which is why this can also be built from a reply.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw identifier, for the transport that carries it with the reply.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Where the window service is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ServiceState {
    /// No snapshot yet: the compositor has not answered.
    #[default]
    Connecting,
    /// A snapshot of `generation` is in hand.
    Ready {
        /// The session the snapshot describes.
        generation: SessionGeneration,
        /// How many windows it reported.
        windows: usize,
    },
    /// The service is not available; the panel offers a retry.
    Unavailable {
        /// Why it is not available, for the message and the log.
        reason: ServiceFailure,
    },
}

impl ServiceState {
    /// The generation of the snapshot in hand.
    pub const fn generation(&self) -> Option<SessionGeneration> {
        match self {
            Self::Ready { generation, .. } => Some(*generation),
            Self::Connecting | Self::Unavailable { .. } => None,
        }
    }

    /// Whether a snapshot is in hand, so window events can be applied.
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Whether the panel can ask the service to do something.
    ///
    /// A stale window list stays on screen while the service is away, but its
    /// buttons are disabled with a readable reason rather than sending requests
    /// nobody will answer (D05).
    pub const fn accepts_commands(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// Why the window service is not available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceFailure {
    /// The connection to the service was lost.
    Disconnected,
    /// The service did not answer within its budget.
    Timeout,
    /// The service answered with an error of its own.
    ///
    /// No transport produces this yet — the window service is not implemented —
    /// but it is the shape of an answer the contract already has words for, so
    /// it stays part of the vocabulary rather than becoming a fourth case later.
    #[allow(dead_code)]
    Reported {
        /// The service's own words, for the log and the message.
        detail: String,
    },
}

/// What became of a request the panel sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandState {
    /// The transport has the request; the service has not answered yet.
    Sent,
    /// The service accepted the request and will act on it.
    Accepted,
    /// The service reports the action done.
    Applied,
    /// The service refused the request, with a diagnosable reason.
    Rejected {
        /// Why it was refused.
        reason: RejectionReason,
    },
    /// The service did not answer in time.
    TimedOut,
}

impl CommandState {
    /// Whether the request is finished, one way or another.
    pub const fn is_settled(&self) -> bool {
        matches!(self, Self::Applied | Self::Rejected { .. } | Self::TimedOut)
    }
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RejectionReason {
    /// The caller is not permitted to do this.
    #[error("the window service refused the request: not permitted")]
    NotPermitted,
    /// The window no longer exists.
    ///
    /// Normal, not exceptional: the user clicked a button for a window that
    /// closed a moment earlier.
    #[error("the window is no longer known to the window service")]
    UnknownWindow,
    /// The service is gone.
    #[error("the window service is not available")]
    ServiceGone,
    /// The service answered with a protocol error.
    #[error("the window service answered with an error: {detail}")]
    Protocol {
        /// The service's own words.
        detail: String,
    },
}

/// The requests the panel has sent and not yet had settled.
///
/// One request per window at a time. That is the whole of ADR 0013 decision 4's
/// "同一动作无重复请求" as far as the panel is concerned: an impatient second
/// click on a task button is dropped instead of becoming a second activation.
#[derive(Debug, Default)]
pub struct PendingRequests {
    next: u64,
    in_flight: BTreeMap<WindowKey, RequestId>,
}

impl PendingRequests {
    /// No requests in flight.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many requests are in flight.
    pub fn len(&self) -> usize {
        self.in_flight.len()
    }

    /// Whether no request is in flight.
    pub fn is_empty(&self) -> bool {
        self.in_flight.is_empty()
    }

    /// Whether a request for `window` is in flight.
    pub fn is_pending(&self, window: WindowKey) -> bool {
        self.in_flight.contains_key(&window)
    }

    /// Starts a request for `window`, unless one is already in flight.
    ///
    /// Returns `None` when the request would be a duplicate — the caller drops
    /// it rather than queueing a second one.
    pub fn begin(&mut self, window: WindowKey) -> Option<RequestId> {
        if self.in_flight.contains_key(&window) {
            return None;
        }

        self.next += 1;
        let id = RequestId(self.next);
        self.in_flight.insert(window, id);

        Some(id)
    }

    /// Releases the window `id` was about.
    ///
    /// Returns the window whose request left the in-flight set, or `None` when
    /// the identifier was never issued or has already been settled — a duplicate
    /// reply, or one for a request the panel never sent (a restarted service
    /// reusing an identifier), changes nothing.
    ///
    /// What the service *answered* is the caller's to interpret
    /// ([`CommandState`]): only the release of the window belongs here.
    pub fn settle(&mut self, id: RequestId) -> Option<WindowKey> {
        let window = self
            .in_flight
            .iter()
            .find(|(_, pending)| **pending == id)
            .map(|(window, _)| *window)?;

        self.in_flight.remove(&window);

        Some(window)
    }

    /// Forgets any request about `window`, which no longer exists.
    ///
    /// A window that goes away takes its request with it: the reply can only be
    /// an error about a window nobody can see any more.
    pub fn forget(&mut self, window: WindowKey) -> Option<RequestId> {
        self.in_flight.remove(&window)
    }

    /// Forgets every in-flight request.
    ///
    /// Used when the connection is re-established: a request sent to the old
    /// service will never be answered, and the panel must not stay disabled
    /// waiting for a reply that cannot come.
    pub fn clear(&mut self) {
        self.in_flight.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::WindowId;

    fn key(window: u64) -> WindowKey {
        WindowKey::new(WindowId::new(window), SessionGeneration::new(1))
    }

    #[test]
    fn a_request_is_started_once_per_window() {
        let mut pending = PendingRequests::new();

        let first = pending.begin(key(1)).expect("nothing is in flight");
        assert!(pending.is_pending(key(1)));
        assert_eq!(pending.len(), 1);

        assert_eq!(
            pending.begin(key(1)),
            None,
            "a second request for the same window is dropped"
        );
        assert_eq!(pending.len(), 1);

        let other = pending.begin(key(2)).expect("a different window");
        assert_ne!(first, other);
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn settling_a_request_frees_its_window() {
        let mut pending = PendingRequests::new();
        let id = pending.begin(key(1)).expect("nothing is in flight");

        assert_eq!(pending.settle(id), Some(key(1)));
        assert!(!pending.is_pending(key(1)));
        assert!(pending.is_empty());
        assert!(pending.begin(key(1)).is_some(), "the window is free again");
    }

    #[test]
    fn a_duplicate_or_unknown_settlement_changes_nothing() {
        let mut pending = PendingRequests::new();
        let id = pending.begin(key(1)).expect("nothing is in flight");

        assert_eq!(pending.settle(id), Some(key(1)));
        assert_eq!(
            pending.settle(id),
            None,
            "the same reply twice releases nothing the second time"
        );
    }

    #[test]
    fn a_settlement_for_a_request_we_never_sent_is_ignored() {
        let mut pending = PendingRequests::new();
        let id = pending.begin(key(1)).expect("nothing is in flight");

        assert_eq!(
            pending.settle(RequestId::new(id.get() + 7)),
            None,
            "an identifier the panel never issued is not ours"
        );
        assert!(pending.is_pending(key(1)));
    }

    #[test]
    fn forgetting_a_window_takes_its_request() {
        let mut pending = PendingRequests::new();
        let id = pending.begin(key(1)).expect("nothing is in flight");
        pending.begin(key(2));

        assert_eq!(pending.forget(key(1)), Some(id));
        assert_eq!(pending.forget(key(1)), None, "twice is the same as once");
        assert!(!pending.is_pending(key(1)));
        assert!(pending.is_pending(key(2)), "other windows keep theirs");
    }

    #[test]
    fn clearing_releases_every_window() {
        let mut pending = PendingRequests::new();
        pending.begin(key(1));
        pending.begin(key(2));

        pending.clear();

        assert!(pending.is_empty());
        assert!(!pending.is_pending(key(1)));
        assert!(pending.begin(key(1)).is_some());
    }

    #[test]
    fn a_command_is_settled_only_when_it_finished() {
        assert!(!CommandState::Sent.is_settled());
        assert!(!CommandState::Accepted.is_settled());
        assert!(CommandState::Applied.is_settled());
        assert!(CommandState::TimedOut.is_settled());
        assert!(
            CommandState::Rejected {
                reason: RejectionReason::NotPermitted
            }
            .is_settled()
        );
    }

    #[test]
    fn the_service_state_reports_what_it_is() {
        let connecting = ServiceState::Connecting;
        assert!(!connecting.is_ready());
        assert!(!connecting.accepts_commands());
        assert_eq!(connecting.generation(), None);

        let ready = ServiceState::Ready {
            generation: SessionGeneration::new(4),
            windows: 3,
        };
        assert!(ready.is_ready());
        assert!(ready.accepts_commands());
        assert_eq!(ready.generation(), Some(SessionGeneration::new(4)));

        let gone = ServiceState::Unavailable {
            reason: ServiceFailure::Disconnected,
        };
        assert!(!gone.is_ready());
        assert!(!gone.accepts_commands(), "a stale list sends no requests");
        assert_eq!(gone.generation(), None);
    }

    #[test]
    fn a_rejection_explains_itself() {
        for reason in [
            RejectionReason::NotPermitted,
            RejectionReason::UnknownWindow,
            RejectionReason::ServiceGone,
            RejectionReason::Protocol {
                detail: "unknown opcode".to_owned(),
            },
        ] {
            assert!(!reason.to_string().is_empty());
        }
    }
}
