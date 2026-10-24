//! The PAM conversation: the only place in the workspace where a password is
//! checked (ADR 0015, ADR 0029), and the session a leader opens after it.
//!
//! Three decisions are visible in this file, and each of them is a rule rather
//! than an implementation detail:
//!
//! - **The credential never becomes a `String`.** The conversation owns a
//!   [`Secret`] and answers PAM's blind prompt from it, so there is no growable,
//!   reallocating copy of the password in this process that a later `format!`
//!   could reach. What does exist is the `CString` the `pam` crate constructs for
//!   `libpam` and frees when the conversation returns, and what `libpam` and its
//!   modules do internally — both are named in ADR 0029 as reviewed limitations
//!   rather than papered over here.
//! - **PAM's message text is not logged.** An authenticator's informational and
//!   error messages are written by third-party modules and can contain the
//!   account name or a guess at what was typed. This boundary records *that* a
//!   message arrived, never what it said.
//! - **The verdict distinguishes "wrong credentials" from "PAM is broken".**
//!   That is the reason this crate is built against `pam` 0.8 rather than 0.7: 0.8
//!   exposes the returned [`PamReturnCode`], so a module that failed to load
//!   cannot be presented to the person at the keyboard as a wrong password. A
//!   login screen that says "wrong password" when the account database is down is
//!   a login screen that makes people type their password into a broken machine
//!   forever.
//!
//! # Two PAM transactions, two processes (ADR 0037)
//!
//! [`PamBoundary`] authenticates — it runs `pam_authenticate` and `pam_acct_mgmt`
//! for the account the login screen named and then ends the transaction. The
//! *session* is opened by [`open_session`], which runs in the session leader: PAM's
//! session stack (`pam_systemd`, `pam_env`, `pam_limits`) registers a logind
//! session for the calling process, so it has to run in the process that is about
//! to become the login.
//!
//! The leader holds no credential. The safe `pam` wrapper refuses
//! `open_session` on a client that has not authenticated, so the leader's
//! transaction authenticates against a **session-only PAM service** whose `auth`
//! line permits unconditionally (`auth required pam_permit.so`) and whose
//! `account`/`session` lines are the real login stack. That is a packaging
//! requirement recorded in this crate's `AGENTS.md` and `README.md`; the boundary's
//! own authentication of the password always goes through [`PamBoundary`] with the
//! configured authentication service.
//!
//! `required` rather than `optional`: `pam_permit` cannot fail, so the two are the
//! same while the module is installed. They differ when it is not — `optional`
//! turns a missing `pam_permit.so` into a line PAM silently ignores, and the
//! failure surfaces later as a session that does not start with a message pointing
//! at logind. `required` fails where the packaging problem is.

use std::ffi::{CStr, CString};

use inox_common::secret::Secret;
use pam::{Client, Conversation, PamReturnCode};
use tracing::{debug, warn};

use crate::boundary::{Boundary, BoundaryFactory, Verdict};
use crate::session::{Account, SessionError};
use crate::wire::UnavailableReason;

/// The one conversation authentication runs: a name, a credential, and nothing to
/// say.
struct LoginConversation {
    /// The account the attempt names.
    username: String,
    /// The credential, held for the length of the conversation.
    credential: Secret,
}

impl Conversation for LoginConversation {
    fn prompt_echo(&mut self, _message: &CStr) -> Result<CString, ()> {
        CString::new(self.username.as_bytes()).map_err(|_| ())
    }

    fn prompt_blind(&mut self, _message: &CStr) -> Result<CString, ()> {
        CString::new(self.credential.expose()).map_err(|_| ())
    }

    fn info(&mut self, _message: &CStr) {
        debug!("the authenticator sent an informational message; its text is not logged");
    }

    fn error(&mut self, _message: &CStr) {
        debug!("the authenticator reported an error; its text is not logged");
    }
}

/// The conversation the session leader runs.
///
/// It answers PAM's "who is logging in?" prompt with the account the root parent
/// named — that is how `PAM_USER` gets set, since the leader cannot set the item
/// itself without `unsafe` — and refuses every blind prompt: this process holds no
/// credential, and a session stack that asks for one is a stack this process cannot
/// serve (ADR 0037 §失败、恢复与安全).
struct SessionConversation {
    /// The account the session is for.
    username: String,
}

impl Conversation for SessionConversation {
    fn prompt_echo(&mut self, _message: &CStr) -> Result<CString, ()> {
        CString::new(self.username.as_bytes()).map_err(|_| ())
    }

    fn prompt_blind(&mut self, _message: &CStr) -> Result<CString, ()> {
        Err(())
    }

    fn info(&mut self, _message: &CStr) {
        debug!("the session phase sent an informational message; its text is not logged");
    }

