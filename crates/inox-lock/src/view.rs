//! The projection QML reads: the lock screen's state, as one JSON document.
//!   nothing to redact (acceptance Q02, ADR 0015). The field is drawn only where
//! This module replaces the iced widget tree the crate used to build. QML is a
//! pure renderer, which means every decision the widget tree made —
//! which line is danger-coloured, whether there is a field at all, whether the
//! unlock control is enabled, how wide the card is relative to the surface — has
//! to be made here, in Rust, on the value [`crate::view_model`] produced. QML
//! then binds to the record and decides nothing:
//!   password it would discard (ADR 0025 decision 5).
//! ```text
//! LockViewModel + Theme ──view::screen──▶ JSON ──cxx-qt──▶ QML
//! ```
//!
//! The shape is deliberately one record rather than a QML property per field
//! (contract §3): a per-field bridge would become a second definition of the
//! interface, and the state's projection would have to be kept in sync with it.
//!
//! # The credential is not in here
//!
//! [`LockViewModel::mask`] is one glyph per character *typed*, built from the
//! credential's length. This module receives the projection and never the
//! [`crate::credential::Secret`], so the plaintext cannot be written into the JSON
//! by accident: the value it would have to read is not in scope. That is the
//! security property the migration had to keep, and the
//! test at the bottom of this file types a credential and asserts the frame's
//! document does not contain it.
//!
//! # Layout
//!
//! The proportions the iced tree expressed as `Length::FillPortion` are still
//! ratios and still have no pixels in them: the card takes [`CARD_PORTION`] of
//! the columns beside the [`MARGIN_PORTION`] gutters, a control takes
//! [`CONTROL_PORTION`] of the card beside its [`CONTROL_GUTTER_PORTION`] gutter.
//! They are projected as fractions of the surface so QML multiplies by the size
//! the compositor configured, rather than by a size this component chose.

use inox_config::Theme as Tokens;
use serde_json::{Value, json};

use crate::theme::{self, Emphasis};
use crate::view_model::{Face, LockViewModel};

/// How much of the surface the card takes, as a ratio of the columns beside it.
const CARD_PORTION: f64 = 2.0;

/// The portion of the surface left free on each side of the card.
const MARGIN_PORTION: f64 = 1.0;

/// How much of the card a field or a control takes, as a ratio of the gutter
/// beside it.
///
/// A control is a password field, not a banner: two thirds of the card is a width
/// that says how much goes in it, where the full width would say "write an essay"
/// (guidelines §4).
const CONTROL_PORTION: f64 = 2.0;

/// The gutter that keeps a control off the card's edge.
const CONTROL_GUTTER_PORTION: f64 = 1.0;

/// The projection of `model` under `tokens`, for a surface of `width` × `height`.
///
/// The surface size is carried in the document because the frame is drawn for the
/// size the compositor configured, and a test that reads the frame back has to be
/// able to assert what was asked for — the same reason the iced interface kept its
/// dimensions as the integers the configure delivered.
pub fn screen(model: &LockViewModel, tokens: &Tokens, width: u32, height: u32) -> Value {
    let (card_background, card_border, card_border_width) = theme::card(tokens);
    let scrim = theme::scrim(tokens);

    json!({
        "ready": true,
        "surface": { "width": width, "height": height },
        "palette": {
            "scrim": theme::qml_color(scrim),
            "card": theme::qml_color(card_background),
            "cardBorder": theme::qml_color(card_border),
            "text": theme::qml_color(theme::text(tokens)),
        },
        "metrics": {
            "spacingSm": f64::from(tokens.spacing.sm),
            "spacingLg": f64::from(tokens.spacing.lg),
            "cardPadding": f64::from(tokens.spacing.lg),
            "cardRadius": f64::from(theme::card_radius(tokens)),
            "controlRadius": f64::from(theme::control_radius(tokens)),
            "controlPadding": f64::from(theme::control_padding(tokens)),
            "hairline": f64::from(theme::hairline(tokens)),
            "cardWidthFraction": card_width_fraction(),
            "controlWidthFraction": control_width_fraction(),
        },
        "fonts": {
            "family": tokens.typography.font_family,
            "titleSize": f64::from(theme::title_size(tokens)),
            "headingSize": f64::from(theme::heading_size(tokens)),
            "bodySize": f64::from(theme::body_size(tokens)),
            "captionSize": f64::from(theme::caption_size(tokens)),
        },
        "headline": line(
            &model.headline,
            theme::heading_size(tokens),
            Emphasis::None,
            tokens,
        ),
        "lines": face_lines(model, tokens),
        "field": field(model, tokens),
        "notices": notices(model, tokens),
        "actions": actions(model, tokens),
        "hint": model.hint.as_ref().map(|hint| {
            line(hint, theme::caption_size(tokens), Emphasis::Muted, tokens)
        }),
        "cardBorderWidth": f64::from(card_border_width),
    })
}

