//! Login transitions. Only a current reply from the reviewed boundary may start a session.
//!
//! the authentication boundary turned this crate's boundary from a shape into a real one, and two
//! things about the model are worth reading before changing anything here:
//!
//! - **The credential lives in the model for exactly as long as the person is
//!   typing it.** It is a [`Secret`] — zeroed on clear and on drop, unprintable,
//!   with one accessor — and the only thing that ever takes it out is
//!   [`Event::Submit`], which *moves* it into the command that carries it to the
//!   boundary. There is no field left holding it afterwards, which is why a refused
//!   login asks for the password again rather than keeping it for the retry.
//! - **Nothing about an answer is trusted because it arrived.** A reply counts
//!   only for the attempt that is in flight, and only inside its deadline; the
//!   identifier in it is checked against the one the model issued.
//! - **The identity is chosen, never typed.** The account the login would use is
//!   one of the accounts the system offers ([`crate::accounts`]), selected by
//!   [`Event::SelectAccount`], and changing it clears the credential: a password
//!   typed for one account is not a password for another. What is still checked
//!   here is the same grammar the boundary checks (`valid_username`), because the
//!   account database is not this crate's to trust — an account whose name the
//!   contract would refuse is refused here too, with a reason on screen.
//! - **A session that started is a dwell, not the end of the model.** The display
//!   manager hands the display to a session on another VT, which this process
//!   cannot see; the screen it has to show when that session
//!   ends is this one. So [`Phase::SessionStarted`] lasts [`SESSION_STARTED_DWELL`]
//!   and then returns to [`Phase::Ready`] with the account kept and the credential
//!   gone (§决策 4). The dwell runs on the model's own clock — [`Event::Tick`] —
//!   like the reply deadline, and a session that failed to start does not get one.
use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use inox_common::secret::Secret;

use crate::accounts::{Account, Accounts};

impl fmt::Display for AttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AttemptId {
    /// An identifier no model ever issued.
    ///
    /// Test-only: it exists so a test can express "an answer arrived from
    /// nowhere", which is the shape a forged or replayed success takes, without
    /// making the identifier constructible in the code that has to trust it.
    #[cfg(test)]
    pub const fn that_was_never_issued(raw: u64) -> Self {
        Self(raw)
    }

    /// The identifier as the wire carries it.
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// The identifier a reply from the boundary names.
    ///
    /// Crate-internal, and that is the whole point: an identifier that arrives from
    /// somewhere else is data to be *compared* with the one the model issued, not an
    /// authority. The model checks `pending == id` before it believes anything, so
    /// this constructor grants nothing on its own.
    pub(crate) const fn from_reply(raw: u64) -> Self {
        Self(raw)
    }
}

/// How long the screen stays on the started face before the login comes back.
///
/// 4: the session runs on a VT this process cannot see, so the
/// screen that has to be here when the session ends is this one. The started face
/// is therefore a *dwell* rather than the end of the model's life — long enough to
/// be read, short enough that whoever stays in front of the greeter is not looking
/// at a screen that says nothing can be done.
///
/// It is a deadline on the model's own clock, like the reply deadline: the screen
/// advances it with [`Event::Tick`] and with nothing else. The tick is the
/// once-a-second one the login screen already runs, so the face ends on the first
/// tick at or after this delay rather than on a timer of its own — the granularity
/// the reply deadline has, and the reason this crate grows no second clock.
pub const SESSION_STARTED_DWELL: Duration = Duration::from_secs(3);

/// The most bytes of identity data the model retains.
///
/// A floor rather than a policy: it bounds what a paste can put into the model,
/// so a window that receives a megabyte of text still has a finite amount of it
/// to keep, to lay out and to hand to the system authenticator.
pub const MAX_IDENTITY_BYTES: usize = 128;

/// Correlation identifier for a single authentication and session-start attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttemptId(u64);

/// Visible lifecycle; no authentication implementation is embedded here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// No reviewed authentication transport is connected.
    Unavailable,
    /// A user may submit a name.
    Ready,
    /// Waiting for the system authenticator.
    Authenticating,
    /// Authenticated; waiting for the display manager to confirm session startup.
    Starting,
    /// A rejected attempt or failed transport. A new attempt may be submitted.
    Failed,
    /// The display manager confirmed session startup.
    ///
    /// Not the end of the login: the screen holds this face for
    /// [`SESSION_STARTED_DWELL`] and then comes back to [`Phase::Ready`], because
    /// the session runs on a VT this process cannot see and the screen that has to
    /// be here when it ends is this one.
    SessionStarted,
}

