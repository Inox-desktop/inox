//! The Elm loop: state, messages, and `update`.
//!
//! ADR 0003 fixes this shape for every component and ADR 0013 repeats it for
//! interactions: a message goes in, the state changes, and anything slow comes
//! back as a task. Nothing here draws, and nothing blocks — the two things a
//! greeter asks of other processes (check this account, start that session) are
//! queued in [`Command`] for a transport this milestone does not have, which is
//! what keeps [`update`] a pure state transition.
//!
//! Three rules the handler exists to keep.
//!
//! - **Only a current reply starts a session.** [`Message::AuthAnswered`] and
//!   [`Message::SessionAnswered`] carry the attempt they are about, and
//!   [`crate::state::Greeter`] refuses anything that is not the attempt in hand —
//!   an answer for an attempt already over, a duplicate about one already
//!   answered, an answer for an attempt that was never submitted, and a success
//!   that arrives after the deadline are all dropped rather than obeyed.
//! - **The focus always lands on something that can be used.** The projection
//!   only lists operable controls, and the update loop keeps its own focus inside
//!   that list, so a control that becomes disabled takes the focus with it rather
//!   than stranding the keyboard (acceptance Q01).
//! - **The clock only moves when a tick says so.** The reply deadline and the
//!   rolling allowance are computed from the instant the state holds, not from
//!   `Instant::now()`, so a test can drive an hour of a login in a microsecond.

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced::keyboard::{Key, Modifiers, key::Named};
use iced::{Element, Subscription, Task};
use inox_common::i18n::Localizer;
use inox_config::Theme as Tokens;
use tracing::{debug, info, warn};

use crate::state::{AttemptId, AuthReply, Command, Event, Greeter};
use crate::view;
use crate::view_model::{self, FocusTarget, GreeterViewModel, Projection};

/// How often the login screen re-reads its clock.
///
/// The only things it is for are the reply deadline and a spent allowance
/// expiring while nobody is typing: every other state change is caused by a
/// message. It is a policy period rather than a design token, because nothing is
/// drawn with it.
const TICK: Duration = Duration::from_secs(1);

/// Everything the login screen can be told.
///
/// The two replies are the trusted boundary's vocabulary. Nothing in the shipped
/// binary produces them — ADR 0015 makes the authentication boundary a human
/// security review item, and this build has none — but the handler's rules about
/// stale, duplicated and out-of-order replies are exactly what has to hold once
/// one exists, so they are here, tested, rather than arriving with the transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// A character was typed into the identity field.
    Identity(String),
    /// The user submitted the identity.
    Submitted,
    /// The user cleared the identity.
    Cancelled,
    /// Move the keyboard focus to the next operable control.
    FocusNext,
    /// Move the keyboard focus to the previous operable control.
    FocusPrevious,
    /// Operate whatever the keyboard focus is on.
    Activate,
    /// The boundary answered the attempt.
    AuthAnswered(AttemptId, AuthReply),
    /// The display manager finished launching the session.
    SessionAnswered(AttemptId, bool),
    /// The clock moved.
    Ticked(Instant),
}

/// The login screen's state.
pub struct State {
    localizer: Rc<Localizer>,
    tokens: Tokens,
    greeter: Greeter,
    clock: Instant,
    /// Where the keyboard focus is. Held separately from the projection, which
    /// reads it and then reports where it actually landed.
    focus: Option<FocusTarget>,
    model: GreeterViewModel,
    outgoing: VecDeque<Command>,
}

impl State {
    /// A login screen with no reviewed authentication transport connected.
    ///
    /// This is the truth about the shipped binary (ADR 0015, ADR 0021), and the
    /// first model is projected from the same state as every later one, so there
    /// is no frame in which the screen shows something the projection would not
    /// have produced.
    pub fn new(localizer: Rc<Localizer>, tokens: Tokens) -> Self {
        Self::with_boundary(localizer, tokens, false)
    }

