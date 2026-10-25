//! The projection from lock state to what the view draws.
//!
//! Everything a user reads is decided here and only here: the labels come from
//! the Fluent catalogs, a disabled field carries the reason it cannot be typed
//! into, and the state the acceptance criteria name — prompting, checking
//! (loading), failed (error), rate limited, held without a UI, and unavailable,
//! with a retry where one could help (acceptance D05) — is one variant of
//! [`Face`]. The view then has
//! no decisions left: it walks this value and draws it, which is what keeps a
//! colour or a message out of the widget tree.
//!
//! Two details are deliberate and are the parts a reviewer should check.
//!
//! - **The mask is a count, not the credential.** [`LockViewModel::mask`] is one
//!   glyph per character typed, built from `credential_len` alone. The plaintext
//!   never enters a view model, a widget, or iced's text editor, so there is
//!   nothing to redact when a screenshot of the lock screen is taken. The length
//!   is visible, because a password field shows how much has been typed.
//! - **A wait says nothing about itself.** [`Face::Waiting`] reports *that*
//!   attempts are limited; the remaining time and the remaining allowance stay in
//!   [`crate::attempts`] where the user cannot read them.
//!
//! The module is pure: state in, value out. No iced, no clock, no I/O, so every
//! state above is a unit test.

use inox_common::i18n::Localizer;

use crate::attempts::Gate;
use crate::contract::{BoundaryState, LockProtocol};
use crate::state::Phase;

/// The glyph a typed character is drawn as.
///
/// A symbol rather than text: it is not translated, it is not a message, and no
/// catalog may change it — every locale masks a password the same way.
pub const MASK_GLYPH: char = '•';

/// A control the keyboard can reach (acceptance Q01).
///
/// The order of [`LockViewModel::focus`] is the order Tab visits them, and it
/// contains only what can actually be operated, so "focus is visible and
/// unique" holds by construction rather than by a rule the view has to remember.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    /// The credential field.
    Credential,
    /// The unlock button.
    Submit,
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

/// The state under the prompt.
///
/// Two of these are not password screens at all: [`Face::Unavailable`] and
/// [`Face::HeldWithoutUi`] are states in which nothing can be typed, and the
/// projection gives them the screen's *headline* rather than a line under a
/// prompt. That is the difference between a screen that explains why it cannot
/// check a password and one that asks for a password nobody will check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Face {
    /// Nothing to report: the prompt is the whole screen.
    Prompt,
    /// An attempt is with the boundary. The loading state of D05.
    Verifying {
        /// What the screen says while it waits.
        line: String,
    },
    /// The last attempt was refused. The error state of D05.
    Failed {
        /// The headline.
        line: String,
        /// What the boundary said, in the user's language.
        reason: String,
    },
    /// The allowance is spent for now.
    Waiting {
        /// The line the user reads.
        line: String,
    },
    /// The session is locked with no trusted UI running.
    HeldWithoutUi {
        /// What is happening instead.
        detail: String,
    },
    /// The session lock cannot run at all, with a retry where one could help.
    Unavailable {
        /// Why, in the user's language.
        reason: String,
        /// The retry label, when retrying is meaningful.
        retry: Option<String>,
    },
}

/// What the lock screen draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockViewModel {
    /// The name of the surface.
    pub title: String,
    /// The question the screen asks — or, when it cannot check anything, what it
    /// is instead of asking one.
    pub headline: String,
    /// The label of the credential field, which doubles as its accessible name.
    pub field_label: String,
    /// What the empty field says.
    pub placeholder: String,
    /// One glyph per character typed — and nothing about the characters.
    pub mask: String,
    /// The keyboard hint, when the keyboard has something to do here.
    pub hint: Option<String>,
    /// Whether this screen asks for a credential at all.
    ///
    /// False in the two states that cannot check one. The label and the
    /// placeholder are still in the model — they are what the field would say —
    /// but nothing draws them: a screen that explains it cannot verify a password
    /// must not also hold out a password box. `accepts_input` is the narrower
    /// question ("can this be typed into right now"), and it is false for a screen
    /// that is busy with a credential it already has.
    pub prompts_for_credential: bool,
    /// Whether the credential field accepts input.
    pub accepts_input: bool,
    /// Why it does not, when it does not.
    pub disabled_reason: Option<String>,
    /// The unlock control.
    pub submit: Action,
    /// The state under the prompt.
    pub face: Face,
    /// The controls the keyboard can reach, in tab order.
    pub focus: Vec<FocusTarget>,
}

