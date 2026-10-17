//! The lock state machine (ADR 0015).
//!
//! ADR 0015 puts the lock state in the compositor and makes `inox-lock` its
//! trusted client. What this module owns is the part that has to hold whatever
//! the two sides are: the *state*, its transitions, and the invariants that must
//! survive every sequence of events a session can produce.
//!
//! Three of those invariants are why this is a state machine and not a boolean.
//!
//! 1. **Locked means covered.** The session may only be reported locked once
//!    every output that exists has presented a secure lock frame. An output that
//!    appears while the session is locked has not, so the machine goes
//!    [`Phase::Locked`] → [`Phase::Covering`] → [`Phase::Locked`] around it and
//!    reports nothing in between. A session with no outputs at all is covered
//!    vacuously and locks as soon as it is asked to.
//! 2. **Nothing unlocks but an authentication success.** The UI exiting, the
//!    boundary failing, a timeout, a suspend, a resume, an output change: none of
//!    them unlock, and none of them are an error to recover from.
//!    [`LockMachine::unlock`] takes an [`AuthOutcome`] and is the only method in
//!    this crate that can make the phase [`Phase::Unlocking`]. [`LockEvent`] has
//!    no success variant at all, which is what makes that claim readable from the
//!    types rather than trusting the branches: a caller cannot even *express*
//!    "authenticate successfully" through the event door.
//! 3. **The UI is not the lock.** [`Phase::LockedWithoutUi`] is a phase, not a
//!    failure. The trusted UI may die; the session stays locked while the session
//!    restarts it, and the machine keeps reporting locked.
//!
//! The whole module is pure: no I/O, no clock, no iced, no protocol. Every rule
//! above is a unit test, and the two that are about *sequences* are property
//! tests over a thousand generated event sequences.
//!
//! ```text
//!                  Requested
//!   Unlocked ────────────────▶ Locking ──covered───▶ Locked
//!      ▲                         │  ▲                  │  ▲
//!      │                         │  └───covered────────┤  │ output added
//!      │                    ui exited                 │  ▼
//!      │                         ▼                 Covering
//!      │                 LockedWithoutUi ◀────────────┘
//!      │                         │
//!      │                     ui started
//!      │
//!      └──── Unlocking ◀──── AuthOutcome::Success ──── (any locked phase)
//!                │
//!                └── UnlockCompleted ──▶ Unlocked
//! ```

use std::collections::BTreeSet;
use std::fmt;

use crate::contract::{AuthFailure, AuthOutcome};

/// The compositor's name for one output (`eDP-1`, `HDMI-A-1`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputId(String);

impl OutputId {
    /// Names an output the way the compositor names it.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The name, for a log line or a diagnostic.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OutputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why the session was asked to lock.
///
/// The reason does not change what happens to the session — it is recorded so
/// that the log, and a diagnostic a user reports, say which of the three asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    /// A person asked for it (a keybinding, a menu entry, `inoxctl`).
    User,
    /// The idle policy asked for it.
    Idle,
    /// A suspend is being prepared.
    Suspend,
}

/// Where the lock is in its lifecycle (ADR 0015 names all five of these).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    /// The session is available to ordinary clients.
    #[default]
    Unlocked,
    /// A lock was asked for and at least one existing output has not presented a
    /// secure frame yet.
    Locking,
    /// Held, with every existing output covered and a trusted UI running.
    Locked,
    /// Held, with an output that appeared while the session was already locked
    /// and has not been covered yet.
    Covering,
    /// Held, with no trusted UI running. Not an error: the session restarts the
    /// UI while the session stays locked.
    LockedWithoutUi,
    /// An authentication success was received and the lock surfaces are being
    /// torn down. The session is still held until the teardown is confirmed.
    Unlocking,
}

impl Phase {
    /// Whether the session is held away from ordinary clients.
    ///
    /// True in every phase but [`Phase::Unlocked`] — including
    /// [`Phase::Unlocking`], because the secure frames are still up until the
    /// teardown is confirmed.
    pub const fn holds_session(self) -> bool {
        !matches!(self, Self::Unlocked)
    }

    /// Whether the lock is held and no authentication has succeeded.
    ///
    /// These are the phases in which the lock screen authenticates, and the ones
    /// that survive a UI crash, a boundary failure and a timeout.
    pub const fn is_locked_family(self) -> bool {
        matches!(
            self,
            Self::Locking | Self::Locked | Self::Covering | Self::LockedWithoutUi
        )
    }

    /// Whether a trusted UI is running to draw the prompt.
    pub const fn ui_running(self) -> bool {
        matches!(self, Self::Locking | Self::Locked | Self::Covering)
    }
}