    fn error(&mut self, _message: &CStr) {
        debug!("the session phase reported an error; its text is not logged");
    }
}

/// The PAM-backed boundary, which authenticates one attempt.
pub struct PamBoundary {
    service: String,
}

impl PamBoundary {
    /// A boundary that authenticates through the `service` PAM stack
    /// (`/etc/pam.d/<service>`).
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

impl Boundary for PamBoundary {
    fn authenticate(&mut self, username: &str, credential: &Secret) -> Verdict {
        // A copy, because the conversation lives for as long as PAM's handle does
        // and the caller's `Secret` is moved into the job that ends here. It is
        // cleared the moment the conversation is over — see below.
        let conversation = LoginConversation {
            username: username.to_owned(),
            credential: Secret::from_bytes(credential.expose()),
        };

        let mut client = match Client::with_conversation(&self.service, conversation) {
            Ok(client) => client,
            Err(error) => {
                // The code only, never the message: it says whether the *service*
                // could be started, and the name of the account is already in the
                // surrounding span where an operator needs it.
                warn!(code = %error.0, service = %self.service, "the authenticator could not be started");
                return Verdict::Unavailable(UnavailableReason::Failed);
            }
        };

        let outcome = client.authenticate();

        // Whatever PAM decided, the credential is not needed after this point:
        // opening the session is a different transaction in a different process
        // (ADR 0037 §决策 5), and no session stack asks for a credential.
        client.conversation_mut().credential.clear();

        // Ending the transaction here rather than holding the handle for the
        // session is the ADR 0037 shape: this process only ever authenticates. The
        // `Drop` runs `pam_setcred(Delete_Cred)` and `pam_end`.
        drop(client);

        match outcome {
            Ok(()) => Verdict::Accepted,
            Err(error) => verdict_for(error.0),
        }
    }
}

/// A PAM session opened by this process, alive for as long as it is held.
///
/// Dropping it closes the session: `pam`'s `Client` calls `pam_close_session` in its
/// own `Drop`. The session leader therefore holds one until it `exec`s — an `exec`
/// replaces the process image, so the drop never happens and the session lives as
/// long as the login does.
pub struct OpenedSession {
    /// Kept only so that the session is closed if this is dropped.
    _client: Client<'static, SessionConversation>,
}

/// Opens a PAM session for `account` in this process.
///
/// This is the session leader's job (ADR 0037 §决策 5): `pam_open_session` runs
/// `pam_systemd`, which asks logind to create a session **for this pid**, and logind
/// then places this process (and everything it goes on to `exec`) in the session's
/// scope and on the VT in `XDG_VTNR`. A long-lived helper doing it would put the
/// session's cgroup and VT on the wrong process.
///
/// # The service
///
/// `service` must be a stack whose `auth` line permits (`auth required
/// pam_permit.so`): the leader has no credential to authenticate with, and the safe
/// `pam` wrapper refuses to open a session on a client that has not authenticated.
/// Its `account` and `session` lines must be the real login stack (`system-login`),
/// so `pam_nologin`, expiry, `pam_env`, `pam_limits` and `pam_systemd` still run.
/// This crate's `AGENTS.md` records it; `packaging/pam.d/inox-auth-session` owns the
/// file.
///
/// # Errors
///
/// [`SessionError::Boundary`] when PAM will not open the session (the stack is
/// missing, the account is refused by `pam_nologin`, or the session modules fail).
pub fn open_session(service: &str, account: &Account) -> Result<OpenedSession, SessionError> {
    let conversation = SessionConversation {
        username: account.username.clone(),
    };

    let mut client = Client::with_conversation(service, conversation).map_err(|error| {
        warn!(
            code = %error.0,
            service = %service,
            "the session leader's authenticator could not be started"
        );
        SessionError::Boundary
    })?;

    // `Client::open_session` requires an authenticated transaction. With the
    // session-only stack this is `pam_permit` for `auth` and the real account stack
    // for `account` — no credential is involved, and none could be: the leader has
    // none.
    client.authenticate().map_err(|error| {
        warn!(
            code = %error.0,
            service = %service,
            "the session leader's account stack refused the account"
        );
        SessionError::Boundary
    })?;

    // `PAM_USER` is resolved by PAM itself through the conversation above; this
    // makes sure it resolved to the account the parent named rather than to
    // something a stack invented.
    match client.get_user() {
        Ok(user) if user == account.username => {}
        Ok(user) => {
            warn!(
                expected = %account.username,
                resolved = %user,
                "the session stack resolved a different account"
            );
            return Err(SessionError::Boundary);
        }
        Err(error) => {
            warn!(code = %error.0, "the session stack did not resolve an account");
            return Err(SessionError::Boundary);
        }
    }

    client.open_session().map_err(|error| {
        warn!(
            code = %error.0,
            service = %service,
            "the session leader could not open the session"
        );
        SessionError::Boundary
    })?;

    Ok(OpenedSession { _client: client })
}

/// Maps a PAM return code onto what the interface may say.
///
/// The split is between "this account or this credential" and "this boundary":
/// everything on the first list is answered exactly like a wrong password, and
/// everything else is reported as the boundary failing, never as a refusal of what
/// the person typed.
fn verdict_for(code: PamReturnCode) -> Verdict {
    use PamReturnCode::*;

    match code {
        Auth_Err | User_Unknown | Cred_Insufficient | Authinfo_Unavail | Perm_Denied | MaxTries
        | Acct_Expired | Cred_Expired | Cred_Err | New_Authtok_Reqd | Try_Again
        | AuthTok_Expired | AuthTok_Err | AuthTok_Recovery_Err => Verdict::Rejected,
        // `Success` cannot reach here (it means the call returned `Ok`), and every
        // other code — `System_Err`, `Service_Err`, `Module_Unknown`, `Abort`,
        // `Conv_Err`, … — is the boundary's own failure.
        _ => Verdict::Unavailable(UnavailableReason::Failed),
    }
}

/// Creates a PAM boundary per attempt.
pub struct PamFactory {
    service: String,
}

impl PamFactory {
    /// A factory for the `service` PAM stack.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

impl BoundaryFactory for PamFactory {
    fn create(&self) -> Box<dyn Boundary> {
        Box::new(PamBoundary::new(self.service.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_password_and_a_broken_boundary_are_not_the_same_answer() {
        // The distinction the interface depends on.
        assert_eq!(verdict_for(PamReturnCode::Auth_Err), Verdict::Rejected);
        assert_eq!(verdict_for(PamReturnCode::User_Unknown), Verdict::Rejected);
        assert_eq!(verdict_for(PamReturnCode::MaxTries), Verdict::Rejected);
        assert_eq!(
            verdict_for(PamReturnCode::System_Err),
            Verdict::Unavailable(UnavailableReason::Failed)
        );
        assert_eq!(
            verdict_for(PamReturnCode::Service_Err),
            Verdict::Unavailable(UnavailableReason::Failed)
        );
        assert_eq!(
            verdict_for(PamReturnCode::Module_Unknown),
            Verdict::Unavailable(UnavailableReason::Failed)
        );
        assert_eq!(
            verdict_for(PamReturnCode::Abort),
            Verdict::Unavailable(UnavailableReason::Failed)
        );
    }

    #[test]
    fn an_unknown_account_is_not_told_apart_from_a_wrong_password() {
        // PAM's `User_Unknown` means "no such account"; the answer is the same one
        // a wrong password gets, which is what keeps the screen from being an
        // account-enumeration oracle.
        assert_eq!(
            verdict_for(PamReturnCode::User_Unknown),
            verdict_for(PamReturnCode::Auth_Err)
        );
    }

    #[test]
    fn the_conversation_answers_prompts_from_the_credential_it_holds() {
        let mut conversation = LoginConversation {
            username: "alice".to_owned(),
            credential: Secret::from_bytes(b"hunter2"),
        };
        let prompt = CString::new("Password: ").expect("a literal");

        let echoed = conversation.prompt_echo(&prompt).expect("an answer");
        let blind = conversation.prompt_blind(&prompt).expect("an answer");

        assert_eq!(echoed.to_bytes(), b"alice");
        assert_eq!(blind.to_bytes(), b"hunter2");
    }

    #[test]
    fn a_conversation_ending_leaves_no_credential_behind() {
        let mut conversation = LoginConversation {
            username: "alice".to_owned(),
            credential: Secret::from_bytes(b"hunter2"),
        };

        conversation.credential.clear();

        assert!(conversation.credential.is_empty());
        assert_eq!(conversation.credential.expose(), b"");
    }

    #[test]
    fn the_session_conversation_answers_the_account_and_no_prompt_else() {
        let mut conversation = SessionConversation {
            username: "alice".to_owned(),
        };
        let prompt = CString::new("login: ").expect("a literal");

        assert_eq!(
            conversation
                .prompt_echo(&prompt)
                .expect("the account is an answer")
                .to_bytes(),
            b"alice"
        );
        assert!(
            conversation.prompt_blind(&prompt).is_err(),
            "the session leader holds no credential and must not invent one"
        );
    }

    #[test]
    fn the_default_service_is_this_projects_own_pam_stack() {
        // Named so that a reader of the code and a reader of
        // `packaging/pam.d/inox-auth` see the same string.
        assert_eq!(PamBoundary::new("inox-auth").service, "inox-auth");
    }
}