/// The projection as the string the bridge hands QML.
pub fn screen_json(model: &LockViewModel, tokens: &Tokens, width: u32, height: u32) -> String {
    screen(model, tokens, width, height).to_string()
}

/// How much of the surface's width the card takes.
fn card_width_fraction() -> f64 {
    CARD_PORTION / (CARD_PORTION + 2.0 * MARGIN_PORTION)
}

/// How much of the card's width a control takes.
fn control_width_fraction() -> f64 {
    CONTROL_PORTION / (CONTROL_PORTION + CONTROL_GUTTER_PORTION)
}

/// One line of text: what it says, how large it is, and in which emphasis.
fn line(content: &str, size: f32, emphasis: Emphasis, tokens: &Tokens) -> Value {
    json!({
        "text": content,
        "size": f64::from(size),
        "color": emphasis.qml_colour(tokens),
    })
}

/// The lines under the heading, in the order a person reads them.
///
/// A refusal is two facts and they are read in that order: what happened, and what
/// the boundary said about it. Everything else is one line, and the three states
/// that are not password screens carry theirs as a caption — what the screen
/// explains, rather than what it asks.
fn face_lines(model: &LockViewModel, tokens: &Tokens) -> Value {
    match &model.face {
        Face::Prompt => json!([]),
        Face::Verifying { line: says } => json!([line(
            says,
            theme::body_size(tokens),
            Emphasis::Muted,
            tokens
        )]),
        Face::Failed { line: says, reason } => json!([
            line(says, theme::body_size(tokens), Emphasis::Danger, tokens),
            line(reason, theme::caption_size(tokens), Emphasis::Muted, tokens),
        ]),
        Face::Waiting { line: says } => json!([line(
            says,
            theme::body_size(tokens),
            Emphasis::Warning,
            tokens
        )]),
        Face::HeldWithoutUi { detail } => json!([line(
            detail,
            theme::caption_size(tokens),
            Emphasis::Muted,
            tokens
        )]),
        Face::Unavailable { reason, .. } => json!([line(
            reason,
            theme::caption_size(tokens),
            Emphasis::Danger,
            tokens
        )]),
    }
}

/// The credential field: a label and the mask, and nothing when there is nothing
/// to type into.
///
/// `visible` is [`LockViewModel::prompts_for_credential`], which is false for the
/// states that cannot check a credential at all. The label and the placeholder
/// travel with the record even then — they are what the field *would* say — and QML
/// draws none of them, because a screen that explains it cannot verify a password
/// must not also hold out a password box.
fn field(model: &LockViewModel, tokens: &Tokens) -> Value {
    let empty = model.mask.is_empty();
    let value = if empty {
        &model.placeholder
    } else {
        &model.mask
    };
    let (background, border, value_colour) = theme::field(tokens, empty);

    json!({
        "visible": model.prompts_for_credential,
        "enabled": model.accepts_input,
        "empty": empty,
        "label": model.field_label,
        "value": value,
        "labelSize": f64::from(theme::caption_size(tokens)),
        "labelColor": Emphasis::Muted.qml_colour(tokens),
        "valueSize": f64::from(theme::body_size(tokens)),
        "valueColor": theme::qml_color(value_colour),
        "background": theme::qml_color(background),
        "border": theme::qml_color(border),
        "radius": f64::from(theme::control_radius(tokens)),
        "padding": f64::from(theme::control_padding(tokens)),
    })
}