/// Why an event changed nothing.
///
/// Returned inside [`Transition::Ignored`] so that a caller — and a test — can
/// tell "considered and refused, for this reason" from "not looked at".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoreReason {
    /// A secure frame was already recorded for that output.
    AlreadyCovered,
    /// The event named an output the machine does not have.
    UnknownOutput,
    /// The event added an output the machine already had.
    AlreadyKnown,
    /// A lock was requested while the session was not unlocked.
    NotUnlocked,
    /// A teardown confirmation arrived with no unlock in flight.
    NotUnlocking,
    /// The event belongs to another part of the session (a suspend, a resume) and
    /// deliberately changes nothing here.
    OutOfScope,
}

/// What an event did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// The phase moved.
    Changed {
        /// The phase before the event.
        from: Phase,
        /// The phase after it.
        to: Phase,
    },
    /// The phase is the same, but the machine's bookkeeping moved: an output was
    /// added, covered or removed, or a failure was recorded.
    Recorded,
    /// The event was considered and changed nothing.
    Ignored(IgnoreReason),
}

impl Transition {
    /// Whether the phase moved.
    ///
    /// A `Recorded` bookkeeping change and an `Ignored` event both leave the
    /// phase alone, which is the distinction a caller logging "the lock state
    /// moved" wants.
    pub const fn is_change(&self) -> bool {
        matches!(self, Self::Changed { .. })
    }
}

/// Everything that happens *to* a locked session.
///
/// There is no success variant by construction: the only event that could carry
/// one is [`LockEvent::AuthFailed`], and it carries a failure. An authentication
/// success reaches the machine through [`LockMachine::unlock`] and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockEvent {
    /// The session was asked to lock.
    Requested(LockReason),
    /// An output appeared.
    OutputAdded(OutputId),
    /// An output went away.
    OutputRemoved(OutputId),
    /// A secure frame was presented on an output.
    OutputCovered(OutputId),
    /// The trusted lock screen UI exited, crashed or was disconnected.
    UiExited,
    /// The session started a trusted UI again.
    UiStarted,
    /// An authentication attempt failed.
    AuthFailed(AuthFailure),
    /// A suspend is starting. The lock state must not change.
    Suspended,
    /// The session came back from a suspend. The lock state must not change: ADR
    /// 0015 requires a resume to leave the session locked.
    Resumed,
    /// The compositor confirmed the lock surfaces are gone.
    UnlockCompleted,
}

/// The invariant-preserving core of the lock screen.
///
/// Construct with [`LockMachine::new`] (unlocked, no outputs known, UI assumed
/// running) and drive with [`LockMachine::apply`] and
/// [`LockMachine::unlock`].
#[derive(Debug, Clone)]
pub struct LockMachine {
    phase: Phase,
    reason: Option<LockReason>,
    outputs: BTreeSet<OutputId>,
    covered: BTreeSet<OutputId>,
    ui_present: bool,
    last_failure: Option<AuthFailure>,
}

impl Default for LockMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl LockMachine {
    /// A machine for a session that is not locked, with no outputs known yet.
    pub fn new() -> Self {
        Self {
            phase: Phase::Unlocked,
            reason: None,
            outputs: BTreeSet::new(),
            covered: BTreeSet::new(),
            ui_present: true,
            last_failure: None,
        }
    }

    /// The phase.
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether the session is held away from ordinary clients.
    pub const fn holds_session(&self) -> bool {
        self.phase.holds_session()
    }

    /// Why the session was locked, once it was.
    pub const fn reason(&self) -> Option<LockReason> {
        self.reason
    }

    /// Whether a trusted UI is running.
    pub const fn ui_present(&self) -> bool {
        self.ui_present
    }

    /// The last authentication failure recorded, if any.
    pub const fn last_failure(&self) -> Option<AuthFailure> {
        self.last_failure
    }

    /// Whether every output that exists has presented a secure lock frame.
    ///
    /// Vacuously true for a session with no outputs: there is nothing that could
    /// leak a frame, so there is nothing to cover.
    pub fn covers_all_outputs(&self) -> bool {
        self.uncovered_outputs().next().is_none()
    }

    /// The outputs that have not presented a secure frame yet.
    pub fn uncovered_outputs(&self) -> impl Iterator<Item = &OutputId> {
        self.outputs.difference(&self.covered)
    }

    /// The outputs the machine knows about.
    pub fn outputs(&self) -> impl Iterator<Item = &OutputId> {
        self.outputs.iter()
    }

