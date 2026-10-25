//! The Qt-free projection QML renders: state and tokens as one JSON value.
//!
//! Everything a person reads on the login screen is decided in Rust and handed to
//! QML as data (docs/qml-migration-contract.md §3/§4). This module is that data:
//! the [`GreeterViewModel`] the projection produced, the theme's colours and
//! distances, and the layout proportions — one `serde_json::Value` per state
//! change, re-sent whole, so QML has no decision left to make but "walk the object
//! and draw it".
//!
//! Three properties are deliberate and are what a reviewer should check.
//!
//! - **The credential is a count, never a value.** What reaches QML is the mask
//!   [`crate::view_model::MASK_GLYPH`] repeated by length; there is no field, here
//!   or in the wire format below, that could carry a password, and
//!   [`tests`](self) asserts that a projection of a typed credential does not
//!   contain it.
//! - **Every colour and size is a token.** Colours go out as the `#AARRGGBB`
//!   strings Qt parses ([`crate::theme::qml_color`]); sizes as numbers derived from
//!   spacing, radii and typography. QML names the *token* a line is drawn with
//!   (`build.colors[line.color]`) and never a value, so 200 % text and both built-in
//!   themes are one projection away.
//! - **The strings are already sentences.** Every user-visible line was resolved
//!   from Fluent by [`crate::view_model`]; QML concatenates nothing. A new line is
//!   a new catalog entry, not a new literal in the interface.
//!
//! The module is pure: state in, JSON out. No Qt, no clock, no I/O — which is what
//! lets the projection be tested for both locales, both themes and the 200 % font
//! scale without a display.

use serde_json::{Value, json};

use crate::app::State;
use crate::theme::{self, Emphasis};
use crate::view_model::{AccountView, Action, Credential, Face, FocusTarget, GreeterViewModel};
use inox_config::Theme as Tokens;

/// Width of the login window, in logical pixels.
/// (the credential is a mask, ADR 0015), so the ring is the projection's answer to
/// Window geometry is a layout decision of this component rather than a theme
/// token, so it lives in Rust and QML binds to it. 1280×720 is the size
/// acceptance Q02 fixes for the component's evidence.
pub const WINDOW_WIDTH: i32 = 1280;

/// Height of the login window, in logical pixels.
pub const WINDOW_HEIGHT: i32 = 720;

/// How much of the window the card takes, as a ratio of the margins beside it.
///
/// A proportion rather than a width: the theme tokens carry no size for a card,
/// and a screen that were a fixed number of pixels wide would be wrong on every
/// window it did not match.
const CARD_PORTION: u32 = 2;

/// The portion of the window left free on each side of the card.
const MARGIN_PORTION: u32 = 1;

/// How much of the card a control takes, as a ratio of the gutters beside it.
///
/// A password field is not a banner: two thirds of the card is a width that says
/// how much goes in it, where the full width would say "write an essay"
/// (guidelines §4).
const CONTROL_PORTION: u32 = 4;

/// The gutter on each side of a control.
const CONTROL_GUTTER_PORTION: u32 = 1;

/// The projection of `state`, as one JSON string for the QML property.
///
/// A serialisation that cannot fail (the value is built here, not parsed) still
/// ends in a total answer rather than a panic: the interface gets an empty object
/// and the failure is logged, because a login screen with no model is bad and a
/// login screen that aborts is worse.
pub fn model_json(state: &State) -> String {
    let value = projection(state);

    match serde_json::to_string(&value) {
        Ok(serialised) => serialised,
        Err(error) => {
            tracing::warn!(%error, "the login screen's projection could not be serialised");
            "{}".to_owned()
        }
    }
}

/// The projection of `state`, as a value.
pub fn projection(state: &State) -> Value {
    let model = state.model();
    let mut root = scaffolding(state.tokens());

    // The window title is the one string the scaffolding cannot invent: it is the
    // localized `greet-window-title` the model carries, and the tree reports it
    // back through `reportReady`.
    root["window"]["title"] = Value::from(model.title.clone());
    root["ui"] = ui_value(model);

    root
}

/// The projection the QML element starts from when no state has been installed.
///
/// The engine instantiates the Rust-backed element itself, which can happen
/// before (or without) the binary installing a state — in a unit test, or if the
/// host is installed late. This keeps every QML binding total in that case: the
/// token values are real, and every string is empty rather than missing.
pub fn scaffolding_json(tokens: &Tokens) -> String {
    let mut root = scaffolding(tokens);
    root["ui"] = ui_value(&neutral_model());

    match serde_json::to_string(&root) {
        Ok(serialised) => serialised,
        Err(error) => {
            tracing::warn!(%error, "the login screen's scaffolding could not be serialised");
            "{}".to_owned()
        }
    }
}

