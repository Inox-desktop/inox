//! The projection from greeter state to what the login interface draws.
//!
//! Everything a person reads on the login screen is decided here and only here.
//! The labels come from the Fluent catalogs, a control that cannot be operated
//! carries the reason it cannot, and the lifecycle the session goes through —
//! unavailable, ready, checking, starting, failed, started — is one variant of
//! [`Face`]. The view then has no decisions left: it walks this value and draws
//! it.
//!
//! Three things are deliberate and are what a reviewer should check.
//!
//! - **The identity is an account, not a string.** The screen draws the accounts
//!   the system offers ([`crate::accounts`]), one of them marked as the one a
//!   login would use, and an account list is what the person operates
//!   ([`GreeterViewModel::accounts`]) rather than a text field. There is therefore
//!   no "the name you typed is wrong" state: what is left of that idea is the
//!   contract's own grammar, checked here so that an account whose name the
//!   boundary would refuse is refused on screen with a reason
//!   ([`Action::disabled_reason`]).
//! - **The interface never claims more than the model knows.** [`Phase::Failed`]
//!   covers a refused account and a session that would not start, so [`Face::Failed`]
//!   carries one line and invents no reason; the specific reason a control cannot
//!   be used is derived from the same state the control is disabled by.
//! - **What is drawn is what can be operated.** [`GreeterViewModel::focus_order`]
//!   lists only controls that are enabled, so "the keyboard focus is visible,
//!   unique and cannot be trapped" (acceptance Q01) holds by construction rather
//!   than by a rule the view has to remember.
//!
//! And one thing that is a *count* rather than a value: the credential. The
//! projection is handed the number of characters the model is holding and builds a
//! mask from it, so a value that is printable-in-principle (this type derives
//! `Debug`, and every test prints it on failure) can never carry a password into a
//! log, a panic message or a screenshot.
//!
//! The module is pure: state in, value out. No iced, no clock, no I/O, so every
//! lifecycle above is a unit test.

use inox_common::i18n::Localizer;

use crate::accounts::Account;
use crate::state::{Phase, valid_username};

/// The character a credential is drawn with.
///
/// One glyph for every locale, deliberately: what a password looks like on screen
/// is not something a translation may change, and a translator who changed it would
/// be changing how much of a password is visible to whoever is standing behind the
/// person typing it.
pub const MASK_GLYPH: char = '•';

/// A control the keyboard can reach (acceptance Q01).
///
/// The order of [`GreeterViewModel::focus_order`] is the order Tab visits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// The account at this index of the account list, while the list is open.
    Account(u16),
    /// The password field.
    Credential,
    /// The log-in button.
    Submit,
    /// The control that opens the account list.
    SwitchUser,
    /// The control that clears the password — and, while the account list is
    /// open, the one that closes it again.
    Cancel,
}

impl FocusTarget {
    /// The focus on the account at `index` of the account list.
    ///
    /// One constructor for one arithmetic decision, so that the model that lists
    /// the accounts and the view that draws them cannot disagree about which index
    /// a row is.
    pub fn account(index: usize) -> Self {
        Self::Account(u16::try_from(index).unwrap_or(u16::MAX))
    }
}

/// A labelled control, and whether it can be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// What the control says.
    pub label: String,
    /// Whether it can be operated.
    pub enabled: bool,
    /// Why it cannot, when it cannot (acceptance D05: the reason is readable,
    /// not discovered by pressing).
    pub disabled_reason: Option<String>,
}

/// One account, as the screen draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountView {
    /// The login name: what the boundary is asked to authenticate.
    pub name: String,
    /// What the name line and the account row show.
    pub label: String,
    /// The one character drawn on the medallion.
    pub initial: String,
    /// Whether this is the account a login would use.
    pub selected: bool,
}

/// The credential field.
///
/// The value is absent by construction: what the view draws is [`Credential::mask`],
/// which is derived from the credential's length and carries none of its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// The field's label, which is also its accessible name.
    pub label: String,
    /// What the empty field says.
    pub placeholder: String,
    /// One glyph per character typed.
    pub mask: String,
    /// What is wrong with it, when something is.
    pub error: Option<String>,
    /// Whether the field accepts input.
    pub editable: bool,
    /// Why it does not, when it does not.
    pub disabled_reason: Option<String>,
}

