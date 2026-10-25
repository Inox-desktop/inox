//! The two boundaries this component talks across.
//! The two boundaries this component talks across.
//! A lock screen has no state of its own that matters: the compositor owns the
//! lock, and a privileged authentication boundary owns the answer to "is this
//! the user?". What is left for `inox-lock` is to be an honest client of both,
//! and the interaction contract fixes the vocabulary for that honesty — a state, a generation,
//! and a request whose fate is distinguishable rather than assumed:
//!
//! ```text
//!   Idle ──begin──▶ Sent ──▶ Accepted ──▶ Answered { Success | Failure }
//!                     │
//!                     └────▶ Unavailable { failure }   (a retry, when one helps)
//! ```
//!
//! Two things are deliberately *not* modelled here.
//!
//! - **No `Success` value that anything can make up cheaply.** [`AuthOutcome`]
//!   is what the boundary reports; in the real system that report arrives over
//!   the reviewed boundary the session-lock boundary requires, and the only thing this crate does
//!   with it is hand it to [`crate::state::LockMachine::unlock`].
//! - **No protocol detail.** [`LockProtocol`] records whether the compositor
//!   offers `ext-session-lock-v1` at all, because that is the difference between
//!   "this build cannot lock" and "the lock is held"; the requests themselves
//!   (`get_lock_surface`, `locked`, `unlock_and_destroy`) are the protocol leg
//!   this crate does not implement.

use std::fmt;

use crate::credential::Secret;

/// A counter that tells a current answer from a stale one.
///
/// the interaction contract, rule 2 asks a component not to let a late result overwrite a
/// newer request. An attempt that was abandoned (a suspend, a cancel) can still
/// be answered afterwards; [`BoundaryState::is_current`] is what refuses that
/// answer instead of acting on it, and [`BoundaryState::is_stale`] names the
/// narrower question — "is this about an attempt already over?" — underneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

impl Generation {
    /// The generation of the first attempt.
    pub const fn first() -> Self {
        Self(1)
    }

    /// Builds a generation from the transport that carries one.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw counter, for an implementation that also logs it.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The next generation.
    ///
    /// Wrapping rather than saturating: a counter that pins at its maximum stops
    /// distinguishing attempts, and a wrap after 2^64 attempts is unreachable.
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Whether the compositor offers the protocol this client needs.
///
/// The state today is [`LockProtocol::NotAdvertised`]: `inox-compositor` does not
/// advertise `ext-session-lock-v1`, and the session-lock boundary requires a human security review
/// of the authentication boundary before the screen that would use it may land.
/// Modelling the absence as a value rather than an error is what lets the
/// interface say which of the two is missing, with a reason a user can act on
/// (acceptance D05).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockProtocol {
    /// `ext_session_lock_manager_v1` is in the registry and a lock may be asked
    /// for.
    Advertised,
    /// It is not, for `reason`.
    NotAdvertised {
        /// Why the protocol is unusable.
        reason: ProtocolAbsence,
    },
}

impl LockProtocol {
    /// Whether this build can ask the compositor to lock the session.
    pub const fn is_advertised(&self) -> bool {
        matches!(self, Self::Advertised)
    }

    /// Why it cannot, when it cannot.
    pub const fn absence(&self) -> Option<ProtocolAbsence> {
        match self {
            Self::Advertised => None,
            Self::NotAdvertised { reason } => Some(*reason),
        }
    }
}

impl Default for LockProtocol {
    /// Unadvertised, with the reason that is true of every Inox session today.
    fn default() -> Self {
        Self::NotAdvertised {
            reason: ProtocolAbsence::GlobalMissing,
        }
    }
}

/// Why a session lock cannot be established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolAbsence {
    /// The compositor does not advertise `ext_session_lock_manager_v1`.
    GlobalMissing,
    /// The compositor's registry could not be read at all, so nothing can be
    /// concluded about what it offers.
    RegistryUnavailable,
}

impl ProtocolAbsence {
    /// The catalog id that says this in the user's language.
    pub const fn message_id(self) -> &'static str {
        match self {
            Self::GlobalMissing => "lock-protocol-missing-global",
            Self::RegistryUnavailable => "lock-protocol-registry-unavailable",
        }
    }
}

/// Why the trusted authentication boundary is not usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceFailure {
    /// The boundary cannot be reached.
    Disconnected,
    /// The boundary did not answer within its budget.
    Timeout,
    /// This build ships no authentication boundary at all.
    ///
    /// Not a fault in a service but the absence of one: the session-lock boundary makes the
    /// authentication boundary — PAM or whatever else, the helper's privileges,
    /// the handling of the credential — a human security review item, and this
    /// build has not had that review. There is nothing to reach, and nothing a
    /// retry could reach either, which is why the interface offers none (see
    /// [`ServiceFailure::is_retryable`]).
    NotProvided,
}