/// Result supplied exclusively by the reviewed system boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthReply {
    /// The boundary authenticated the current user.
    Accepted,
    /// Authentication was rejected.
    Rejected,
    /// The authenticator failed or disconnected.
    Unavailable,
}

/// Events from UI actions, a monotonic timer and the trusted system transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Offer a different account, while no request is in flight.
    ///
    /// A name the account list does not contain is ignored rather than adopted:
    /// the list the screen draws and the list this model holds are the same one,
    /// so a name from anywhere else is not an identity this model can act on.
    SelectAccount(String),
    /// A character was typed into the credential field.
    Credential(char),
    /// Remove the last character of the credential.
    Backspace,
    /// Begin authentication.
    Submit,
    /// Cancel the attempt in flight and clear the credential.
    Cancel,
    /// Reply to the current authentication request.
    AuthAnswered(AttemptId, AuthReply),
    /// The display manager finished launching the authenticated session.
    SessionAnswered(AttemptId, bool),
    /// Enforce the finite reply deadline.
    Tick,
}

/// Requests for the transport this crate does not own.
///
/// Not `Clone` and not `PartialEq`: [`Command::Authenticate`] carries the
/// credential, and [`Secret`] is neither. A command is a thing that is handed over
/// once, and the type says so.
#[derive(Debug)]
pub enum Command {
    /// Authenticate the named user through the system boundary.
    Authenticate {
        /// Request to correlate.
        id: AttemptId,
        /// Validated user identity.
        username: String,
        /// What the user typed, moved out of the model by [`Event::Submit`].
        credential: Secret,
    },
    /// Start the fixed Inox session for the authenticated identity.
    StartSession(AttemptId),
    /// Cancel the old conversation on the system boundary.
    Cancel(AttemptId),
    /// No transport request.
    None,
}

/// User identity, credential and correlation state.
#[derive(Debug)]
pub struct Greeter {
    accounts: Vec<Account>,
    username: String,
    identity_too_long: bool,
    credential: Secret,
    phase: Phase,
    available: bool,
    serial: u64,
    pending: Option<(AttemptId, Instant)>,
    attempts: VecDeque<Instant>,
    /// When the started face ends and the login comes back, once a session has
    /// really started.
    ///
    /// `None` in every other phase: a session that did not start, and a screen
    /// that never started one, are not on a timer.
    resume_at: Option<Instant>,
}
impl Greeter {
    /// Create a greeter that offers `roster`'s accounts and the boundary's actual
    /// availability.
    ///
    /// The account the roster preselects is the one every later answer is about
    /// until the person chooses another: "which account is being logged in" is a
    /// fact of this model from the first frame rather than something the screen
    /// keeps to itself.
    pub fn new(available: bool, roster: &Accounts) -> Self {
        let mut greeter = Self {
            accounts: roster.accounts.clone(),
            username: String::new(),
            identity_too_long: false,
            credential: Secret::new(),
            phase: if available {
                Phase::Ready
            } else {
                Phase::Unavailable
            },
            available,
            serial: 0,
            pending: None,
            attempts: VecDeque::new(),
            resume_at: None,
        };

        if let Some(index) = roster.selected() {
            greeter.adopt(roster.accounts[index].name());
        }

        greeter
    }

    /// Adopts `name` as the identity, bounded by what this model retains.
    ///
    /// The bound is a floor rather than a policy: the account database is not
    /// this crate's to trust, and a name past what the contract carries has to be
    /// noticed by the model rather than handed on.
    fn adopt(&mut self, name: &str) {
        self.identity_too_long = name.len() > MAX_IDENTITY_BYTES;
        self.username = name.chars().take(MAX_IDENTITY_BYTES).collect();
    }
    /// The current display phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Whether a reviewed authentication transport is connected at all.
    pub const fn available(&self) -> bool {
        self.available
    }