/// The state under the account name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Face {
    /// No reviewed authentication transport is connected: the screen can collect
    /// nothing and check nothing.
    Unavailable {
        /// The headline.
        line: String,
        /// Why, in the user's language.
        reason: String,
    },
    /// A password may be entered. The resting state.
    Prompt,
    /// An attempt is with the authenticator. The loading state of D05.
    Authenticating {
        /// What the screen says while it waits.
        line: String,
    },
    /// Authentication succeeded and the session is being launched.
    Starting {
        /// What the screen says while it waits.
        line: String,
    },
    /// The last attempt was not accepted. The error state of D05.
    Failed {
        /// The headline.
        line: String,
    },
    /// The display manager confirmed that the session started.
    Started {
        /// The line the user reads last.
        line: String,
    },
}

/// What the login interface draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GreeterViewModel {
    /// The name of the surface, and of the component.
    pub title: String,
    /// The account a login would use, when the system offers one.
    pub selected: Option<AccountView>,
    /// Every account the system offers, in the order it is offered.
    pub accounts: Vec<AccountView>,
    /// Whether the account list is what the person is looking at.
    pub choosing: bool,
    /// What the account list says it is.
    pub accounts_heading: String,
    /// The credential field.
    pub credential: Credential,
    /// What the screen says about this build, under the account name.
    pub banner: String,
    /// The label of the session the login would start.
    pub session_label: String,
    /// The session the login would start.
    pub session: String,
    /// The state under the account name.
    pub face: Face,
    /// The log-in control.
    pub submit: Action,
    /// The control that opens the account list, where there is one to open.
    pub switch_user: Option<Action>,
    /// The control that clears the password, or closes the account list.
    pub cancel: Action,
    /// Why there is nobody to log in as, when the system offered no account.
    ///
    /// Drawn beside the account name rather than beside a control, because it is
    /// about the account the screen would have drawn there.
    pub accounts_reason: Option<String>,
    /// The one line under the controls that says what else is in the way.
    ///
    /// Never a second copy of [`GreeterViewModel::accounts_reason`] or of the
    /// face: those are already on screen, and the same sentence printed twice
    /// reads as two problems.
    pub notice: Option<String>,
    /// The keyboard hint, when the keyboard has something to do here.
    pub hint: Option<String>,
    /// The controls the keyboard can reach, in tab order.
    pub focus_order: Vec<FocusTarget>,
    /// Where the keyboard focus is, when it is on one of them.
    pub focus: Option<FocusTarget>,
}

/// Everything the projection reads.
///
/// A borrow of the pieces rather than of [`crate::app::State`], so the projection
/// stays testable without a state to build and so it cannot reach for anything
/// the view is not meant to see.
#[derive(Debug, Clone, Copy)]
pub struct Projection<'a> {
    /// Where the login lifecycle is.
    pub phase: Phase,
    /// Whether a reviewed authentication transport is connected at all.
    pub available: bool,
    /// The accounts the system offers.
    pub accounts: &'a [Account],
    /// Whether the account database could not be read at all.
    pub unreadable: bool,
    /// The login name of the account a login would use.
    pub selected: Option<&'a str>,
    /// Whether the last identity supplied was cut down to [`crate::state::MAX_IDENTITY_BYTES`].
    pub identity_too_long: bool,
    /// How many characters of credential the model is holding.
    ///
    /// A count, not a value: this is the whole of what the interface may know about
    /// a password.
    pub credential_chars: usize,
    /// Whether the credential is longer than the contract will carry.
    pub credential_too_long: bool,
    /// Whether an attempt may be submitted right now.
    pub can_submit: bool,
    /// Where the keyboard focus is.
    pub focus: Option<FocusTarget>,
    /// Whether the person asked to see the account list.
    pub choosing: bool,
    /// The catalogs.
    pub localizer: &'a Localizer,
}