/// The reasons a control cannot be used, drawn beside it rather than discovered by
/// pressing it (acceptance D05, guidelines §3).
fn notices(model: &LockViewModel, tokens: &Tokens) -> Value {
    match &model.disabled_reason {
        Some(reason) => json!([line(
            reason,
            theme::caption_size(tokens),
            Emphasis::Warning,
            tokens
        )]),
        None => json!([]),
    }
}

/// The controls the screen offers, in the order it offers them.
///
/// The retry control only exists where the projection offered a label for it: a
/// boundary this build does not contain cannot be retried into existence, so the
/// screen must not hold out an action that cannot do anything (guidelines §6).
fn actions(model: &LockViewModel, tokens: &Tokens) -> Value {
    let mut controls = Vec::new();

    if let Face::Unavailable {
        retry: Some(label), ..
    } = &model.face
    {
        controls.push(control(
            "retry",
            label,
            theme::action_style(tokens, theme::ControlState::Resting),
            tokens,
        ));
    }

    // A control that cannot be pressed is drawn in the disabled state, which is
    // the same fact as "the projection enabled nothing": there is no second place
    // where "disabled" is decided.
    let state = if model.submit.enabled {
        theme::ControlState::Resting
    } else {
        theme::ControlState::Disabled
    };
    controls.push(control(
        "submit",
        &model.submit.label,
        theme::submit_style(tokens, state),
        tokens,
    ));

    json!(controls)
}

