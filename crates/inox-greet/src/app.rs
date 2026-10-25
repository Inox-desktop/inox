//! The Elm loop: state, messages, and `update`.
//! the technology stack fixes this shape for every component and the interaction contract repeats it for
//! the technology stack fixes this shape for every component and the interaction contract repeats it for
//! interactions: a message goes in, the state changes, and anything slow leaves as
//! a command. Nothing here draws and nothing here blocks: the two things a greeter
//! asks of other processes — check this account, start that session — come out of
//! the model as [`Command`]s, and [`update`] hands them back to the caller, which
//! is the QML bridge (`crate::cxxqt_object`) and, behind it, the transport in
//! [`crate::runtime`]. The state transition itself is still pure, which is what lets
//! every rule below be a unit test rather than a race.
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
//!
//! Since the interface is QML the keyboard arrives as
//! [`KeyPress`] — a key name or one typed character, reported by the QML tree —
//! not as a widget toolkit event. What it maps to is still this module's decision,
//! and it is still a pure function with a test.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use inox_common::i18n::Localizer;
use inox_config::Theme as Tokens;
use tracing::{debug, info, warn};

use crate::accounts::Accounts;
use crate::boundary::Report;
use crate::state::{AttemptId, AuthReply, Command, Event, Greeter, Phase};
use crate::view_model::{self, FocusTarget, GreeterViewModel, Projection};

/// Everything the login screen can be told.
///
/// The `AuthAnswered`/`SessionAnswered` variants are the trusted boundary's
/// vocabulary *inside* this crate: the transport turns a reply it received into one
/// of them, and the model's rules about stale, duplicated and out-of-order replies
/// are applied to that. Nothing here can construct a success by hand — the reports
/// they are built from ([`Report`]) only ever come from [`crate::boundary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// A different account was chosen from the account list.
    AccountChosen(String),
    /// The person asked to see the accounts to log in as.
    SwitchUser,
    /// A character was typed while the credential field had the focus.
    Credential(char),
    /// Backspace, while the credential field had the focus.
    Backspace,
    /// The user submitted the identity.
    Submitted,
    /// The user cleared the credential — or, with the account list open, closed it.
    Cancelled,
    /// Move the keyboard focus to the next operable control.
    FocusNext,
    /// Move the keyboard focus to the previous operable control.
    FocusPrevious,
    /// Operate whatever the keyboard focus is on.
    Activate,
    /// The boundary answered the attempt.
    /// itself — the boundary hears about it.
    /// The transport answers with [`Message::TransportAnswered`] and this is what
    /// that becomes once the wire's identifier is the model's again; the tests drive
    /// it directly, which is why a release build has no constructor for it.
    #[cfg_attr(not(test), allow(dead_code))]
    AuthAnswered(AttemptId, AuthReply),
    /// The display manager finished launching the session.
    #[cfg_attr(not(test), allow(dead_code))]
    SessionAnswered(AttemptId, bool),
    /// The transport finished one request.
    TransportAnswered {
        /// The attempt the request was about, as the wire carries it.
        attempt: u64,
        /// What the boundary did.
        report: Report,
    },
    /// The transport finished a request that has no answer.
    TransportFinished,
    /// The clock moved.
    Ticked(Instant),
}

/// The login screen's state.
pub struct State {
    localizer: Rc<Localizer>,
    tokens: Tokens,
    greeter: Greeter,
    /// The accounts the system offers, and where the one a session was started
    /// for is recorded.
    roster: Accounts,
    /// Whether the account list is what the person asked to see. Held separately
    /// from the projection, which reads it and then reports whether the list is
    /// actually on screen.
    choosing: bool,
    /// Whether the account a session was started for has already been recorded for
    /// this session, so that it is recorded once rather than once per message — and
    /// cleared when the screen returns to the prompt, so the next session is
    /// recorded too.
    recorded: bool,
    clock: Instant,
    /// Where the keyboard focus is. Held separately from the projection, which
    /// reads it and then reports where it actually landed.
    focus: Option<FocusTarget>,
    model: GreeterViewModel,
    outgoing: VecDeque<Command>,
    /// Where the boundary is, when this build has one.
    ///
    /// `None` is the `--preview` path: the screen is drawn in the state a build
    /// with no boundary is in, and every command the model would queue is dropped
    /// rather than attempted.
    socket: Option<PathBuf>,
}

impl State {
    /// A login screen with no reviewed authentication transport connected.
    ///
    /// This is the `--preview` path, and the first model is
    /// projected from the same state as every later one, so there is no frame in
    /// which the screen shows something the projection would not have produced.
    pub fn new(localizer: Rc<Localizer>, tokens: Tokens, roster: Accounts) -> Self {
        Self::with_boundary(localizer, tokens, false, roster)
    }

    /// A login screen that talks to `socket`, which the caller has already found.
    ///
    /// The caller checks the socket *before* this exists (`boundary::is_available`),
    /// so a screen drawn as able to log somebody in is a screen with somewhere to
    /// send them.
    pub fn connected(
        localizer: Rc<Localizer>,
        tokens: Tokens,
        socket: PathBuf,
        roster: Accounts,
    ) -> Self {
        let mut state = Self::with_boundary(localizer, tokens, true, roster);
        state.socket = Some(socket);

        state
    }