    /// The accounts this greeter offers, in the order they are offered.
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// The account a login would authenticate, when there is one.
    pub fn selected(&self) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|account| account.name() == self.username)
    }

    /// The username being edited or authenticated.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn username(&self) -> &str {
        &self.username
    }

    /// The attempt that is in flight, when one is.
    ///
    /// For a caller that drives the model without a transport — the tests do, and a
    /// diagnostic line can — the identifier is otherwise unobservable. It is not a
    /// capability: the model checks it against the attempt it issued before it
    /// believes any answer, so knowing it buys nothing.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn attempt_in_flight(&self) -> Option<AttemptId> {
        self.pending.map(|(id, _)| id)
    }

    /// How many characters of credential the model is holding.
    ///
    /// The only property of the credential that reaches the screen, and the only
    /// way anything outside this module learns anything about it: there is no
    /// accessor for the value, because the value has exactly one destination.
    pub fn credential_len(&self) -> usize {
        self.credential.len()
    }

    /// Whether the user has typed a credential.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn credential_is_empty(&self) -> bool {
        self.credential.is_empty()
    }

    /// Whether the credential is longer than the contract will carry.
    ///
    /// A credential longer than this is *not* truncated, unlike the identity: a
    /// truncated password is a different password, and sending one would mean the
    /// boundary checking something the person did not type. The screen says so
    /// instead and refuses the submission.
    pub fn credential_too_long(&self) -> bool {
        self.credential.byte_len() > inox_auth::wire::MAX_CREDENTIAL_BYTES
    }

    /// Whether the last identity the user supplied was longer than the model
    /// keeps, and was therefore cut down to [`MAX_IDENTITY_BYTES`].
    ///
    /// The interface shows this rather than silently submitting a name the user
    /// did not type.
    pub const fn identity_too_long(&self) -> bool {
        self.identity_too_long
    }
    /// Whether submission is currently enabled, including the rolling rate limit.
    pub fn can_submit(&self, now: Instant) -> bool {
        self.available
            && self.selected().is_some()
            && !self.identity_too_long
            && !self.credential_too_long()
            && matches!(self.phase, Phase::Ready | Phase::Failed)
            && valid_username(&self.username)
            && self
                .attempts
                .iter()
                .filter(|t| now.saturating_duration_since(**t) < Duration::from_secs(60))
                .count()
                < 3
    }
    /// Takes the screen back to the login prompt when the started face has had its
    /// moment.
    ///
    /// Two things carry over and one does not: the account stays, because it is the
    /// account this login used and the next login is most likely the same one; the
    /// credential does not, and this clears it even though [`Event::Submit`] already
    /// moved it out — a reset that relied on that would be one refactor away from
    /// leaving a password on a screen nobody is typing into; and the phase becomes
    /// [`Phase::Ready`], or [`Phase::Unavailable`] if the boundary has gone away,
    /// which is the state a new login starts from.
    fn resume(&mut self) {
        self.resume_at = None;
        self.credential.clear();
        self.phase = if self.available {
            Phase::Ready
        } else {
            Phase::Unavailable
        };
    }

    /// Apply one event, rejecting stale, duplicated and expired successes.
    pub fn apply(&mut self, event: Event, now: Instant) -> Command {
        // The started face is a dwell, not a dead end: the only event that can leave
        // it is the clock reaching the end of that dwell. Everything else — another
        // submission, a cancellation, a name from the account list, a duplicate
        // session reply — is refused here rather than drawn on top of a session that
        // is already someone else's display.
        if self.phase == Phase::SessionStarted {
            if matches!(event, Event::Tick) && self.resume_at.is_some_and(|end| now >= end) {
                self.resume();
            }

            return Command::None;
        }
        if self.pending.is_some_and(|(_, deadline)| now >= deadline) {
            let old = self.pending.take();
            self.phase = Phase::Failed;
            if let Some((id, _)) = old {
                return Command::Cancel(id);
            }
        }
        match event {
            Event::SelectAccount(name)
                if self.pending.is_none() && self.phase != Phase::SessionStarted =>
            {
                let Some(account) = self
                    .accounts
                    .iter()
                    .find(|account| account.name() == name)
                    .cloned()
                else {
                    return Command::None;
                };

                self.adopt(account.name());
                // The credential belongs to the account it was typed for, so
                // choosing another account is choosing an empty field.
                self.credential.clear();
                Command::None
            }
            // Typing is only meaningful while the screen is collecting a login: not
            // after the session has started, and not while an attempt is in flight,
            // where the field is drawn as uneditable for the same reason.
            Event::Credential(character)
                if self.pending.is_none() && matches!(self.phase, Phase::Ready | Phase::Failed) =>
            {
                self.credential.push(character);
                Command::None
            }
            Event::Backspace
                if self.pending.is_none() && matches!(self.phase, Phase::Ready | Phase::Failed) =>
            {
                self.credential.backspace();
                Command::None
            }
            Event::Submit if self.can_submit(now) => {
                let Some(serial) = self.serial.checked_add(1) else {
                    self.available = false;
                    self.phase = Phase::Unavailable;
                    return Command::None;
                };
                self.serial = serial;
                let id = AttemptId(serial);
                self.pending = Some((id, now + Duration::from_secs(30)));
                self.phase = Phase::Authenticating;
                self.attempts
                    .retain(|t| now.saturating_duration_since(*t) < Duration::from_secs(60));
                self.attempts.push_back(now);
                Command::Authenticate {
                    id,
                    username: self.username.clone(),
                    // Moved, not copied: from here on the credential exists only in
                    // the command on its way to the boundary, and a rejected attempt
                    // is retyped rather than replayed from a buffer this process kept.
                    credential: std::mem::take(&mut self.credential),
                }
            }
            Event::Cancel => {
                // The credential goes; the account stays. A cancelled login is a
                // login that was abandoned, not a different person at the keyboard.
                self.credential.clear();
                self.phase = if self.available {
                    Phase::Ready
                } else {
                    Phase::Unavailable
                };
                self.pending
                    .take()
                    .map_or(Command::None, |(id, _)| Command::Cancel(id))
            }
            Event::AuthAnswered(id, reply)
                if self.phase == Phase::Authenticating
                    && self.pending.is_some_and(|(current, _)| current == id) =>
            {
                match reply {
                    AuthReply::Accepted => {
                        self.phase = Phase::Starting;
                        self.pending = Some((id, now + Duration::from_secs(30)));
                        Command::StartSession(id)
                    }
                    AuthReply::Rejected => {
                        self.pending = None;
                        self.phase = Phase::Failed;
                        Command::None
                    }
                    AuthReply::Unavailable => {
                        self.pending = None;
                        self.available = false;
                        self.phase = Phase::Unavailable;
                        Command::None
                    }
                }
            }
            Event::SessionAnswered(id, success)
                if self.phase == Phase::Starting
                    && self.pending.is_some_and(|(current, _)| current == id) =>
            {
                self.pending = None;
                if success {
                    self.phase = Phase::SessionStarted;
                    // The started face is a moment on screen, not the end of this
                    // screen's life: the login comes back when the dwell is over
                    //.
                    self.resume_at = Some(now + SESSION_STARTED_DWELL);
                } else {
                    // A session that would not start is a failed login, with the
                    // reason on screen and no timer behind it.
                    self.phase = Phase::Failed;
                    self.resume_at = None;
                }
                Command::None
            }
            _ => Command::None,
        }
    }
}