impl ServiceFailure {
    /// The catalog id that says this in the user's language.
    pub const fn message_id(self) -> &'static str {
        match self {
            Self::Disconnected => "lock-boundary-disconnected",
            Self::Timeout => "lock-boundary-timeout",
            Self::NotProvided => "lock-boundary-not-provided",
        }
    }

    /// The catalog id that says, next to the disabled field, why it cannot be
    /// typed into.
    ///
    /// The same fact as [`message_id`](Self::message_id) seen from a different
    /// place: the headline explains what the screen cannot do, this explains what
    /// one control cannot do, and a person reading either should be able to tell
    /// "there is no such service in this build" from "the service is not
    /// answering".
    pub const fn disabled_message_id(self) -> &'static str {
        match self {
            Self::NotProvided => "lock-disabled-boundary-missing",
            Self::Disconnected | Self::Timeout => "lock-disabled-boundary",
        }
    }

    /// Whether trying the boundary again could help.
    ///
    /// True for a service that is down or slow, because those can come back.
    /// False for a boundary this build does not contain: a retry would be an
    /// invitation to keep pressing a control that can never do anything, and the
    /// interface draws no such control when this is false.
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Disconnected | Self::Timeout)
    }
}

/// Why an authentication attempt did not succeed.
///
/// These are the boundary's words about *one attempt*, as opposed to
/// [`ServiceFailure`], which is what is wrong with the service as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    /// The boundary checked the credential and refused it.
    Rejected,
    /// The attempt was not answered within the boundary's budget.
    TimedOut,
    /// The boundary could not be reached.
    Unavailable,
    /// The attempt was abandoned before it was answered — the screen was hidden,
    /// the user cancelled, the session suspended.
    Cancelled,
}

impl AuthFailure {
    /// The catalog message that explains this to the user.
    pub const fn message_id(self) -> &'static str {
        match self {
            Self::Rejected => "lock-failure-rejected",
            Self::TimedOut => "lock-failure-timed-out",
            Self::Unavailable => "lock-failure-unavailable",
            Self::Cancelled => "lock-failure-cancelled",
        }
    }
}

/// What the trusted boundary reports about an attempt.
///
/// [`AuthOutcome::Success`] is the only value that unlocks a session, and
/// [`crate::state::LockMachine::unlock`] is the only place it is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthOutcome {
    /// The user is who they say they are.
    Success,
    /// They are not, for `reason`.
    Failure(AuthFailure),
}

impl AuthOutcome {
    /// Whether this outcome is an authenticated success.
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// The failure, when this is one.
    pub const fn failure(&self) -> Option<AuthFailure> {
        match self {
            Self::Success => None,
            Self::Failure(reason) => Some(*reason),
        }
    }
}

/// What the authentication boundary last said.
///
/// `Sent`, `Accepted` and `Answered` are the distinguishable fates of one
/// request. `Answered` carries the outcome, so "the boundary answered" and "the
/// answer was a refusal" are one state rather than a state and a flag that can
/// disagree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BoundaryState {
    /// No attempt has been made in this lock.
    #[default]
    Idle,
    /// The transport has the attempt; the boundary has not spoken.
    Sent {
        /// The attempt this answer will be about.
        generation: Generation,
    },
    /// The boundary has the attempt and is checking it.
    Accepted {
        /// The attempt being checked.
        generation: Generation,
    },
    /// The boundary answered.
    Answered {
        /// The attempt that was answered.
        generation: Generation,
        /// What it said.
        outcome: AuthOutcome,
    },
    /// The boundary is not usable at all.
    ///
    /// The interface offers a retry only when the failure is one that could come
    /// back — [`ServiceFailure::is_retryable`] is that question, and the projection
    /// is what asks it.
    Unavailable {
        /// What is wrong with it.
        failure: ServiceFailure,
    },
}

impl BoundaryState {
    /// Whether an attempt is being checked, so a second one must not be sent.
    ///
    /// This is acceptance D05's "the same action is not requested twice" for the
    /// one action a lock screen has.
    pub const fn in_flight(&self) -> bool {
        matches!(self, Self::Sent { .. } | Self::Accepted { .. })
    }

    /// The generation this state is about, if any.
    ///
    /// `Sent` and `Accepted` report the attempt being checked; `Answered` keeps
    /// reporting the attempt it answered, so that a late message about it can be
    /// told from a message about the attempt in hand. Only
    /// [`is_current`](Self::is_current) says whether that attempt is still worth
    /// answering.
    pub const fn generation(&self) -> Option<Generation> {
        match self {
            Self::Sent { generation } | Self::Accepted { generation } => Some(*generation),
            Self::Answered { generation, .. } => Some(*generation),
            Self::Idle | Self::Unavailable { .. } => None,
        }
    }