    /// A login screen with an authenticator, for the states the shipped binary
    /// cannot reach.
    ///
    /// `available` says whether a reviewed transport is connected. Nothing in
    /// the binary passes `true`: the model is exercised in that configuration by
    /// tests, so that the interface for it exists and is verified before the
    /// transport it describes does.
    pub fn with_boundary(
        localizer: Rc<Localizer>,
        tokens: Tokens,
        available: bool,
        roster: Accounts,
    ) -> Self {
        let greeter = Greeter::new(available, &roster);
        let clock = Instant::now();
        let focus = None;
        let choosing = false;

        let model = project(&greeter, &roster, choosing, focus, clock, &localizer);

        Self {
            localizer,
            tokens,
            greeter,
            roster,
            choosing,
            recorded: false,
            clock,
            focus: model.focus,
            model,
            outgoing: VecDeque::new(),
            socket: None,
        }
    }

    /// The design tokens.
    pub const fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    /// The login lifecycle model.
    ///
    /// Kept for the tests that assert what the model did with a message; a release
    /// build has no caller, and the allowance says so rather than hiding it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn greeter(&self) -> &Greeter {
        &self.greeter
    }

    /// What the view draws.
    pub const fn model(&self) -> &GreeterViewModel {
        &self.model
    }

    /// Where the keyboard focus is.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn focus(&self) -> Option<FocusTarget> {
        self.focus
    }

    /// The instant the model's deadlines are measured from.
    ///
    /// Only the tests read it; the runtime advances it with a tick.
    ///
    /// It only moves when a [`Message::Ticked`] says so, which is what lets a test
    /// (and a runtime with a slow timer) drive an hour of a login in a microsecond.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn clock(&self) -> Instant {
        self.clock
    }

    /// Takes what the login screen has asked for since this was last called.
    ///
    /// The interface a transport implements, and in the shipped binary the only
    /// caller is [`update`], which hands the commands to the QML bridge. Nothing
    /// accumulates in the queue: every turn of the update loop drains it.
    pub fn take_outgoing(&mut self) -> Vec<Command> {
        self.outgoing.drain(..).collect()
    }

    /// The window title.
    pub fn title(&self) -> String {
        self.model.title.clone()
    }

    /// Where the boundary is, when this build has one.
    ///
    /// `None` is the `--preview` path: the screen is drawn in the state a build
    /// with no boundary is in, and every command the model would queue is dropped
    /// rather than attempted.
    pub fn socket(&self) -> Option<&PathBuf> {
        self.socket.as_ref()
    }

    /// Recomputes what the view draws, and moves the focus and the account list
    /// with it.
    fn refresh_model(&mut self) {
        self.model = project(
            &self.greeter,
            &self.roster,
            self.choosing,
            self.focus,
            self.clock,
            &self.localizer,
        );
        // The projection already moved a focus that is no longer reachable, and
        // already closed an account list that is no longer a choice; what this does
        // is adopt both answers, so the two never disagree.
        self.focus = self.model.focus;
        self.choosing = self.model.choosing;
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
    roster: &Accounts,
    choosing: bool,
    focus: Option<FocusTarget>,
    clock: Instant,
    localizer: &Localizer,
) -> GreeterViewModel {
    view_model::project(Projection {
        phase: greeter.phase(),
        available: greeter.available(),
        accounts: greeter.accounts(),
        unreadable: roster.unreadable,
        selected: greeter.selected().map(crate::accounts::Account::name),
        identity_too_long: greeter.identity_too_long(),
        can_submit: greeter.can_submit(clock),
        credential_chars: greeter.credential_len(),
        credential_too_long: greeter.credential_too_long(),
        focus,
        choosing,
        localizer,
    })
}

/// A key press the QML tree reported.
///
/// The tree has no text field to hand a password to: it forwards what
/// the keyboard produced and Rust decides what that means. This is the whole
/// vocabulary it may forward — five keys with a meaning of their own, and one
/// character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPress {
    /// Tab: move the focus to the next operable control.
    Tab,
    /// Shift+Tab: move the focus to the previous one.
    BackTab,
    /// Enter or Return: operate what the focus is on.
    Enter,
    /// Escape: clear the credential, or close the account list.
    Escape,
    /// Backspace: remove the last character of the credential.
    Backspace,
    /// One character someone typed.
    Character(char),
}

/// The message a key press maps to.
///
/// Pure, and tested on its own: the bridge is a two-line adapter, so "Tab moves,
/// Shift+Tab moves back, Enter operates, Escape clears, printable characters
/// belong to the field" is a property of this function rather than of a running
/// window (acceptance Q01).
///
/// **Space is a character here, not an operation.** A password may contain one,
/// and the credential field is a mask rather than a text box, so the
/// unprintable keys are the only shortcuts and everything a person can type is
/// data.
///
/// Character messages are produced whatever has the focus and *applied* only while
/// the ring is on the credential field: no other control on this screen takes text,
/// so a character that arrives anywhere else was not typed into anything.
pub fn key_binding(press: KeyPress) -> Option<Message> {
    match press {
        KeyPress::Tab => Some(Message::FocusNext),
        KeyPress::BackTab => Some(Message::FocusPrevious),
        KeyPress::Enter => Some(Message::Activate),
        KeyPress::Escape => Some(Message::Cancelled),
        KeyPress::Backspace => Some(Message::Backspace),
        // A control character is not text.
        KeyPress::Character(character) if character.is_control() => None,
        KeyPress::Character(character) => Some(Message::Credential(character)),
    }
}