    /// Whether the session may be reported locked.
    ///
    /// ADR 0015: `locked` may only be declared once every existing output is
    /// safely covered. A held session with an output still waiting for its frame
    /// is not reported locked, and a session that was reported locked stops being
    /// reported so the moment an output appears — it never stops being held.
    pub fn reports_locked(&self) -> bool {
        self.phase.is_locked_family() && self.covers_all_outputs()
    }

    /// Applies an event, preserving every invariant above.
    pub fn apply(&mut self, event: LockEvent) -> Transition {
        match event {
            LockEvent::Requested(reason) => self.lock(reason),
            LockEvent::OutputAdded(id) => self.output_added(id),
            LockEvent::OutputRemoved(id) => self.output_removed(id),
            LockEvent::OutputCovered(id) => self.output_covered(id),
            LockEvent::UiExited => self.ui_exited(),
            LockEvent::UiStarted => self.ui_started(),
            LockEvent::AuthFailed(failure) => self.record_failure(failure),
            LockEvent::Suspended | LockEvent::Resumed => {
                Transition::Ignored(IgnoreReason::OutOfScope)
            }
            LockEvent::UnlockCompleted => self.unlock_completed(),
        }
    }

    /// Interprets the trusted boundary's answer.
    ///
    /// **The only door out of the locked state.** An [`AuthOutcome::Success`]
    /// moves the session to [`Phase::Unlocking`]; every failure is recorded and
    /// changes nothing, and a success from a session that is not locked is
    /// ignored. There is deliberately no second method, no `set_phase`, and no
    /// way to reach [`Phase::Unlocked`] except through
    /// [`LockEvent::UnlockCompleted`] after this method has run.
    pub fn unlock(&mut self, outcome: AuthOutcome) -> Transition {
        match outcome {
            AuthOutcome::Success => {
                if !self.phase.is_locked_family() {
                    return Transition::Ignored(IgnoreReason::NotUnlocked);
                }
                let from = self.phase;
                self.phase = Phase::Unlocking;
                self.last_failure = None;
                Transition::Changed {
                    from,
                    to: Phase::Unlocking,
                }
            }
            AuthOutcome::Failure(failure) => self.record_failure(failure),
        }
    }

    fn lock(&mut self, reason: LockReason) -> Transition {
        // Only an unlocked session can be locked. A second request while the
        // session is already held is not an error, but it does not relabel the
        // lock either: the first reason is the one that stands.
        if self.phase.holds_session() {
            return Transition::Ignored(IgnoreReason::NotUnlocked);
        }
        let from = self.phase;
        self.reason = Some(reason);
        // Every output has to present a fresh secure frame: a cover recorded for
        // an earlier lock is not evidence about this one.
        self.covered.clear();
        self.phase = if self.covers_all_outputs() {
            Phase::Locked
        } else {
            Phase::Locking
        };
        Transition::Changed {
            from,
            to: self.phase,
        }
    }

    fn output_added(&mut self, id: OutputId) -> Transition {
        if !self.outputs.insert(id.clone()) {
            return Transition::Ignored(IgnoreReason::AlreadyKnown);
        }
        // An output that has just appeared has covered nothing.
        self.covered.remove(&id);

        match self.phase {
            // While held, a new output delays the locked report until it has a
            // frame of its own: this is the `Locked → Covering → Locked` rule.
            Phase::Locked | Phase::Covering | Phase::LockedWithoutUi => {
                self.cover_up(Phase::Covering)
            }
            // Already waiting for frames; the new output joins the queue.
            Phase::Locking => Transition::Recorded,
            // Nothing is being held, and the teardown has already been
            // authorised, so the output does not need covering.
            Phase::Unlocked | Phase::Unlocking => Transition::Recorded,
        }
    }

    fn output_removed(&mut self, id: OutputId) -> Transition {
        if !self.outputs.remove(&id) {
            return Transition::Ignored(IgnoreReason::UnknownOutput);
        }
        self.covered.remove(&id);

        // The output that was still waiting for a frame may have been the last
        // one; removing it can be what completes the coverage, which is why this
        // is the same path an output appearing takes in reverse.
        match self.phase {
            Phase::Locking | Phase::Covering => self.cover_up(self.phase),
            Phase::Unlocked | Phase::Locked | Phase::LockedWithoutUi | Phase::Unlocking => {
                Transition::Recorded
            }
        }
    }

    fn output_covered(&mut self, id: OutputId) -> Transition {
        if !self.outputs.contains(&id) {
            return Transition::Ignored(IgnoreReason::UnknownOutput);
        }
        if !self.covered.insert(id) {
            return Transition::Ignored(IgnoreReason::AlreadyCovered);
        }

        if matches!(self.phase, Phase::Locking | Phase::Covering) && self.covers_all_outputs() {
            let to = if self.ui_present {
                Phase::Locked
            } else {
                Phase::LockedWithoutUi
            };
            return self.move_to(to);
        }

        Transition::Recorded
    }