/// The token half of the projection: window geometry, colours, distances and type
/// sizes. Everything here comes from [`inox_config::Theme`].
fn scaffolding(tokens: &Tokens) -> Value {
    let fonts = &tokens.typography;
    let colors = tokens.colors;

    json!({
        "window": {
            "title": "",
            "width": WINDOW_WIDTH,
            "height": WINDOW_HEIGHT,
            "background": theme::qml_color(colors.background),
            "text": theme::qml_color(colors.text),
        },
        "fonts": {
            "family": fonts.font_family,
            "monoFamily": fonts.font_mono_family,
            "title": theme::title_size(tokens),
            "heading": theme::heading_size(tokens),
            "body": theme::body_size(tokens),
            "caption": theme::caption_size(tokens),
        },
        "space": {
            "xs": tokens.spacing.xs,
            "sm": tokens.spacing.sm,
            "md": tokens.spacing.md,
            "lg": tokens.spacing.lg,
            "xl": tokens.spacing.xl,
        },
        "radius": {
            "sm": theme::control_radius(tokens),
            "md": theme::card_radius(tokens),
        },
        "metrics": {
            "hairline": theme::hairline(tokens),
            "focusRing": theme::focus_ring_width(tokens),
            "controlPadding": theme::control_padding(tokens),
            "medallion": theme::medallion_size(tokens),
            "rowMedallion": theme::row_medallion_size(tokens),
        },
        "layout": {
            "cardPortion": CARD_PORTION,
            "marginPortion": MARGIN_PORTION,
            "controlPortion": CONTROL_PORTION,
            "controlGutterPortion": CONTROL_GUTTER_PORTION,
        },
        // The focus ring is the one token that is legible on the fill it is drawn
        // on (guidelines §5, acceptance Q01): the accent on a plain control and on
        // the field, the on-accent colour on the filled log-in button and on the
        // row a login would use. It is resolved here rather than in QML because it
        // is a decision, not a value.
        "focusRing": {
            "onBackground": theme::focus_ring(tokens, colors.background),
            "onSurface": theme::focus_ring(tokens, colors.surface),
            "onAccent": theme::focus_ring(tokens, colors.accent),
        },
        "colors": {
            "background": theme::qml_color(colors.background),
            "surface": theme::qml_color(colors.surface),
            "surfaceVariant": theme::qml_color(colors.surface_variant),
            "border": theme::qml_color(colors.border),
            "text": theme::qml_color(colors.text),
            "textMuted": theme::qml_color(colors.text_muted),
            "accent": theme::qml_color(colors.accent),
            "onAccent": theme::qml_color(colors.on_accent),
            "danger": theme::qml_color(colors.danger),
            "warning": theme::qml_color(colors.warning),
            "success": theme::qml_color(colors.success),
        },
    })
}

/// The interface half of the projection, from the view model.
fn ui_value(model: &GreeterViewModel) -> Value {
    let focused = |target: FocusTarget| model.focus == Some(target);

    json!({
        "choosing": model.choosing,
        "accountsHeading": model.choosing.then(|| text_line(&model.accounts_heading, Emphasis::None, "heading")),
        "accounts": model
            .accounts
            .iter()
            .enumerate()
            .map(|(index, account)| account_value(account, focused(FocusTarget::account(index))))
            .collect::<Vec<Value>>(),
        "selected": model.selected.as_ref().map(|account| json!({
            "label": account.label,
            "initial": account.initial,
        })),
        "accountsReason": model
            .accounts_reason
            .as_ref()
            .map(|reason| text_line(reason, Emphasis::Warning, "caption")),
        "banner": text_line(&model.banner, Emphasis::Warning, "caption"),
        "credential": credential_value(&model.credential, focused(FocusTarget::Credential)),
        "face": face_value(&model.face),
        "submit": action_value(&model.submit, focused(FocusTarget::Submit)),
        "switchUser": model
            .switch_user
            .as_ref()
            .map(|action| action_value(action, focused(FocusTarget::SwitchUser))),
        "cancel": action_value(&model.cancel, focused(FocusTarget::Cancel)),
        "notice": model.notice.as_ref().map(|notice| text_line(notice, Emphasis::Warning, "caption")),
        "hint": model.hint.as_ref().map(|hint| text_line(hint, Emphasis::Muted, "caption")),
        "sessionLabel": text_line(&model.session_label, Emphasis::Muted, "caption"),
        "session": text_line(&model.session, Emphasis::None, "caption"),
        "focus": model.focus.map(focus_key).unwrap_or(Value::Null),
        "accountsFocusIndex": model
            .focus
            .and_then(|target| match target {
                FocusTarget::Account(index) => Some(i64::from(index)),
                _ => None,
            })
            .unwrap_or(-1),
        // The number of items the QML tree must expose with an accessibility
        // role (root AGENTS.md §7.4). The tree counts the same nodes and reports
        // the number back, so a control that lost its `Accessible.role` fails the
        // headless check instead of passing silently.
        "a11yItems": a11y_items(model),
    })
}

/// One account, as the account list draws it.
fn account_value(account: &AccountView, focused: bool) -> Value {
    json!({
        "name": account.name,
        "label": account.label,
        "initial": account.initial,
        "selected": account.selected,
        "focused": focused,
    })
}

/// The credential field.
///
/// The value is absent by construction: what QML draws is the mask, which carries
/// none of the credential's bytes.
fn credential_value(credential: &Credential, focused: bool) -> Value {
    // The same rendering rules the Iced tree had: a mask where something was
    // typed (muted when the field cannot be typed into), the placeholder where
    // the field is still empty and typable, and nothing at all where it is empty
    // and disabled — a placeholder there would be an invitation to fill it in.
    let (body, muted) = if !credential.mask.is_empty() {
        (Some(credential.mask.clone()), !credential.editable)
    } else if credential.editable {
        (Some(credential.placeholder.clone()), true)
    } else {
        (None, false)
    };

    let body = body.map(|text| {
        text_line(
            &text,
            if muted {
                Emphasis::Muted
            } else {
                Emphasis::None
            },
            "body",
        )
    });

    json!({
        "label": text_line(&credential.label, Emphasis::Muted, "caption"),
        "body": body,
        "error": credential
            .error
            .as_ref()
            .map(|error| text_line(error, Emphasis::Danger, "caption")),
        "editable": credential.editable,
        "focused": focused,
        "disabledReason": credential
            .disabled_reason
            .as_ref()
            .map(|reason| text_line(reason, Emphasis::Muted, "caption")),
    })
}