    /// Whether `generation` names an attempt this state has already moved past.
    ///
    /// True when the answer belongs to an earlier attempt than the one this state
    /// is about, and true whenever there is no attempt to be about at all
    /// (`Idle`, `Unavailable`) — an unsolicited answer is stale by definition and
    /// must not unlock anything.
    ///
    /// **This is not the question a handler acts on.** A generation that was
    /// never sent is not stale, it is unknown, and `Answered` still reports the
    /// attempt it answered although that attempt is not in flight — so an answer
    /// about it is not stale either. A handler obeys a message only when
    /// [`is_current`](Self::is_current) holds.
    pub const fn is_stale(&self, generation: Generation) -> bool {
        match self.generation() {
            Some(current) => generation.get() < current.get(),
            None => true,
        }
    }

    /// Whether `generation` names the attempt being checked right now.
    ///
    /// The check a handler must make before it acts on anything the boundary says:
    /// only an attempt that is still in flight can be answered, and only the
    /// attempt in flight is the one a message is about. An answer for an earlier
    /// attempt, a duplicate about one that has already been answered (or a
    /// replayed answer for it), a generation that was never sent, and a message
    /// with no attempt behind it at all are all refused by this — and it is only
    /// the first and the last of those that [`is_stale`](Self::is_stale) catches.
    pub const fn is_current(&self, generation: Generation) -> bool {
        match self {
            Self::Sent {
                generation: current,
            }
            | Self::Accepted {
                generation: current,
            } => generation.get() == current.get(),
            Self::Idle | Self::Answered { .. } | Self::Unavailable { .. } => false,
        }
    }

    /// The outcome, when the boundary has given one.
    pub const fn outcome(&self) -> Option<AuthOutcome> {
        match self {
            Self::Answered { outcome, .. } => Some(*outcome),
            _ => None,
        }
    }

    /// Why the boundary is unusable, when it is.
    pub const fn failure(&self) -> Option<ServiceFailure> {
        match self {
            Self::Unavailable { failure } => Some(*failure),
            _ => None,
        }
    }
}

/// An attempt on its way to the trusted authentication boundary.
///
/// The credential is **moved** into this value, not copied: by the time the
/// attempt is queued for the transport, the lock screen's own state no longer
/// holds it. `Debug` is derived, which is safe only because
/// [`Secret`]'s is redacted — that is what keeps `tracing::debug!(?command)` from
/// printing a password.
#[derive(Debug)]
pub struct AuthAttempt {
    generation: Generation,
    credential: Secret,
}

impl AuthAttempt {
    /// Hands `credential` to the boundary as attempt `generation`.
    pub fn new(generation: Generation, credential: Secret) -> Self {
        Self {
            generation,
            credential,
        }
    }