/// Everything the projection reads.
///
/// A borrow of the pieces rather than of [`crate::app::State`], so the projection
/// stays testable without a state to build and so it cannot reach for anything
/// the view is not meant to see — the credential, in particular, is represented
/// by a length here and is not in scope at all.
#[derive(Debug, Clone, Copy)]
pub struct Projection<'a> {
    /// Where the lock lifecycle is.
    pub phase: Phase,
    /// Whether the compositor offers the lock protocol.
    pub protocol: &'a LockProtocol,
    /// What the boundary last said.
    pub boundary: &'a BoundaryState,
    /// Whether an attempt may be made now.
    pub gate: Gate,
    /// How many characters the credential holds.
    pub credential_len: usize,
    /// The catalogs.
    pub localizer: &'a Localizer,
}

/// Projects the state into what the view draws.
pub fn project(input: Projection<'_>) -> LockViewModel {
    let localizer = input.localizer;
    let message = |id: &str| localizer.message(id);

    let waiting = matches!(input.gate, Gate::Waiting);
    let in_flight = input.boundary.in_flight();
    let protocol_ready = input.protocol.is_advertised();
    // A boundary that cannot be reached is not a reason to let a user type a
    // password nobody will check.
    let boundary_usable = input.boundary.failure().is_none();
    // Input is accepted only once the compositor has *confirmed* the lock and every
    // output is covered. `Locking` and `Covering` still run the interface, but a
    // credential submitted there could authenticate a session whose lock was never
    // confirmed: the screen would leave the person looking at "verifying" over an
    // unlocked desktop.
    let locked = matches!(input.phase, Phase::Locked);
    let accepts_input = locked && protocol_ready && boundary_usable && !in_flight && !waiting;

    // The reasons are ordered the way a user meets them: with no protocol there
    // is no lock at all, then no boundary to check anything, then a session that
    // is not locked, then a screen that is not there to type into, and only then
    // the two states in which the lock screen itself is busy.
    let disabled_reason = if accepts_input {
        None
    } else if !protocol_ready {
        Some(message("lock-disabled-protocol"))
    } else if let Some(failure) = input.boundary.failure() {
        Some(message(failure.disabled_message_id()))
    } else if input.phase == Phase::Unlocked {
        Some(message("lock-disabled-unlocked"))
    } else if !input.phase.ui_running() {
        Some(message("lock-disabled-no-ui"))
    } else if !locked {
        Some(message("lock-disabled-locking"))
    } else if in_flight {
        Some(message("lock-disabled-verifying"))
    } else if waiting {
        Some(message("lock-disabled-waiting"))
    } else {
        // `accepts_input` is false and none of the reasons above holds, which
        // cannot happen while a locked phase with a usable boundary is exactly
        // what `accepts_input` describes. Answering with a reason rather than
        // `None` keeps a disabled control from ever being disabled without one,
        // which is the property the test below asserts.
        Some(message("lock-disabled-unlocked"))
    };

    let submit = Action {
        label: message("lock-submit"),
        enabled: accepts_input,
        disabled_reason: disabled_reason.clone(),
    };

    let mut focus = Vec::new();
    if accepts_input {
        focus.push(FocusTarget::Credential);
        focus.push(FocusTarget::Submit);
    }

    let face = face(input);
    // What a screen that cannot check a password says instead of asking for one.
    // "Enter your password" over a disabled field, under a sentence saying no
    // password will be checked, is an invitation this build cannot honour — and
    // the whole point of reporting `ServiceFailure::NotProvided` is that the screen
    // says what it is instead of asking for something it will discard. The three
    // answers travel together: a screen either prompts — headline, hint and field —
    // or states what it is and offers none of the three.
    let (headline, hint, prompts_for_credential) = match &face {
        Face::Unavailable { .. } => (message("lock-unavailable"), None, false),
        Face::HeldWithoutUi { .. } => (message("lock-held-title"), None, false),
        Face::Prompt | Face::Verifying { .. } | Face::Failed { .. } | Face::Waiting { .. } => {
            (message("lock-prompt"), Some(message("lock-hint")), true)
        }
    };

    LockViewModel {
        title: message("lock-window-title"),
        headline,
        field_label: message("lock-field-label"),
        placeholder: message("lock-field-placeholder"),
        mask: MASK_GLYPH.to_string().repeat(input.credential_len),
        hint,
        prompts_for_credential,
        accepts_input,
        disabled_reason,
        submit,
        face,
        focus,
    }
}

