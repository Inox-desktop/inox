//! The Elm loop: state, messages, and `update`.
//!
//! ADR 0003 fixes this shape for every shell component and ADR 0013 repeats it
//! for interactions: a message goes in, the state changes, and anything slow
//! comes back as a task. Nothing here draws, and nothing blocks — the two things
//! a lock screen asks of other processes (lock the session, check a credential)
//! are queued in [`OutgoingCommand`] for a transport this milestone does not
//! have, which is what keeps [`update`] a pure state transition.
//!
//! Four rules the handler exists to keep:
//!
//! - **Nothing unlocks but the boundary.** [`Message::AuthAnswered`] is the only
//!   path to [`LockMachine::unlock`], and it refuses anything that is not an answer
//!   to the attempt in flight — an unsolicited success with no attempt behind it, an
//!   answer for an attempt already over, a duplicate about one already answered,
//!   and a generation that was never sent are all dropped rather than obeyed
//!   ([`BoundaryState::is_current`]).
//! - **Suspends wait for a confirmed lock.** A suspend request locks first, and
//!   the suspend is allowed only once the machine reports every output covered —
//!   `State::confirm_lock` is that watch (ADR 0015).
//! - **A credential is cleared whenever it stops being needed** — on a cancel, when
//!   the trusted UI hides or dies, on a successful unlock, and when a suspend takes
//!   the session away — and it leaves the state by being *moved* into the outgoing
//!   attempt rather than copied.
//! - **The field is disabled rather than surprising.** Typing and submitting are
//!   gated on [`State::model`], which already carries the reason, so the rules
//!   about who may type live in one projection instead of being repeated here.

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced::keyboard::Key;
use iced::keyboard::key::Named;
use iced::{Element, Task};
use inox_common::i18n::Localizer;
use inox_config::Theme as Tokens;
use tracing::{debug, info, warn};

use crate::attempts::{AttemptLimiter, AttemptPolicy, Gate};
use crate::contract::{
    AuthAttempt, AuthOutcome, BoundaryState, Generation, LockProtocol, ProtocolAbsence,
    ServiceFailure,
};
use crate::credential::Secret;
use crate::state::{LockEvent, LockMachine, LockReason, OutputId, Transition};
use crate::suspend::{
    SuspendCoordinator, SuspendDecision, SuspendEvent, SuspendPhase, SuspendPolicy, SuspendRefusal,
};
use crate::view_model::{self, LockViewModel, Projection};

/// How often the lock screen re-reads its clock.
///
/// The only thing it is for is letting a spent attempt allowance expire while
/// nobody is typing: every other state change is caused by a message. It is a
/// policy period rather than a design token, because nothing is drawn with it.
const TICK: Duration = Duration::from_secs(1);

/// Something the lock screen asks of a process it does not own.
///
/// The transports are not implemented in this milestone (ADR 0018): the
/// compositor does not advertise `ext-session-lock-v1`, and the authentication
/// boundary is the review item ADR 0015 names. This queue is the interface those
/// two will drain, and it is the only place the credential ever leaves [`State`].
#[derive(Debug)]
pub enum OutgoingCommand {
    /// Ask the compositor to lock the session.
    Lock(LockReason),
    /// Ask for a lock surface on an output, so it can present a secure frame.
    CoverOutput(OutputId),
    /// Hand an attempt to the trusted authentication boundary.
    Authenticate(AuthAttempt),
    /// Tell the session the lock is confirmed and the suspend may proceed.
    AllowSuspend(Generation),
    /// Tell the session the suspend must not happen.
    RefuseSuspend(SuspendRefusal),
}

/// Everything the lock screen can be told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The compositor offers the lock protocol.
    ProtocolPresent,
    /// The compositor does not offer it, for this reason.
    ProtocolAbsent(ProtocolAbsence),
    /// The session asked for a lock.
    LockRequested(LockReason),
    /// An output appeared.
    OutputAdded(OutputId),
    /// An output went away.
    OutputRemoved(OutputId),
    /// An output presented a secure lock frame.
    OutputCovered(OutputId),
    /// The trusted lock screen UI exited or died.
    UiExited,
    /// The session started a trusted UI again.
    UiStarted,
    /// A character was typed.
    Character(char),
    /// Backspace was pressed.
    Backspace,
    /// The user hid or cancelled the credential without submitting it.
    Cancelled,
    /// The user submitted the credential.
    Submitted,
    /// The boundary has the attempt and is checking it.
    AuthAccepted(Generation),
    /// The boundary answered, for a generation.
    AuthAnswered(Generation, AuthOutcome),
    /// The boundary cannot be reached.
    AuthUnavailable(ServiceFailure),
    /// The user asked to try an unusable boundary again.
    BoundaryRetried,
    /// The compositor confirmed the lock surfaces are gone.
    UnlockConfirmed,
    /// A suspend is being prepared.
    SuspendRequested,
    /// The lock did not confirm for a request.
    SuspendLockFailed(Generation),
    /// The lock did not confirm in time for a request.
    SuspendTimedOut(Generation),
    /// The session is back from a suspend.
    Resumed,
    /// The clock moved.
    Tick(Instant),
}

/// The lock screen's state.
pub struct State {
    localizer: Rc<Localizer>,
    tokens: Tokens,
    machine: LockMachine,
    limiter: AttemptLimiter,
    suspend: SuspendCoordinator,
    boundary: BoundaryState,
    protocol: LockProtocol,
    credential: Secret,
    generation: Generation,
    clock: Instant,
    model: LockViewModel,
    outgoing: VecDeque<OutgoingCommand>,
}