    /// Which attempt this is.
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// The credential bytes, for the authentication boundary and nothing else.
    ///
    /// There is no other accessor in this crate that reaches the plaintext, and
    /// the boundary itself does not exist yet: the session-lock boundary makes it a review item.
    pub fn expose(&self) -> &[u8] {
        self.credential.expose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation(raw: u64) -> Generation {
        Generation::new(raw)
    }

    #[test]
    fn an_idle_boundary_has_nothing_in_flight() {
        let state = BoundaryState::Idle;

        assert!(!state.in_flight());
        assert_eq!(state.generation(), None);
        assert_eq!(state.outcome(), None);
        assert_eq!(state.failure(), None);
    }

    #[test]
    fn an_answer_is_only_stale_against_a_later_attempt() {
        let state = BoundaryState::Sent {
            generation: generation(4),
        };

        assert!(!state.is_stale(generation(4)), "the current attempt");
        assert!(state.is_stale(generation(3)), "an attempt already over");
        assert!(
            !state.is_stale(generation(5)),
            "an attempt from the future is not stale, it is unknown — the \
             caller decides what to do with it, and `update` refuses it unless \
             it names the attempt in flight (`is_current`)"
        );
        assert!(
            !state.is_current(generation(5)),
            "and `is_current` is what refuses it: a generation nobody sent is \
             not an answer to the attempt in hand"
        );
    }

    #[test]
    fn an_answer_with_no_attempt_behind_it_is_stale() {
        assert!(BoundaryState::Idle.is_stale(generation(1)));
        assert!(
            BoundaryState::Unavailable {
                failure: ServiceFailure::Disconnected
            }
            .is_stale(generation(1)),
            "an unsolicited success must not unlock anything: with no attempt in \
             flight, every answer arrives out of nowhere"
        );
    }

    #[test]
    fn an_answered_state_still_knows_which_attempt_it_was() {
        let state = BoundaryState::Answered {
            generation: generation(2),
            outcome: AuthOutcome::Failure(AuthFailure::Rejected),
        };

        assert_eq!(state.generation(), Some(generation(2)));
        assert_eq!(
            state.outcome(),
            Some(AuthOutcome::Failure(AuthFailure::Rejected))
        );
        assert!(!state.in_flight());
        assert!(state.is_stale(generation(1)));
    }

    #[test]
    fn only_the_attempt_in_flight_is_current() {
        let sent = BoundaryState::Sent {
            generation: generation(4),
        };

        assert!(sent.is_current(generation(4)), "the attempt being checked");
        assert!(!sent.is_current(generation(3)), "an attempt already over");
        assert!(
            !sent.is_current(generation(5)),
            "a generation that was never sent is not the attempt in hand"
        );

        let accepted = BoundaryState::Accepted {
            generation: generation(4),
        };

        assert!(accepted.is_current(generation(4)));
        assert!(!accepted.is_current(generation(5)));
    }

    #[test]
    fn nothing_is_current_once_the_attempt_is_over() {
        let answered = BoundaryState::Answered {
            generation: generation(2),
            outcome: AuthOutcome::Failure(AuthFailure::Rejected),
        };

        assert!(
            !answered.is_current(generation(2)),
            "the attempt has been answered: a duplicate acceptance or a replayed \
             answer is about nothing in hand"
        );

        assert!(!BoundaryState::Idle.is_current(generation(2)));
        assert!(
            !BoundaryState::Unavailable {
                failure: ServiceFailure::Timeout
            }
            .is_current(generation(2))
        );
    }

    #[test]
    fn only_a_boundary_that_could_come_back_is_retryable() {
        assert!(ServiceFailure::Disconnected.is_retryable());
        assert!(ServiceFailure::Timeout.is_retryable());
        assert!(
            !ServiceFailure::NotProvided.is_retryable(),
            "a boundary this build does not contain cannot be retried into \
             existence, and the interface must not offer to try"
        );
    }

    #[test]
    fn an_unavailable_boundary_is_not_in_flight_and_offers_no_outcome() {
        let state = BoundaryState::Unavailable {
            failure: ServiceFailure::Timeout,
        };

        assert!(!state.in_flight());
        assert_eq!(state.outcome(), None);
        assert_eq!(state.failure(), Some(ServiceFailure::Timeout));
    }

    #[test]
    fn generations_advance_and_wrap_without_panicking() {
        assert_eq!(Generation::first().get(), 1);
        assert_eq!(Generation::first().next(), Generation::new(2));
        assert_eq!(Generation::new(u64::MAX).next(), Generation::new(0));
    }

    #[test]
    fn every_outcome_reports_whether_it_unlocks() {
        assert!(AuthOutcome::Success.is_success());
        assert_eq!(AuthOutcome::Success.failure(), None);

        for failure in [
            AuthFailure::Rejected,
            AuthFailure::TimedOut,
            AuthFailure::Unavailable,
            AuthFailure::Cancelled,
        ] {
            let outcome = AuthOutcome::Failure(failure);
            assert!(!outcome.is_success());
            assert_eq!(outcome.failure(), Some(failure));
        }
    }

    #[test]
    fn the_default_protocol_is_the_one_inox_has_today() {
        assert!(!LockProtocol::default().is_advertised());
        assert_eq!(
            LockProtocol::default().absence(),
            Some(ProtocolAbsence::GlobalMissing)
        );
        assert_eq!(LockProtocol::Advertised.absence(), None);
    }

    #[test]
    fn every_reason_names_a_catalog_message() {
        for reason in [
            ProtocolAbsence::GlobalMissing,
            ProtocolAbsence::RegistryUnavailable,
        ] {
            assert!(reason.message_id().starts_with("lock-"));
        }

        for failure in [
            ServiceFailure::Disconnected,
            ServiceFailure::Timeout,
            ServiceFailure::NotProvided,
        ] {
            assert!(failure.message_id().starts_with("lock-"));
        }

        for failure in [
            AuthFailure::Rejected,
            AuthFailure::TimedOut,
            AuthFailure::Unavailable,
            AuthFailure::Cancelled,
        ] {
            assert!(failure.message_id().starts_with("lock-"));
        }
    }

    #[test]
    fn an_attempt_carries_the_credential_without_printing_it() {
        let mut credential = Secret::default();
        for character in "zqx".chars() {
            credential.push(character);
        }
        let attempt = AuthAttempt::new(generation(7), credential);

        assert_eq!(attempt.generation(), generation(7));
        assert_eq!(attempt.expose(), b"zqx");

        let rendered = format!("{attempt:?}");
        assert!(rendered.contains("Secret(<redacted>)"), "{rendered}");
        assert!(
            !rendered.contains("zqx"),
            "an attempt that prints its credential is an attempt that logs it: {rendered}"
        );
    }
}