/// Applies one message, and hands back what the boundary should be asked for.
///
/// The commands are returned rather than sent: this module stays free of the
/// transport and of threads, which is what lets every rule it holds be a unit test.
/// [`crate::runtime::Host`] is the caller that queues them, and
/// [`crate::cxxqt_object`] is what calls *it*.
///
/// There is deliberately no "the session started, so this window is finished"
/// branch: the session runs on a VT this process cannot see, and
/// the screen that has to be in front of the person when that session ends is this
/// one — closing the window would mean the host starting a fresh client to replace
/// it, with nobody able to say when. The started face is a dwell in the model
/// instead (§决策 4), which is why this loop is still just messages in and a
/// projection out.
pub fn update(state: &mut State, message: Message) -> Vec<Command> {
    handle(state, message);
    state.refresh_model();

    state.take_outgoing()
}

/// The message handler, without the projection step.
fn handle(state: &mut State, message: Message) {
    dispatch(state, message);

    // Whatever the message was, it may have been the one that started a session.
    remember_started(state);
}

/// Records the account a session was started for: once for that session, and only
/// once the session really started.
///
/// This is the one fact about the past the screen keeps: the next login offers the
/// account this one used first (see [`crate::accounts`]). It is recorded here rather
/// than when the boundary accepts an attempt, because a login that was accepted but
/// whose session never started is not a login this screen should preselect.
///
/// It is recorded for the *session*, not for the process. The screen now stays up
/// after a session starts and becomes a login screen again when the dwell is over
///, so the same process can start a second session — and that one
/// is recorded when it starts, not when the first one did. Returning to the prompt
/// is not a login and records nothing, which is what the flag is cleared for.
fn remember_started(state: &mut State) {
    if state.greeter.phase() != Phase::SessionStarted {
        // Not on the started face: there is nothing to record, and the next session
        // will be a new one rather than this one continuing.
        state.recorded = false;
        return;
    }

    if state.recorded {
        return;
    }

    let Some(account) = state.greeter.selected() else {
        return;
    };

    state.roster.record_started(account.name());
    state.recorded = true;
}