/// The state under the prompt, in the order the user needs to know about them.
fn face(input: Projection<'_>) -> Face {
    let message = |id: &str| input.localizer.message(id);

    // Nothing can happen at all without the protocol, so that comes first: the
    // state of the boundary is not even known yet.
    if let Some(absence) = input.protocol.absence() {
        return Face::Unavailable {
            reason: message(absence.message_id()),
            retry: None,
        };
    }

    // The boundary being unusable is next: it is what the user has to wait for.
    // A boundary that is down or slow can come back, so it offers a retry; a
    // boundary this build does not contain cannot, and a retry control for it
    // would be a control that is there only to be pressed for nothing.
    if let Some(failure) = input.boundary.failure() {
        return Face::Unavailable {
            reason: message(failure.message_id()),
            retry: failure
                .is_retryable()
                .then(|| message("lock-unavailable-retry")),
        };
    }

    if input.phase == Phase::LockedWithoutUi {
        return Face::HeldWithoutUi {
            detail: message("lock-held-detail"),
        };
    }

    if input.boundary.in_flight() {
        return Face::Verifying {
            line: message("lock-verifying"),
        };
    }

    if matches!(input.gate, Gate::Waiting) {
        return Face::Waiting {
            line: message("lock-attempts-waiting"),
        };
    }

    if let Some(failure) = input
        .boundary
        .outcome()
        .and_then(|outcome| outcome.failure())
    {
        return Face::Failed {
            line: message("lock-failed"),
            reason: message(failure.message_id()),
        };
    }

    Face::Prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{AuthFailure, AuthOutcome, Generation, ProtocolAbsence, ServiceFailure};
    use inox_common::i18n::Locale;

    fn localizer() -> Localizer {
        crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse")
    }

    /// A projection of a locked session with nothing typed and no attempt made.
    fn locked<'a>(
        localizer: &'a Localizer,
        protocol: &'a LockProtocol,
        boundary: &'a BoundaryState,
    ) -> Projection<'a> {
        Projection {
            phase: Phase::Locked,
            protocol,
            boundary,
            gate: Gate::Permitted,
            credential_len: 0,
            localizer,
        }
    }

    #[test]
    fn a_locked_prompt_accepts_input_and_offers_both_focus_targets() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Idle;

        let model = project(locked(&localizer, &protocol, &boundary));

        assert_eq!(model.title, "Inox Session Lock");
        assert_eq!(model.headline, "Enter your password");
        assert_eq!(model.face, Face::Prompt);
        assert!(model.accepts_input);
        assert_eq!(model.disabled_reason, None);
        assert!(model.submit.enabled);
        assert_eq!(model.submit.label, "Unlock");
        assert_eq!(
            model.focus,
            vec![FocusTarget::Credential, FocusTarget::Submit]
        );
        assert!(model.mask.is_empty());
    }

    #[test]
    fn the_mask_counts_the_characters_and_carries_none_of_them() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Idle;
        let mut input = locked(&localizer, &protocol, &boundary);
        input.credential_len = 4;

        let model = project(input);

        assert_eq!(model.mask, "••••");
        assert_eq!(model.mask.chars().count(), 4);
    }

    #[test]
    fn an_empty_field_shows_the_placeholder() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Idle;

        let model = project(locked(&localizer, &protocol, &boundary));

        assert_eq!(model.placeholder, "Type your password");
        assert_eq!(model.field_label, "Password");
        assert_eq!(model.hint.as_deref(), Some("Press Enter to unlock"));
        assert_eq!(model.headline, "Enter your password");
    }

    #[test]
    fn only_a_screen_that_prompts_draws_a_password_field() {
        let localizer = localizer();

        for phase in [
            Phase::Unlocked,
            Phase::Locking,
            Phase::Locked,
            Phase::Covering,
            Phase::Unlocking,
        ] {
            for boundary in [
                BoundaryState::Idle,
                BoundaryState::Unavailable {
                    failure: ServiceFailure::NotProvided,
                },
                BoundaryState::Unavailable {
                    failure: ServiceFailure::Disconnected,
                },
            ] {
                let mut input = locked(&localizer, &LockProtocol::Advertised, &boundary);
                input.phase = phase;

                let model = project(input);
                let prompts = !matches!(
                    model.face,
                    Face::Unavailable { .. } | Face::HeldWithoutUi { .. }
                );

                assert_eq!(
                    model.prompts_for_credential, prompts,
                    "{:?} with {boundary:?} disagrees with its own face",
                    model.face
                );
                assert_eq!(
                    model.hint.is_some(),
                    prompts,
                    "a screen with nothing to type has nothing to hint about"
                );
                assert_eq!(model.headline == "Enter your password", prompts);
            }
        }
    }

    #[test]
    fn a_screen_that_cannot_check_a_password_does_not_ask_for_one() {
        // The whole point of reporting that this build has no boundary: the
        // screen says what it is instead of inviting a credential it would
        // discard. The prompt is not decoration — it is what a person reads as
        // "type here".
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Unavailable {
            failure: ServiceFailure::NotProvided,
        };

        let model = project(locked(&localizer, &protocol, &boundary));

        assert_eq!(model.headline, "The session lock cannot run");
        assert_ne!(model.headline, "Enter your password");
        assert_eq!(
            model.hint, None,
            "there is nothing to press, so there is nothing to hint about"
        );

        let held = {
            let mut input = locked(&localizer, &protocol, &BoundaryState::Idle);
            input.phase = Phase::LockedWithoutUi;
            project(input)
        };

        assert_eq!(held.headline, "The session is locked");
        assert_ne!(held.headline, "Enter your password");
        assert_eq!(held.hint, None);
    }

    #[test]
    fn an_attempt_in_flight_is_the_loading_state_and_disables_the_field() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Sent {
            generation: Generation::first(),
        };

        let model = project(locked(&localizer, &protocol, &boundary));

        assert!(matches!(model.face, Face::Verifying { .. }));
        assert!(!model.accepts_input);
        assert!(!model.submit.enabled);
        assert_eq!(
            model.disabled_reason.as_deref(),
            Some("An attempt is already being checked")
        );
        assert!(
            model.focus.is_empty(),
            "a disabled control is not reachable by the keyboard"
        );
    }

    #[test]
    fn an_accepted_attempt_is_still_the_loading_state() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Accepted {
            generation: Generation::first(),
        };

        let model = project(locked(&localizer, &protocol, &boundary));

        assert!(matches!(model.face, Face::Verifying { .. }));
        assert!(!model.accepts_input);
    }

    #[test]
    fn a_refused_attempt_is_the_error_state_with_a_reason() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Answered {
            generation: Generation::first(),
            outcome: AuthOutcome::Failure(AuthFailure::Rejected),
        };

        let model = project(locked(&localizer, &protocol, &boundary));

        assert_eq!(
            model.face,
            Face::Failed {
                line: "That password was not accepted".to_owned(),
                reason: "the authentication service rejected the password".to_owned(),
            }
        );
        assert!(
            model.accepts_input,
            "a failure must leave the user able to try again"
        );
        assert!(model.submit.enabled);
    }

    #[test]
    fn every_failure_reason_reaches_the_user_in_words() {
        for failure in [
            AuthFailure::Rejected,
            AuthFailure::TimedOut,
            AuthFailure::Unavailable,
            AuthFailure::Cancelled,
        ] {
            let localizer = localizer();
            let protocol = LockProtocol::Advertised;
            let boundary = BoundaryState::Answered {
                generation: Generation::first(),
                outcome: AuthOutcome::Failure(failure),
            };

            let model = project(locked(&localizer, &protocol, &boundary));

            let Face::Failed { reason, .. } = model.face else {
                panic!("a failure is the Failed face, not {:?}", model.face);
            };
            assert!(
                !reason.starts_with("lock-"),
                "`{reason}` is a message id, not a message"
            );
        }
    }

    #[test]
    fn a_spent_allowance_is_the_waiting_state_without_any_timing() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Answered {
            generation: Generation::first(),
            outcome: AuthOutcome::Failure(AuthFailure::Rejected),
        };
        let mut input = locked(&localizer, &protocol, &boundary);
        input.gate = Gate::Waiting;
        input.credential_len = 2;

        let model = project(input);

        assert!(matches!(model.face, Face::Waiting { .. }));
        assert!(!model.accepts_input);
        assert_eq!(
            model.disabled_reason.as_deref(),
            Some("Too many attempts for now")
        );

        let Face::Waiting { line } = &model.face else {
            panic!("the waiting face");
        };
        assert!(
            !line.chars().any(|character| character.is_ascii_digit()),
            "`{line}` says how long the wait is, which is the limiter's own state"
        );
        assert_eq!(
            model.mask, "••",
            "the typed characters are still shown while waiting"
        );
    }

    #[test]
    fn a_lock_with_no_ui_says_so_and_takes_no_input() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Idle;
        let mut input = locked(&localizer, &protocol, &boundary);
        input.phase = Phase::LockedWithoutUi;

        let model = project(input);

        assert_eq!(
            model.face,
            Face::HeldWithoutUi {
                detail: "The lock screen is restarting. Your session stays locked.".to_owned(),
            }
        );
        assert_eq!(model.headline, "The session is locked");
        assert!(!model.accepts_input);
        assert_eq!(
            model.disabled_reason.as_deref(),
            Some("The lock screen is not running")
        );
    }

    #[test]
    fn a_missing_protocol_is_unavailable_and_names_the_reason() {
        for absence in [
            ProtocolAbsence::GlobalMissing,
            ProtocolAbsence::RegistryUnavailable,
        ] {
            let localizer = localizer();
            let protocol = LockProtocol::NotAdvertised { reason: absence };
            let boundary = BoundaryState::Idle;

            let model = project(locked(&localizer, &protocol, &boundary));

            let Face::Unavailable { reason, retry, .. } = &model.face else {
                panic!("a missing protocol is the unavailable face");
            };
            assert!(!reason.starts_with("lock-"), "`{reason}` is not a message");
            assert_eq!(
                *retry, None,
                "retrying cannot make a compositor advertise a protocol"
            );
            assert!(!model.accepts_input);
            assert_eq!(
                model.disabled_reason.as_deref(),
                Some("The session lock protocol is not available")
            );
        }
    }

    #[test]
    fn a_boundary_this_build_does_not_have_offers_no_retry() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Unavailable {
            failure: ServiceFailure::NotProvided,
        };

        let model = project(locked(&localizer, &protocol, &boundary));

        let Face::Unavailable { reason, retry, .. } = &model.face else {
            panic!("a missing boundary is the unavailable face");
        };
        assert!(!reason.starts_with("lock-"), "`{reason}` is not a message");
        assert_eq!(
            *retry, None,
            "retrying cannot produce an authentication boundary this build does \
             not ship"
        );
        assert!(!model.accepts_input);
        assert!(
            model.focus.is_empty(),
            "nothing on the screen can be operated, so nothing is reachable"
        );
        assert_eq!(
            model.disabled_reason.as_deref(),
            Some("This session has no authentication service to use")
        );
    }

    #[test]
    fn an_unusable_boundary_is_unavailable_and_offers_a_retry() {
        for failure in [ServiceFailure::Disconnected, ServiceFailure::Timeout] {
            let localizer = localizer();
            let protocol = LockProtocol::Advertised;
            let boundary = BoundaryState::Unavailable { failure };

            let model = project(locked(&localizer, &protocol, &boundary));

            let Face::Unavailable { reason, retry, .. } = &model.face else {
                panic!("an unusable boundary is the unavailable face");
            };
            assert!(!reason.starts_with("lock-"), "`{reason}` is not a message");
            assert_eq!(retry.as_deref(), Some("Try again"));
            assert!(
                !model.accepts_input,
                "a credential typed into a screen that cannot check it is a \
                 credential typed for nothing"
            );
            assert_eq!(
                model.disabled_reason.as_deref(),
                Some("The authentication service is not available")
            );
        }
    }

    #[test]
    fn an_unlocked_session_takes_no_input_and_says_why() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Idle;
        let mut input = locked(&localizer, &protocol, &boundary);
        input.phase = Phase::Unlocked;

        let model = project(input);

        assert_eq!(model.face, Face::Prompt);
        assert!(!model.accepts_input);
        assert_eq!(
            model.disabled_reason.as_deref(),
            Some("The session is not locked")
        );
    }

    #[test]
    fn the_teardown_takes_no_input_either() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;
        let boundary = BoundaryState::Idle;
        let mut input = locked(&localizer, &protocol, &boundary);
        input.phase = Phase::Unlocking;

        let model = project(input);

        assert!(!model.accepts_input);
        assert!(model.disabled_reason.is_some());
    }

    #[test]
    fn a_disabled_control_always_has_a_reason() {
        let localizer = localizer();
        let protocol = LockProtocol::Advertised;

        let cases = [
            Phase::Unlocked,
            Phase::Unlocking,
            Phase::LockedWithoutUi,
            Phase::Locking,
        ];

        for phase in cases {
            for boundary in [
                BoundaryState::Idle,
                BoundaryState::Sent {
                    generation: Generation::first(),
                },
            ] {
                for gate in [Gate::Permitted, Gate::Waiting] {
                    let mut input = locked(&localizer, &protocol, &boundary);
                    input.phase = phase;
                    input.gate = gate;

                    let model = project(input);

                    if !model.accepts_input {
                        assert!(
                            model.disabled_reason.is_some(),
                            "{phase:?} with {boundary:?} and {gate:?} is disabled \
                             with no reason"
                        );
                    }
                    if model.submit.enabled {
                        assert!(model.submit.disabled_reason.is_none());
                    } else {
                        assert!(model.submit.disabled_reason.is_some());
                    }
                }
            }
        }
    }
}