/// Projects the state into what the view draws.
pub fn project(input: Projection<'_>) -> GreeterViewModel {
    let message = |id: &str| input.localizer.message(id);

    let accounts: Vec<AccountView> = input
        .accounts
        .iter()
        .map(|account| AccountView {
            name: account.name().to_owned(),
            label: account.display().to_owned(),
            initial: account.initial(),
            selected: input.selected == Some(account.name()),
        })
        .collect();

    let selected = accounts.iter().find(|account| account.selected).cloned();

    // Why the screen has nobody to authenticate, when it has nobody. It comes
    // before every other reason because nothing else on the screen can be done
    // about it.
    let accounts_reason = nobody_reason(&input).map(message);

    // The account list is only worth showing where there is a choice and the
    // screen is collecting a login at all.
    let choosing = input.choosing
        && accounts.len() > 1
        && matches!(input.phase, Phase::Ready | Phase::Failed)
        && input.available
        && accounts_reason.is_none();

    let editable = input.available
        && accounts_reason.is_none()
        && matches!(input.phase, Phase::Ready | Phase::Failed);

    let credential = Credential {
        label: message("greet-credential-label"),
        placeholder: message("greet-credential-placeholder"),
        mask: MASK_GLYPH.to_string().repeat(input.credential_chars),
        error: credential_error(&input).map(message),
        editable,
        disabled_reason: credential_disabled_reason(&input),
    };

    // The log-in control is enabled exactly when the model would accept a
    // submission, so "the control is enabled" and "pressing it does something"
    // are the same fact rather than two that could disagree. The phase is checked
    // here as well as in `can_submit` on purpose: the projection is a total
    // function of what it is handed, and a caller that passed an optimistic
    // `can_submit` for a screen that is already checking an attempt would
    // otherwise get a model with an enabled button that submits nothing.
    let can_submit = input.can_submit && editable;

    let submit = Action {
        label: message("greet-submit"),
        enabled: can_submit,
        disabled_reason: if can_submit {
            None
        } else {
            credential
                .disabled_reason
                .clone()
                .or_else(|| submit_reason(&input).map(message))
        },
    };

    // Switching accounts is offered where there is a choice to make. A screen
    // with one account has nothing to switch to, and a screen that is checking
    // an attempt has no business offering a different name to check.
    let switch_user = (editable && accounts.len() > 1).then(|| Action {
        label: message("greet-switch-user"),
        enabled: true,
        disabled_reason: None,
    });

    // Clearing is meaningful in every state but the one where the session has
    // already started: after that there is no credential left to clear and the
    // model would refuse the event anyway. While the account list is open, the
    // same control is the one that closes it, and says so.
    let cancel_enabled = input.phase != Phase::SessionStarted;
    let cancel = Action {
        label: if choosing {
            message("greet-back")
        } else {
            message("greet-clear")
        },
        enabled: cancel_enabled,
        disabled_reason: if cancel_enabled {
            None
        } else {
            Some(message("greet-disabled-started"))
        },
    };

    let mut focus_order = Vec::new();

    if choosing {
        focus_order.extend((0..accounts.len()).map(FocusTarget::account));
    } else {
        if credential.editable {
            focus_order.push(FocusTarget::Credential);
        }
        if submit.enabled {
            focus_order.push(FocusTarget::Submit);
        }
        if switch_user.is_some() {
            focus_order.push(FocusTarget::SwitchUser);
        }
    }

    if cancel.enabled {
        focus_order.push(FocusTarget::Cancel);
    }

    // The focus is always on something reachable: a screen whose focus had been
    // left on a control that has just become disabled would have no focus at all,
    // and a keyboard user would be stranded (acceptance Q01).
    let focus = input
        .focus
        .filter(|target| focus_order.contains(target))
        .or_else(|| focus_order.first().copied());

    // One line, not three: the account problem is beside the account, the face is
    // the state of the login itself, and what is left for this line is the reason
    // nothing else says — the name the boundary would refuse, or the spent
    // allowance.
    let notice = if accounts_reason.is_some()
        || !matches!(input.phase, Phase::Ready | Phase::Failed)
        || !input.available
    {
        None
    } else {
        submit_reason(&input).map(message)
    };

    GreeterViewModel {
        title: message("greet-window-title"),
        selected,
        accounts,
        choosing,
        accounts_heading: message("greet-choose-account"),
        credential,
        // The banner is the screen's statement about *this build*. A build with a
        // boundary is not a preview and must not say it is: the notice that no
        // credential is collected would be false exactly when collecting one matters.
        banner: if input.available {
            message("greet-banner-live")
        } else {
            message("greet-banner")
        },
        session_label: message("greet-session-label"),
        session: message("greet-session-name"),
        face: face(&input),
        submit,
        switch_user,
        cancel,
        accounts_reason,
        notice,
        hint: hint(&input, can_submit, cancel_enabled, focus, choosing),
        focus_order,
        focus,
    }
}