    fn ui_exited(&mut self) -> Transition {
        self.ui_present = false;

        match self.phase {
            // The lock screen's UI is not the lock: the session stays held, with
            // whatever is already covered still covered, and the session restarts
            // the UI in a bounded loop (ADR 0015).
            Phase::Locking | Phase::Locked | Phase::Covering => {
                self.move_to(Phase::LockedWithoutUi)
            }
            Phase::LockedWithoutUi | Phase::Unlocked | Phase::Unlocking => {
                Transition::Ignored(IgnoreReason::OutOfScope)
            }
        }
    }

    fn ui_started(&mut self) -> Transition {
        self.ui_present = true;

        if self.phase != Phase::LockedWithoutUi {
            return Transition::Ignored(IgnoreReason::OutOfScope);
        }

        let to = if self.covers_all_outputs() {
            Phase::Locked
        } else {
            Phase::Locking
        };
        self.move_to(to)
    }

    fn record_failure(&mut self, failure: AuthFailure) -> Transition {
        self.last_failure = Some(failure);
        // Deliberately no phase change: a failed attempt, a timeout and an
        // unreachable boundary all leave the session exactly as locked as it was.
        Transition::Recorded
    }

    fn unlock_completed(&mut self) -> Transition {
        if self.phase != Phase::Unlocking {
            return Transition::Ignored(IgnoreReason::NotUnlocking);
        }
        let from = self.phase;
        self.phase = Phase::Unlocked;
        self.reason = None;
        self.last_failure = None;
        self.covered.clear();
        Transition::Changed {
            from,
            to: Phase::Unlocked,
        }
    }

    /// Moves to `to`, or reports why not.
    ///
    /// The one place that assigns a phase other than via a rule of its own, and
    /// it never assigns [`Phase::Unlocked`]: that is reachable only from
    /// [`LockMachine::unlock_completed`], which is in turn reachable only after
    /// [`LockMachine::unlock`] has accepted a success.
    fn move_to(&mut self, to: Phase) -> Transition {
        if !matches!(
            to,
            Phase::Locking
                | Phase::Locked
                | Phase::Covering
                | Phase::LockedWithoutUi
                | Phase::Unlocking
        ) {
            return Transition::Ignored(IgnoreReason::OutOfScope);
        }
        let from = self.phase;
        if from == to {
            return Transition::Recorded;
        }
        self.phase = to;
        Transition::Changed { from, to }
    }