/// One labelled control, and whether it can be used.
fn action_value(action: &Action, focused: bool) -> Value {
    json!({
        "label": action.label,
        "enabled": action.enabled,
        "focused": focused,
        "reason": action
            .disabled_reason
            .as_ref()
            .map(|reason| text_line(reason, Emphasis::Warning, "caption")),
    })
}

/// The state under the account name.
fn face_value(face: &Face) -> Value {
    match face {
        Face::Prompt => json!({"kind": "prompt", "line": Value::Null, "reason": Value::Null}),
        Face::Authenticating { line } => json!({
            "kind": "authenticating",
            "line": text_line(line, Emphasis::Muted, "body"),
            "reason": Value::Null,
        }),
        Face::Starting { line } => json!({
            "kind": "starting",
            "line": text_line(line, Emphasis::Muted, "body"),
            "reason": Value::Null,
        }),
        Face::Failed { line } => json!({
            "kind": "failed",
            "line": text_line(line, Emphasis::Danger, "body"),
            "reason": Value::Null,
        }),
        Face::Started { line } => json!({
            "kind": "started",
            "line": text_line(line, Emphasis::Success, "body"),
            "reason": Value::Null,
        }),
        Face::Unavailable { line, reason } => json!({
            "kind": "unavailable",
            "line": text_line(line, Emphasis::Danger, "body"),
            "reason": text_line(reason, Emphasis::Muted, "caption"),
        }),
    }
}

/// One line of text: what it says, which token colour it is drawn in, and which
/// type role sizes it.
fn text_line(text: &str, emphasis: Emphasis, size: &str) -> Value {
    json!({
        "text": text,
        "color": emphasis.key(),
        "size": size,
    })
}

/// The name a focus target has in the projection.
fn focus_key(target: FocusTarget) -> Value {
    Value::from(match target {
        FocusTarget::Account(_) => "account",
        FocusTarget::Credential => "credential",
        FocusTarget::Submit => "submit",
        FocusTarget::SwitchUser => "switchUser",
        FocusTarget::Cancel => "cancel",
    })
}

/// The number of nodes the QML tree must expose with an accessibility role.
///
/// This mirrors `qml/Main.qml`'s tree exactly, which is the point: the headless
/// check compares the count the tree walks against this number, so a control that
/// lost its `Accessible.role` — or a row that stopped being announced — fails
/// instead of quietly shrinking the semantic tree.
pub fn a11y_items(model: &GreeterViewModel) -> usize {
    let optional = |present: bool| usize::from(present);

    // The content pane itself.
    let pane = 1;

    if model.choosing {
        // Heading, one row per account, and the way back.
        return pane + 1 + model.accounts.len() + 1;
    }

    // The card's head (the medallion and the account's name), what the screen says
    // about this build, the credential field itself and the label and value it
    // draws, and the credential's error.
    let head = if model.selected.is_some() { 2 } else { 0 };
    let banner = 1;
    // The field is one node (the container the tree gives the `EditableText` role
    // to), its label is another, and the value line is a third that only exists
    // while there is something to draw.
    let credential_field = 1;
    let credential_label = 1;
    let credential_body = optional(credential_body_present(&model.credential));
    let credential_error = optional(model.credential.error.is_some());
    // The face: a line, and for an unavailable authenticator a reason as well.
    let face = optional(face_line_present(&model.face));
    let face_reason = optional(matches!(model.face, Face::Unavailable { .. }));
    let notice = optional(model.notice.is_some());
    let accounts_reason = optional(model.accounts_reason.is_some());
    // The controls: the log-in action, the switch action where there is one, and
    // the clear/back control, plus the reason drawn under the log-in action when it
    // cannot be used; then the hint and the session line.
    let controls = 1 + optional(model.switch_user.is_some()) + 1;
    let submit_reason = optional(model.submit.disabled_reason.is_some());
    let hint = optional(model.hint.is_some());
    let session = 2;

    pane + head
        + accounts_reason
        + banner
        + credential_field
        + credential_label
        + credential_body
        + credential_error
        + face
        + face_reason
        + notice
        + controls
        + submit_reason
        + hint
        + session
}

/// Whether the credential field draws a value line at all.
fn credential_body_present(credential: &Credential) -> bool {
    !credential.mask.is_empty() || credential.editable
}

/// Whether the face draws a line under the account name.
fn face_line_present(face: &Face) -> bool {
    !matches!(face, Face::Prompt)
}