/// The message handler itself.
fn dispatch(state: &mut State, message: Message) {
    let now = state.clock;

    match message {
        Message::AccountChosen(name) => {
            // A name the screen is not showing is not an account this screen can
            // choose: the rows are drawn from the projection's own list, so a name
            // from anywhere else is dropped rather than acted on.
            if !state
                .model
                .accounts
                .iter()
                .any(|account| account.name == name)
            {
                debug!(%name, "an account the screen did not offer");
                return;
            }

            let command = state.greeter.apply(Event::SelectAccount(name), now);
            state.queue(command, "another account was chosen");
            // The list has answered the question it was opened for.
            state.choosing = false;
        }
        Message::SwitchUser => {
            // "Is there anything to switch to" is the projection's answer, not this
            // function's: a screen with one account draws no control that sends this
            // message, and a screen that is checking a login draws none either.
            if state.model.switch_user.is_none() {
                debug!("the screen offers no other account to log in as");
                return;
            }

            state.choosing = true;
            // The rows are what the keyboard can reach now, and which row it lands
            // on is the projection's decision rather than this one's.
            state.focus = None;
        }
        Message::Credential(character) => {
            // "The ring is on the credential field" is the condition, and the ring is
            // the projection's answer rather than the toolkit's: it is the same fact
            // the person sees.
            if state.model.focus != Some(FocusTarget::Credential) {
                debug!("a character arrived while the credential field was not focused");
                return;
            }

            let command = state.greeter.apply(Event::Credential(character), now);
            state.queue(command, "the credential changed");
        }
        Message::Backspace => {
            if state.model.focus != Some(FocusTarget::Credential) {
                return;
            }

            let command = state.greeter.apply(Event::Backspace, now);
            state.queue(command, "the credential changed");
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
        }
        Message::Cancelled if state.model.choosing => {
            // With the account list open, the key that leaves what is open leaves
            // the list: nothing is cleared, because nothing has been typed.
            state.choosing = false;
            state.focus = None;
        }
        Message::Cancelled => {
            let command = state.greeter.apply(Event::Cancel, now);
            state.queue(command, "the credential was cleared");

            // Clearing puts the focus back where the typing starts. `None` rather
            // than the credential field directly: the projection knows which
            // controls survived the change, and this does not.
            state.focus = None;
        }
        Message::FocusNext => {
            move_focus(state, true);
        }
        Message::FocusPrevious => {
            move_focus(state, false);
        }
        Message::Activate => {
            // The focused control is the one that is operated. There is no
            // "submit" key that bypasses the focus, so "the keyboard reached the
            // control" and "the keyboard operated it" are one fact.
            match state.model.focus {
                Some(FocusTarget::Cancel) => handle(state, Message::Cancelled),
                // Enter on a row chooses that account. The name comes from the
                // projection's own list, so a row can only ever choose an account
                // the model is holding.
                Some(FocusTarget::Account(index)) => {
                    let Some(name) = state
                        .model
                        .accounts
                        .get(usize::from(index))
                        .map(|account| account.name.clone())
                    else {
                        debug!(index, "the account row is not one the screen drew");
                        return;
                    };

                    handle(state, Message::AccountChosen(name))
                }
                Some(FocusTarget::SwitchUser) => handle(state, Message::SwitchUser),
                Some(FocusTarget::Credential | FocusTarget::Submit) => {
                    handle(state, Message::Submitted)
                }
                None => {
                    debug!("nothing is focused, so nothing was activated");
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
        }
        Message::TransportAnswered { attempt, report } => {
            // The wire's identifier becomes the model's again here, and nowhere else:
            // it is only ever compared with the attempt the model has in flight, so a
            // report about anything else changes nothing.
            let id = AttemptId::from_reply(attempt);
            let event = match report {
                Report::Accepted => Event::AuthAnswered(id, AuthReply::Accepted),
                Report::Rejected => Event::AuthAnswered(id, AuthReply::Rejected),
                // A boundary that says it cannot answer, and a boundary that is not
                // there at all, are the same thing to the person at the keyboard:
                // this build cannot check a login right now.
                Report::Unavailable | Report::Unreachable => {
                    Event::AuthAnswered(id, AuthReply::Unavailable)
                }
                Report::SessionStarted(started) => Event::SessionAnswered(id, started),
            };

            let before = state.greeter.phase();
            let command = state.greeter.apply(event, now);

            if state.greeter.phase() == before {
                warn!(
                    attempt,
                    ?report,
                    "an answer for an attempt that is not in flight"
                );
            }

            state.queue(command, "the boundary answered");
        }
        Message::TransportFinished => {}
        Message::Ticked(now) => {
            let before = state.greeter.phase();
            state.clock = now;
            // The deadline, the rolling allowance and the started face's dwell are
            // the model's, and this is the only thing that advances them. The dwell
            // is why a message that started a session no longer ends this window: a
            // tick is what ends the started face instead.
            let command = state.greeter.apply(Event::Tick, now);

            if before == Phase::SessionStarted && state.greeter.phase() != Phase::SessionStarted {
                info!("the session has started; the login screen is ready for the next one");
            }

            state.queue(command, "the reply deadline passed");
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
    use crate::accounts::Account;
    use crate::state::SESSION_STARTED_DWELL;
    use inox_common::i18n::Locale;
    use std::time::Duration;

    /// The accounts a test offers: two ordinary accounts, in uid order.
    fn roster() -> Accounts {
        Accounts {
            accounts: vec![
                Account::new("alice", 1000, Some("Alice Doe".to_owned())),
                Account::new("bob", 1001, None),
            ],
            last_used: None,
            record: None,
            unreadable: false,
        }
    }

    fn login() -> State {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");

        State::with_boundary(Rc::new(localizer), Tokens::builtin_dark(), true, roster())
    }

    /// One message through the handler, without a transport.
    ///
    /// The tests drive the state machine rather than the queue: what they are about
    /// is which event a message turns into, and `update` hands the queue to a
    /// boundary that these tests do not have. The end-to-end path over a real socket
    /// is `crate::headless_tests::boundary`.
    fn send(state: &mut State, message: Message) {
        handle(state, message);
        state.refresh_model();
    }

    /// One message, and what it asked the transport for.
    fn send_and_take(state: &mut State, message: Message) -> Vec<Command> {
        send(state, message);
        state.take_outgoing()
    }

    /// Types `text` into the credential field, which must have the focus.
    fn type_credential(state: &mut State, text: &str) {
        for character in text.chars() {
            assert_eq!(
                state.model().focus,
                Some(FocusTarget::Credential),
                "the credential field has to hold the ring to receive a character"
            );
            send(state, Message::Credential(character));
        }
    }

    /// Puts the ring on the credential field, wherever it is now.
    ///
    /// By moving the focus rather than by assigning it: "the credential field is
    /// reachable" is one of the things these tests are here to check.
    fn focus_credential(state: &mut State) {
        for _ in 0..state.model().focus_order.len() + 1 {
            if state.model().focus == Some(FocusTarget::Credential) {
                return;
            }
            send(state, Message::FocusNext);
        }

        panic!("the credential field is not reachable");
    }

    /// Submits `credential` for the account on screen and returns the attempt the
    /// model asked for.
    fn submit(state: &mut State, credential: &str) -> AttemptId {
        focus_credential(state);
        type_credential(state, credential);

        send_and_take(state, Message::Submitted)
            .into_iter()
            .find_map(|command| match command {
                Command::Authenticate { id, .. } => Some(id),
                _ => None,
            })
            .expect("an authentication request")
    }

    #[test]
    fn a_preview_screen_cannot_log_anyone_in() {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let mut state = State::new(Rc::new(localizer), Tokens::builtin_dark(), roster());

        assert_eq!(state.greeter().phase(), Phase::Unavailable);
        assert!(!state.model().credential.editable);
        assert!(!state.model().submit.enabled);
        assert!(
            matches!(state.model().face, view_model::Face::Unavailable { .. }),
            "the first thing the screen says is that it cannot log anyone in"
        );
        assert!(state.take_outgoing().is_empty());

        // And it never claims to be anything else.
        assert!(
            state.model().banner.contains("Preview"),
            "{}",
            state.model().banner
        );
    }

    #[test]
    fn a_connected_screen_promises_only_what_it_can_do() {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let state = State::connected(
            Rc::new(localizer),
            Tokens::builtin_dark(),
            PathBuf::from("/run/inox/auth.sock"),
            roster(),
        );

        assert_eq!(state.greeter().phase(), Phase::Ready);
        assert!(state.model().credential.editable);
        assert!(
            !state.model().banner.contains("Preview"),
            "a build with a boundary is not a preview: {}",
            state.model().banner
        );
        assert!(state.model().banner.contains("Inox"));
    }

    #[test]
    fn characters_reach_the_credential_only_when_the_ring_is_on_it() {
        let mut state = login();

        // Nothing here takes text but the password field: a character that arrives
        // while the ring is on a button was not typed into anything.
        send(&mut state, Message::FocusNext);
        assert_eq!(state.model().focus, Some(FocusTarget::Submit));
        send(&mut state, Message::Credential('x'));
        send(&mut state, Message::Backspace);
        assert!(state.greeter().credential_is_empty());

        // With the ring on the credential field the same messages are the typing.
        focus_credential(&mut state);
        send(&mut state, Message::Credential('h'));
        send(&mut state, Message::Credential('i'));
        send(&mut state, Message::Backspace);
        assert_eq!(state.greeter().credential_len(), 1);

        // And the placeholder is gone, the mask is what is drawn.
        assert_eq!(state.model().credential.mask, "•");
    }

    #[test]
    fn space_is_a_credential_character_and_not_a_shortcut() {
        let mut state = login();
        type_credential(&mut state, "a b");

        assert_eq!(state.greeter().credential_len(), 3);
    }

    #[test]
    fn the_focus_starts_on_the_credential_and_wraps_through_the_controls() {
        let mut state = login();

        assert_eq!(
            state.focus(),
            Some(FocusTarget::Credential),
            "the password field is what the person types into"
        );

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Submit));

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::SwitchUser));

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Cancel));

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Credential), "it wrapped");

        send(&mut state, Message::FocusPrevious);
        assert_eq!(state.focus(), Some(FocusTarget::Cancel), "and back again");
    }

    #[test]
    fn the_account_list_is_walked_with_the_keyboard_and_escaped_out_of() {
        let mut state = login();

        send(&mut state, Message::SwitchUser);

        assert!(state.model().choosing, "the account list is open");
        assert_eq!(
            state.focus(),
            Some(FocusTarget::account(0)),
            "the keyboard lands on the accounts, and the list says so"
        );

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::account(1)));

        send(&mut state, Message::FocusNext);
        assert_eq!(
            state.focus(),
            Some(FocusTarget::Cancel),
            "and the way back is reachable too"
        );

        // Escape leaves the list without choosing anything, and without clearing
        // anything either: nothing has been typed while the list was open.
        let commands = send_and_take(&mut state, Message::Cancelled);

        assert!(!state.model().choosing);
        assert_eq!(state.greeter().username(), "alice", "nothing was chosen");
        assert!(commands.is_empty(), "{commands:?}");
        assert_eq!(state.focus(), Some(FocusTarget::Credential));
    }

    #[test]
    fn the_keyboard_can_log_in_as_another_account() {
        let mut state = login();
        type_credential(&mut state, "hunter2");

        send(&mut state, Message::SwitchUser);
        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::account(1)));
        send(&mut state, Message::Activate);

        assert_eq!(state.greeter().username(), "bob");
        assert!(
            !state.model().choosing,
            "choosing an account closes the list"
        );
        assert!(
            state.greeter().credential_is_empty(),
            "a password typed for one account is not a password for another"
        );
        assert_eq!(state.focus(), Some(FocusTarget::Credential));

        // And the next submission is about the account the person chose.
        type_credential(&mut state, "hunter2");

        let commands = send_and_take(&mut state, Message::Submitted);
        let username = commands
            .into_iter()
            .find_map(|command| match command {
                Command::Authenticate { username, .. } => Some(username),
                _ => None,
            })
            .expect("an authentication request");

        assert_eq!(username, "bob");
    }

    #[test]
    fn the_account_the_roster_offers_first_is_the_one_a_login_would_use() {
        let state = login();

        assert_eq!(state.greeter().username(), "alice");
        assert_eq!(
            state
                .model()
                .selected
                .as_ref()
                .map(|account| account.name.as_str()),
            Some("alice"),
            "what the screen draws as the chosen account is what would be authenticated"
        );
    }

    #[test]
    fn an_account_row_the_screen_did_not_draw_operates_nothing() {
        let mut state = login();
        send(&mut state, Message::SwitchUser);

        // The row is drawn from the projection's list, so a name from outside it is
        // not an account this screen can choose — and is not a reason to close a list
        // the person is still looking at either.
        send(&mut state, Message::AccountChosen("eve".into()));

        assert_eq!(state.greeter().username(), "alice");
        assert_eq!(state.greeter().phase(), Phase::Ready);
        assert!(state.model().choosing, "the list is still open");
    }

    #[test]
    fn the_account_list_is_not_opened_where_there_is_nothing_to_choose() {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let single = Accounts {
            accounts: vec![Account::new("alice", 1000, None)],
            ..Accounts::default()
        };
        let mut state =
            State::with_boundary(Rc::new(localizer), Tokens::builtin_dark(), true, single);

        assert!(state.model().switch_user.is_none());
        send(&mut state, Message::SwitchUser);

        assert!(!state.model().choosing);
        assert_eq!(state.focus(), Some(FocusTarget::Credential));
    }

    #[test]
    fn the_account_a_session_started_for_is_recorded_for_the_next_login() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let record = directory.path().join("greet").join("last-user");
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let accounts = Accounts {
            record: Some(record.clone()),
            ..roster()
        };
        let mut state =
            State::with_boundary(Rc::new(localizer), Tokens::builtin_dark(), true, accounts);

        send(&mut state, Message::SwitchUser);
        send(&mut state, Message::FocusNext);
        send(&mut state, Message::Activate);
        assert_eq!(state.greeter().username(), "bob");

        let id = submit(&mut state, "hunter2");
        send(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));

        assert_eq!(
            std::fs::read_to_string(&record).ok(),
            None,
            "an accepted attempt is not a session that started"
        );

        send(&mut state, Message::SessionAnswered(id, true));
        assert_eq!(state.greeter().phase(), Phase::SessionStarted);
        assert_eq!(
            std::fs::read_to_string(&record).expect("the account is recorded"),
            "bob\n",
            "the next login offers the account this one used"
        );
    }

    #[test]
    fn a_started_session_stays_on_screen_and_the_login_comes_back() {
        let mut state = login();

        // Bob, so that "the account is kept" is about a choice rather than about
        // the account the roster happened to preselect.
        send(&mut state, Message::SwitchUser);
        send(&mut state, Message::FocusNext);
        send(&mut state, Message::Activate);
        assert_eq!(state.greeter().username(), "bob");

        let id = submit(&mut state, "hunter2");
        send(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));
        // The accepted attempt is the one the session start is asked for, once.
        assert!(
            state
                .take_outgoing()
                .iter()
                .any(|command| matches!(command, Command::StartSession(started) if *started == id))
        );

        send(&mut state, Message::SessionAnswered(id, true));

        // The window does not go away: the screen says the session started and
        // operates nothing, which is a face rather than the end of the login screen
        //.
        assert_eq!(state.greeter().phase(), Phase::SessionStarted);
        assert!(matches!(
            state.model().face,
            view_model::Face::Started { .. }
        ));
        assert!(state.model().focus_order.is_empty());
        assert_eq!(state.focus(), None);
        assert!(state.greeter().credential_is_empty());
        assert_eq!(
            state.model().credential.mask,
            "",
            "there is nothing left of the password to draw"
        );
        assert!(
            state.take_outgoing().is_empty(),
            "the started face asks the boundary for nothing"
        );

        let started_at = state.clock;

        // The dwell runs on the clock this component already keeps: one second is
        // not three.
        send(
            &mut state,
            Message::Ticked(started_at + Duration::from_secs(1)),
        );
        assert_eq!(
            state.greeter().phase(),
            Phase::SessionStarted,
            "the started face lasts three seconds, not one"
        );

        send(
            &mut state,
            Message::Ticked(started_at + SESSION_STARTED_DWELL),
        );

        assert_eq!(state.greeter().phase(), Phase::Ready);
        assert_eq!(
            state.greeter().username(),
            "bob",
            "the account the session was started for is kept"
        );
        assert!(
            state.greeter().credential_is_empty(),
            "and the password is not"
        );
        assert_eq!(state.model().credential.mask, "");
        assert_eq!(
            state.focus(),
            Some(FocusTarget::Credential),
            "the keyboard is back on the first operable control"
        );
        assert_eq!(
            state.model().focus_order,
            vec![
                FocusTarget::Credential,
                FocusTarget::Submit,
                FocusTarget::SwitchUser,
                FocusTarget::Cancel
            ]
        );
        assert!(state.model().submit.enabled);
        assert!(
            state.take_outgoing().is_empty(),
            "returning to the login is not a request to the boundary"
        );

        // And the screen it returns to is one a login can be made from.
        let id = submit(&mut state, "hunter3");
        send(&mut state, Message::AuthAnswered(id, AuthReply::Rejected));
        assert_eq!(state.greeter().phase(), Phase::Failed);
    }

    #[test]
    fn a_session_that_did_not_start_stays_a_failure() {
        let mut state = login();
        let id = submit(&mut state, "hunter2");
        send(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));
        assert_eq!(state.greeter().phase(), Phase::Starting);

        send(&mut state, Message::SessionAnswered(id, false));

        assert_eq!(state.greeter().phase(), Phase::Failed);
        assert!(
            matches!(state.model().face, view_model::Face::Failed { .. }),
            "the reason a session did not start stays on screen"
        );

        // The dwell belongs to the face that started a session. Time passing must
        // not turn a failed start into a prompt that says nothing happened.
        let failed_at = state.clock;
        for later in [1, 5, 600] {
            send(
                &mut state,
                Message::Ticked(failed_at + Duration::from_secs(later)),
            );
            assert_eq!(state.greeter().phase(), Phase::Failed, "after {later} s");
        }

        assert!(
            state.model().submit.enabled,
            "a session that did not start is retryable, not terminal"
        );
    }

    #[test]
    fn the_dwell_only_runs_for_a_session_that_started() {
        // A screen with no boundary cannot start a session, and a screen nobody
        // submitted on has nothing in flight: neither is on a timer, so a clock
        // that jumps an hour leaves both exactly where they were.
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let mut preview = State::new(Rc::new(localizer), Tokens::builtin_dark(), roster());
        let later = preview.clock + Duration::from_secs(3600);
        send(&mut preview, Message::Ticked(later));
        assert_eq!(preview.greeter().phase(), Phase::Unavailable);
        assert!(matches!(
            preview.model().face,
            view_model::Face::Unavailable { .. }
        ));

        let mut untouched = login();
        let later = untouched.clock + Duration::from_secs(3600);
        send(&mut untouched, Message::Ticked(later));
        assert_eq!(untouched.greeter().phase(), Phase::Ready);
        assert_eq!(untouched.focus(), Some(FocusTarget::Credential));
    }

    #[test]
    fn the_reset_records_nothing_and_the_next_session_is_recorded_when_it_starts() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let record = directory.path().join("greet").join("last-user");
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let accounts = Accounts {
            record: Some(record.clone()),
            ..roster()
        };
        let mut state =
            State::with_boundary(Rc::new(localizer), Tokens::builtin_dark(), true, accounts);

        let id = submit(&mut state, "hunter2");
        send(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));
        send(&mut state, Message::SessionAnswered(id, true));
        assert_eq!(
            std::fs::read_to_string(&record).expect("the account is recorded"),
            "alice\n"
        );

        // A sentinel in place of the record: anything that writes while the started
        // face is up, or when it returns to the login, replaces it.
        std::fs::write(&record, "sentinel\n").expect("the record can be replaced");

        let started_at = state.clock;
        send(
            &mut state,
            Message::Ticked(started_at + Duration::from_secs(1)),
        );
        assert_eq!(state.greeter().phase(), Phase::SessionStarted);
        assert_eq!(
            std::fs::read_to_string(&record).expect("the record is still there"),
            "sentinel\n",
            "the started face is not a second write"
        );

        send(
            &mut state,
            Message::Ticked(started_at + SESSION_STARTED_DWELL),
        );
        assert_eq!(state.greeter().phase(), Phase::Ready);
        assert_eq!(
            std::fs::read_to_string(&record).expect("the record is still there"),
            "sentinel\n",
            "returning to the login is not a login"
        );

        // The same process starting another session later is another login, and it
        // is recorded when it starts — the flag follows the face, not the process.
        send(&mut state, Message::SwitchUser);
        send(&mut state, Message::FocusNext);
        send(&mut state, Message::Activate);
        assert_eq!(state.greeter().username(), "bob");

        let id = submit(&mut state, "hunter2");
        send(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));
        send(&mut state, Message::SessionAnswered(id, true));

        assert_eq!(
            std::fs::read_to_string(&record).expect("the account is recorded"),
            "bob\n"
        );
    }

    #[test]
    fn operate_acts_on_the_focused_control_and_not_on_enter_alone() {
        let mut state = login();

        // The focus is on the password field, so Enter submits the form.
        send(&mut state, Message::Activate);
        assert_eq!(state.greeter().phase(), Phase::Authenticating);
        assert_eq!(state.take_outgoing().len(), 1);

        // Submitting took the field and the button away, so the focus moved to the
        // one operable control that is left — and Enter now does what the ring says
        // it does, which is clear rather than submit.
        assert_eq!(state.focus(), Some(FocusTarget::Cancel));

        send(&mut state, Message::Activate);
        assert_eq!(state.greeter().phase(), Phase::Ready);
        assert_eq!(
            state.greeter().username(),
            "alice",
            "the account survives an attempt that was abandoned"
        );
    }

    #[test]
    fn the_credential_is_handed_over_once_and_not_kept() {
        let mut state = login();
        let _ = submit(&mut state, "hunter2");

        assert!(
            state.greeter().credential_is_empty(),
            "the screen does not keep what it handed over"
        );
        assert_eq!(state.model().credential.mask, "");
    }

    #[test]
    fn a_refused_login_keeps_the_account_and_asks_for_the_password_again() {
        let mut state = login();
        let id = submit(&mut state, "hunter2");

        send(&mut state, Message::AuthAnswered(id, AuthReply::Rejected));

        assert_eq!(state.greeter().phase(), Phase::Failed);
        assert_eq!(state.greeter().username(), "alice");
        assert!(state.greeter().credential_is_empty());
        assert!(
            state.model().credential.editable && state.model().submit.enabled,
            "a refusal has to leave the form usable"
        );
    }

    #[test]
    fn activating_the_clear_control_clears_and_asks_the_transport_to_cancel() {
        let mut state = login();
        let id = submit(&mut state, "hunter2");

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Cancel));
        let commands = send_and_take(&mut state, Message::Activate);

        assert!(state.greeter().credential_is_empty());
        assert_eq!(
            state.greeter().username(),
            "alice",
            "clearing the password is not choosing a different account"
        );
        assert_eq!(state.greeter().phase(), Phase::Ready);
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::Cancel(cancelled) if *cancelled == id)),
            "the boundary has to hear about the cancellation"
        );
    }

    #[test]
    fn the_focus_leaves_a_control_that_can_no_longer_be_used() {
        let mut state = login();
        focus_credential(&mut state);
        send(&mut state, Message::FocusNext);
        assert_eq!(state.focus(), Some(FocusTarget::Submit));

        // Submitting takes the fields and the button away; the focus must not stay
        // on a control that no longer exists.
        send(&mut state, Message::Activate);
        let _ = state.take_outgoing();

        assert_eq!(state.greeter().phase(), Phase::Authenticating);
        assert_eq!(state.focus(), Some(FocusTarget::Cancel));
    }

    #[test]
    fn a_current_success_starts_a_session_once() {
        let mut state = login();
        let id = submit(&mut state, "hunter2");

        let commands = send_and_take(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));
        assert_eq!(state.greeter().phase(), Phase::Starting);
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::StartSession(started) if *started == id)),
            "the session the person asked for is the boundary's to start"
        );

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
        let first = submit(&mut state, "one");
        send(&mut state, Message::Cancelled);
        let _ = state.take_outgoing();

        let current = submit(&mut state, "two");
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
        let commands = send_and_take(
            &mut state,
            Message::AuthAnswered(current, AuthReply::Accepted),
        );
        assert_eq!(state.greeter().phase(), Phase::Starting);
        assert!(commands.iter().any(
            |command| matches!(command, Command::StartSession(started) if *started == current)
        ));

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
        let id = submit(&mut state, "hunter2");
        let _ = state.take_outgoing();

        let expired = state.clock + Duration::from_secs(30);
        let commands = send_and_take(&mut state, Message::Ticked(expired));
        assert_eq!(state.greeter().phase(), Phase::Failed);
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::Cancel(cancelled) if *cancelled == id)),
            "an attempt past its deadline is abandoned on the boundary too"
        );

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
            let id = submit(&mut state, "hunter2");
            let _ = state.take_outgoing();
            send(&mut state, Message::AuthAnswered(id, AuthReply::Rejected));
            assert_eq!(state.greeter().phase(), Phase::Failed);
        }

        assert!(!state.model().submit.enabled, "the allowance is spent");
        assert_eq!(
            state.model().submit.disabled_reason.as_deref(),
            Some("Too many attempts for now")
        );

        // The window passes and the screen becomes usable again: a limit that never
        // lifts locks the person out of their own machine.
        let window = state.clock + Duration::from_secs(60);
        send(&mut state, Message::Ticked(window));
        assert!(state.model().submit.enabled);
    }

    #[test]
    fn the_clock_only_moves_when_a_tick_says_so() {
        let mut state = login();
        let before = state.clock;
        send(&mut state, Message::Submitted);
        let _ = state.take_outgoing();

        assert_eq!(state.clock, before);

        let later = before + Duration::from_secs(5);
        send(&mut state, Message::Ticked(later));
        assert_eq!(state.clock, later);
    }

    #[test]
    fn a_transport_report_drives_the_model_through_its_own_events() {
        let mut state = login();
        let id = submit(&mut state, "hunter2");
        let attempt = id.raw();

        // A refusal.
        send(
            &mut state,
            Message::TransportAnswered {
                attempt,
                report: Report::Rejected,
            },
        );
        assert_eq!(state.greeter().phase(), Phase::Failed);

        // A boundary that is not there is the same thing to the person at the
        // keyboard as one that says it cannot answer.
        let id = submit(&mut state, "hunter2");
        send(
            &mut state,
            Message::TransportAnswered {
                attempt: id.raw(),
                report: Report::Unreachable,
            },
        );
        assert_eq!(state.greeter().phase(), Phase::Unavailable);
        assert!(!state.model().submit.enabled);

        // A session that started.
        let mut state = login();
        let id = submit(&mut state, "hunter2");
        send(
            &mut state,
            Message::TransportAnswered {
                attempt: id.raw(),
                report: Report::Accepted,
            },
        );
        send(
            &mut state,
            Message::TransportAnswered {
                attempt: id.raw(),
                report: Report::SessionStarted(true),
            },
        );
        assert_eq!(state.greeter().phase(), Phase::SessionStarted);
    }

    #[test]
    fn a_transport_report_about_another_attempt_changes_nothing() {
        let mut state = login();
        let id = submit(&mut state, "hunter2");

        send(
            &mut state,
            Message::TransportAnswered {
                attempt: id.raw() + 1000,
                report: Report::SessionStarted(true),
            },
        );

        assert_eq!(
            state.greeter().phase(),
            Phase::Authenticating,
            "a report about an attempt the model never issued cannot start anything"
        );
    }

    #[test]
    fn update_hands_the_queue_to_the_transport_and_leaves_nothing_behind() {
        // The queue is an interface, not storage: every turn of the loop gives what
        // the model asked for to the caller, which is the transport. Nothing is left
        // in the state once it has been handed over.
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
        let mut state = State::connected(
            Rc::new(localizer),
            Tokens::builtin_dark(),
            PathBuf::from("/run/inox/auth.sock"),
            roster(),
        );

        let commands = update(&mut state, Message::Submitted);

        assert!(
            matches!(commands.as_slice(), [Command::Authenticate { .. }]),
            "{commands:?}"
        );
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn the_keyboard_bindings_are_the_whole_keyboard_interface() {
        assert_eq!(key_binding(KeyPress::Tab), Some(Message::FocusNext));
        assert_eq!(key_binding(KeyPress::BackTab), Some(Message::FocusPrevious));
        assert_eq!(key_binding(KeyPress::Enter), Some(Message::Activate));
        assert_eq!(key_binding(KeyPress::Escape), Some(Message::Cancelled));
        assert_eq!(key_binding(KeyPress::Backspace), Some(Message::Backspace));

        // Space is a password character: a shortcut on it would eat a character
        // somebody may type into their own password.
        assert_eq!(
            key_binding(KeyPress::Character(' ')),
            Some(Message::Credential(' '))
        );
        assert_eq!(
            key_binding(KeyPress::Character('a')),
            Some(Message::Credential('a'))
        );
        assert_eq!(
            key_binding(KeyPress::Character('在')),
            Some(Message::Credential('在')),
            "a password is not ASCII"
        );
        // A control character is not text.
        assert_eq!(key_binding(KeyPress::Character('\u{7}')), None);
    }

    #[test]
    fn every_queued_command_says_what_it_is_without_saying_a_credential() {
        let mut state = login();
        focus_credential(&mut state);
        type_credential(&mut state, "hunter2");
        send(&mut state, Message::Submitted);

        let queued = format!("{:?}", state.take_outgoing());

        assert!(queued.contains("Authenticate"), "{queued}");
        assert!(queued.contains("alice"), "{queued}");
        assert!(!queued.contains("hunter2"), "{queued}");
        assert!(queued.contains("<redacted>"), "{queued}");
    }

    #[test]
    fn nothing_a_reply_can_carry_is_printable_as_a_credential() {
        let mut state = login();
        let id = submit(&mut state, "hunter2");
        send(&mut state, Message::AuthAnswered(id, AuthReply::Accepted));
        send(&mut state, Message::SessionAnswered(id, true));

        let rendered = format!("{:?}", state.greeter());

        assert!(!rendered.contains("hunter2"), "{rendered}");
    }
}