    /// Moves to `covering` if that is a change, and to a settled locked phase
    /// once the new output has been covered.
    ///
    /// Used by the two paths whose *purpose* is to stop reporting locked: an
    /// output appearing while held, and the last uncovered output going away.
    fn cover_up(&mut self, waiting: Phase) -> Transition {
        if self.covers_all_outputs() {
            let to = if self.ui_present {
                Phase::Locked
            } else {
                Phase::LockedWithoutUi
            };
            return self.move_to(to);
        }
        self.move_to(waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str) -> OutputId {
        OutputId::new(name)
    }

    /// A machine that has been asked to lock, with two outputs, neither covered.
    fn locking() -> LockMachine {
        let mut machine = LockMachine::new();
        machine.apply(LockEvent::OutputAdded(output("eDP-1")));
        machine.apply(LockEvent::OutputAdded(output("HDMI-A-1")));
        machine.apply(LockEvent::Requested(LockReason::User));
        machine
    }

    /// A machine that is fully locked on two outputs.
    fn locked() -> LockMachine {
        let mut machine = locking();
        machine.apply(LockEvent::OutputCovered(output("eDP-1")));
        machine.apply(LockEvent::OutputCovered(output("HDMI-A-1")));
        assert_eq!(machine.phase(), Phase::Locked);
        machine
    }

    #[test]
    fn a_fresh_machine_is_unlocked() {
        let machine = LockMachine::new();

        assert_eq!(machine.phase(), Phase::Unlocked);
        assert!(!machine.holds_session());
        assert!(!machine.reports_locked());
        assert_eq!(machine.reason(), None);
    }

    #[test]
    fn a_lock_with_no_outputs_is_locked_immediately() {
        let mut machine = LockMachine::new();

        let transition = machine.apply(LockEvent::Requested(LockReason::Idle));

        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::Unlocked,
                to: Phase::Locked
            }
        );
        assert_eq!(machine.reason(), Some(LockReason::Idle));
        assert!(machine.reports_locked());
    }

    #[test]
    fn a_lock_with_uncovered_outputs_is_only_locking() {
        let machine = locking();

        assert_eq!(machine.phase(), Phase::Locking);
        assert!(machine.holds_session());
        assert!(
            !machine.reports_locked(),
            "a held session with an uncovered output is not reported locked"
        );
        assert_eq!(
            machine.uncovered_outputs().collect::<Vec<_>>(),
            [&output("HDMI-A-1"), &output("eDP-1")]
        );
    }

    #[test]
    fn the_last_cover_makes_it_locked() {
        let mut machine = locking();

        machine.apply(LockEvent::OutputCovered(output("eDP-1")));
        assert_eq!(machine.phase(), Phase::Locking);

        let transition = machine.apply(LockEvent::OutputCovered(output("HDMI-A-1")));
        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::Locking,
                to: Phase::Locked
            }
        );
        assert!(machine.reports_locked());
    }

    #[test]
    fn an_output_appearing_while_locked_stops_the_locked_report_until_it_is_covered() {
        let mut machine = locked();

        let transition = machine.apply(LockEvent::OutputAdded(output("DP-1")));
        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::Locked,
                to: Phase::Covering
            }
        );
        assert!(
            machine.holds_session(),
            "the safety state never becomes unlocked in the meantime"
        );
        assert!(!machine.reports_locked());

        // Covering a *different* output does not finish the job.
        machine.apply(LockEvent::OutputCovered(output("eDP-1")));
        assert_eq!(machine.phase(), Phase::Covering);
        assert!(!machine.reports_locked());

        machine.apply(LockEvent::OutputCovered(output("DP-1")));
        assert_eq!(machine.phase(), Phase::Locked);
        assert!(machine.reports_locked());
    }

    #[test]
    fn an_output_appearing_while_locking_stays_locking() {
        let mut machine = locking();

        machine.apply(LockEvent::OutputAdded(output("DP-1")));

        assert_eq!(machine.phase(), Phase::Locking);
        assert!(!machine.reports_locked());
    }

    #[test]
    fn removing_the_uncovered_output_completes_the_coverage() {
        let mut machine = locking();
        machine.apply(LockEvent::OutputCovered(output("eDP-1")));

        let transition = machine.apply(LockEvent::OutputRemoved(output("HDMI-A-1")));

        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::Locking,
                to: Phase::Locked
            }
        );
        assert!(machine.reports_locked());
    }

    #[test]
    fn removing_an_output_while_covering_restores_the_locked_report() {
        let mut machine = locked();
        machine.apply(LockEvent::OutputAdded(output("DP-1")));

        let transition = machine.apply(LockEvent::OutputRemoved(output("DP-1")));

        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::Covering,
                to: Phase::Locked
            }
        );
        assert!(machine.reports_locked());
    }

    #[test]
    fn a_duplicate_add_does_not_uncover_an_output() {
        let mut machine = locked();

        let transition = machine.apply(LockEvent::OutputAdded(output("eDP-1")));

        assert_eq!(transition, Transition::Ignored(IgnoreReason::AlreadyKnown));
        assert_eq!(
            machine.phase(),
            Phase::Locked,
            "an output the machine already has cannot be newly uncovered"
        );
        assert!(machine.reports_locked());
    }

    #[test]
    fn covering_an_unknown_output_is_refused() {
        let mut machine = locked();

        let transition = machine.apply(LockEvent::OutputCovered(output("DP-9")));

        assert_eq!(transition, Transition::Ignored(IgnoreReason::UnknownOutput));
        assert_eq!(machine.phase(), Phase::Locked);
    }

    #[test]
    fn removing_an_unknown_output_is_refused() {
        let mut machine = locked();

        let transition = machine.apply(LockEvent::OutputRemoved(output("DP-9")));

        assert_eq!(transition, Transition::Ignored(IgnoreReason::UnknownOutput));
        assert_eq!(machine.phase(), Phase::Locked);
    }

    #[test]
    fn a_second_cover_for_the_same_output_is_refused() {
        let mut machine = locked();

        let transition = machine.apply(LockEvent::OutputCovered(output("eDP-1")));

        assert_eq!(
            transition,
            Transition::Ignored(IgnoreReason::AlreadyCovered)
        );
    }

    #[test]
    fn a_lock_request_while_held_is_refused() {
        let mut machine = locked();

        let transition = machine.apply(LockEvent::Requested(LockReason::Idle));

        assert_eq!(transition, Transition::Ignored(IgnoreReason::NotUnlocked));
        assert_eq!(
            machine.reason(),
            Some(LockReason::User),
            "the first reason stands; a second request does not relabel the lock"
        );
        assert!(machine.reports_locked());
    }

    #[test]
    fn locking_again_requires_fresh_frames() {
        let mut machine = locked();
        machine.apply(LockEvent::UnlockCompleted); // ignored: nothing to complete
        machine.unlock(AuthOutcome::Success);
        machine.apply(LockEvent::UnlockCompleted);
        assert_eq!(machine.phase(), Phase::Unlocked);

        machine.apply(LockEvent::Requested(LockReason::User));

        assert_eq!(machine.phase(), Phase::Locking);
        assert!(
            !machine.reports_locked(),
            "a cover from the previous lock is not evidence about this one"
        );
    }

    #[test]
    fn the_ui_exiting_keeps_the_session_locked() {
        let mut machine = locked();

        let transition = machine.apply(LockEvent::UiExited);

        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::Locked,
                to: Phase::LockedWithoutUi
            }
        );
        assert!(machine.holds_session());
        assert!(
            machine.reports_locked(),
            "a dead UI is still a locked session: the frames are up"
        );
        assert!(!machine.ui_present());
    }

    #[test]
    fn the_ui_exiting_before_the_frames_are_up_still_holds_the_session() {
        let mut machine = locking();

        machine.apply(LockEvent::UiExited);

        assert_eq!(machine.phase(), Phase::LockedWithoutUi);
        assert!(machine.holds_session());
        assert!(
            !machine.reports_locked(),
            "held, but nothing has covered the outputs yet"
        );
    }

    #[test]
    fn covering_after_the_ui_died_reports_locked_without_a_ui() {
        let mut machine = locking();
        machine.apply(LockEvent::UiExited);

        machine.apply(LockEvent::OutputCovered(output("eDP-1")));
        machine.apply(LockEvent::OutputCovered(output("HDMI-A-1")));

        assert_eq!(machine.phase(), Phase::LockedWithoutUi);
        assert!(machine.reports_locked());
    }

    #[test]
    fn the_ui_coming_back_restores_the_locked_phase() {
        let mut machine = locked();
        machine.apply(LockEvent::UiExited);

        let transition = machine.apply(LockEvent::UiStarted);

        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::LockedWithoutUi,
                to: Phase::Locked
            }
        );
    }

    #[test]
    fn the_ui_coming_back_while_an_output_is_uncovered_re_enters_locking() {
        let mut machine = locking();
        machine.apply(LockEvent::UiExited);

        machine.apply(LockEvent::UiStarted);

        assert_eq!(machine.phase(), Phase::Locking);
        assert!(!machine.reports_locked());
    }

    #[test]
    fn a_failed_attempt_keeps_the_session_locked() {
        for failure in [
            AuthFailure::Rejected,
            AuthFailure::TimedOut,
            AuthFailure::Unavailable,
            AuthFailure::Cancelled,
        ] {
            let mut machine = locked();

            let transition = machine.apply(LockEvent::AuthFailed(failure));

            assert_eq!(transition, Transition::Recorded);
            assert_eq!(machine.last_failure(), Some(failure));
            assert_eq!(machine.phase(), Phase::Locked);
            assert!(machine.holds_session());
        }
    }

    #[test]
    fn a_failure_outcome_is_the_same_as_a_failure_event() {
        let mut machine = locked();

        let transition = machine.unlock(AuthOutcome::Failure(AuthFailure::Rejected));

        assert_eq!(transition, Transition::Recorded);
        assert_eq!(machine.phase(), Phase::Locked);
        assert_eq!(machine.last_failure(), Some(AuthFailure::Rejected));
    }

    #[test]
    fn success_starts_unlocking_but_does_not_unlock() {
        let mut machine = locked();

        let transition = machine.unlock(AuthOutcome::Success);

        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::Locked,
                to: Phase::Unlocking
            }
        );
        assert!(
            machine.holds_session(),
            "the frames are still up until the teardown is confirmed"
        );
        assert!(
            !machine.reports_locked(),
            "Unlocking is not a locked report"
        );
    }

    #[test]
    fn the_teardown_confirmation_is_what_unlocks() {
        let mut machine = locked();
        machine.unlock(AuthOutcome::Success);

        let transition = machine.apply(LockEvent::UnlockCompleted);

        assert_eq!(
            transition,
            Transition::Changed {
                from: Phase::Unlocking,
                to: Phase::Unlocked
            }
        );
        assert_eq!(machine.phase(), Phase::Unlocked);
        assert_eq!(machine.reason(), None);
        assert_eq!(machine.last_failure(), None);
    }

    #[test]
    fn a_teardown_confirmation_with_nothing_in_flight_cannot_unlock() {
        let mut machine = locked();

        let transition = machine.apply(LockEvent::UnlockCompleted);

        assert_eq!(transition, Transition::Ignored(IgnoreReason::NotUnlocking));
        assert_eq!(machine.phase(), Phase::Locked);
    }

    #[test]
    fn a_success_from_an_unlocked_session_does_nothing() {
        let mut machine = LockMachine::new();

        let transition = machine.unlock(AuthOutcome::Success);

        assert_eq!(transition, Transition::Ignored(IgnoreReason::NotUnlocked));
        assert_eq!(machine.phase(), Phase::Unlocked);
    }

    #[test]
    fn a_success_can_be_applied_to_a_locked_session_with_no_ui() {
        let mut machine = locked();
        machine.apply(LockEvent::UiExited);

        machine.unlock(AuthOutcome::Success);
        machine.apply(LockEvent::UnlockCompleted);

        assert_eq!(machine.phase(), Phase::Unlocked);
    }

    #[test]
    fn a_success_can_be_applied_while_an_output_is_still_being_covered() {
        let mut machine = locked();
        machine.apply(LockEvent::OutputAdded(output("DP-1")));
        assert_eq!(machine.phase(), Phase::Covering);

        machine.unlock(AuthOutcome::Success);
        machine.apply(LockEvent::UnlockCompleted);

        assert_eq!(
            machine.phase(),
            Phase::Unlocked,
            "the user authenticated; a pending cover must not keep the lock held"
        );
    }

    #[test]
    fn a_suspend_and_a_resume_do_not_touch_the_lock_state() {
        let mut machine = locked();

        assert_eq!(
            machine.apply(LockEvent::Suspended),
            Transition::Ignored(IgnoreReason::OutOfScope)
        );
        assert_eq!(
            machine.apply(LockEvent::Resumed),
            Transition::Ignored(IgnoreReason::OutOfScope)
        );

        assert_eq!(machine.phase(), Phase::Locked);
        assert!(
            machine.reports_locked(),
            "ADR 0015: a resume leaves the session locked"
        );
    }

    #[test]
    fn output_changes_while_unlocked_are_remembered_but_change_nothing() {
        let mut machine = LockMachine::new();

        machine.apply(LockEvent::OutputAdded(output("eDP-1")));
        machine.apply(LockEvent::OutputCovered(output("eDP-1")));
        machine.apply(LockEvent::OutputRemoved(output("eDP-1")));

        assert_eq!(machine.phase(), Phase::Unlocked);
        assert_eq!(machine.outputs().count(), 0);
    }

    #[test]
    fn an_output_appearing_during_the_teardown_needs_no_frame() {
        let mut machine = locked();
        machine.unlock(AuthOutcome::Success);

        machine.apply(LockEvent::OutputAdded(output("DP-1")));

        assert_eq!(machine.phase(), Phase::Unlocking);
        assert_eq!(machine.uncovered_outputs().count(), 1);
    }

    use proptest::prelude::*;

    /// One step of the sequences the property tests drive the machine with.
    ///
    /// Deliberately the set ADR 0015 names: what a session can throw at a lock
    /// screen that must not let go of it, plus the success that is allowed to.
    #[derive(Debug, Clone)]
    enum Step {
        UiExited,
        UiStarted,
        AuthFailed,
        AuthTimedOut,
        AuthSucceeded,
        OutputAdded(usize),
        OutputRemoved(usize),
        OutputCovered(usize),
        SuspendRequested,
        SuspendCompleted,
    }

    fn arb_step() -> impl Strategy<Value = Step> {
        prop_oneof![
            Just(Step::UiExited),
            Just(Step::UiStarted),
            Just(Step::AuthFailed),
            Just(Step::AuthTimedOut),
            Just(Step::AuthSucceeded),
            Just(Step::SuspendRequested),
            Just(Step::SuspendCompleted),
            (0usize..4).prop_map(Step::OutputAdded),
            (0usize..4).prop_map(Step::OutputRemoved),
            (0usize..4).prop_map(Step::OutputCovered),
        ]
    }

    /// What the outputs should look like, derived independently of the machine.
    ///
    /// The point of a second implementation is that agreement between the two is
    /// evidence: if the machine's coverage bookkeeping drifts by one branch, the
    /// oracle notices.
    #[derive(Debug, Default)]
    struct Oracle {
        present: BTreeSet<usize>,
        covered: BTreeSet<usize>,
    }

    impl Oracle {
        fn apply(&mut self, step: &Step) {
            match step {
                Step::OutputAdded(index) => {
                    if self.present.insert(*index) {
                        self.covered.remove(index);
                    }
                }
                Step::OutputRemoved(index) => {
                    self.present.remove(index);
                    self.covered.remove(index);
                }
                // Only an output that exists can be covered: the machine
                // refuses a cover for one it has never heard of.
                Step::OutputCovered(index) if self.present.contains(index) => {
                    self.covered.insert(*index);
                }
                _ => {}
            }
        }

        fn all_covered(&self) -> bool {
            self.present.difference(&self.covered).next().is_none()
        }
    }

    fn drive(machine: &mut LockMachine, oracle: &mut Oracle, step: &Step) -> bool {
        match step {
            Step::UiExited => {
                machine.apply(LockEvent::UiExited);
            }
            Step::UiStarted => {
                machine.apply(LockEvent::UiStarted);
            }
            Step::AuthFailed => {
                machine.apply(LockEvent::AuthFailed(AuthFailure::Rejected));
            }
            Step::AuthTimedOut => {
                machine.apply(LockEvent::AuthFailed(AuthFailure::TimedOut));
            }
            Step::AuthSucceeded => {
                machine.unlock(AuthOutcome::Success);
                return true;
            }
            Step::SuspendRequested => {
                machine.apply(LockEvent::Suspended);
            }
            Step::SuspendCompleted => {
                machine.apply(LockEvent::Resumed);
            }
            Step::OutputAdded(index) => {
                machine.apply(LockEvent::OutputAdded(OutputId::new(format!(
                    "out-{index}"
                ))));
            }
            Step::OutputRemoved(index) => {
                machine.apply(LockEvent::OutputRemoved(OutputId::new(format!(
                    "out-{index}"
                ))));
            }
            Step::OutputCovered(index) => {
                machine.apply(LockEvent::OutputCovered(OutputId::new(format!(
                    "out-{index}"
                ))));
            }
        }

        oracle.apply(step);
        false
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// No sequence of failures, UI crashes, output changes or suspends ever
        /// unlocks the session.
        #[test]
        fn nothing_but_an_authentication_success_unlocks(
            steps in proptest::collection::vec(arb_step(), 0..48),
        ) {
            let mut machine = LockMachine::new();
            let mut oracle = Oracle::default();
            machine.apply(LockEvent::Requested(LockReason::User));

            let mut succeeded = false;
            for step in &steps {
                let before = machine.phase();
                let just_succeeded = drive(&mut machine, &mut oracle, step);
                succeeded |= just_succeeded;

                if !succeeded {
                    prop_assert!(
                        machine.holds_session(),
                        "the session was released without an authentication success \
                         after {:?}: {:?}",
                        step,
                        machine,
                    );
                }
                if machine.phase() == Phase::Unlocking && before != Phase::Unlocking {
                    prop_assert!(
                        just_succeeded,
                        "Unlocking was entered by something other than a success: \
                         {:?} did, from {:?}",
                        step,
                        before,
                    );
                }
            }
        }

        /// The coverage bookkeeping matches an independently written model, and
        /// the locked report follows it.
        #[test]
        fn the_locked_report_follows_the_coverage(
            steps in proptest::collection::vec(arb_step(), 0..48),
        ) {
            let mut machine = LockMachine::new();
            let mut oracle = Oracle::default();
            machine.apply(LockEvent::Requested(LockReason::User));

            for step in &steps {
                let present_before = oracle.present.clone();
                drive(&mut machine, &mut oracle, step);

                if machine.holds_session() {
                    prop_assert_eq!(
                        machine.covers_all_outputs(),
                        oracle.all_covered(),
                        "coverage drifted after {:?}: machine {} vs model {}",
                        step,
                        machine.outputs().map(OutputId::as_str).collect::<Vec<_>>().join(","),
                        oracle.present.iter().map(usize::to_string).collect::<Vec<_>>().join(","),
                    );
                    prop_assert_eq!(
                        machine.reports_locked(),
                        machine.phase().is_locked_family() && oracle.all_covered(),
                        "the locked report does not follow the coverage after {:?}",
                        step,
                    );
                }

                // An output that appeared while the session was held has no
                // secure frame yet, so it cannot be counted as covered: this is
                // the `Locked -> Covering -> Locked` rule, checked against the
                // model rather than against the machine's own bookkeeping.
                if let Step::OutputAdded(index) = step
                    && !present_before.contains(index)
                    && machine.holds_session()
                {
                    prop_assert!(
                        !oracle.covered.contains(index),
                        "an output that has just appeared is already covered",
                    );
                    prop_assert!(
                        machine.uncovered_outputs().any(|output| output.as_str() == format!("out-{index}")),
                        "the machine does not think the new output needs a frame",
                    );
                }
            }
        }
    }
}