/// A view model with nothing in it, for [`scaffolding_json`].
fn neutral_model() -> GreeterViewModel {
    GreeterViewModel {
        title: String::new(),
        selected: None,
        accounts: Vec::new(),
        choosing: false,
        accounts_heading: String::new(),
        credential: Credential {
            label: String::new(),
            placeholder: String::new(),
            mask: String::new(),
            error: None,
            editable: false,
            disabled_reason: None,
        },
        banner: String::new(),
        session_label: String::new(),
        session: String::new(),
        face: Face::Prompt,
        submit: Action {
            label: String::new(),
            enabled: false,
            disabled_reason: None,
        },
        switch_user: None,
        cancel: Action {
            label: String::new(),
            enabled: false,
            disabled_reason: None,
        },
        accounts_reason: None,
        notice: None,
        hint: None,
        focus_order: Vec::new(),
        focus: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{Account, Accounts};
    use crate::app;
    use crate::i18n;
    use crate::state::{AuthReply, Phase};
    use inox_common::i18n::{Locale, Localizer};
    use inox_config::{Appearance, BUILTIN_DARK, BUILTIN_LIGHT};
    use std::collections::BTreeSet;
    use std::rc::Rc;

    /// The accounts a test offers: two ordinary accounts in uid order.
    fn roster() -> Accounts {
        Accounts {
            accounts: vec![
                Account::new("alice", 1000, Some("Alice Doe".to_owned())),
                Account::new("bob", 1001, None),
            ],
            ..Accounts::default()
        }
    }

    fn localizer(tag: &str) -> Rc<Localizer> {
        let locale = Locale::parse(tag).expect("a well-formed locale");
        Rc::new(i18n::load(&locale).expect("the catalogs parse"))
    }

    /// A login screen with an authenticator, as the tests drive it.
    fn login() -> State {
        State::with_boundary(localizer("en-US"), Tokens::builtin_dark(), true, roster())
    }

    /// A login screen in another language and theme.
    fn login_with(tag: &str, tokens: Tokens) -> State {
        State::with_boundary(localizer(tag), tokens, true, roster())
    }

    fn projected(state: &State) -> Value {
        projection(state)
    }

    fn send(state: &mut State, message: app::Message) {
        let _ = app::update(state, message);
    }

    #[test]
    fn the_window_is_named_sized_and_coloured_from_the_tokens() {
        let state = login();
        let value = projected(&state);
        let tokens = state.tokens();

        assert_eq!(value["window"]["title"], "Inox login");
        assert_eq!(value["window"]["width"], WINDOW_WIDTH);
        assert_eq!(value["window"]["height"], WINDOW_HEIGHT);
        assert_eq!(
            value["window"]["background"],
            theme::qml_color(tokens.colors.background)
        );
        assert_eq!(
            value["colors"]["accent"],
            theme::qml_color(tokens.colors.accent)
        );
        assert_eq!(
            value["colors"]["onAccent"],
            theme::qml_color(tokens.colors.on_accent)
        );
        assert_eq!(
            value["colors"]["surfaceVariant"],
            theme::qml_color(tokens.colors.surface_variant)
        );
    }

    #[test]
    fn every_colour_is_a_token_and_every_token_is_a_colour() {
        for tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
            let state = State::with_boundary(localizer("en-US"), tokens.clone(), true, roster());
            let value = projected(&state);
            let colors = value["colors"].as_object().expect("an object of colours");

            // The conversion is the theme's, channel for channel.
            assert_eq!(
                colors["background"],
                theme::qml_color(tokens.colors.background)
            );
            assert_eq!(colors["surface"], theme::qml_color(tokens.colors.surface));
            assert_eq!(colors["border"], theme::qml_color(tokens.colors.border));
            assert_eq!(colors["text"], theme::qml_color(tokens.colors.text));
            assert_eq!(
                colors["textMuted"],
                theme::qml_color(tokens.colors.text_muted)
            );
            assert_eq!(colors["danger"], theme::qml_color(tokens.colors.danger));
            assert_eq!(colors["warning"], theme::qml_color(tokens.colors.warning));
            assert_eq!(colors["success"], theme::qml_color(tokens.colors.success));

            for (name, colour) in colors {
                let colour = colour.as_str().expect("a colour string");
                assert_eq!(colour.len(), 9, "{name} is not #AARRGGBB: {colour}");
                assert!(colour.starts_with('#'), "{name}: {colour}");
            }
        }
    }

    #[test]
    fn the_focus_ring_is_a_token_colour_that_is_visible_on_the_fill() {
        let state = login();
        let value = projected(&state);
        let tokens = state.tokens();
        let ring = &value["focusRing"];

        assert_eq!(
            ring["onBackground"],
            theme::qml_color(tokens.colors.accent),
            "the ring on a plain control is the accent"
        );
        assert_eq!(
            ring["onSurface"],
            theme::qml_color(tokens.colors.accent),
            "the ring on the card is the accent too"
        );
        assert_eq!(
            ring["onAccent"],
            theme::qml_color(tokens.colors.on_accent),
            "the ring on the filled log-in button cannot be the accent it is filled with"
        );
        assert_ne!(ring["onBackground"], ring["onAccent"]);
        assert_ne!(ring["onAccent"], value["colors"]["accent"]);
    }

    #[test]
    fn every_line_names_a_token_colour_that_exists() {
        // A line says which token it is drawn with, and QML resolves it through
        // `build.colors`. A key the object does not carry would be a binding that
        // silently renders nothing, so the keys are checked here.
        let mut state = login();
        send(&mut state, app::Message::SwitchUser);
        let choosing = projected(&state);
        send(&mut state, app::Message::Cancelled);
        let login = projected(&state);

        for value in [choosing, login] {
            let colors = value["colors"].as_object().expect("colours");
            let mut lines = Vec::new();
            collect_lines(&value["ui"], &mut lines);
            assert!(!lines.is_empty(), "the projection draws no lines at all");

            for line in lines {
                let key = line["color"].as_str().expect("a colour key");
                assert!(colors.contains_key(key), "`{key}` is not a token colour");
                let size = line["size"].as_str().expect("a type role");
                assert!(
                    ["title", "heading", "body", "caption"].contains(&size),
                    "`{size}` is not one of the four type roles"
                );
                assert!(
                    !line["text"].as_str().expect("text").is_empty(),
                    "an empty line is a layout bug with a colour"
                );
            }
        }
    }

    /// Every line object anywhere under `value`.
    fn collect_lines(value: &Value, into: &mut Vec<Value>) {
        match value {
            Value::Object(map) => {
                if map.contains_key("text") && map.contains_key("color") && map.contains_key("size")
                {
                    into.push(value.clone());
                }
                for child in map.values() {
                    collect_lines(child, into);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| collect_lines(item, into)),
            _ => {}
        }
    }

    #[test]
    fn the_sizes_are_the_ones_the_tokens_derive() {
        let mut tokens = Tokens::builtin_dark();
        tokens.typography.scale = 2.0;
        let state = State::with_boundary(localizer("en-US"), tokens.clone(), true, roster());
        let value = projected(&state);

        assert_eq!(value["fonts"]["body"], theme::body_size(&tokens));
        assert_eq!(value["fonts"]["title"], theme::title_size(&tokens));
        assert_eq!(value["fonts"]["heading"], theme::heading_size(&tokens));
        assert_eq!(value["fonts"]["caption"], theme::caption_size(&tokens));
        assert_eq!(value["fonts"]["body"], 2.0 * f64::from(tokens.spacing.lg));
        assert_eq!(value["space"]["lg"], tokens.spacing.lg);
        assert_eq!(value["space"]["md"], tokens.spacing.md);
        assert_eq!(value["radius"]["sm"], tokens.radii.sm);
        assert_eq!(value["radius"]["md"], tokens.radii.md);
        assert_eq!(value["metrics"]["hairline"], theme::hairline(&tokens));
        assert_eq!(
            value["metrics"]["focusRing"],
            theme::focus_ring_width(&tokens)
        );
        assert_eq!(value["metrics"]["controlPadding"], tokens.spacing.md);
        assert_eq!(
            value["metrics"]["medallion"],
            theme::medallion_size(&tokens)
        );
        assert_eq!(
            value["metrics"]["rowMedallion"],
            theme::row_medallion_size(&tokens)
        );
        assert_eq!(value["fonts"]["family"], tokens.typography.font_family);
    }

    #[test]
    fn the_strings_follow_the_negotiated_language() {
        let en = projected(&login_with("en-US", Tokens::builtin_dark()));
        let zh = projected(&login_with("zh-CN", Tokens::builtin_dark()));

        assert_eq!(en["ui"]["credential"]["label"]["text"], "Password");
        assert_eq!(zh["ui"]["credential"]["label"]["text"], "口令");
        assert_eq!(en["ui"]["submit"]["label"], "Log in");
        assert_eq!(zh["ui"]["submit"]["label"], "登录");
        assert_eq!(en["window"]["title"], "Inox login");
        assert_eq!(zh["window"]["title"], "Inox 登录");
        assert_eq!(zh["ui"]["session"]["text"], "Inox");
        assert_eq!(zh["ui"]["banner"]["text"], "登录后将启动 Inox 会话。");
    }

    #[test]
    fn both_themes_and_both_encodings_are_projected() {
        // The old render matrix was two themes × two locales × the font scale;
        // the projection carries every value that matrix varied.
        for tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
            for tag in ["en-US", "zh-CN"] {
                let state = login_with(tag, tokens.clone());
                let value = projected(&state);

                assert_eq!(
                    value["colors"]["background"],
                    theme::qml_color(tokens.colors.background)
                );
                assert_eq!(
                    value["ui"]["credential"]["label"]["text"],
                    state.model().credential.label
                );
                assert_ne!(
                    value["ui"]["credential"]["label"]["text"],
                    Value::Null,
                    "a locale produced no credential label"
                );
            }
        }

        assert_eq!(
            Tokens::builtin_light().appearance,
            Appearance::Light,
            "the light theme must stay light"
        );
    }

    #[test]
    fn the_account_a_login_would_use_is_the_selected_row_and_the_head() {
        let state = login();
        let value = projected(&state);

        assert_eq!(value["ui"]["selected"]["label"], "Alice Doe");
        assert_eq!(value["ui"]["selected"]["initial"], "A");

        let accounts = value["ui"]["accounts"].as_array().expect("rows");
        assert_eq!(accounts.len(), 2, "the screen draws the roster it holds");
        assert_eq!(accounts[0]["name"], "alice");
        assert_eq!(accounts[0]["selected"], true);
        assert_eq!(accounts[1]["name"], "bob");
        assert_eq!(accounts[1]["selected"], false);
        assert_eq!(accounts[1]["label"], "bob");
        assert_eq!(accounts[1]["initial"], "B");
    }

    #[test]
    fn the_account_list_is_a_list_of_reachable_rows_and_a_way_back() {
        let mut state = login();
        send(&mut state, app::Message::SwitchUser);
        let value = projected(&state);

        assert_eq!(value["ui"]["choosing"], true);
        assert_eq!(value["ui"]["accountsHeading"]["text"], "Choose an account");
        assert_eq!(value["ui"]["cancel"]["label"], "Back");
        assert_eq!(
            state.model().focus_order,
            vec![
                FocusTarget::Account(0),
                FocusTarget::Account(1),
                FocusTarget::Cancel
            ],
            "the rows are reachable with the keyboard, and so is the way back"
        );
        assert_eq!(value["ui"]["focus"], "account");
        assert_eq!(value["ui"]["accountsFocusIndex"], 0);
    }

    #[test]
    fn the_focus_ring_is_projected_onto_exactly_one_control() {
        let mut state = login();
        let value = projected(&state);

        assert_eq!(value["ui"]["credential"]["focused"], true);
        assert_eq!(value["ui"]["submit"]["focused"], false);
        assert_eq!(value["ui"]["cancel"]["focused"], false);

        send(&mut state, app::Message::FocusNext);
        let value = projected(&state);
        assert_eq!(value["ui"]["credential"]["focused"], false);
        assert_eq!(value["ui"]["submit"]["focused"], true);
    }

    #[test]
    fn a_screen_that_cannot_log_anyone_in_projects_no_typable_field() {
        let state = State::new(localizer("en-US"), Tokens::builtin_dark(), roster());
        let value = projected(&state);

        assert_eq!(value["ui"]["credential"]["editable"], false);
        assert_eq!(value["ui"]["credential"]["body"], Value::Null);
        assert_eq!(value["ui"]["submit"]["enabled"], false);
        assert_eq!(value["ui"]["face"]["kind"], "unavailable");
        assert!(
            value["ui"]["banner"]["text"]
                .as_str()
                .is_some_and(|text| text.contains("Preview")),
            "a build without a boundary says it is a preview: {}",
            value["ui"]["banner"]["text"]
        );
        assert!(
            value["ui"]["submit"]["reason"]["text"]
                .as_str()
                .is_some_and(|text| text.contains("not available")),
            "the control that cannot be used says why"
        );
    }

    #[test]
    fn a_boundary_build_does_not_call_itself_a_preview() {
        let state = login();
        let value = projected(&state);
        let banner = value["ui"]["banner"]["text"].as_str().expect("a banner");

        assert!(!banner.contains("Preview"), "{banner}");
        assert!(banner.contains("Inox"), "{banner}");
    }

    #[test]
    fn an_empty_account_list_is_said_beside_the_account_name() {
        let state = State::with_boundary(
            localizer("en-US"),
            Tokens::builtin_dark(),
            true,
            Accounts::default(),
        );
        let value = projected(&state);

        assert_eq!(value["ui"]["selected"], Value::Null);
        assert_eq!(
            value["ui"]["accountsReason"]["text"],
            "No account on this system can be logged in"
        );
        assert_eq!(value["ui"]["credential"]["editable"], false);
    }

    #[test]
    fn an_unreadable_account_database_says_which_problem_it_is() {
        let unreadable = Accounts {
            unreadable: true,
            ..Accounts::default()
        };
        let state =
            State::with_boundary(localizer("en-US"), Tokens::builtin_dark(), true, unreadable);
        let value = projected(&state);

        assert_eq!(
            value["ui"]["accountsReason"]["text"],
            "The list of accounts could not be read"
        );
    }

    #[test]
    fn a_name_the_boundary_would_refuse_is_projected_as_a_refusal() {
        let odd = Accounts {
            accounts: vec![Account::new("a b", 1000, None)],
            ..Accounts::default()
        };
        let state = State::with_boundary(localizer("en-US"), Tokens::builtin_dark(), true, odd);
        let value = projected(&state);

        assert_eq!(value["ui"]["submit"]["enabled"], false);
        assert_eq!(
            value["ui"]["submit"]["reason"]["text"],
            "That account name cannot be sent to the authentication service"
        );
        assert_eq!(
            value["ui"]["notice"]["text"], value["ui"]["submit"]["reason"]["text"],
            "the reason is on screen, once"
        );
    }

    #[test]
    fn an_attempt_in_flight_is_the_loading_face_and_offers_only_the_way_out() {
        let mut state = login();
        send(&mut state, app::Message::Submitted);
        let value = projected(&state);

        assert_eq!(value["ui"]["face"]["kind"], "authenticating");
        assert_eq!(
            value["ui"]["face"]["line"]["text"],
            "Checking your account…"
        );
        assert_eq!(value["ui"]["credential"]["editable"], false);
        assert_eq!(value["ui"]["switchUser"], Value::Null);
        assert_eq!(value["ui"]["cancel"]["enabled"], true);
        assert_eq!(value["ui"]["focus"], "cancel");
    }

    #[test]
    fn a_refused_login_is_a_retryable_danger_face() {
        let mut state = login();
        let _ = app::update(&mut state, app::Message::Submitted);
        let attempt = state
            .greeter()
            .attempt_in_flight()
            .expect("an attempt was submitted");

        send(
            &mut state,
            app::Message::AuthAnswered(attempt, AuthReply::Rejected),
        );
        let value = projected(&state);

        assert_eq!(value["ui"]["face"]["kind"], "failed");
        assert_eq!(value["ui"]["face"]["line"]["color"], "danger");
        assert_eq!(value["ui"]["credential"]["editable"], true);
        assert_eq!(value["ui"]["submit"]["enabled"], true);
    }

    #[test]
    fn a_started_session_projects_nothing_to_operate() {
        let mut state = login();
        let _ = app::update(&mut state, app::Message::Submitted);
        let attempt = state
            .greeter()
            .attempt_in_flight()
            .expect("an attempt was submitted");
        send(
            &mut state,
            app::Message::AuthAnswered(attempt, AuthReply::Accepted),
        );
        send(&mut state, app::Message::SessionAnswered(attempt, true));
        let value = projected(&state);

        assert_eq!(state.greeter().phase(), Phase::SessionStarted);
        assert_eq!(value["ui"]["face"]["kind"], "started");
        assert_eq!(value["ui"]["face"]["line"]["color"], "success");
        assert_eq!(value["ui"]["cancel"]["enabled"], false);
        assert_eq!(value["ui"]["focus"], Value::Null);
    }

    #[test]
    fn a_spent_allowance_says_a_wait_and_not_how_long() {
        let mut state = login();

        for _ in 0..3 {
            let _ = app::update(&mut state, app::Message::Submitted);
            let attempt = state
                .greeter()
                .attempt_in_flight()
                .expect("an attempt was submitted");
            send(
                &mut state,
                app::Message::AuthAnswered(attempt, AuthReply::Rejected),
            );
        }

        let value = projected(&state);
        let reason = value["ui"]["submit"]["reason"]["text"]
            .as_str()
            .expect("a reason");

        assert_eq!(reason, "Too many attempts for now");
        assert!(
            !reason.chars().any(|character| character.is_ascii_digit()),
            "`{reason}` counts down"
        );
    }

    #[test]
    fn the_mask_counts_the_credential_and_never_carries_it() {
        let mut state = login();
        let credential = "hunter2 swallow";
        for character in credential.chars() {
            send(&mut state, app::Message::Credential(character));
        }

        let json = model_json(&state);
        let value = projected(&state);
        let mask = value["ui"]["credential"]["body"]["text"]
            .as_str()
            .expect("the mask");

        assert_eq!(mask.chars().count(), credential.chars().count());
        assert!(
            mask.chars()
                .all(|glyph| glyph == crate::view_model::MASK_GLYPH),
            "the field draws one glyph per character, and no character"
        );
        assert!(
            !json.contains(credential),
            "the projection carries the password"
        );
        assert!(
            !json.contains("hunter2"),
            "the projection carries the password"
        );
        // And the credential really is gone from the model when it is submitted.
        send(&mut state, app::Message::Submitted);
        assert!(
            !model_json(&state).contains("hunter2"),
            "the projection after a submission carries the password"
        );
        assert_eq!(state.greeter().credential_len(), 0);
    }

    #[test]
    fn a_credential_past_the_contracts_bound_is_projected_as_an_error() {
        let mut state = login();
        for _ in 0..(inox_auth::wire::MAX_CREDENTIAL_BYTES + 8) {
            send(&mut state, app::Message::Credential('x'));
        }

        let value = projected(&state);

        assert_eq!(
            value["ui"]["credential"]["error"]["text"],
            "That password is too long."
        );
        assert_eq!(value["ui"]["credential"]["error"]["color"], "danger");
        assert_eq!(value["ui"]["submit"]["enabled"], false);
    }

    #[test]
    fn an_unavailable_authenticator_says_why_beside_the_face() {
        let mut state = login();
        let _ = app::update(&mut state, app::Message::Submitted);
        let attempt = state
            .greeter()
            .attempt_in_flight()
            .expect("an attempt was submitted");
        send(
            &mut state,
            app::Message::AuthAnswered(attempt, AuthReply::Unavailable),
        );

        let value = projected(&state);

        assert_eq!(value["ui"]["face"]["kind"], "unavailable");
        assert_eq!(value["ui"]["face"]["line"]["text"], "Log in cannot run");
        assert!(
            value["ui"]["face"]["reason"]["text"]
                .as_str()
                .is_some_and(|reason| reason.contains("authentication boundary")),
            "the reason names what is missing: {}",
            value["ui"]["face"]["reason"]["text"]
        );
    }

    #[test]
    fn the_focus_always_lands_on_a_control_that_can_be_used() {
        let mut state = login();
        send(&mut state, app::Message::FocusNext);
        assert_eq!(state.model().focus, Some(FocusTarget::Submit));

        // The boundary goes away, which takes the button with it.
        let _ = app::update(&mut state, app::Message::Submitted);
        let attempt = state
            .greeter()
            .attempt_in_flight()
            .expect("an attempt was submitted");
        send(
            &mut state,
            app::Message::AuthAnswered(attempt, AuthReply::Unavailable),
        );

        let value = projected(&state);
        assert_eq!(value["ui"]["focus"], "cancel");
        assert_eq!(
            value["ui"]["cancel"]["focused"], true,
            "the ring moved with the keyboard rather than staying on a control \
             that no longer exists"
        );
    }

    #[test]
    fn the_layout_proportions_come_from_rust_and_are_two_thirds_of_the_card() {
        let value = projected(&login());
        let layout = &value["layout"];

        assert_eq!(layout["cardPortion"], CARD_PORTION);
        assert_eq!(layout["marginPortion"], MARGIN_PORTION);
        assert_eq!(layout["controlPortion"], CONTROL_PORTION);
        assert_eq!(layout["controlGutterPortion"], CONTROL_GUTTER_PORTION);
        assert_eq!(
            layout["controlPortion"].as_u64().unwrap_or(0),
            2 * u64::from(CONTROL_GUTTER_PORTION) * 2,
            "a control and its two gutters are two thirds of the card"
        );
    }

    #[test]
    fn the_projection_and_the_scaffolding_have_the_same_shape() {
        // The engine can build the element before the host is installed, so the
        // scaffolding has to keep every binding QML has: a key that exists in one
        // and not the other is a QML binding error waiting for a race.
        //
        // The comparison is structural rather than a flat list of key paths,
        // because two documents of the same shape legitimately differ in values —
        // an empty account list, a hint that only the signed-out state carries.
        // What must not differ is the set of keys at a path both documents reach.
        let projected = projection(&login());
        let scaffolding: Value =
            serde_json::from_str(&scaffolding_json(&Tokens::builtin_dark())).expect("valid JSON");

        assert_same_shape(&projected, &scaffolding, "");
    }

    /// Asserts two documents are built from the same keys.
    ///
    /// Objects must carry the same keys wherever both sides are objects; arrays
    /// are compared through their first element, so an empty list is a value and
    /// not a shape difference; `null` is a leaf, which is how QML reads it
    /// (`root.ui.selected !== null` guards every use).
    fn assert_same_shape(left: &Value, right: &Value, path: &str) {
        match (left, right) {
            (Value::Object(left), Value::Object(right)) => {
                let left_keys: BTreeSet<&String> = left.keys().collect();
                let right_keys: BTreeSet<&String> = right.keys().collect();

                assert_eq!(
                    left_keys, right_keys,
                    "the projection and the scaffolding disagree under `{path}`"
                );

                for (key, value) in left {
                    assert_same_shape(value, &right[key], &format!("{path}.{key}"));
                }
            }
            (Value::Array(left), Value::Array(right)) => {
                if let (Some(left), Some(right)) = (left.first(), right.first()) {
                    assert_same_shape(left, right, &format!("{path}[0]"));
                }
            }
            _ => {}
        }
    }

    #[test]
    fn the_projection_serialises_to_one_valid_json_document() {
        let state = login();
        let json = model_json(&state);
        let parsed: Value = serde_json::from_str(&json).expect("the projection is valid JSON");

        assert_eq!(parsed["ui"]["submit"]["label"], "Log in");
        assert!(json.starts_with('{') && json.ends_with('}'));
    }

    #[test]
    fn the_accessible_tree_count_matches_the_nodes_the_tree_draws() {
        // Every one of these numbers is a node `qml/Main.qml` draws with an
        // `Accessible.role`; the headless check compares the count the tree walks
        // against the same formula.
        let state = login();
        assert_eq!(
            a11y_items(state.model()),
            // pane, medallion, account name, banner, the credential field, its
            // label, the value line, log in, switch user, clear, hint, session
            // label and session.
            13
        );

        let mut choosing = login();
        send(&mut choosing, app::Message::SwitchUser);
        assert_eq!(
            a11y_items(choosing.model()),
            // pane, heading, two rows, back.
            5
        );

        let unavailable = State::new(localizer("en-US"), Tokens::builtin_dark(), roster());
        assert_eq!(
            a11y_items(unavailable.model()),
            // pane, medallion, name, banner, the field and its label, the face's
            // line and its reason, log in and the reason under it, switch user,
            // clear, hint, session label and session — and no credential value,
            // because a field that cannot be typed into draws none.
            14
        );
    }

    #[test]
    fn the_scaffolding_is_usable_without_any_state() {
        let json = scaffolding_json(&Tokens::builtin_dark());
        let value: Value = serde_json::from_str(&json).expect("valid JSON");

        assert_eq!(value["window"]["title"], "");
        assert_eq!(value["window"]["width"], WINDOW_WIDTH);
        assert_eq!(value["ui"]["choosing"], false);
        assert_eq!(value["ui"]["accounts"], json!([]));
        assert_eq!(value["ui"]["face"]["kind"], "prompt");
        // The count is the neutral model's, not a placeholder: the pane, the build
        // banner, the credential field, its label, the log-in control, the clear
        // control and the two lines of the session row — the same nodes the tree
        // draws when it has no host, and no account head or credential value
        // because nothing is selected and nothing is typed.
        assert_eq!(value["ui"]["a11yItems"], 8);
    }

    #[test]
    fn the_builtin_dark_theme_is_the_one_the_scaffolding_names() {
        let value: Value =
            serde_json::from_str(&scaffolding_json(&Tokens::builtin_dark())).expect("valid JSON");

        assert_eq!(
            value["colors"]["background"],
            theme::qml_color(Tokens::builtin_dark().colors.background)
        );
        assert!(BUILTIN_DARK.starts_with("inox"));
        assert_ne!(BUILTIN_LIGHT, BUILTIN_DARK);
        assert_ne!(
            Tokens::builtin_dark().colors.background,
            Tokens::builtin_light().colors.background
        );
    }
}