impl State {
    /// A lock screen that has not locked anything yet.
    ///
    /// The protocol starts [`LockProtocol::default`] — unadvertised — because that
    /// is the truth about every Inox session today, and a state that claimed the
    /// protocol was available until the compositor denied it would put a working
    /// prompt on screen for a frame, which is exactly the frame a lock screen must
    /// not have.
    pub fn new(localizer: Rc<Localizer>, tokens: Tokens) -> Self {
        let clock = Instant::now();
        let machine = LockMachine::new();
        let boundary = BoundaryState::Idle;
        let protocol = LockProtocol::default();
        let credential = Secret::default();
        let mut limiter = AttemptLimiter::new(AttemptPolicy::standard());

        // The first model is projected from the same state as every later one, so
        // there is no frame in which the lock screen shows something the
        // projection would not have produced.
        let model = view_model::project(Projection {
            phase: machine.phase(),
            protocol: &protocol,
            boundary: &boundary,
            gate: limiter.gate(clock),
            credential_len: credential.len(),
            localizer: &localizer,
        });

        Self {
            localizer,
            tokens,
            machine,
            limiter,
            suspend: SuspendCoordinator::new(SuspendPolicy::default()),
            boundary,
            protocol,
            credential,
            generation: Generation::first(),
            clock,
            model,
            outgoing: VecDeque::new(),
        }
    }

    /// The catalogs.
    pub fn localizer(&self) -> &Localizer {
        &self.localizer
    }

    /// The design tokens.
    pub const fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    /// The lock state machine.
    pub const fn machine(&self) -> &LockMachine {
        &self.machine
    }

    /// What the authentication boundary last said.
    pub const fn boundary(&self) -> &BoundaryState {
        &self.boundary
    }

    /// Whether the compositor offers the lock protocol.
    pub const fn protocol(&self) -> &LockProtocol {
        &self.protocol
    }

    /// The suspend coordination.
    pub const fn suspend(&self) -> &SuspendCoordinator {
        &self.suspend
    }

    /// How many characters the credential holds.
    pub fn credential_len(&self) -> usize {
        self.credential.len()
    }

    /// What the view draws.
    pub const fn model(&self) -> &LockViewModel {
        &self.model
    }

    /// Whether an attempt may be made at the current clock.
    pub fn gate(&mut self) -> Gate {
        self.limiter.gate(self.clock)
    }

    /// Takes what the lock screen has asked for since this was last called.
    pub fn take_outgoing(&mut self) -> Vec<OutgoingCommand> {
        self.outgoing.drain(..).collect()
    }

    /// Recomputes what the view draws.
    fn refresh_model(&mut self) {
        let gate = self.limiter.gate(self.clock);

        self.model = view_model::project(Projection {
            phase: self.machine.phase(),
            protocol: &self.protocol,
            boundary: &self.boundary,
            gate,
            credential_len: self.credential.len(),
            localizer: &self.localizer,
        });
    }

    /// Applies a lock event, logging a phase change.
    fn lock_event(&mut self, event: LockEvent) -> Transition {
        let transition = self.machine.apply(event);

        if let Transition::Changed { from, to } = &transition {
            debug!(?from, ?to, "the lock state moved");
        }

        transition
    }

    /// Interprets an authentication outcome through the machine's one door.
    fn auth_outcome(&mut self, outcome: AuthOutcome) -> Transition {
        let transition = self.machine.unlock(outcome);

        if let Transition::Changed { from, to } = &transition {
            debug!(?from, ?to, "the lock state moved");
        }

        transition
    }

    /// Asks the compositor for a lock surface on every output that exists.
    fn cover_every_output(&mut self) {
        let outputs: Vec<OutputId> = self.machine.outputs().cloned().collect();

        for output in outputs {
            self.outgoing
                .push_back(OutgoingCommand::CoverOutput(output));
        }
    }

    /// Lets a suspend proceed, if the lock has just become fully covered.
    ///
    /// This is the ordering of ADR 0015 in one place: the suspend is allowed only
    /// when the machine reports every existing output covered, and only for the
    /// request currently being prepared.
    fn confirm_lock(&mut self) {
        if !self.machine.reports_locked() || self.suspend.phase() != SuspendPhase::Locking {
            return;
        }

        let decision = self
            .suspend
            .apply(SuspendEvent::LockConfirmed(self.suspend.generation()));
        self.queue(decision);
    }

    /// Puts a suspend decision on the queue, and says so in the log.
    ///
    /// Letting a suspend proceed is also where the credential is cleared for the
    /// last time: the session is going away, and the field was still typable while
    /// the lock that authorised the suspend was being confirmed (ADR 0015).
    fn queue(&mut self, decision: SuspendDecision) {
        match decision {
            SuspendDecision::AllowSuspend { generation } => {
                // The request cleared the field, but the lock is confirmed later
                // and the field is typable in between; what was typed while the
                // lock was being established must not ride into the suspend
                // either (ADR 0015).
                self.forget_credential("the suspend is taking the session away");
                info!(
                    %generation,
                    outputs = self.machine.outputs().count(),
                    "the lock is confirmed: the suspend may proceed"
                );
                self.outgoing
                    .push_back(OutgoingCommand::AllowSuspend(generation));
            }
            SuspendDecision::Refuse { generation, reason } => {
                warn!(
                    %generation,
                    ?reason,
                    "the suspend is refused: the lock could not be confirmed"
                );
                self.outgoing
                    .push_back(OutgoingCommand::RefuseSuspend(reason));
            }
            SuspendDecision::Cancel { generation, reason } => {
                warn!(
                    %generation,
                    ?reason,
                    "the suspend is cancelled: the lock could not be confirmed"
                );
                self.outgoing
                    .push_back(OutgoingCommand::RefuseSuspend(reason));
            }
            SuspendDecision::Lock { generation } => {
                debug!(%generation, "a lock is being prepared for a suspend");
                self.outgoing
                    .push_back(OutgoingCommand::Lock(LockReason::Suspend));
            }
            SuspendDecision::Ignore(reason) => {
                debug!(?reason, "the suspend request changed nothing");
            }
        }
    }