/// Conservative portable account names, never shell command strings.
///
/// The grammar is the `auth-v1` contract's (`inox_auth::wire::valid_username`), not
/// a second copy of it: the field must accept exactly what the boundary will accept,
/// and the only way to keep those two from drifting apart is for both to call the
/// same function. The dependency is L4 → L2 and carries no PAM: `inox-greet` takes
/// `inox-auth` with `default-features = false` on purpose.
pub use inox_auth::wire::valid_username;

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A greeter with a boundary and the accounts above.
    fn greeter() -> Greeter {
        Greeter::new(true, &roster())
    }

    fn submit(greeter: &mut Greeter, now: Instant) -> AttemptId {
        greeter.apply(Event::Credential('x'), now);
        match greeter.apply(Event::Submit, now) {
            Command::Authenticate { id, .. } => id,
            other => panic!("{other:?}"),
        }
    }

    /// Whether a command asks the transport for nothing.
    fn is_none(command: &Command) -> bool {
        matches!(command, Command::None)
    }

    /// Whether a command is the cancellation of `id`.
    fn cancels(command: &Command, id: AttemptId) -> bool {
        matches!(command, Command::Cancel(cancelled) if *cancelled == id)
    }

    /// Whether a command is a session start for `id`.
    fn starts(command: &Command, id: AttemptId) -> bool {
        matches!(command, Command::StartSession(started) if *started == id)
    }

    #[test]
    fn the_account_the_roster_offers_first_is_the_one_a_login_would_use() {
        let greeter = greeter();

        assert_eq!(greeter.username(), "alice");
        assert_eq!(greeter.selected().map(Account::name), Some("alice"));
        assert_eq!(
            greeter.accounts().len(),
            2,
            "the screen offers exactly what the roster holds"
        );
    }

    #[test]
    fn choosing_another_account_changes_the_identity_and_drops_the_password() {
        let now = Instant::now();
        let mut greeter = greeter();
        greeter.apply(Event::Credential('s'), now);
        greeter.apply(Event::Credential('e'), now);
        assert_eq!(greeter.credential_len(), 2);

        assert!(is_none(
            &greeter.apply(Event::SelectAccount("bob".into()), now)
        ));

        assert_eq!(greeter.username(), "bob");
        assert!(
            greeter.credential_is_empty(),
            "a password typed for one account is not a password for another"
        );
    }

    #[test]
    fn an_account_the_roster_does_not_offer_is_not_adopted() {
        let now = Instant::now();
        let mut greeter = greeter();

        assert!(is_none(
            &greeter.apply(Event::SelectAccount("eve".into()), now)
        ));

        assert_eq!(
            greeter.username(),
            "alice",
            "a name from outside the offered list is not an identity this model acts on"
        );
        assert!(greeter.selected().is_some());
    }

    #[test]
    fn no_boundary_never_authenticates_or_starts_a_session() {
        let now = Instant::now();
        let mut greeter = Greeter::new(false, &roster());
        greeter.apply(Event::Credential('x'), now);

        assert!(is_none(&greeter.apply(Event::Submit, now)));
        assert!(is_none(&greeter.apply(
            Event::AuthAnswered(AttemptId(1), AuthReply::Accepted),
            now
        )));
        assert_eq!(greeter.phase(), Phase::Unavailable);
    }

    #[test]
    fn an_empty_roster_cannot_be_logged_into() {
        let now = Instant::now();
        let mut greeter = Greeter::new(true, &Accounts::default());

        assert_eq!(greeter.username(), "");
        assert_eq!(greeter.selected(), None);
        assert!(
            !greeter.can_submit(now),
            "there is nobody to authenticate, so there is nothing to submit"
        );
        assert!(is_none(&greeter.apply(Event::Submit, now)));
    }

    #[test]
    fn only_one_current_success_can_request_session_start() {
        let now = Instant::now();
        let mut greeter = greeter();
        let old = submit(&mut greeter, now);
        assert!(cancels(&greeter.apply(Event::Cancel, now), old));
        assert_eq!(
            greeter.username(),
            "alice",
            "cancelling abandons the attempt, not the account"
        );
        let current = submit(&mut greeter, now);
        assert!(is_none(
            &greeter.apply(Event::AuthAnswered(old, AuthReply::Accepted), now)
        ));
        assert!(is_none(
            &greeter.apply(Event::SessionAnswered(current, true), now)
        ));
        assert_eq!(greeter.phase(), Phase::Authenticating);
        assert!(starts(
            &greeter.apply(Event::AuthAnswered(current, AuthReply::Accepted), now),
            current
        ));
        assert!(is_none(
            &greeter.apply(Event::AuthAnswered(current, AuthReply::Accepted), now)
        ));
        assert_eq!(greeter.phase(), Phase::Starting);
        greeter.apply(Event::SessionAnswered(current, true), now);
        assert_eq!(greeter.phase(), Phase::SessionStarted);
        assert!(is_none(&greeter.apply(Event::Cancel, now)));
        assert_eq!(greeter.phase(), Phase::SessionStarted);
    }

    #[test]
    fn a_started_session_returns_to_the_prompt_when_the_dwell_is_over() {
        let now = Instant::now();
        let mut greeter = greeter();
        let id = submit(&mut greeter, now);
        greeter.apply(Event::AuthAnswered(id, AuthReply::Accepted), now);
        greeter.apply(Event::SessionAnswered(id, true), now);
        assert_eq!(greeter.phase(), Phase::SessionStarted);

        // The started face is not the end of this model: it lasts a dwell, and the
        // login comes back with the account it used and nothing typed into it.
        greeter.apply(Event::Tick, now + Duration::from_secs(1));
        assert_eq!(
            greeter.phase(),
            Phase::SessionStarted,
            "one second is not the dwell"
        );

        assert!(is_none(
            &greeter.apply(Event::Tick, now + SESSION_STARTED_DWELL)
        ));
        assert_eq!(greeter.phase(), Phase::Ready);
        assert_eq!(
            greeter.username(),
            "alice",
            "the account survives the reset"
        );
        assert!(
            greeter.credential_is_empty(),
            "the password is not waiting on the screen the session came back to"
        );
        assert!(
            greeter.can_submit(now + SESSION_STARTED_DWELL),
            "the screen it comes back to is one a login can be made from"
        );
    }

    #[test]
    fn the_started_face_refuses_everything_but_its_own_clock() {
        let now = Instant::now();
        let mut greeter = greeter();
        let id = submit(&mut greeter, now);
        greeter.apply(Event::AuthAnswered(id, AuthReply::Accepted), now);
        greeter.apply(Event::SessionAnswered(id, true), now);

        // A session that has started is somebody else's display: nothing typed,
        // nothing chosen and nothing abandoned on top of it.
        assert!(is_none(&greeter.apply(Event::Credential('x'), now)));
        assert!(is_none(&greeter.apply(Event::Backspace, now)));
        assert!(is_none(
            &greeter.apply(Event::SelectAccount("bob".into()), now)
        ));
        assert!(is_none(&greeter.apply(Event::Submit, now)));
        assert!(is_none(&greeter.apply(Event::Cancel, now)));
        assert!(is_none(
            &greeter.apply(Event::SessionAnswered(id, true), now)
        ));

        assert_eq!(greeter.phase(), Phase::SessionStarted);
        assert_eq!(greeter.username(), "alice");
        assert!(greeter.credential_is_empty());
    }

    #[test]
    fn a_session_that_did_not_start_does_not_get_the_dwell() {
        let now = Instant::now();
        let mut greeter = greeter();
        let id = submit(&mut greeter, now);
        greeter.apply(Event::AuthAnswered(id, AuthReply::Accepted), now);
        greeter.apply(Event::SessionAnswered(id, false), now);

        assert_eq!(greeter.phase(), Phase::Failed);

        // A failed session start is a failure with a reason, not a moment to sit
        // through: time passing is not an answer to it.
        greeter.apply(Event::Tick, now + Duration::from_secs(600));
        assert_eq!(greeter.phase(), Phase::Failed);
    }

    #[test]
    fn failures_and_expired_success_never_start_sessions() {
        let now = Instant::now();
        for reply in [AuthReply::Rejected, AuthReply::Unavailable] {
            let mut greeter = greeter();
            let id = submit(&mut greeter, now);
            assert!(is_none(&greeter.apply(Event::AuthAnswered(id, reply), now)));
            assert_ne!(greeter.phase(), Phase::Starting);
        }
        let mut greeter = greeter();
        let id = submit(&mut greeter, now);
        let command = greeter.apply(
            Event::AuthAnswered(id, AuthReply::Accepted),
            now + Duration::from_secs(30),
        );
        assert!(cancels(&command, id), "an expired attempt is cancelled");
        assert_eq!(greeter.phase(), Phase::Failed);
    }

    #[test]
    fn cancellations_do_not_bypass_the_rolling_attempt_limit() {
        let now = Instant::now();
        let mut greeter = greeter();
        for _ in 0..3 {
            submit(&mut greeter, now);
            greeter.apply(Event::Cancel, now);
        }
        assert!(!greeter.can_submit(now));
        assert!(greeter.can_submit(now + Duration::from_secs(60)));
    }

    #[test]
    fn invalid_names_and_oversized_identity_data_are_bounded() {
        for name in [
            "", "-user", "0user", "alice\n", "alice;id", "../root", "a b",
        ] {
            assert!(
                !valid_username(name),
                "`{name}` should not be an account name"
            );
        }
        assert!(valid_username("alice-1.test"));

        // A database that carries a name past what the contract takes is bounded
        // by the model, and the screen is told the name was cut down.
        let long = Accounts {
            accounts: vec![Account::new("a".repeat(10_000), 1000, None)],
            ..Accounts::default()
        };
        let greeter = Greeter::new(true, &long);

        assert_eq!(greeter.username().len(), MAX_IDENTITY_BYTES);
        assert!(!greeter.can_submit(Instant::now()));
        assert!(
            greeter.identity_too_long(),
            "the interface has to be able to say that the name was cut down"
        );
    }

    #[test]
    fn an_account_name_the_contract_would_refuse_cannot_be_submitted() {
        // The account database is not this crate's to trust: a name the boundary
        // would reject is rejected here, with a reason instead of a silent no.
        let odd = Accounts {
            accounts: vec![Account::new("a b", 1000, None)],
            ..Accounts::default()
        };
        let greeter = Greeter::new(true, &odd);

        assert_eq!(greeter.selected().map(Account::name), Some("a b"));
        assert!(!greeter.can_submit(Instant::now()));
        assert!(
            !greeter.identity_too_long(),
            "the name is short; it is the grammar it fails, not the bound"
        );
    }

    #[test]
    fn the_credential_is_moved_out_of_the_model_when_it_is_submitted() {
        let now = Instant::now();
        let mut greeter = greeter();
        for character in "hunter2".chars() {
            greeter.apply(Event::Credential(character), now);
        }
        assert_eq!(greeter.credential_len(), 7);

        let command = greeter.apply(Event::Submit, now);

        let Command::Authenticate { credential, .. } = command else {
            panic!("a submission asks the transport to authenticate");
        };
        assert_eq!(credential.expose(), b"hunter2");
        assert!(
            greeter.credential_is_empty(),
            "the model must not keep a second copy of what it handed over"
        );

        // And the command is safe to log: the credential renders as redacted.
        let rendered = format!(
            "{:?}",
            Command::Authenticate {
                id: AttemptId(1),
                username: "alice".to_owned(),
                credential,
            }
        );
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn a_refused_login_is_retyped_rather_than_replayed() {
        let now = Instant::now();
        let mut greeter = greeter();
        let id = submit(&mut greeter, now);
        greeter.apply(Event::AuthAnswered(id, AuthReply::Rejected), now);

        assert_eq!(greeter.phase(), Phase::Failed);
        assert!(greeter.credential_is_empty());
        assert_eq!(greeter.username(), "alice");
        assert!(
            greeter.can_submit(now),
            "the account is still there, so a second attempt needs only the password"
        );
    }

    #[test]
    fn cancelling_clears_the_credential_and_keeps_the_account() {
        let now = Instant::now();
        let mut greeter = greeter();
        greeter.apply(Event::Credential('s'), now);
        greeter.apply(Event::Credential('e'), now);
        assert_eq!(greeter.credential_len(), 2);

        greeter.apply(Event::Cancel, now);

        assert!(greeter.credential_is_empty());
        assert_eq!(greeter.username(), "alice");
    }

    #[test]
    fn typing_is_refused_while_an_attempt_is_in_flight() {
        let now = Instant::now();
        let mut greeter = greeter();
        let _ = submit(&mut greeter, now);

        greeter.apply(Event::Credential('z'), now);
        greeter.apply(Event::Backspace, now);
        greeter.apply(Event::SelectAccount("bob".into()), now);

        assert!(
            greeter.credential_is_empty(),
            "an attempt in flight has taken the credential with it"
        );
        assert_eq!(
            greeter.username(),
            "alice",
            "and the account it is about cannot be changed underneath it"
        );
    }

    #[test]
    fn an_empty_password_can_be_submitted() {
        // Some accounts have no password, and a screen that refused to submit an
        // empty field would lock those accounts out entirely. What is *not* allowed
        // is submitting without an account.
        let now = Instant::now();
        let mut greeter = greeter();

        assert!(greeter.credential_is_empty());
        assert!(matches!(
            greeter.apply(Event::Submit, now),
            Command::Authenticate { .. }
        ));
    }

    #[test]
    fn the_credential_is_bounded_by_what_the_wire_carries() {
        let now = Instant::now();
        let mut greeter = greeter();

        for _ in 0..(inox_auth::wire::MAX_CREDENTIAL_BYTES + 64) {
            greeter.apply(Event::Credential('x'), now);
        }

        assert!(
            greeter.credential_too_long(),
            "the model has to notice that the credential is past the contract's bound"
        );
        assert!(
            !greeter.can_submit(now),
            "an over-long credential must be refused rather than truncated: a \
             truncated password is a different password"
        );
        assert!(is_none(&greeter.apply(Event::Submit, now)));
    }
}