/// What the keyboard can actually do where the focus is.
///
/// A hint is a promise about a key. Promising "Enter logs in" on a screen whose
/// log-in control is disabled — or while the ring is on the clear control, which is
/// where Escape clears — is the same mistake as a prompt over a field that cannot
/// be typed into: it describes an interface that is not there. `focus` is the one
/// the projection settled on, not the one that was asked for, so a hint can never
/// be about a control the screen is not showing as focused.
fn hint(
    input: &Projection<'_>,
    submit: bool,
    cancel: bool,
    focus: Option<FocusTarget>,
    choosing: bool,
) -> Option<String> {
    if choosing {
        // The account list is operated with the arrow keys every list is operated
        // with, and every row says what it is; a sentence about Enter would be a
        // sentence about the obvious.
        return None;
    }

    let message = |id: &str| input.localizer.message(id);

    match (submit, cancel, focus) {
        // Enter does what the focus does, and here that is clearing.
        (true, _, Some(FocusTarget::Cancel)) => Some(message("greet-hint-clear")),
        (true, _, _) => Some(message("greet-hint")),
        (false, true, _) => Some(message("greet-hint-clear")),
        (false, false, _) => None,
    }
}

/// The state under the account name.
fn face(input: &Projection<'_>) -> Face {
    let message = |id: &str| input.localizer.message(id);

    match input.phase {
        // Nothing can be checked at all, so that comes first: the state of the
        // authenticator is not even known yet.
        Phase::Unavailable => Face::Unavailable {
            line: message("greet-unavailable"),
            reason: message("greet-blocked"),
        },
        Phase::Ready => Face::Prompt,
        Phase::Authenticating => Face::Authenticating {
            line: message("greet-checking"),
        },
        Phase::Starting => Face::Starting {
            line: message("greet-starting"),
        },
        Phase::Failed => Face::Failed {
            line: message("greet-failed"),
        },
        Phase::SessionStarted => Face::Started {
            line: message("greet-started"),
        },
    }
}