    /// A login screen with an authenticator, for the states the shipped binary
    /// cannot reach.
    ///
    /// `available` says whether a reviewed transport is connected. Nothing in
    /// the binary passes `true`: the model is exercised in that configuration by
    /// tests, so that the interface for it exists and is verified before the
    /// transport it describes does.
    pub fn with_boundary(localizer: Rc<Localizer>, tokens: Tokens, available: bool) -> Self {
        let greeter = Greeter::new(available);
        let clock = Instant::now();
        let focus = None;

        let model = project(&greeter, focus, clock, &localizer);

        Self {
            localizer,
            tokens,
            greeter,
            clock,
            focus: model.focus,
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

    /// The login lifecycle model.
    pub const fn greeter(&self) -> &Greeter {
        &self.greeter
    }

    /// What the view draws.
    pub const fn model(&self) -> &GreeterViewModel {
        &self.model
    }

    /// Where the keyboard focus is.
    pub const fn focus(&self) -> Option<FocusTarget> {
        self.focus
    }

    /// Takes what the login screen has asked for since this was last called.
    ///
    /// The only interface a transport has to implement: whatever comes out of
    /// here is a request for a process this component does not own. **Nothing
    /// drains it in the shipped binary yet** — there is no transport to drain it
    /// into — and nothing accumulates there either: with no authenticator the
    /// model refuses every submission (`Greeter::can_submit` requires
    /// `available`), so `Authenticate` is never queued, and `Cancel` is only
    /// queued for an attempt that was. A caller that wires a real transport must
    /// drain this on every turn of the loop, the way `inox-lock`'s protocol leg
    /// drains its own.
    pub fn take_outgoing(&mut self) -> Vec<Command> {
        self.outgoing.drain(..).collect()
    }

    /// The window title.
    pub fn title(&self) -> String {
        self.model.title.clone()
    }

    /// The window theme, from the theme tokens.
    pub fn theme(&self) -> iced::Theme {
        crate::theme::from_tokens(&self.tokens)
    }

    /// A task that puts the keyboard focus where the projection says it is.
    ///
    /// iced keeps its own idea of which widget is focused, and this is how the
    /// projection's answer gets there. It only ever has one thing to do: the
    /// identity field is the only focusable widget this crate builds (`iced` 0.14
    /// has no focusable buttons, and its text box is drawn only while the ring is
    /// on it), so every other position is a screen with no iced focus at all —
    /// which is exactly what the visible ring says.
    pub fn focus_task(&self) -> Task<Message> {
        match self.focus {
            Some(FocusTarget::Identity) => iced::widget::operation::focus(view::IDENTITY_ID),
            Some(FocusTarget::Submit) | Some(FocusTarget::Cancel) | None => Task::none(),
        }
    }

    /// Recomputes what the view draws, and moves the focus with it.
    fn refresh_model(&mut self) {
        self.model = project(&self.greeter, self.focus, self.clock, &self.localizer);
        // The projection already moved a focus that is no longer reachable; what
        // this does is adopt that answer, so the two never disagree.
        self.focus = self.model.focus;
    }

    /// Queues everything a command asks the outside world for.
    fn queue(&mut self, command: Command, why: &'static str) {
        match command {
            Command::None => {}
            Command::Cancel(id) => {
                debug!(%id, why, "the attempt is being abandoned");
                self.outgoing.push_back(Command::Cancel(id));
            }
            other => {
                debug!(why, "the login screen asked a transport for something");
                self.outgoing.push_back(other);
            }
        }
    }
}

/// Projects the state into what the view draws.
fn project(
    greeter: &Greeter,
    focus: Option<FocusTarget>,
    clock: Instant,
    localizer: &Localizer,
) -> GreeterViewModel {
    view_model::project(Projection {
        phase: greeter.phase(),
        available: greeter.available(),
        username: greeter.username(),
        identity_too_long: greeter.identity_too_long(),
        can_submit: greeter.can_submit(clock),
        focus,
        localizer,
    })
}

/// The keys the login screen handles, and the clock.
///
/// Enter operates whatever is focused and Escape clears the identity; **Space is
/// deliberately not a binding**, because it is a character someone may want to
/// type into their own name and a keyboard shortcut that eats it would be a
/// shortcut against the field. Key repeat is ignored for the same reason: holding
/// a key down is a person leaning on it, not a second request.
pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::batch([
        iced::keyboard::listen().filter_map(|event| match event {
            iced::keyboard::Event::KeyPressed {
                key,
                modifiers,
                repeat: false,
                ..
            } => key_binding(&key, modifiers),
            _ => None,
        }),
        Subscription::run(tick),
    ])
}

/// The message a key press maps to.
///
/// Pure, and tested on its own: the subscription above is a two-line adapter, so
/// "Tab moves, Shift+Tab moves back, Enter operates, Escape clears" is a property
/// of this function rather than of a running window (acceptance Q01).
pub fn key_binding(key: &Key, modifiers: Modifiers) -> Option<Message> {
    match key {
        Key::Named(Named::Tab) if modifiers.shift() => Some(Message::FocusPrevious),
        Key::Named(Named::Tab) => Some(Message::FocusNext),
        Key::Named(Named::Enter) => Some(Message::Activate),
        Key::Named(Named::Escape) => Some(Message::Cancelled),
        _ => None,
    }
}

/// A stream that produces one [`Message::Ticked`] a second, for as long as the
/// subscription is alive.
///
/// The thread ends when the runtime drops the receiver, which is what stops the
/// timer when the window closes.
fn tick() -> iced::futures::channel::mpsc::Receiver<Message> {
    let (mut sender, receiver) = iced::futures::channel::mpsc::channel(1);

    std::thread::spawn(move || {
        loop {
            match sender.try_send(Message::Ticked(Instant::now())) {
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

/// The login screen's widget tree, from the projection.
pub fn view(state: &State) -> Element<'_, Message> {
    crate::view::view(state)
}

/// Applies one message.
pub fn update(state: &mut State, message: Message) -> Task<Message> {
    let focus_before = state.focus;
    let task = handle(state, message);
    state.refresh_model();

    // iced is told where the focus went only when it went somewhere. Re-issuing a
    // focus request on every message is not harmless: focusing a text input moves
    // its caret, so the message that says "a character arrived" would also say
    // "start again from the end", and somebody editing the middle of a name would
    // watch the caret jump on every keystroke.
    if state.focus == focus_before {
        task
    } else {
        Task::batch([task, state.focus_task()])
    }
}

/// The message handler, without the projection step.
fn handle(state: &mut State, message: Message) -> Task<Message> {
    let now = state.clock;

    match message {
        Message::Identity(value) => {
            let command = state.greeter.apply(Event::Username(value), now);
            state.queue(command, "the identity changed");
            Task::none()
        }
        Message::Submitted => {
            let command = state.greeter.apply(Event::Submit, now);
            match &command {
                Command::Authenticate { id, .. } => info!(
                    %id,
                    "an attempt was handed to the authentication boundary"
                ),
                _ => debug!("the submission was refused by the model"),
            }
            state.queue(command, "the identity was submitted");
            Task::none()
        }
        Message::Cancelled => {
            let command = state.greeter.apply(Event::Cancel, now);
            state.queue(command, "the identity was cleared");

            // Clearing puts the focus back where the typing starts. `None` rather
            // than the identity field directly: the projection knows which
            // controls survived the change, and this does not.
            state.focus = None;
            Task::none()
        }
        Message::FocusNext => {
            move_focus(state, true);
            Task::none()
        }
        Message::FocusPrevious => {
            move_focus(state, false);
            Task::none()
        }
        Message::Activate => {
            // The focused control is the one that is operated. There is no
            // "submit" key that bypasses the focus, so "the keyboard reached the
            // control" and "the keyboard operated it" are one fact.
            match state.model.focus {
                Some(FocusTarget::Cancel) => handle(state, Message::Cancelled),
                Some(FocusTarget::Identity | FocusTarget::Submit) => {
                    handle(state, Message::Submitted)
                }
                None => {
                    debug!("nothing is focused, so nothing was activated");
                    Task::none()
                }
            }
        }
        Message::AuthAnswered(id, reply) => {
            let before = state.greeter.phase();
            let command = state.greeter.apply(Event::AuthAnswered(id, reply), now);

            if state.greeter.phase() == before {
                // The model refused the answer: it was about an attempt that is
                // not the one in hand, or one that never happened at all. ADR
                // 0013 decision 2, and the reason a late success cannot start a
                // session.
                warn!(%id, ?reply, "an answer for an attempt that is not in flight");
            }

            state.queue(command, "the boundary answered");
            Task::none()
        }
        Message::SessionAnswered(id, success) => {
            let before = state.greeter.phase();
            let command = state
                .greeter
                .apply(Event::SessionAnswered(id, success), now);

            if state.greeter.phase() == before {
                warn!(%id, success, "a session reply for an attempt that is not in flight");
            }

            state.queue(command, "the display manager answered");
            Task::none()
        }
        Message::Ticked(now) => {
            state.clock = now;
            // The deadline and the rolling allowance are the model's, and this is
            // the only thing that advances them.
            let command = state.greeter.apply(Event::Tick, now);
            state.queue(command, "the reply deadline passed");
            Task::none()
        }
    }
}

/// Moves the keyboard focus within the operable controls, wrapping at both ends.
///
/// It never lands on a control the projection has left out, which is what makes
/// "the focus is visible and unique, and cannot be trapped" a property of the
/// model rather than of this function's arithmetic.
fn move_focus(state: &mut State, forward: bool) {
    let order = &state.model.focus_order;

    if order.is_empty() {
        state.focus = None;
        return;
    }

    let current = state
        .focus
        .and_then(|focused| order.iter().position(|target| *target == focused));

    let next = match (current, forward) {
        (Some(index), true) => (index + 1) % order.len(),
        (Some(index), false) => (index + order.len() - 1) % order.len(),
        (None, true) => 0,
        (None, false) => order.len() - 1,
    };

    state.focus = order.get(next).copied();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Phase;
    use inox_common::i18n::Locale;

    fn login() -> State {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");

        State::with_boundary(Rc::new(localizer), Tokens::builtin_dark(), true)
    }

    fn send(state: &mut State, message: Message) {
        let _ = update(state, message);
    }

    /// Submits `alice` and returns the attempt the model asked the transport for.
    fn submit(state: &mut State) -> AttemptId {
        send(state, Message::Identity("alice".into()));
        send(state, Message::Submitted);

        state
            .take_outgoing()
            .into_iter()
            .find_map(|command| match command {
                Command::Authenticate { id, .. } => Some(id),
                _ => None,
            })
            .expect("an authentication request")
    }

    #[test]
    fn a_fresh_screen_with_no_boundary_cannot_log_anyone_in() {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let mut state = State::new(Rc::new(localizer), Tokens::builtin_dark());

        assert_eq!(state.greeter().phase(), Phase::Unavailable);
        assert!(!state.model().identity.editable);
        assert!(!state.model().submit.enabled);
        assert!(
            matches!(state.model().face, view_model::Face::Unavailable { .. }),
            "the first thing the screen says is that it cannot log anyone in"
        );
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn the_focus_starts_on_the_first_operable_control_and_wraps() {
        let mut state = login();
        send(&mut state, Message::Identity("alice".into()));

        assert_eq!(state.focus(), Some(FocusTarget::Identity));

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Submit));

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Cancel));

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Identity), "it wrapped");

        send(&mut state, Message::FocusPrevious);
        assert_eq!(state.focus(), Some(FocusTarget::Cancel), "and back again");
    }