    /// Drops a credential that is no longer needed.
    fn forget_credential(&mut self, why: &'static str) {
        if self.credential.is_empty() {
            return;
        }

        self.credential.clear();
        // Deliberately says nothing about what was cleared.
        info!(why, "the credential was cleared");
    }
}

/// The message a key press maps to.
///
/// Pure, and tested on its own: the subscription below is a two-line adapter, so
/// "Enter submits, Backspace deletes, Escape clears" is a property of this
/// function rather than of a running window (acceptance Q01: the whole lock and
/// unlock path is reachable from the keyboard).
pub fn key_binding(key: &Key) -> Option<Message> {
    match key {
        Key::Named(Named::Enter) => Some(Message::Submitted),
        Key::Named(Named::Backspace) => Some(Message::Backspace),
        Key::Named(Named::Escape) => Some(Message::Cancelled),
        Key::Character(typed) => {
            let mut characters = typed.chars();

            match (characters.next(), characters.next()) {
                (Some(character), None) => Some(Message::Character(character)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The lock screen's widget tree, from the projection.
pub fn view(state: &State) -> Element<'_, Message> {
    crate::view::view(state)
}

/// The window title.
pub fn title(state: &State) -> String {
    state.localizer.message("lock-window-title")
}

/// The window theme, from the theme tokens.
pub fn theme(state: &State) -> iced::Theme {
    crate::theme::from_tokens(&state.tokens)
}

/// The keys the lock screen handles, and the clock.
///
/// Key repeat is ignored: holding a key down is a person leaning on it, not a
/// request, and a lock screen must not accept a credential that was typed by
/// accident. The clock is the only subscription that is not an input — it is how
/// a spent attempt allowance expires while nobody is typing.
pub fn subscription(_state: &State) -> iced::Subscription<Message> {
    iced::Subscription::batch([
        iced::keyboard::listen().filter_map(|event| match event {
            iced::keyboard::Event::KeyPressed {
                key, repeat: false, ..
            } => key_binding(&key),
            _ => None,
        }),
        iced::Subscription::run(tick),
    ])
}

/// A stream that produces one [`Message::Tick`] a second, for as long as the
/// subscription is alive.
///
/// The thread ends when the runtime drops the receiver, which is what stops the
/// timer when the lock screen exits.
fn tick() -> iced::futures::channel::mpsc::Receiver<Message> {
    let (mut sender, receiver) = iced::futures::channel::mpsc::channel(1);

    std::thread::spawn(move || {
        loop {
            match sender.try_send(Message::Tick(Instant::now())) {
                Ok(()) => {}
                // The runtime has not read the last tick yet; skipping this one
                // is better than blocking the timer thread.
                Err(error) if error.is_full() => {}
                Err(_) => break,
            }

            std::thread::sleep(TICK);
        }
    });

    receiver
}

/// Applies one message.
pub fn update(state: &mut State, message: Message) -> Task<Message> {
    handle(state, message);
    state.refresh_model();
    Task::none()
}

/// The message handler, without the projection step.
fn handle(state: &mut State, message: Message) {
    match message {
        Message::ProtocolPresent => {
            state.protocol = LockProtocol::Advertised;
            info!("the compositor offers the session lock protocol");
        }
        Message::ProtocolAbsent(reason) => {
            state.protocol = LockProtocol::NotAdvertised { reason };
            warn!(
                ?reason,
                "the compositor does not offer the session lock protocol: this \
                 build cannot lock the session (ADR 0015, ADR 0018)"
            );

            // No lock can be asked for, so a suspend waiting on one is answered
            // now rather than left to time out (ADR 0015).
            if state.suspend.phase() == SuspendPhase::Locking {
                let decision = state.suspend.refuse_unlockable();
                state.queue(decision);
            }
        }
        Message::LockRequested(reason) => {
            // Without the protocol there is no lock to hold, and the machine
            // claiming one would be a claim about the session's safety that
            // nothing behind it supports (ADR 0015).
            if !state.protocol.is_advertised() {
                warn!(
                    ?reason,
                    "the session was asked to lock, but this compositor has no \
                     session lock protocol: nothing is locked"
                );
                return;
            }

            if state.lock_event(LockEvent::Requested(reason)).is_change() {
                info!(?reason, "locking the session");
                state.outgoing.push_back(OutgoingCommand::Lock(reason));
                state.cover_every_output();
            }
            state.confirm_lock();
        }
        Message::OutputAdded(output) => {
            let held = state.machine.holds_session();
            state.lock_event(LockEvent::OutputAdded(output.clone()));

            if held {
                debug!(%output, "covering an output that appeared while locked");
                state
                    .outgoing
                    .push_back(OutgoingCommand::CoverOutput(output));
            }
            state.confirm_lock();
        }
        Message::OutputRemoved(output) => {
            state.lock_event(LockEvent::OutputRemoved(output));
            state.confirm_lock();
        }
        Message::OutputCovered(output) => {
            state.lock_event(LockEvent::OutputCovered(output));
            state.confirm_lock();
        }
        Message::UiExited => {
            // The field is gone with the UI that drew it. ADR 0015 clears
            // sensitive input when the UI hides, and a UI that died is hidden
            // whether it meant to be or not.
            state.forget_credential("the trusted lock screen UI exited");
            state.lock_event(LockEvent::UiExited);
            warn!(
                "the trusted lock screen UI exited: the session stays locked \
                 while the session restarts it (ADR 0015)"
            );
        }
        Message::UiStarted => {
            state.lock_event(LockEvent::UiStarted);
            info!("the trusted lock screen UI is running again");
        }
        Message::Character(character) => {
            if state.model.accepts_input {
                state.credential.push(character);
            } else {
                debug!("a character was ignored: the credential field is disabled");
            }
        }
        Message::Backspace => {
            if state.model.accepts_input {
                state.credential.backspace();
            }
        }
        Message::Cancelled => state.forget_credential("the credential was cancelled"),
        Message::Submitted => submit(state),
        Message::AuthAccepted(generation) => {
            if !state.boundary.is_current(generation) {
                warn!(%generation, "an acceptance for an attempt that is not in flight");
                return;
            }
            state.boundary = BoundaryState::Accepted { generation };
        }
        Message::AuthAnswered(generation, outcome) => {
            if !state.boundary.is_current(generation) {
                // Four ways to land here, none of which may be obeyed: an answer
                // for an attempt already over, a duplicate about one already
                // answered, a generation that was never sent, and an unsolicited
                // success with no attempt in flight at all (ADR 0015).
                warn!(
                    %generation,
                    success = outcome.is_success(),
                    "an answer for an attempt that is not in flight: ignored"
                );
                return;
            }

            state.boundary = BoundaryState::Answered {
                generation,
                outcome,
            };
            // The one call that can move the machine towards Unlocked.
            state.auth_outcome(outcome);

            match outcome {
                AuthOutcome::Success => {
                    state.limiter.record_success();
                    state.forget_credential("the session was unlocked");
                    info!("the trusted boundary authenticated the user");
                }
                AuthOutcome::Failure(failure) => {
                    state.limiter.record_failure(state.clock);
                    warn!(?failure, "the attempt was refused");
                }
            }
        }
        Message::AuthUnavailable(failure) => {
            warn!(?failure, "the authentication boundary is not usable");
            state.boundary = BoundaryState::Unavailable { failure };
        }
        Message::BoundaryRetried => {
            // A retry is not a re-submission: it puts the screen back into a
            // state where the user can type, and the next attempt is a new one
            // with a new generation (acceptance D05's "retry", not D05's
            // "the same action twice").
            if state.boundary.failure().is_some() {
                info!("trying the authentication boundary again");
                state.boundary = BoundaryState::Idle;
            }
        }
        Message::UnlockConfirmed => {
            if state.lock_event(LockEvent::UnlockCompleted).is_change() {
                info!("the lock surfaces are gone: the session is unlocked");
                state.boundary = BoundaryState::Idle;
                state.forget_credential("the lock surfaces are gone");
            }
        }
        Message::SuspendRequested => {
            // First, and whatever happens next: the session is about to go away,
            // and sensitive input does not stay behind for the resume (ADR 0015).
            // A suspend that is refused afterwards has still taken the screen
            // away, and a user who typed nothing they meant to lose types again.
            state.forget_credential("the session is suspending");

            if !state.protocol.is_advertised() {
                let decision = state.suspend.refuse_unlockable();
                state.queue(decision);
                return;
            }

            let decision = state.suspend.apply(SuspendEvent::Requested);

            if state.machine.reports_locked() {
                // Already locked with every output covered: there is no lock to
                // ask for, and the confirmation is immediate.
                state.confirm_lock();
            } else {
                state.queue(decision);

                // The suspend must not be allowed before the lock is confirmed,
                // so the same lock the user would have asked for is asked for
                // here, and every output has to present a frame before it
                // happens.
                if state
                    .lock_event(LockEvent::Requested(LockReason::Suspend))
                    .is_change()
                {
                    state.cover_every_output();
                }
                state.confirm_lock();
            }
        }
        Message::SuspendLockFailed(generation) => {
            let decision = state.suspend.apply(SuspendEvent::LockFailed(generation));
            state.queue(decision);
        }
        Message::SuspendTimedOut(generation) => {
            let decision = state.suspend.apply(SuspendEvent::TimedOut(generation));
            state.queue(decision);
        }
        Message::Resumed => {
            state.lock_event(LockEvent::Resumed);
            let decision = state.suspend.apply(SuspendEvent::Resumed);
            debug!(
                ?decision,
                phase = ?state.machine.phase(),
                "back from a suspend; the lock state is unchanged (ADR 0015)"
            );
        }
        Message::Tick(now) => state.clock = now,
    }
}

/// Sends the typed credential to the boundary, at most once per attempt.
fn submit(state: &mut State) {
    if !state.model.accepts_input {
        debug!(
            reason = state.model.disabled_reason.as_deref().unwrap_or("disabled"),
            "the attempt was not sent"
        );
        return;
    }

    if state.credential.is_empty() {
        debug!("the attempt was not sent: nothing was typed");
        return;
    }

    state.generation = state.generation.next();
    let generation = state.generation;
    // The credential is *moved* out: the state no longer holds it, so there is
    // no second copy to leak and nothing left to clear but the empty buffer the
    // move left behind.
    let attempt = AuthAttempt::new(generation, std::mem::take(&mut state.credential));

    state
        .outgoing
        .push_back(OutgoingCommand::Authenticate(attempt));
    state.boundary = BoundaryState::Sent { generation };
    debug!(%generation, "an attempt was handed to the authentication boundary");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::AuthFailure;
    use crate::view_model::{Face, FocusTarget};
    use iced::theme::Base;
    use inox_common::i18n::Locale;

    fn state() -> State {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");

        State::new(Rc::new(localizer), Tokens::builtin_dark())
    }

    fn send(state: &mut State, message: Message) {
        let _ = update(state, message);
    }

    fn output(name: &str) -> OutputId {
        OutputId::new(name)
    }

    /// A lock screen whose compositor offers the protocol, with two outputs, and
    /// which the session has asked to lock.
    fn locking() -> State {
        let mut state = state();
        send(&mut state, Message::ProtocolPresent);
        send(&mut state, Message::OutputAdded(output("eDP-1")));
        send(&mut state, Message::OutputAdded(output("HDMI-A-1")));
        let _ = state.take_outgoing();
        send(&mut state, Message::LockRequested(LockReason::User));
        state
    }

    /// A lock screen that is locked, with every output covered.
    fn locked() -> State {
        let mut state = locking();
        send(&mut state, Message::OutputCovered(output("eDP-1")));
        send(&mut state, Message::OutputCovered(output("HDMI-A-1")));
        let _ = state.take_outgoing();
        state
    }

    fn type_text(state: &mut State, text: &str) {
        for character in text.chars() {
            send(state, Message::Character(character));
        }
    }

    /// The attempts that have been sent, in order.
    fn attempts(commands: &[OutgoingCommand]) -> Vec<Generation> {
        commands
            .iter()
            .filter_map(|command| match command {
                OutgoingCommand::Authenticate(attempt) => Some(attempt.generation()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_fresh_lock_screen_cannot_lock_anything_yet() {
        let mut state = state();

        assert!(!state.protocol().is_advertised());
        assert_eq!(state.machine().phase(), crate::state::Phase::Unlocked);
        assert!(!state.model().accepts_input);
        assert!(
            matches!(state.model().face, Face::Unavailable { .. }),
            "the first thing the screen says is that it cannot lock: {:?}",
            state.model().face
        );
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn a_lock_request_asks_the_compositor_and_covers_every_output() {
        let mut state = locking();

        assert_eq!(state.machine().phase(), crate::state::Phase::Locking);
        let commands = state.take_outgoing();

        assert!(matches!(commands.first(), Some(OutgoingCommand::Lock(_))));
        let covered: Vec<&OutputId> = commands
            .iter()
            .filter_map(|command| match command {
                OutgoingCommand::CoverOutput(output) => Some(output),
                _ => None,
            })
            .collect();
        assert_eq!(covered.len(), 2, "every existing output is covered");
    }

    #[test]
    fn nothing_is_typed_into_a_session_that_is_not_locked() {
        let mut state = state();
        send(&mut state, Message::ProtocolPresent);

        type_text(&mut state, "secret");

        assert_eq!(state.credential_len(), 0);
        assert!(matches!(state.model().face, Face::Prompt));
        assert_eq!(
            state.model().disabled_reason.as_deref(),
            Some("The session is not locked")
        );
    }

    #[test]
    fn a_lock_request_without_the_protocol_locks_nothing() {
        let mut state = state();
        send(&mut state, Message::OutputAdded(output("eDP-1")));

        send(&mut state, Message::LockRequested(LockReason::User));

        assert_eq!(
            state.machine().phase(),
            crate::state::Phase::Unlocked,
            "a compositor that cannot lock must not be reported as having locked"
        );
        assert!(!state.machine().reports_locked());
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn typing_reaches_the_credential_while_the_field_is_enabled() {
        let mut state = locked();

        type_text(&mut state, "abc");

        assert_eq!(state.credential_len(), 3);
        assert_eq!(state.model().mask, "•••");
        assert!(state.model().accepts_input);
    }

    #[test]
    fn backspace_takes_a_character_back() {
        let mut state = locked();
        type_text(&mut state, "abc");

        send(&mut state, Message::Backspace);
        send(&mut state, Message::Backspace);
        send(&mut state, Message::Backspace);
        send(&mut state, Message::Backspace);

        assert_eq!(state.credential_len(), 0);
    }

    #[test]
    fn submitting_sends_the_credential_once_and_only_once() {
        let mut state = locked();
        type_text(&mut state, "hunter2");
        let _ = state.take_outgoing();

        send(&mut state, Message::Submitted);

        let commands = state.take_outgoing();
        assert_eq!(attempts(&commands), vec![Generation::new(2)]);
        assert_eq!(
            state.credential_len(),
            0,
            "the credential left the state with the attempt"
        );
        assert!(state.boundary().in_flight());

        // A second submission while the first is in flight is a duplicate
        // request, which acceptance D05 forbids.
        send(&mut state, Message::Submitted);
        assert!(
            state.take_outgoing().is_empty(),
            "no second attempt while one is being checked"
        );
    }

    #[test]
    fn an_empty_credential_is_not_sent() {
        let mut state = locked();

        send(&mut state, Message::Submitted);

        assert!(state.take_outgoing().is_empty());
        assert!(!state.boundary().in_flight());
    }

    #[test]
    fn the_queued_attempt_carries_the_credential_and_prints_none_of_it() {
        let mut state = locked();
        type_text(&mut state, "hunter2");
        let _ = state.take_outgoing();
        send(&mut state, Message::Submitted);

        let rendered = format!("{:?}", state.take_outgoing());

        assert!(
            !rendered.contains("hunter2"),
            "the queued command prints the credential: {rendered}"
        );
        assert!(rendered.contains("Secret(<redacted>)"), "{rendered}");
    }

    #[test]
    fn the_boundarys_acceptance_is_remembered() {
        let mut state = locked();
        type_text(&mut state, "hunter2");
        send(&mut state, Message::Submitted);
        let generation = state.boundary().generation().expect("an attempt");

        send(&mut state, Message::AuthAccepted(generation));

        assert!(matches!(state.boundary(), BoundaryState::Accepted { .. }));
        assert!(matches!(state.model().face, Face::Verifying { .. }));
    }

    #[test]
    fn a_success_only_unlocks_through_the_teardown_confirmation() {
        let mut state = locked();
        type_text(&mut state, "hunter2");
        send(&mut state, Message::Submitted);
        let generation = state.boundary().generation().expect("an attempt");
        let _ = state.take_outgoing();

        send(
            &mut state,
            Message::AuthAnswered(generation, AuthOutcome::Success),
        );

        assert_eq!(
            state.machine().phase(),
            crate::state::Phase::Unlocking,
            "the frames are still up until the compositor confirms"
        );
        assert!(state.machine().holds_session());

        send(&mut state, Message::UnlockConfirmed);

        assert_eq!(state.machine().phase(), crate::state::Phase::Unlocked);
        assert_eq!(state.boundary(), &BoundaryState::Idle);
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn an_answer_for_an_attempt_that_never_happened_is_ignored() {
        let mut state = locked();
        let _ = state.take_outgoing();

        send(
            &mut state,
            Message::AuthAnswered(Generation::first(), AuthOutcome::Success),
        );

        assert_eq!(
            state.machine().phase(),
            crate::state::Phase::Locked,
            "an unsolicited success must not unlock anything"
        );
    }

    #[test]
    fn an_answer_for_a_generation_that_was_never_sent_is_ignored() {
        let mut state = locked();
        type_text(&mut state, "hunter2");
        send(&mut state, Message::Submitted);
        let generation = state.boundary().generation().expect("an attempt");

        send(
            &mut state,
            Message::AuthAnswered(Generation::new(generation.get() + 42), AuthOutcome::Success),
        );

        assert_eq!(
            state.machine().phase(),
            crate::state::Phase::Locked,
            "a generation that was never sent is not an answer to the attempt in              hand, however successful it is"
        );
        assert_eq!(
            state.boundary(),
            &BoundaryState::Sent { generation },
            "a refused answer leaves no trace to make the real one look stale"
        );

        // The genuine answer still lands.
        send(
            &mut state,
            Message::AuthAnswered(generation, AuthOutcome::Success),
        );
        assert_eq!(state.machine().phase(), crate::state::Phase::Unlocking);
    }

    #[test]
    fn a_duplicate_acceptance_does_not_strand_the_screen_in_the_loading_face() {
        let mut state = locked();
        type_text(&mut state, "wrong");
        send(&mut state, Message::Submitted);
        let generation = state.boundary().generation().expect("an attempt");
        send(
            &mut state,
            Message::AuthAnswered(generation, AuthOutcome::Failure(AuthFailure::Rejected)),
        );

        // A reordered or duplicated transport message about the attempt that has
        // already been answered.
        send(&mut state, Message::AuthAccepted(generation));

        assert!(
            matches!(state.model().face, Face::Failed { .. }),
            "the answered attempt stays answered: {:?}",
            state.model().face
        );
        assert!(
            state.model().accepts_input,
            "and the field comes back rather than spinning with no way out              (acceptance D05)",
        );
    }

    #[test]
    fn a_replayed_success_for_an_answered_attempt_does_not_unlock() {
        let mut state = locked();
        type_text(&mut state, "wrong");
        send(&mut state, Message::Submitted);
        let generation = state.boundary().generation().expect("an attempt");
        send(
            &mut state,
            Message::AuthAnswered(generation, AuthOutcome::Failure(AuthFailure::Rejected)),
        );

        send(
            &mut state,
            Message::AuthAnswered(generation, AuthOutcome::Success),
        );

        assert_eq!(
            state.machine().phase(),
            crate::state::Phase::Locked,
            "the attempt was already answered, and the answer was a refusal"
        );
        assert_eq!(
            state.boundary().outcome(),
            Some(AuthOutcome::Failure(AuthFailure::Rejected)),
            "and the recorded outcome is not overwritten"
        );
    }

    #[test]
    fn an_answer_for_an_older_attempt_is_ignored() {
        let mut state = locked();

        // Two attempts: the first is abandoned by a cancel, the second is
        // submitted, and then the first answers.
        type_text(&mut state, "first");
        send(&mut state, Message::Submitted);
        let first = state.boundary().generation().expect("an attempt");
        send(&mut state, Message::Cancelled);
        send(
            &mut state,
            Message::AuthAnswered(first, AuthOutcome::Failure(AuthFailure::Cancelled)),
        );
        type_text(&mut state, "second");
        send(&mut state, Message::Submitted);
        let second = state.boundary().generation().expect("an attempt");
        assert!(second > first);

        send(
            &mut state,
            Message::AuthAnswered(first, AuthOutcome::Success),
        );

        assert_eq!(
            state.machine().phase(),
            crate::state::Phase::Locked,
            "a late success from an abandoned attempt unlocks nothing"
        );
    }

    #[test]
    fn a_failed_attempt_keeps_the_session_locked_and_leaves_it_retryable() {
        let mut state = locked();
        type_text(&mut state, "wrong");
        send(&mut state, Message::Submitted);
        let generation = state.boundary().generation().expect("an attempt");

        send(
            &mut state,
            Message::AuthAnswered(generation, AuthOutcome::Failure(AuthFailure::Rejected)),
        );

        assert_eq!(state.machine().phase(), crate::state::Phase::Locked);
        assert!(state.machine().holds_session());
        assert_eq!(state.machine().last_failure(), Some(AuthFailure::Rejected));
        assert!(state.model().accepts_input, "and it can be tried again");
        assert!(matches!(state.model().face, Face::Failed { .. }));
    }

    #[test]
    fn the_allowance_ends_in_the_waiting_face_and_a_disabled_field() {
        let mut state = locked();

        for _ in 0..AttemptPolicy::standard().allowance {
            type_text(&mut state, "wrong");
            send(&mut state, Message::Submitted);
            let generation = state.boundary().generation().expect("an attempt");
            send(
                &mut state,
                Message::AuthAnswered(generation, AuthOutcome::Failure(AuthFailure::Rejected)),
            );
        }

        assert!(matches!(state.model().face, Face::Waiting { .. }));
        assert!(!state.model().accepts_input);
        assert_eq!(state.gate(), Gate::Waiting);

        // And it is the field, not the credential, that is refused: typing into
        // a disabled field changes nothing.
        type_text(&mut state, "more");
        assert_eq!(state.credential_len(), 0);
    }

    #[test]
    fn the_waiting_face_ends_when_the_window_has_passed() {
        let mut state = locked();
        let now = Instant::now();

        for _ in 0..AttemptPolicy::standard().allowance {
            type_text(&mut state, "wrong");
            send(&mut state, Message::Submitted);
            let generation = state.boundary().generation().expect("an attempt");
            send(
                &mut state,
                Message::AuthAnswered(generation, AuthOutcome::Failure(AuthFailure::Rejected)),
            );
        }
        assert!(matches!(state.model().face, Face::Waiting { .. }));

        let later = now + AttemptPolicy::standard().window + Duration::from_secs(1);
        send(&mut state, Message::Tick(later));

        assert!(state.model().accepts_input, "{:?}", state.model().face);
        assert_eq!(
            state.model().focus,
            vec![FocusTarget::Credential, FocusTarget::Submit]
        );
    }

    #[test]
    fn the_ui_exiting_keeps_the_session_locked_and_takes_no_input() {
        let mut state = locked();
        let _ = state.take_outgoing();

        send(&mut state, Message::UiExited);

        assert_eq!(
            state.machine().phase(),
            crate::state::Phase::LockedWithoutUi
        );
        assert!(state.machine().reports_locked());
        assert!(matches!(state.model().face, Face::HeldWithoutUi { .. }));
        assert!(!state.model().accepts_input);
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn an_output_appearing_while_locked_is_covered_before_the_lock_is_reported_again() {
        let mut state = locked();

        send(&mut state, Message::OutputAdded(output("DP-1")));

        assert_eq!(state.machine().phase(), crate::state::Phase::Covering);
        assert!(!state.machine().reports_locked());
        let commands = state.take_outgoing();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            OutgoingCommand::CoverOutput(covered) if covered.as_str() == "DP-1"
        ));

        send(&mut state, Message::OutputCovered(output("DP-1")));

        assert_eq!(state.machine().phase(), crate::state::Phase::Locked);
        assert!(state.machine().reports_locked());
    }

    #[test]
    fn a_suspend_locks_first_and_is_allowed_only_once_the_lock_is_confirmed() {
        let mut state = state();
        send(&mut state, Message::ProtocolPresent);
        send(&mut state, Message::OutputAdded(output("eDP-1")));
        send(&mut state, Message::OutputAdded(output("HDMI-A-1")));
        let _ = state.take_outgoing();

        send(&mut state, Message::SuspendRequested);

        let commands = state.take_outgoing();
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, OutgoingCommand::Lock(_))),
            "a suspend asks for the lock first: {commands:?}"
        );
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, OutgoingCommand::AllowSuspend(_))),
            "and it suspends nothing before the lock is confirmed: {commands:?}"
        );
        assert_eq!(state.suspend().phase(), SuspendPhase::Locking);

        // Only one output is covered so far.
        send(&mut state, Message::OutputCovered(output("eDP-1")));
        assert!(
            state.take_outgoing().is_empty(),
            "the lock is not confirmed while an output is uncovered"
        );

        send(&mut state, Message::OutputCovered(output("HDMI-A-1")));

        let commands = state.take_outgoing();
        assert!(
            matches!(commands.first(), Some(OutgoingCommand::AllowSuspend(_))),
            "the suspend is allowed once every output is covered: {commands:?}"
        );
    }

    #[test]
    fn a_suspend_on_an_already_locked_session_asks_for_no_second_lock() {
        let mut state = locked();

        send(&mut state, Message::SuspendRequested);

        let commands = state.take_outgoing();
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, OutgoingCommand::Lock(_))),
            "the session is already locked: {commands:?}"
        );
        assert!(
            matches!(commands.first(), Some(OutgoingCommand::AllowSuspend(_))),
            "{commands:?}"
        );
    }

    #[test]
    fn a_suspend_without_the_lock_protocol_is_refused_rather_than_deferred() {
        let mut state = state();

        send(&mut state, Message::SuspendRequested);

        assert_eq!(state.suspend().phase(), SuspendPhase::Refused);
        let commands = state.take_outgoing();
        assert!(
            matches!(
                commands.as_slice(),
                [OutgoingCommand::RefuseSuspend(
                    SuspendRefusal::NoLockProtocol
                )]
            ),
            "ADR 0015: never suspend first and hope to lock on resume: {commands:?}"
        );
    }

    #[test]
    fn a_suspend_that_never_confirms_is_refused_at_its_deadline() {
        let mut state = state();
        send(&mut state, Message::ProtocolPresent);
        send(&mut state, Message::OutputAdded(output("eDP-1")));
        send(&mut state, Message::SuspendRequested);
        let generation = state.suspend().generation();
        let _ = state.take_outgoing();

        send(&mut state, Message::SuspendTimedOut(generation));

        assert_eq!(state.suspend().phase(), SuspendPhase::Refused);
        let commands = state.take_outgoing();
        assert!(
            matches!(
                commands.as_slice(),
                [OutgoingCommand::RefuseSuspend(SuspendRefusal::TimedOut)]
            ),
            "{commands:?}"
        );
    }

    #[test]
    fn a_lost_lock_protocol_refuses_a_suspend_that_is_waiting_on_one() {
        let mut state = state();
        send(&mut state, Message::ProtocolPresent);
        send(&mut state, Message::OutputAdded(output("eDP-1")));
        send(&mut state, Message::SuspendRequested);
        assert_eq!(state.suspend().phase(), SuspendPhase::Locking);
        let _ = state.take_outgoing();

        send(
            &mut state,
            Message::ProtocolAbsent(ProtocolAbsence::GlobalMissing),
        );

        assert_eq!(state.suspend().phase(), SuspendPhase::Refused);
        let commands = state.take_outgoing();
        assert!(
            matches!(
                commands.as_slice(),
                [OutgoingCommand::RefuseSuspend(
                    SuspendRefusal::NoLockProtocol
                )]
            ),
            "{commands:?}"
        );
    }

    #[test]
    fn coming_back_from_a_suspend_leaves_the_session_locked() {
        let mut state = locked();
        send(&mut state, Message::SuspendRequested);

        send(&mut state, Message::Resumed);

        assert_eq!(
            state.machine().phase(),
            crate::state::Phase::Locked,
            "ADR 0015: the lock state does not reset on resume"
        );
        assert!(state.machine().reports_locked());
        assert_eq!(state.suspend().phase(), SuspendPhase::Idle);
    }

    #[test]
    fn cancelling_clears_the_credential() {
        let mut state = locked();
        type_text(&mut state, "hunter2");
        assert_eq!(state.credential_len(), 7);

        send(&mut state, Message::Cancelled);

        assert_eq!(state.credential_len(), 0);
        assert!(state.model().mask.is_empty());
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn a_suspend_clears_the_credential() {
        let mut state = locked();
        type_text(&mut state, "hunter2");
        assert_eq!(state.credential_len(), 7);

        send(&mut state, Message::SuspendRequested);

        assert_eq!(
            state.credential_len(),
            0,
            "ADR 0015: sensitive input does not survive a suspend"
        );
        assert!(state.model().mask.is_empty());
    }

    #[test]
    fn a_suspend_that_cannot_lock_still_clears_the_credential() {
        let mut state = locked();
        type_text(&mut state, "hunter2");
        send(
            &mut state,
            Message::ProtocolAbsent(ProtocolAbsence::GlobalMissing),
        );

        send(&mut state, Message::SuspendRequested);

        assert_eq!(state.suspend().phase(), SuspendPhase::Refused);
        assert_eq!(
            state.credential_len(),
            0,
            "a refused suspend still takes the session away from the screen, and              the credential is not kept for the resume"
        );
    }

    #[test]
    fn what_is_typed_while_a_suspend_waits_for_the_lock_does_not_ride_into_it() {
        let mut state = locking();
        send(&mut state, Message::SuspendRequested);
        assert_eq!(state.suspend().phase(), SuspendPhase::Locking);

        // The lock is still being confirmed and the field is still typable, so
        // there is a second credential to clear by the time the suspend is
        // allowed: the request alone is not the whole of the suspend.
        type_text(&mut state, "typed while the lock is confirmed");
        assert!(
            state.credential_len() > 0,
            "the window this test is about does not exist: the field is disabled              while the lock is confirmed"
        );
        let _ = state.take_outgoing();

        send(&mut state, Message::OutputCovered(output("eDP-1")));
        send(&mut state, Message::OutputCovered(output("HDMI-A-1")));

        assert!(
            state
                .take_outgoing()
                .iter()
                .any(|command| matches!(command, OutgoingCommand::AllowSuspend(_))),
            "the suspend is allowed once every output is covered"
        );
        assert_eq!(
            state.credential_len(),
            0,
            "ADR 0015: nothing typed survives the session it was typed into              walking away"
        );
    }

    #[test]
    fn the_ui_exiting_clears_the_credential() {
        let mut state = locked();
        type_text(&mut state, "hunter2");

        send(&mut state, Message::UiExited);

        assert_eq!(
            state.credential_len(),
            0,
            "ADR 0015: a hidden UI takes the credential with it, and a UI that              died is hidden whatever it intended"
        );
        assert!(state.model().mask.is_empty());
    }

    #[test]
    fn a_success_clears_whatever_was_typed_after_the_attempt() {
        let mut state = locked();
        type_text(&mut state, "first");
        send(&mut state, Message::Submitted);
        let generation = state.boundary().generation().expect("an attempt");
        type_text(&mut state, "second");

        send(
            &mut state,
            Message::AuthAnswered(generation, AuthOutcome::Success),
        );

        assert_eq!(state.credential_len(), 0);
    }

    #[test]
    fn an_unusable_boundary_can_be_retried_and_then_accepts_an_attempt_again() {
        let mut state = locked();
        send(
            &mut state,
            Message::AuthUnavailable(ServiceFailure::Disconnected),
        );
        assert!(!state.model().accepts_input);
        assert!(matches!(state.model().face, Face::Unavailable { .. }));

        send(&mut state, Message::BoundaryRetried);

        assert_eq!(state.boundary(), &BoundaryState::Idle);
        assert!(state.model().accepts_input, "{:?}", state.model().face);
        assert!(matches!(state.model().face, Face::Prompt));
    }

    #[test]
    fn retrying_a_boundary_that_is_fine_changes_nothing() {
        let mut state = locked();

        send(&mut state, Message::BoundaryRetried);

        assert_eq!(state.boundary(), &BoundaryState::Idle);
        assert!(state.model().accepts_input);
    }

    #[test]
    fn the_clock_only_moves_when_a_tick_says_so() {
        let mut state = locked();
        let started = state.clock;

        send(&mut state, Message::Tick(started + Duration::from_secs(30)));

        assert_eq!(state.clock, started + Duration::from_secs(30));
    }

    #[test]
    fn key_bindings_are_the_whole_keyboard_interface() {
        assert_eq!(
            key_binding(&Key::Named(Named::Enter)),
            Some(Message::Submitted)
        );
        assert_eq!(
            key_binding(&Key::Named(Named::Backspace)),
            Some(Message::Backspace)
        );
        assert_eq!(
            key_binding(&Key::Named(Named::Escape)),
            Some(Message::Cancelled)
        );
        assert_eq!(
            key_binding(&Key::Character("a".into())),
            Some(Message::Character('a'))
        );
        assert_eq!(
            key_binding(&Key::Character("在".into())),
            Some(Message::Character('在')),
            "a password may be any character, not only an ASCII one"
        );
        assert_eq!(
            key_binding(&Key::Character("ab".into())),
            None,
            "a paste is not a keystroke: it is refused, not truncated"
        );
        assert_eq!(key_binding(&Key::Named(Named::Tab)), None);
        assert_eq!(key_binding(&Key::Named(Named::Shift)), None);
    }

    #[test]
    fn the_title_comes_from_the_catalog() {
        let state = state();

        assert_eq!(title(&state), "Inox Session Lock");
    }

    #[test]
    fn the_window_theme_follows_the_tokens() {
        let state = state();

        assert_eq!(theme(&state).name(), state.tokens().name);
    }
}
