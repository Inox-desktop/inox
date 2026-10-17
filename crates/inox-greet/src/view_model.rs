//! The projection from greeter state to what the login interface draws.
//!
//! Everything a person reads on the login screen is decided here and only here.
//! The labels come from the Fluent catalogs, a control that cannot be operated
//! carries the reason it cannot, and the lifecycle the session goes through —
//! unavailable, ready, checking, starting, failed, started — is one variant of
//! [`Face`]. The view then has no decisions left: it walks this value and draws
//! it.
//!
//! Two things are deliberate and are what a reviewer should check.
//!
//! - **The interface never claims more than the model knows.** [`Phase::Failed`]
//!   covers a refused account and a session that would not start, so [`Face::Failed`]
//!   carries one line and invents no reason; the specific reason a control cannot
//!   be used is [`Action::disabled_reason`], which is derived from the same state
//!   the control is disabled by.
//! - **What is drawn is what can be operated.** [`GreeterViewModel::focus_order`]
//!   lists only controls that are enabled, so "the keyboard focus is visible,
//!   unique and cannot be trapped" (acceptance Q01) holds by construction rather
//!   than by a rule the view has to remember.
//!
//! The module is pure: state in, value out. No iced, no clock, no I/O, so every
//! lifecycle above is a unit test.

use inox_common::i18n::Localizer;

use crate::state::{Phase, valid_username};

/// A control the keyboard can reach (acceptance Q01).
///
/// The order of [`GreeterViewModel::focus_order`] is the order Tab visits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// The username field.
    Identity,
    /// The log-in button.
    Submit,
    /// The clear button.
    Cancel,
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

/// The username field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The field's label, which is also its accessible name.
    pub label: String,
    /// What the empty field says.
    pub placeholder: String,
    /// What has been typed.
    pub value: String,
    /// What is wrong with it, when something is.
    pub error: Option<String>,
    /// Whether the field accepts input.
    pub editable: bool,
    /// Why it does not, when it does not.
    pub disabled_reason: Option<String>,
}

/// The state under the identity field.
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
    /// A username may be entered. The resting state.
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
    /// The question the screen asks.
    pub headline: String,
    /// The username field.
    pub identity: Identity,
    /// The label of the session the login would start.
    pub session_label: String,
    /// The session the login would start.
    pub session: String,
    /// What the screen says about this build, above everything else.
    pub banner: String,
    /// The state under the identity field.
    pub face: Face,
    /// The log-in control.
    pub submit: Action,
    /// The clear control.
    pub cancel: Action,
    /// The one line under the form that says why it cannot be used.
    ///
    /// The field's reason when the field is not typable, and otherwise the
    /// log-in button's: the two are the same fact seen from two controls, and a
    /// screen that printed it twice would read as two problems.
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
    /// What has been typed into the identity field.
    pub username: &'a str,
    /// Whether the last identity supplied was cut down to [`crate::state::MAX_IDENTITY_BYTES`].
    pub identity_too_long: bool,
    /// Whether an attempt may be submitted right now.
    pub can_submit: bool,
    /// Where the keyboard focus is.
    pub focus: Option<FocusTarget>,
    /// The catalogs.
    pub localizer: &'a Localizer,
}