    #[test]
    fn operate_acts_on_the_focused_control_and_not_on_enter_alone() {
        let mut state = login();
        send(&mut state, Message::Identity("alice".into()));

        // The focus is on the identity field, so Enter submits the form.
        send(&mut state, Message::Activate);
        assert_eq!(state.greeter().phase(), Phase::Authenticating);
        assert_eq!(state.take_outgoing().len(), 1);

        // Submitting took the field and the button away, so the focus moved to
        // the one operable control that is left — and Enter now does what the
        // ring says it does, which is clear rather than submit.
        assert_eq!(state.focus(), Some(FocusTarget::Cancel));

        send(&mut state, Message::Activate);
        assert_eq!(state.greeter().phase(), Phase::Ready);
        assert_eq!(state.greeter().username(), "");
    }

    #[test]
    fn activating_the_clear_control_clears_and_asks_the_transport_to_cancel() {
        let mut state = login();
        let id = submit(&mut state);

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Cancel));
        send(&mut state, Message::Activate);

        assert_eq!(state.greeter().username(), "");
        assert_eq!(state.greeter().phase(), Phase::Ready);
        assert_eq!(state.take_outgoing(), vec![Command::Cancel(id)]);
    }

    #[test]
    fn the_focus_leaves_a_control_that_can_no_longer_be_used() {
        let mut state = login();
        send(&mut state, Message::Identity("alice".into()));
        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Submit));

        // Submitting takes the field and the button away; the focus must not stay
        // on a control that no longer exists.
        send(&mut state, Message::Activate);
        let _ = state.take_outgoing();

        assert_eq!(state.greeter().phase(), Phase::Authenticating);
        assert_eq!(state.focus(), Some(FocusTarget::Cancel));
    }

    #[test]
    fn a_current_success_starts_a_session_once() {
        let mut state = login();
        let id = submit(&mut state);

        send(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));
        assert_eq!(state.greeter().phase(), Phase::Starting);
        assert_eq!(state.take_outgoing(), vec![Command::StartSession(id)]);

        send(&mut state, Message::SessionAnswered(id, true));
        assert_eq!(state.greeter().phase(), Phase::SessionStarted);
        assert!(state.take_outgoing().is_empty());

        // Nothing is operable once the session is starting, and the keyboard
        // cannot ask twice.
        assert!(!state.model().cancel.enabled);
        assert_eq!(state.model().focus_order, Vec::new());
    }

    #[test]
    fn stale_duplicate_and_stranger_replies_never_start_a_session() {
        let mut state = login();
        let first = submit(&mut state);
        send(&mut state, Message::Cancelled);
        let _ = state.take_outgoing();

        let current = submit(&mut state);
        let _ = state.take_outgoing();

        // An answer about the attempt that was abandoned.
        send(
            &mut state,
            Message::AuthAnswered(first, AuthReply::Accepted),
        );
        assert_eq!(state.greeter().phase(), Phase::Authenticating);
        assert!(state.take_outgoing().is_empty());

        // A success for an attempt nobody submitted at all.
        let mut stranger = login();
        send(
            &mut stranger,
            Message::AuthAnswered(AttemptId::that_was_never_issued(9), AuthReply::Accepted),
        );
        assert_ne!(stranger.greeter().phase(), Phase::Starting);

        // The right answer, once.
        send(
            &mut state,
            Message::AuthAnswered(current, AuthReply::Accepted),
        );
        assert_eq!(state.greeter().phase(), Phase::Starting);
        assert_eq!(state.take_outgoing(), vec![Command::StartSession(current)]);

        // And a duplicate of it changes nothing.
        send(
            &mut state,
            Message::AuthAnswered(current, AuthReply::Accepted),
        );
        assert_eq!(state.greeter().phase(), Phase::Starting);
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn a_reply_that_arrives_after_the_deadline_is_refused() {
        let mut state = login();
        let id = submit(&mut state);
        let _ = state.take_outgoing();

        let expired = state.clock + Duration::from_secs(30);
        send(&mut state, Message::Ticked(expired));
        assert_eq!(state.greeter().phase(), Phase::Failed);
        assert_eq!(state.take_outgoing(), vec![Command::Cancel(id)]);

        send(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));
        assert_ne!(
            state.greeter().phase(),
            Phase::Starting,
            "an expired attempt must not start a session"
        );
    }

    #[test]
    fn a_refused_attempt_leaves_the_screen_usable_and_the_allowance_finite() {
        let mut state = login();

        for _ in 0..3 {
            let id = submit(&mut state);
            let _ = state.take_outgoing();
            send(&mut state, Message::AuthAnswered(id, AuthReply::Rejected));
            assert_eq!(state.greeter().phase(), Phase::Failed);
        }

        send(&mut state, Message::Identity("alice".into()));
        assert!(!state.model().submit.enabled, "the allowance is spent");
        assert_eq!(
            state.model().submit.disabled_reason.as_deref(),
            Some("Too many attempts for now")
        );

        // The window passes and the screen becomes usable again: a limit that
        // never lifts locks the person out of their own machine.
        let window = state.clock + Duration::from_secs(60);
        send(&mut state, Message::Ticked(window));
        assert!(state.model().submit.enabled);
    }

    #[test]
    fn the_clock_only_moves_when_a_tick_says_so() {
        let mut state = login();
        let before = state.clock;
        send(&mut state, Message::Identity("alice".into()));
        send(&mut state, Message::Submitted);
        let _ = state.take_outgoing();

        assert_eq!(state.clock, before);

        let later = before + Duration::from_secs(5);
        send(&mut state, Message::Ticked(later));
        assert_eq!(state.clock, later);
    }

    #[test]
    fn the_keyboard_bindings_are_the_whole_keyboard_interface() {
        let plain = Modifiers::default();
        let shift = Modifiers::SHIFT;

        assert_eq!(
            key_binding(&Key::Named(Named::Tab), plain),
            Some(Message::FocusNext)
        );
        assert_eq!(
            key_binding(&Key::Named(Named::Tab), shift),
            Some(Message::FocusPrevious)
        );
        assert_eq!(
            key_binding(&Key::Named(Named::Enter), plain),
            Some(Message::Activate)
        );
        assert_eq!(
            key_binding(&Key::Named(Named::Escape), plain),
            Some(Message::Cancelled)
        );
        assert_eq!(
            key_binding(&Key::Named(Named::Space), plain),
            None,
            "a shortcut on Space would eat a character someone may type into \
             their own name"
        );
        assert_eq!(
            key_binding(&Key::Character("a".into()), plain),
            None,
            "ordinary characters belong to the field, not to a shortcut"
        );
    }

    #[test]
    fn every_queued_command_says_what_it_is_without_saying_a_credential() {
        let mut state = login();
        state
            .greeter
            .apply(Event::Username("alice".into()), state.clock);
        send(&mut state, Message::Submitted);

        let queued = format!("{:?}", state.take_outgoing());

        assert!(queued.contains("Authenticate"), "{queued}");
        assert!(queued.contains("alice"), "{queued}");
        assert!(
            !queued.contains("password") && !queued.contains("secret"),
            "a login screen that queues a credential is a login screen that \
             collected one: {queued}"
        );
    }
}