/// Why there is nobody to log in as, when there is nobody.
///
/// An unreadable database, an empty one and a list nothing was selected from are
/// three different facts, and the person can act on only the last of them, so the
/// screen says which one it is.
fn nobody_reason(input: &Projection<'_>) -> Option<&'static str> {
    if input.unreadable {
        return Some("greet-disabled-accounts-unreadable");
    }

    if input.accounts.is_empty() {
        return Some("greet-disabled-no-accounts");
    }

    if input.selected.is_none() {
        return Some("greet-disabled-account-unselected");
    }

    None
}

/// What is wrong with the credential, when something is.
///
/// Empty is not an error: an account may have no password, and a screen that
/// refused to submit an empty field would lock those accounts out. Over-long *is*
/// an error, and it is reported rather than truncated.
fn credential_error(input: &Projection<'_>) -> Option<&'static str> {
    if input.credential_too_long {
        return Some("greet-credential-too-long");
    }

    None
}

/// Why the credential field cannot be typed into, when it cannot.
fn credential_disabled_reason(input: &Projection<'_>) -> Option<String> {
    let message = |id: &str| input.localizer.message(id);

    if let Some(reason) = nobody_reason(input) {
        return Some(message(reason));
    }

    if !input.available {
        return Some(message("greet-disabled-unavailable"));
    }

    match input.phase {
        Phase::Unavailable => Some(message("greet-disabled-unavailable")),
        Phase::Authenticating => Some(message("greet-disabled-verifying")),
        Phase::Starting | Phase::SessionStarted => Some(message("greet-disabled-started")),
        Phase::Ready | Phase::Failed => None,
    }
}

/// Why the log-in control cannot be used, over and above the credential field.
///
/// The reasons come in the order a user meets them: the name the boundary would
/// not carry, the name it would not accept, and the rolling allowance. An account
/// this build cannot send is a control the person cannot use, and saying which of
/// the three it is is the difference between a grey button and a fixable one.
fn submit_reason(input: &Projection<'_>) -> Option<&'static str> {
    if input.identity_too_long {
        return Some("greet-identity-too-long");
    }

    if input.accounts.is_empty() || input.selected.is_none() {
        return None;
    }

    if !valid_username(input.selected.unwrap_or_default()) {
        return Some("greet-disabled-invalid");
    }

    // Everything above has been checked, so a `can_submit` that is false for a
    // typable screen can only be the rolling allowance.
    if !input.can_submit {
        return Some("greet-disabled-rate-limited");
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::Accounts;
    use crate::i18n;
    use crate::state::MAX_IDENTITY_BYTES;
    use inox_common::i18n::Locale;

    fn localizer() -> Localizer {
        i18n::load(&Locale::default_locale()).expect("the catalogs parse")
    }

    /// The accounts a test offers.
    fn accounts() -> Vec<Account> {
        vec![
            Account::new("alice", 1000, Some("Alice Doe".to_owned())),
            Account::new("bob", 1001, None),
        ]
    }

    /// A projection of an available authenticator, alice selected, and no attempt.
    fn ready<'a>(localizer: &'a Localizer, accounts: &'a [Account]) -> Projection<'a> {
        Projection {
            phase: Phase::Ready,
            available: true,
            accounts,
            unreadable: false,
            selected: Some("alice"),
            identity_too_long: false,
            credential_chars: 0,
            credential_too_long: false,
            can_submit: true,
            focus: None,
            choosing: false,
            localizer,
        }
    }

    #[test]
    fn a_ready_screen_prompts_and_offers_the_controls_it_can() {
        let localizer = localizer();
        let accounts = accounts();

        let model = project(ready(&localizer, &accounts));

        assert_eq!(model.title, "Inox login");
        assert_eq!(
            model
                .selected
                .as_ref()
                .map(|account| account.label.as_str()),
            Some("Alice Doe"),
            "the screen names the account a login would use"
        );
        assert_eq!(
            model
                .selected
                .as_ref()
                .map(|account| account.initial.as_str()),
            Some("A")
        );
        assert_eq!(model.credential.label, "Password");
        assert!(model.credential.editable);
        assert_eq!(model.face, Face::Prompt);
        assert!(model.submit.enabled);
        assert!(model.cancel.enabled);
        assert!(
            model.switch_user.is_some(),
            "two accounts are two accounts to choose between"
        );
        assert_eq!(
            model.focus_order,
            vec![
                FocusTarget::Credential,
                FocusTarget::Submit,
                FocusTarget::SwitchUser,
                FocusTarget::Cancel
            ]
        );
        assert_eq!(
            model.focus,
            Some(FocusTarget::Credential),
            "the password field is what the person types into, so it is where the \
             keyboard starts, and a screen with no focus at all is a screen a \
             keyboard cannot use"
        );
        assert!(
            model
                .accounts
                .iter()
                .any(|account| account.selected && account.name == "alice"),
            "the account the model would authenticate is the one marked selected"
        );
    }

    #[test]
    fn one_account_is_not_a_choice() {
        let localizer = localizer();
        let single = vec![Account::new("alice", 1000, None)];

        let model = project(ready(&localizer, &single));

        assert_eq!(model.accounts.len(), 1);
        assert!(
            model.switch_user.is_none(),
            "there is nothing to switch to, so nothing offers a switch"
        );
        assert!(!model.focus_order.contains(&FocusTarget::SwitchUser));
        assert_eq!(
            model.focus_order,
            vec![
                FocusTarget::Credential,
                FocusTarget::Submit,
                FocusTarget::Cancel
            ]
        );
    }

    #[test]
    fn the_account_list_is_a_list_of_reachable_rows_and_a_way_back() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.choosing = true;

        let model = project(input);

        assert!(model.choosing);
        assert_eq!(model.accounts_heading, "Choose an account");
        assert_eq!(
            model.focus_order,
            vec![
                FocusTarget::Account(0),
                FocusTarget::Account(1),
                FocusTarget::Cancel
            ],
            "the rows are reachable with the keyboard, and so is the way back"
        );
        assert_eq!(model.focus, Some(FocusTarget::Account(0)));
        assert_eq!(
            model.cancel.label, "Back",
            "while the list is open the one control left says what it does"
        );
        assert!(
            !model.focus_order.contains(&FocusTarget::Credential),
            "the password field is not what is on screen, so it is not what Tab \
             should reach"
        );
    }

    #[test]
    fn the_account_list_cannot_be_opened_where_there_is_nothing_to_choose() {
        let localizer = localizer();
        let single = [Account::new("alice", 1000, None)];
        let none: [Account; 0] = [];

        for (label, accounts) in [("one account", &single[..]), ("no accounts", &none[..])] {
            let mut input = ready(&localizer, accounts);
            input.choosing = true;
            let model = project(input);

            assert!(!model.choosing, "{label}: the list stays closed");
            assert!(model.switch_user.is_none(), "{label}");
        }
    }

    #[test]
    fn the_interface_never_claims_more_than_the_model_knows() {
        let localizer = localizer();
        let accounts = accounts();
        let phases = [
            Phase::Unavailable,
            Phase::Ready,
            Phase::Authenticating,
            Phase::Starting,
            Phase::Failed,
            Phase::SessionStarted,
        ];

        for phase in phases {
            for available in [false, true] {
                for selected in [Some("alice"), None] {
                    for can_submit in [false, true] {
                        let mut input = ready(&localizer, &accounts);
                        input.phase = phase;
                        input.available = available;
                        input.selected = selected;
                        input.can_submit = can_submit;

                        let model = project(input);

                        // Every string is a message, never a message id.
                        for line in [
                            model.credential.label.clone(),
                            model.credential.placeholder.clone(),
                            model.hint.clone().unwrap_or_default(),
                            model.submit.label.clone(),
                            model.cancel.label.clone(),
                        ]
                        .into_iter()
                        .chain(model.credential.error.clone())
                        .chain(model.credential.disabled_reason.clone())
                        .chain(model.submit.disabled_reason.clone())
                        .chain(model.cancel.disabled_reason.clone())
                        .chain(model.accounts_reason.clone())
                        .chain(model.notice.clone())
                        {
                            assert!(
                                !line.starts_with("greet-") && !line.starts_with("lock-"),
                                "`{line}` is a message id, not a message"
                            );
                        }

                        if !model.submit.enabled {
                            assert!(
                                model.submit.disabled_reason.is_some(),
                                "{phase:?} disables the log-in control with no reason"
                            );
                        } else {
                            assert!(model.submit.disabled_reason.is_none());
                        }

                        if !model.credential.editable {
                            assert!(
                                model.credential.disabled_reason.is_some(),
                                "{phase:?} disables the credential field with no reason"
                            );
                        }

                        assert!(
                            model
                                .focus
                                .is_none_or(|target| model.focus_order.contains(&target)),
                            "{phase:?} leaves the focus on a control that cannot be used"
                        );

                        // The account problem is said once: in the header when
                        // there is a problem, and never twice.
                        if model.accounts_reason.is_some() {
                            assert_eq!(model.notice, None);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_hint_never_promises_a_key_that_does_nothing() {
        let localizer = localizer();
        let accounts = accounts();

        // The shipped state: nothing can be submitted, so the hint is only about
        // what is left.
        let mut input = ready(&localizer, &accounts);
        input.phase = Phase::Unavailable;
        input.available = false;
        input.can_submit = false;

        let model = project(input);
        let hint = model.hint.clone().expect("clearing is still possible");

        assert!(
            !hint.contains("log in"),
            "`{hint}` promises a login on a screen that cannot log anyone in"
        );
        assert!(hint.contains("Escape"), "{hint}");

        // And a hint about Enter follows the ring: on the clear control, Enter
        // clears, so the hint must not mention logging in.
        let mut focused_on_clear = ready(&localizer, &accounts);
        focused_on_clear.focus = Some(FocusTarget::Cancel);

        let model = project(focused_on_clear);
        let hint = model.hint.clone().expect("there is still something to do");

        assert_eq!(model.focus, Some(FocusTarget::Cancel));
        assert!(
            !hint.contains("log in"),
            "`{hint}` says Enter logs in while the ring is on the control Enter clears"
        );

        // A screen where nothing at all can be done has no hint.
        let mut done = ready(&localizer, &accounts);
        done.phase = Phase::SessionStarted;
        done.can_submit = false;

        assert_eq!(project(done).hint, None);
    }

    #[test]
    fn an_unavailable_authenticator_takes_no_input_and_says_why() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.phase = Phase::Unavailable;
        input.available = false;
        input.can_submit = false;

        let model = project(input);

        let Face::Unavailable { line, reason } = &model.face else {
            panic!("an unavailable authenticator is the unavailable face");
        };
        assert_eq!(line, "Log in cannot run");
        assert!(
            reason.contains("authentication boundary"),
            "the reason has to name what is missing: {reason}"
        );
        assert!(!model.credential.editable);
        assert!(!model.submit.enabled);
        assert_eq!(
            model.credential.disabled_reason.as_deref(),
            Some("The system authentication service is not available")
        );
        assert_eq!(
            model.focus_order,
            vec![FocusTarget::Cancel],
            "clearing is the one thing left to do, so it is the one thing focused"
        );
        assert_eq!(
            model.accounts_reason, None,
            "the accounts are fine; it is the boundary that is missing"
        );
    }

    #[test]
    fn an_empty_account_list_is_said_beside_the_account_name() {
        let localizer = localizer();
        let mut input = ready(&localizer, &[]);
        input.can_submit = false;

        let model = project(input);

        assert_eq!(model.selected, None);
        assert_eq!(
            model.accounts_reason.as_deref(),
            Some("No account on this system can be logged in")
        );
        assert!(!model.credential.editable);
        assert!(!model.submit.enabled);
        assert_eq!(
            model.submit.disabled_reason, model.accounts_reason,
            "the control that cannot be used says the same thing the screen says"
        );
        assert_eq!(
            model.focus_order,
            vec![FocusTarget::Cancel],
            "clearing is the one thing left, so the keyboard is not stranded"
        );
    }

    #[test]
    fn an_unreadable_account_database_says_which_problem_it_is() {
        let localizer = localizer();
        let mut input = ready(&localizer, &[]);
        input.unreadable = true;
        input.can_submit = false;

        let model = project(input);

        assert_eq!(
            model.accounts_reason.as_deref(),
            Some("The list of accounts could not be read"),
            "an empty list and an unreadable database are two different facts"
        );
    }

    #[test]
    fn a_name_the_boundary_would_refuse_disables_the_button_with_a_reason() {
        let localizer = localizer();
        let odd = vec![Account::new("a b", 1000, None)];
        let mut input = ready(&localizer, &odd);
        input.selected = Some("a b");
        input.can_submit = false;

        let model = project(input);

        assert_eq!(model.accounts_reason, None);
        assert!(!model.submit.enabled);
        assert_eq!(
            model.submit.disabled_reason.as_deref(),
            Some("That account name cannot be sent to the authentication service"),
            "the boundary's grammar is the screen's grammar"
        );
        assert_eq!(
            model.notice, model.submit.disabled_reason,
            "the reason is on screen, once"
        );
    }

    #[test]
    fn an_attempt_in_flight_is_the_loading_state() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.phase = Phase::Authenticating;
        input.can_submit = false;

        let model = project(input);

        assert_eq!(
            model.face,
            Face::Authenticating {
                line: "Checking your account…".to_owned()
            }
        );
        assert!(!model.credential.editable);
        assert!(!model.submit.enabled);
        assert!(
            model.switch_user.is_none(),
            "a login being checked is not a moment to offer a different account"
        );
        assert!(
            model.cancel.enabled,
            "a login that cannot be abandoned is a login that traps the user"
        );
        assert_eq!(
            model.focus_order,
            vec![FocusTarget::Cancel],
            "the field and the button are gone, and the focus moved with them"
        );
    }

    #[test]
    fn a_starting_session_cannot_be_pressed_twice_or_cleared() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.phase = Phase::Starting;

        let model = project(input);

        assert!(matches!(model.face, Face::Starting { .. }));
        assert!(!model.submit.enabled);
        assert!(!model.credential.editable);
        assert!(model.cancel.enabled, "the session is still only starting");
        assert_eq!(
            model.submit.disabled_reason.as_deref(),
            Some("The session is already starting")
        );
    }

    #[test]
    fn a_started_session_leaves_nothing_to_operate() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.phase = Phase::SessionStarted;

        let model = project(input);

        assert_eq!(
            model.face,
            Face::Started {
                line: "Your session is starting".to_owned()
            }
        );
        assert!(!model.cancel.enabled);
        assert!(model.focus_order.is_empty());
        assert_eq!(model.focus, None);
    }

    #[test]
    fn a_refused_attempt_is_retryable_and_invents_no_reason() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.phase = Phase::Failed;
        input.can_submit = true;

        let model = project(input);

        assert_eq!(
            model.face,
            Face::Failed {
                line: "That login was not accepted".to_owned()
            }
        );
        assert!(model.credential.editable);
        assert!(model.submit.enabled);
        assert_eq!(
            model.notice, None,
            "a refusal is the face; the controls can be used"
        );
    }

    #[test]
    fn a_cut_down_identity_says_so_instead_of_submitting_what_was_not_offered() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.identity_too_long = true;
        input.can_submit = false;

        let model = project(input);

        assert_eq!(
            model.submit.disabled_reason.as_deref(),
            Some("That account name is too long.")
        );
        assert_eq!(model.notice, model.submit.disabled_reason);
    }

    #[test]
    fn a_spent_allowance_says_a_wait_and_not_how_long() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.can_submit = false;

        let model = project(input);

        assert!(!model.submit.enabled);
        assert_eq!(
            model.submit.disabled_reason.as_deref(),
            Some("Too many attempts for now")
        );
    }

    #[test]
    fn no_catalog_message_is_a_sentence_about_a_clock() {
        // The rolling allowance is the model's own business, and a screen that
        // counted its own seconds down would be telling whoever is guessing at
        // the limit how much budget is left (the same rule the lock screen's
        // limiter follows).
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.can_submit = false;

        let model = project(input);
        let reason = model.submit.disabled_reason.expect("a reason");

        assert!(
            !reason.chars().any(|character| character.is_ascii_digit()),
            "`{reason}` counts down"
        );
    }

    #[test]
    fn the_focus_falls_back_when_the_focused_control_is_disabled() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.focus = Some(FocusTarget::Submit);

        let still_there = project(input);
        assert_eq!(still_there.focus, Some(FocusTarget::Submit));

        // The authenticity became unavailable, which takes the button away.
        input.available = false;
        input.can_submit = false;
        input.phase = Phase::Unavailable;

        let moved = project(input);
        assert_eq!(moved.focus, Some(FocusTarget::Cancel));
    }

    #[test]
    fn the_mask_counts_the_credential_and_never_carries_it() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.credential_chars = 7;

        let model = project(input);

        assert_eq!(model.credential.mask.chars().count(), 7);
        assert!(
            model
                .credential
                .mask
                .chars()
                .all(|glyph| glyph == MASK_GLYPH)
        );
        assert_eq!(model.credential.error, None);
    }

    #[test]
    fn a_credential_past_the_contracts_bound_is_an_error() {
        let localizer = localizer();
        let accounts = accounts();
        let mut input = ready(&localizer, &accounts);
        input.credential_too_long = true;
        input.can_submit = false;

        let model = project(input);

        assert_eq!(
            model.credential.error.as_deref(),
            Some("That password is too long.")
        );
        assert!(!model.submit.enabled);
    }

    #[test]
    fn the_longest_identity_the_model_keeps_is_the_one_the_screen_names() {
        let localizer = localizer();
        let username = "a".repeat(MAX_IDENTITY_BYTES);
        let long = vec![Account::new(username.clone(), 1000, None)];
        let mut input = ready(&localizer, &long);
        input.selected = Some(&username);

        let model = project(input);

        assert_eq!(
            model.selected.as_ref().map(|account| account.name.clone()),
            Some(username)
        );
        assert_eq!(model.accounts_reason, None);
    }

    #[test]
    fn the_roster_the_screen_draws_is_the_one_the_model_holds() {
        // The list the view draws and the list the model authenticates from are
        // the same list: the projection cannot invent an account, and it cannot
        // drop one.
        let localizer = localizer();
        let roster = Accounts {
            accounts: accounts(),
            ..Accounts::default()
        };

        let model = project(Projection {
            selected: roster.selected().map(|index| roster.accounts[index].name()),
            accounts: &roster.accounts,
            ..ready(&localizer, &roster.accounts)
        });

        assert_eq!(model.accounts.len(), roster.accounts.len());
        assert_eq!(
            model.selected.as_ref().map(|account| account.name.as_str()),
            Some("alice")
        );
    }
}