/// Projects the state into what the view draws.
pub fn project(input: Projection<'_>) -> GreeterViewModel {
    let message = |id: &str| input.localizer.message(id);

    let editable = input.available && matches!(input.phase, Phase::Ready | Phase::Failed);
    let identity_disabled_reason = disabled_reason(&input, true);

    let identity = Identity {
        label: message("greet-field-label"),
        placeholder: message("greet-placeholder"),
        value: input.username.to_owned(),
        error: identity_error(&input).map(message),
        editable,
        disabled_reason: identity_disabled_reason,
    };

    // The log-in control is enabled exactly when the model would accept a
    // submission, so "the control is enabled" and "pressing it does something"
    // are the same fact rather than two that could disagree. The phase is checked
    // here as well as in `can_submit` on purpose: the projection is a total
    // function of what it is handed, and a caller that passed an optimistic
    // `can_submit` for a screen that is already checking an attempt would
    // otherwise get a model with an enabled button that submits nothing.
    let can_submit = input.can_submit && matches!(input.phase, Phase::Ready | Phase::Failed);

    let submit = Action {
        label: message("greet-submit"),
        enabled: can_submit,
        disabled_reason: if can_submit {
            None
        } else {
            disabled_reason(&input, false)
        },
    };

    // Clearing is meaningful in every state but the one where the session has
    // already started: after that there is no identity left to clear and the
    // model would refuse the event anyway.
    let cancel_enabled = input.phase != Phase::SessionStarted;
    let cancel = Action {
        label: message("greet-clear"),
        enabled: cancel_enabled,
        disabled_reason: if cancel_enabled {
            None
        } else {
            Some(message("greet-disabled-started"))
        },
    };

    let mut focus_order = Vec::new();
    if identity.editable {
        focus_order.push(FocusTarget::Identity);
    }
    if submit.enabled {
        focus_order.push(FocusTarget::Submit);
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

    let notice = identity.disabled_reason.clone().or_else(|| {
        // The field already says what is wrong with the name; printing the same
        // sentence again in the warning colour under the form would read as two
        // problems rather than one.
        if identity.error.is_some() {
            None
        } else {
            submit.disabled_reason.clone()
        }
    });

    GreeterViewModel {
        title: message("greet-window-title"),
        headline: message("greet-headline"),
        identity,
        session_label: message("greet-session-label"),
        session: message("greet-session-name"),
        banner: message("greet-banner"),
        face: face(&input),
        submit,
        cancel,
        notice,
        hint: hint(&input, can_submit, cancel_enabled, focus),
        focus_order,
        focus,
    }
}

/// What the keyboard can actually do where the focus is.
///
/// A hint is a promise about a key. Promising "Enter logs in" on a screen whose
/// log-in control is disabled — or while the ring is on the clear control, which is
/// where Enter clears — is the same mistake as a prompt over a field that cannot be
/// typed into: it describes an interface that is not there. `focus` is the one the
/// projection settled on, not the one that was asked for, so a hint can never be
/// about a control the screen is not showing as focused.
fn hint(
    input: &Projection<'_>,
    submit: bool,
    cancel: bool,
    focus: Option<FocusTarget>,
) -> Option<String> {
    let message = |id: &str| input.localizer.message(id);

    match (submit, cancel, focus) {
        // Enter does what the focus does, and here that is clearing.
        (true, _, Some(FocusTarget::Cancel)) => Some(message("greet-hint-clear")),
        (true, _, _) => Some(message("greet-hint")),
        (false, true, _) => Some(message("greet-hint-clear")),
        (false, false, _) => None,
    }
}

/// The state under the identity field.
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

/// What is wrong with the identity the user typed, when something is.
///
/// An empty field is not an error: a screen that shouts at someone for not having
/// typed yet is a screen that shouts at everyone the moment it opens.
fn identity_error(input: &Projection<'_>) -> Option<&'static str> {
    if input.username.is_empty() || input.phase == Phase::SessionStarted {
        return None;
    }

    if input.identity_too_long {
        return Some("greet-identity-too-long");
    }

    if !valid_username(input.username) {
        return Some("greet-invalid-user");
    }

    None
}

/// Why a control cannot be used, in the order a user meets the reasons.
///
/// `identity` selects the wording for the field rather than for the button; the
/// order is the same because the states are.
fn disabled_reason(input: &Projection<'_>, identity: bool) -> Option<String> {
    let message = |id: &str| input.localizer.message(id);

    if !input.available {
        return Some(message("greet-disabled-unavailable"));
    }

    match input.phase {
        Phase::Unavailable => return Some(message("greet-disabled-unavailable")),
        Phase::Authenticating => return Some(message("greet-disabled-verifying")),
        Phase::Starting | Phase::SessionStarted => {
            return Some(message("greet-disabled-started"));
        }
        Phase::Ready | Phase::Failed => {}
    }

    if identity {
        return None;
    }

    if input.identity_too_long {
        return Some(message("greet-identity-too-long"));
    }

    if !valid_username(input.username) {
        return Some(message("greet-disabled-invalid"));
    }

    // The phase has already been checked above, so a `can_submit` that is false
    // for a typable screen can only be the rolling allowance.
    if !input.can_submit {
        return Some(message("greet-disabled-rate-limited"));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n;
    use crate::state::MAX_IDENTITY_BYTES;
    use inox_common::i18n::Locale;

    fn localizer() -> Localizer {
        i18n::load(&Locale::default_locale()).expect("the catalogs parse")
    }

    /// A projection of an available authenticator, a valid name, and no attempt.
    fn ready<'a>(localizer: &'a Localizer, username: &'a str) -> Projection<'a> {
        Projection {
            phase: Phase::Ready,
            available: true,
            username,
            identity_too_long: false,
            can_submit: true,
            focus: None,
            localizer,
        }
    }

    #[test]
    fn a_ready_screen_prompts_and_offers_all_three_controls() {
        let localizer = localizer();

        let model = project(ready(&localizer, "alice"));

        assert_eq!(model.title, "Inox login");
        assert_eq!(model.headline, "Log in to Inox");
        assert_eq!(model.identity.label, "Username");
        assert_eq!(model.identity.value, "alice");
        assert_eq!(model.identity.error, None);
        assert!(model.identity.editable);
        assert_eq!(model.face, Face::Prompt);
        assert!(model.submit.enabled);
        assert!(model.cancel.enabled);
        assert_eq!(
            model.focus_order,
            vec![
                FocusTarget::Identity,
                FocusTarget::Submit,
                FocusTarget::Cancel
            ]
        );
        assert_eq!(
            model.focus,
            Some(FocusTarget::Identity),
            "a screen with no focus at all is a screen a keyboard cannot use"
        );
    }

    #[test]
    fn the_interface_never_claims_more_than_the_model_knows() {
        let localizer = localizer();
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
                for username in ["", "alice", "-bad"] {
                    for can_submit in [false, true] {
                        let mut input = ready(&localizer, username);
                        input.phase = phase;
                        input.available = available;
                        input.can_submit = can_submit;

                        let model = project(input);

                        // Every string is a message, never a message id.
                        for line in [
                            model.headline.clone(),
                            model.identity.label.clone(),
                            model.identity.placeholder.clone(),
                            // Absent hints are not strings; the assertion below
                            // is about text that is shown, and an absent line is
                            // not shown at all.
                            model.hint.clone().unwrap_or_default(),
                            model.submit.label.clone(),
                            model.cancel.label.clone(),
                        ]
                        .into_iter()
                        .chain(model.identity.error.clone())
                        .chain(model.identity.disabled_reason.clone())
                        .chain(model.submit.disabled_reason.clone())
                        .chain(model.cancel.disabled_reason.clone())
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

                        if !model.identity.editable {
                            assert!(
                                model.identity.disabled_reason.is_some(),
                                "{phase:?} disables the identity field with no reason"
                            );
                        }

                        assert!(
                            model
                                .focus
                                .is_none_or(|target| model.focus_order.contains(&target)),
                            "{phase:?} leaves the focus on a control that cannot be used"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_hint_never_promises_a_key_that_does_nothing() {
        let localizer = localizer();

        // The shipped state: nothing can be submitted, so the hint is only about
        // what is left.
        let mut input = ready(&localizer, "");
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
        let mut focused_on_clear = ready(&localizer, "alice");
        focused_on_clear.focus = Some(FocusTarget::Cancel);

        let model = project(focused_on_clear);
        let hint = model.hint.clone().expect("there is still something to do");

        assert_eq!(model.focus, Some(FocusTarget::Cancel));
        assert!(
            !hint.contains("log in"),
            "`{hint}` says Enter logs in while the ring is on the control Enter clears"
        );

        // A screen where nothing at all can be done has no hint.
        let mut done = ready(&localizer, "alice");
        done.phase = Phase::SessionStarted;
        done.can_submit = false;

        assert_eq!(project(done).hint, None);
    }

    #[test]
    fn an_unavailable_authenticator_takes_no_input_and_says_why() {
        let localizer = localizer();
        let mut input = ready(&localizer, "");
        input.phase = Phase::Unavailable;
        input.available = false;
        input.can_submit = false;

        let model = project(input);

        let Face::Unavailable { line, reason } = &model.face else {
            panic!("an unavailable authenticator is the unavailable face");
        };
        assert_eq!(line, "Log in cannot run");
        assert!(reason.contains("security review"), "{reason}");
        assert!(!model.identity.editable);
        assert!(!model.submit.enabled);
        assert_eq!(
            model.identity.disabled_reason.as_deref(),
            Some("The system authentication service is not available")
        );
        assert_eq!(
            model.focus_order,
            vec![FocusTarget::Cancel],
            "clearing is the one thing left to do, so it is the one thing focused"
        );
    }

    #[test]
    fn an_attempt_in_flight_is_the_loading_state() {
        let localizer = localizer();
        let mut input = ready(&localizer, "alice");
        input.phase = Phase::Authenticating;
        input.can_submit = false;

        let model = project(input);

        assert_eq!(
            model.face,
            Face::Authenticating {
                line: "Checking your account…".to_owned()
            }
        );
        assert!(!model.identity.editable);
        assert!(!model.submit.enabled);
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
        let mut input = ready(&localizer, "alice");
        input.phase = Phase::Starting;

        let model = project(input);

        assert!(matches!(model.face, Face::Starting { .. }));
        assert!(!model.submit.enabled);
        assert!(!model.identity.editable);
        assert!(model.cancel.enabled, "the session is still only starting");
        assert_eq!(
            model.submit.disabled_reason.as_deref(),
            Some("The session is already starting")
        );
    }

    #[test]
    fn a_started_session_leaves_nothing_to_operate() {
        let localizer = localizer();
        let mut input = ready(&localizer, "alice");
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
        let mut input = ready(&localizer, "alice");
        input.phase = Phase::Failed;
        input.can_submit = true;

        let model = project(input);

        assert_eq!(
            model.face,
            Face::Failed {
                line: "That login was not accepted".to_owned()
            }
        );
        assert!(model.identity.editable);
        assert!(model.submit.enabled);
    }

    #[test]
    fn an_invalid_name_disables_the_button_and_says_which_way() {
        let localizer = localizer();
        let mut input = ready(&localizer, "0alice");
        input.can_submit = false;

        let model = project(input);

        assert_eq!(
            model.identity.error.as_deref(),
            Some("Enter a valid username.")
        );
        assert!(!model.submit.enabled);
        assert_eq!(
            model.submit.disabled_reason.as_deref(),
            Some("Enter a valid username")
        );
        assert_eq!(
            model.focus_order,
            vec![FocusTarget::Identity, FocusTarget::Cancel],
            "a disabled button is not reachable by the keyboard"
        );
        assert_eq!(
            model.notice, None,
            "the field already says what is wrong with the name; saying it again \
             under the form would read as two problems"
        );
    }

    #[test]
    fn the_line_under_the_form_is_printed_once() {
        let localizer = localizer();
        let input = ready(&localizer, "alice");

        let model = project(input);

        assert_eq!(
            model.notice, None,
            "a form that can be used has nothing to explain"
        );

        let mut unavailable = ready(&localizer, "");
        unavailable.phase = Phase::Unavailable;
        unavailable.available = false;
        unavailable.can_submit = false;

        let model = project(unavailable);

        assert_eq!(
            model.notice, model.identity.disabled_reason,
            "with the field gone, the field's reason is the one line"
        );
    }

    #[test]
    fn an_empty_field_is_not_an_error_but_cannot_be_submitted() {
        let localizer = localizer();
        let mut input = ready(&localizer, "");
        input.can_submit = false;

        let model = project(input);

        assert_eq!(model.identity.error, None);
        assert!(!model.submit.enabled);
        assert_eq!(
            model.submit.disabled_reason.as_deref(),
            Some("Enter a valid username")
        );
    }

    #[test]
    fn a_cut_down_identity_says_so_instead_of_submitting_what_was_not_typed() {
        let localizer = localizer();
        let mut input = ready(&localizer, "alice");
        input.identity_too_long = true;
        input.can_submit = false;

        let model = project(input);

        assert_eq!(
            model.identity.error.as_deref(),
            Some("That username is too long.")
        );
        assert_eq!(
            model.submit.disabled_reason.as_deref(),
            Some("That username is too long.")
        );
    }

    #[test]
    fn a_spent_allowance_says_a_wait_and_not_how_long() {
        let localizer = localizer();
        let mut input = ready(&localizer, "alice");
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
        let mut input = ready(&localizer, "alice");
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
        let mut input = ready(&localizer, "alice");
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
    fn the_longest_identity_the_model_keeps_is_the_one_the_field_shows() {
        let localizer = localizer();
        let username = "a".repeat(MAX_IDENTITY_BYTES);
        let input = ready(&localizer, &username);

        let model = project(input);

        assert_eq!(model.identity.value.len(), MAX_IDENTITY_BYTES);
        assert_eq!(model.identity.error, None);
    }
}