/// One control, as the four values and the label QML draws it with.
fn control(kind: &str, label: &str, colours: theme::ControlColours, tokens: &Tokens) -> Value {
    json!({
        "kind": kind,
        "label": label,
        "size": f64::from(theme::body_size(tokens)),
        "color": colours.text_qml(),
        "background": colours.background_qml(),
        "border": colours.border_qml(),
        "borderWidth": f64::from(colours.border_width),
        "radius": f64::from(theme::control_radius(tokens)),
        "padding": f64::from(theme::control_padding(tokens)),
    })
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use inox_common::i18n::Locale;
    use inox_config::Theme as Tokens;

    use super::*;
    use crate::app::{self, Message, State};
    use crate::contract::{AuthFailure, AuthOutcome, Generation, ProtocolAbsence, ServiceFailure};
    use crate::state::{LockReason, OutputId, Phase};
    use crate::theme::qml_color;

    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;

    fn output() -> OutputId {
        OutputId::new("eDP-1")
    }

    fn state_in(locale: &Locale, tokens: Tokens) -> State {
        let localizer = crate::i18n::load(locale).expect("the catalogs parse");

        State::new(Rc::new(localizer), tokens)
    }

    fn state() -> State {
        state_in(&Locale::default_locale(), Tokens::builtin_dark())
    }

    fn send(state: &mut State, message: Message) {
        let _ = app::update(state, message);
    }

    /// A locked screen with its one output covered.
    fn locked_in(locale: &Locale, tokens: Tokens) -> State {
        let mut state = state_in(locale, tokens);

        send(&mut state, Message::ProtocolPresent);
        send(&mut state, Message::OutputAdded(output()));
        send(&mut state, Message::LockRequested(LockReason::User));
        send(&mut state, Message::OutputCovered(output()));

        assert_eq!(state.machine().phase(), Phase::Locked);
        assert!(state.machine().reports_locked());
        state
    }

    fn locked() -> State {
        locked_in(&Locale::default_locale(), Tokens::builtin_dark())
    }

    fn screen_of(state: &State) -> Value {
        screen(state.model(), state.tokens(), WIDTH, HEIGHT)
    }

    #[test]
    fn the_document_carries_the_surfaces_own_size() {
        let document = screen_of(&locked());

        assert_eq!(document["surface"]["width"], WIDTH);
        assert_eq!(document["surface"]["height"], HEIGHT);
        assert_eq!(document["ready"], true);
    }

    #[test]
    fn every_colour_in_the_document_is_a_token_conversion() {
        // The point of this test is that the projection cannot invent a colour: the
        // one place a token becomes a QML colour is `theme::qml_color`, and the
        // document's values have to be exactly what it produced.
        let state = locked();
        let tokens = state.tokens();
        let document = screen_of(&state);

        assert_eq!(
            document["palette"]["scrim"],
            qml_color(theme::scrim(tokens))
        );
        assert_eq!(
            document["palette"]["card"],
            qml_color(tokens.colors.surface)
        );
        assert_eq!(
            document["palette"]["cardBorder"],
            qml_color(tokens.colors.border)
        );
        assert_eq!(document["headline"]["color"], qml_color(tokens.colors.text));
        assert_eq!(
            document["actions"][0]["background"],
            qml_color(tokens.colors.accent),
            "an enabled unlock control is filled with the accent"
        );
        assert_eq!(
            document["actions"][0]["color"],
            qml_color(tokens.colors.on_accent)
        );
    }

    #[test]
    fn every_distance_is_a_token_derivation() {
        let state = locked();
        let tokens = state.tokens();
        let document = screen_of(&state);

        assert_eq!(
            document["metrics"]["spacingSm"],
            f64::from(tokens.spacing.sm)
        );
        assert_eq!(
            document["metrics"]["spacingLg"],
            f64::from(tokens.spacing.lg)
        );
        assert_eq!(
            document["metrics"]["cardRadius"],
            f64::from(tokens.radii.md)
        );
        assert_eq!(
            document["metrics"]["controlRadius"],
            f64::from(tokens.radii.sm)
        );
        assert_eq!(
            document["metrics"]["hairline"],
            f64::from(theme::hairline(tokens))
        );
        assert_eq!(
            document["fonts"]["bodySize"],
            f64::from(theme::body_size(tokens))
        );
        assert_eq!(
            document["fonts"]["family"], tokens.typography.font_family,
            "QML draws the family the tokens name, not a built-in default"
        );
    }

    #[test]
    fn the_layout_fractions_are_proportions_and_not_pixels() {
        let document = screen_of(&locked());
        let card = document["metrics"]["cardWidthFraction"]
            .as_f64()
            .expect("a number");
        let control = document["metrics"]["controlWidthFraction"]
            .as_f64()
            .expect("a number");

        assert!((card - 0.5).abs() < f64::EPSILON, "two of four columns");
        assert!(
            (control - 2.0 / 3.0).abs() < f64::EPSILON,
            "two of three columns"
        );
        assert!(card > 0.0 && card < 1.0);
    }

    #[test]
    fn the_text_comes_from_the_catalog() {
        let document = screen_of(&locked());

        assert_eq!(document["headline"]["text"], "Enter your password");
        assert_eq!(document["field"]["label"], "Password");
        assert_eq!(document["field"]["value"], "Type your password");
        assert_eq!(document["actions"][0]["label"], "Unlock");
        assert_eq!(document["hint"]["text"], "Press Enter to unlock");
    }

    #[test]
    fn the_other_language_reaches_the_same_document() {
        let zh = Locale::parse("zh-CN").expect("a locale");
        let document = screen_of(&locked_in(&zh, Tokens::builtin_dark()));
        let en = screen_of(&locked());

        assert_ne!(
            document["headline"]["text"], en["headline"]["text"],
            "the projection is not translated at all"
        );
        assert_eq!(
            document["actions"][0]["label"], "解锁",
            "the control label is the catalog's"
        );
    }

    #[test]
    fn the_mask_counts_what_was_typed_and_carries_none_of_it() {
        // The security property the migration had to keep:
        // a typed credential reaches the screen as one glyph per character, and no
        // other trace of it reaches the document QML reads.
        let mut state = locked();
        for character in "hunter2".chars() {
            send(&mut state, Message::Character(character));
        }

        let document = screen_json(state.model(), state.tokens(), WIDTH, HEIGHT);

        assert!(
            document.contains("•••••••"),
            "the mask is not drawn: {document}"
        );
        assert!(
            !document.contains("hunter2"),
            "the document carrying the credential is the one thing that must never \
             happen"
        );
        let parsed: Value = serde_json::from_str(&document).expect("the document parses");
        assert_eq!(parsed["field"]["value"], "•••••••");
        assert_eq!(parsed["field"]["empty"], false);
    }

    #[test]
    fn a_screen_that_cannot_check_a_password_draws_no_field() {
        let mut state = locked();
        send(
            &mut state,
            Message::AuthUnavailable(ServiceFailure::NotProvided),
        );

        let document = screen_of(&state);
        let model = state.model();

        assert!(!model.prompts_for_credential);
        assert_eq!(document["field"]["visible"], false);
        assert_eq!(document["headline"]["text"], "The session lock cannot run");
        assert_eq!(
            document["hint"],
            Value::Null,
            "there is nothing to press, so there is nothing to hint about"
        );
        assert_eq!(
            document["actions"][0]["kind"], "submit",
            "a boundary this build does not have offers no retry control"
        );
        assert_eq!(
            document["actions"][0]["background"],
            qml_color(theme::TRANSPARENT)
        );
    }

    #[test]
    fn a_retryable_boundary_draws_a_retry_control_first() {
        let mut state = locked();
        send(
            &mut state,
            Message::AuthUnavailable(ServiceFailure::Disconnected),
        );

        let document = screen_of(&state);

        assert_eq!(document["actions"][0]["kind"], "retry");
        assert_eq!(document["actions"][0]["label"], "Try again");
        assert_eq!(document["actions"][1]["kind"], "submit");
    }

    #[test]
    fn a_refusal_is_two_lines_in_the_order_they_are_read() {
        let mut state = locked();
        for character in "wrong".chars() {
            send(&mut state, Message::Character(character));
        }
        send(&mut state, Message::Submitted);
        let generation = state
            .boundary()
            .generation()
            .expect("the attempt is in flight");
        send(
            &mut state,
            Message::AuthAnswered(generation, AuthOutcome::Failure(AuthFailure::Rejected)),
        );

        let document = screen_of(&state);
        let lines = document["lines"].as_array().expect("the lines");

        assert_eq!(lines.len(), 2, "what happened, and what the boundary said");
        assert_eq!(lines[0]["text"], "That password was not accepted");
        assert_eq!(
            lines[0]["color"],
            qml_color(theme::danger_text(state.tokens()))
        );
        assert_eq!(
            lines[1]["color"],
            qml_color(theme::muted_text(state.tokens()))
        );
    }

    #[test]
    fn the_waiting_face_says_nothing_about_its_timing() {
        let mut state = locked();

        for _ in 0..crate::attempts::AttemptPolicy::standard().allowance {
            for character in "wrong".chars() {
                send(&mut state, Message::Character(character));
            }
            send(&mut state, Message::Submitted);
            let generation = state
                .boundary()
                .generation()
                .expect("the attempt is in flight");
            send(
                &mut state,
                Message::AuthAnswered(generation, AuthOutcome::Failure(AuthFailure::Rejected)),
            );
        }

        let document = screen_json(state.model(), state.tokens(), WIDTH, HEIGHT);

        assert!(document.contains("Too many attempts"), "{document}");
        assert!(
            !document.contains("second"),
            "the limiter's timing is not a user-facing value: {document}"
        );
    }

    #[test]
    fn a_disabled_control_is_drawn_disabled_and_carries_its_reason() {
        let mut state = locked();
        send(
            &mut state,
            Message::AuthUnavailable(ServiceFailure::NotProvided),
        );

        let document = screen_of(&state);
        let notices = document["notices"].as_array().expect("the notices");

        assert_eq!(notices.len(), 1);
        assert_eq!(
            notices[0]["color"],
            qml_color(theme::warning_text(state.tokens()))
        );
        assert_eq!(document["actions"][0]["borderWidth"], 0.0);
        assert_eq!(
            document["actions"][0]["color"],
            qml_color(theme::muted_text(state.tokens())),
            "a control that cannot be used is muted, not emphasized"
        );
    }

    #[test]
    fn a_lock_with_no_ui_says_so_and_draws_no_field() {
        let mut state = locked();
        send(&mut state, Message::UiExited);

        let document = screen_of(&state);

        assert_eq!(document["headline"]["text"], "The session is locked");
        assert_eq!(document["field"]["visible"], false);
        assert_eq!(document["lines"].as_array().expect("lines").len(), 1);
    }

    #[test]
    fn a_missing_protocol_is_unavailable_and_names_the_reason() {
        let mut state = state();
        send(
            &mut state,
            Message::ProtocolAbsent(ProtocolAbsence::GlobalMissing),
        );

        let document = screen_of(&state);

        assert_eq!(document["headline"]["text"], "The session lock cannot run");
        assert_eq!(document["field"]["visible"], false);
        assert_eq!(document["actions"].as_array().expect("actions").len(), 1);
    }

    #[test]
    fn two_hundred_percent_text_doubles_every_size_the_document_carries() {
        let mut tokens = Tokens::builtin_dark();
        let normal = screen(&locked().model().clone(), &tokens, WIDTH, HEIGHT);

        tokens.typography.scale *= 2.0;
        let doubled = screen(&locked().model().clone(), &tokens, WIDTH, HEIGHT);

        for (key, size) in [
            ("titleSize", "title"),
            ("headingSize", "heading"),
            ("bodySize", "body"),
            ("captionSize", "caption"),
        ] {
            let before = normal["fonts"][key].as_f64().expect("a number");
            let after = doubled["fonts"][key].as_f64().expect("a number");

            assert_eq!(
                after,
                before * 2.0,
                "the {size} role does not follow the user's scale"
            );
        }
    }

    #[test]
    fn the_two_builtin_themes_produce_different_documents() {
        let light = screen_of(&locked_in(
            &Locale::default_locale(),
            Tokens::builtin_light(),
        ));
        let dark = screen_of(&locked());

        assert_ne!(light["palette"]["scrim"], dark["palette"]["scrim"]);
        assert_ne!(light["palette"]["card"], dark["palette"]["card"]);
    }

    #[test]
    fn the_qml_carries_no_colour_or_pixel_literal() {
        // Contract §4 and the completion definition (§9): colours, sizes and
        // strings come from the projection. This is a cheap scanner rather than a
        // regular expression, so the rule is enforced by a test instead of by a
        // reviewer's eye.
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/qml/Main.qml"))
            .expect("the module's QML is in the crate");

        let characters: Vec<char> = source.chars().collect();
        for (index, character) in characters.iter().enumerate() {
            if *character == '#' {
                let digits = characters[index + 1..]
                    .iter()
                    .take_while(|next| next.is_ascii_hexdigit())
                    .count();

                assert!(
                    digits < 3,
                    "a colour literal at character {index}: QML binds the projection's \
                     colours, it does not name its own"
                );
            }
        }

        for (index, window) in characters.windows(2).enumerate() {
            if window == ['p', 'x'] {
                assert!(
                    !characters[..index]
                        .last()
                        .is_some_and(|previous| previous.is_ascii_digit()),
                    "a pixel literal at character {index}"
                );
            }
        }
    }

    #[test]
    fn the_document_holds_no_message_id_instead_of_a_message() {
        // The catalog test catches an id in the source; this catches one that made
        // it into the frame, where a person would read `lock-prompt` on a screen
        // they cannot leave.
        for state in [
            locked(),
            {
                let mut state = locked();
                send(
                    &mut state,
                    Message::AuthUnavailable(ServiceFailure::NotProvided),
                );
                state
            },
            {
                let mut state = locked();
                send(&mut state, Message::UiExited);
                state
            },
        ] {
            let document = screen_json(state.model(), state.tokens(), WIDTH, HEIGHT);

            assert!(
                !document.contains("\"lock-"),
                "a message id reached the frame: {document}"
            );
        }
    }

    #[test]
    fn the_generation_is_not_in_the_frame() {
        // A generation names an attempt, not anything a person reads; it is also
        // the handle the boundary's answers are matched by, so it stays in Rust.
        let mut state = locked();
        for character in "x".chars() {
            send(&mut state, Message::Character(character));
        }
        send(&mut state, Message::Submitted);
        assert_eq!(
            state.boundary().generation(),
            Some(Generation::new(2)),
            "the attempt this test is about"
        );

        let document = screen_json(state.model(), state.tokens(), WIDTH, HEIGHT);

        assert!(!document.contains("generation"), "{document}");
    }
}
