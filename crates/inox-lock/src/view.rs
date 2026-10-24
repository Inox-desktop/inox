//! The widget tree.
//!
//! The view makes no decisions: it walks the [`LockViewModel`] the projection
//! produced and draws it with the theme's tokens. Every colour, radius and
//! distance comes from [`crate::theme`], every string from the catalogs, and every
//! proportion from a [`Length::FillPortion`] ratio rather than a pixel count —
//! there is no literal colour or size in this file.
//!
//! # How the screen is laid out
//!
//! One card, centred, on the full-bleed scrim, and inside it three groups in the
//! order a person reads them: what the screen is and what it has to say (a
//! [`heading`](crate::theme::heading_size) and its caption lines), what there is
//! to type into when there is anything to type into, and what can be done about it
//! — the reason a control cannot be used, the control itself, and the hint about
//! the key that reaches it. Groups are `lg` apart and lines inside a group `sm`,
//! which is what makes the grouping visible without a rule being drawn
//! (guidelines §3).
//!
//! # The two things that matter more than the layout
//!
//! - **The field is not a text box, and the screen is not a prompt it cannot
//!   honour.** The credential is drawn as a mask built from its *length*
//!   ([`crate::view_model::MASK_GLYPH`]), and keystrokes arrive as messages from
//!   the keyboard subscription ([`crate::app::key_binding`]). That is deliberate:
//!   iced's own text editor keeps a `String` of its own that this crate cannot
//!   zeroize, it offers copy and paste, and a future widget change could put the
//!   plaintext somewhere nobody is looking — so a screenshot of the lock screen has
//!   nothing to redact (acceptance Q02, ADR 0015). The field is drawn only where
//!   [`LockViewModel::prompts_for_credential`] is true, which is false for every
//!   state in which nothing can be checked: this build ships no authentication
//!   boundary at all, so its screen states what it is instead of asking for a
//!   password it would discard (ADR 0025 decision 5).
//! - **Nothing on this surface is drawn as operable that is not.** The lock
//!   surface reads no keyboard and no pointer, so the view draws no focus ring, no
//!   hover and no pressed state: those are states the surface cannot reach, and
//!   painting them would be a picture of an input path that does not exist. The
//!   controls that remain are drawn muted, with the reason they cannot be used
//!   printed beside them (guidelines §5, acceptance D05).
//!
//! The card takes a share of the covered surface rather than a fixed width,
//! because the tokens carry no width for it and a lock screen is shown at whatever
//! size the output is. The fields inside it take a share of the card for the same
//! reason in the other direction: a control that filled the window would claim to
//! want more than a password (guidelines §4).

use iced::widget::{Space, button, column, container, row, text};
use iced::{Element, Length};

use crate::app::{Message, State};
use crate::theme::{self, Emphasis};
use crate::view_model::{Face, LockViewModel};
use inox_config::Theme as Tokens;

/// How much of the surface the card takes, as a ratio of the columns beside it.
const CARD_PORTION: u16 = 2;

/// The portion of the surface left free on each side of the card.
const MARGIN_PORTION: u16 = 1;

/// How much of the card a field or a control takes, as a ratio of the gutter
/// beside it.
///
/// A control is a password field, not a banner: two thirds of the card is a width
/// that says how much goes in it, where the full width would say "write an essay"
/// (guidelines §4).
const CONTROL_PORTION: u16 = 2;

/// The gutter that keeps a control off the card's edge.
const CONTROL_GUTTER_PORTION: u16 = 1;

/// The lock screen's widget tree.
pub fn view(state: &State) -> Element<'_, Message> {
    let model = state.model();
    let tokens = state.tokens();

    let card = container(column(groups(model, tokens)).spacing(f32::from(tokens.spacing.lg)))
        .width(Length::FillPortion(CARD_PORTION))
        .padding(f32::from(tokens.spacing.lg))
        .style(theme::card(tokens));

    let content = row![
        Space::new().width(Length::FillPortion(MARGIN_PORTION)),
        card,
        Space::new().width(Length::FillPortion(MARGIN_PORTION)),
    ]
    .width(Length::Fill);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_y(Length::Fill)
        .padding(f32::from(tokens.spacing.xl))
        .style(theme::scrim(tokens))
        .into()
}

/// The card's groups, in the order the user reads them.
fn groups<'a>(model: &'a LockViewModel, tokens: &'a Tokens) -> Vec<Element<'a, Message>> {
    let mut groups = vec![
        // What the screen is, and what it has to say about itself.
        column(head_lines(model, tokens))
            .spacing(f32::from(tokens.spacing.sm))
            .into(),
    ];

    // What there is to type into — and nothing at all where nothing can be
    // checked. A password box under a sentence saying no password will be checked
    // is the same invitation as a prompt, in a smaller font.
    if model.prompts_for_credential {
        groups.push(field(model, tokens));
    }

    groups.push(action_group(model, tokens));

    groups
}

/// The heading, and every line that explains or reports under it.
fn head_lines<'a>(model: &'a LockViewModel, tokens: &'a Tokens) -> Vec<Element<'a, Message>> {
    let mut lines = vec![
        text(model.headline.clone())
            .size(theme::heading_size(tokens))
            .color(theme::text(tokens))
            .into(),
    ];

    match &model.face {
        Face::Prompt => {}
        Face::Verifying { line } => lines.push(body_line(line.clone(), tokens, Emphasis::Muted)),
        // A refusal is two facts and they are read in that order: what happened,
        // and what the boundary said about it.
        Face::Failed { line, reason } => {
            lines.push(body_line(line.clone(), tokens, Emphasis::Danger));
            lines.push(caption_line(reason.clone(), tokens, Emphasis::Muted));
        }
        Face::Waiting { line } => lines.push(body_line(line.clone(), tokens, Emphasis::Warning)),
        Face::HeldWithoutUi { detail } => {
            lines.push(caption_line(detail.clone(), tokens, Emphasis::Muted));
        }
        Face::Unavailable { reason, .. } => {
            lines.push(caption_line(reason.clone(), tokens, Emphasis::Danger));
        }
    }

    lines
}

/// The control the screen offers, and everything that has to be said about it.
///
/// The reason a control cannot be used is drawn here rather than in the header:
/// it is about the control, and guidelines §3 puts a label `sm` away from its
/// control rather than in another group. The hint is here for the same reason, and
/// is absent wherever the keyboard has nothing to do — a hint about pressing Enter
/// on a screen with nothing to press is worse than no hint.
fn action_group<'a>(model: &'a LockViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let mut children = Vec::new();

    if let Some(reason) = &model.disabled_reason {
        children.push(caption_line(reason.clone(), tokens, Emphasis::Warning));
    }

    if let Some(label) = retry_label(&model.face) {
        children.push(retry(label, tokens));
    }

    children.push(submit(model, tokens));

    if let Some(hint) = &model.hint {
        children.push(caption_line(hint.clone(), tokens, Emphasis::Muted));
    }

    column(children)
        .spacing(f32::from(tokens.spacing.sm))
        .into()
}

/// The control that can bring an unavailable boundary back, when one could.
///
/// A boundary this build does not contain cannot be retried into existence, so
/// the projection offers no label there and this draws no control: the screen
/// must not hold out an action that cannot do anything (guidelines §6).
fn retry_label(face: &Face) -> Option<&str> {
    match face {
        Face::Unavailable { retry, .. } => retry.as_deref(),
        _ => None,
    }
}

/// One line of body text.
fn body_line<'a>(content: String, tokens: &'a Tokens, emphasis: Emphasis) -> Element<'a, Message> {
    line(content, theme::body_size(tokens), tokens, emphasis)
}

/// One line of caption text: what a line explains, never what it asks.
fn caption_line<'a>(
    content: String,
    tokens: &'a Tokens,
    emphasis: Emphasis,
) -> Element<'a, Message> {
    line(content, theme::caption_size(tokens), tokens, emphasis)
}

/// One line of text, at the size and colour its role asks for.
fn line<'a>(
    content: String,
    size: f32,
    tokens: &'a Tokens,
    emphasis: Emphasis,
) -> Element<'a, Message> {
    text(content)
        .size(size)
        .color(emphasis.colour(tokens))
        .into()
}

/// A control at the width a control carries rather than the width of the card.
fn control_row<'a>(control: Element<'a, Message>) -> Element<'a, Message> {
    row![
        control,
        Space::new().width(Length::FillPortion(CONTROL_GUTTER_PORTION)),
    ]
    .width(Length::Fill)
    .into()
}

/// The credential field: a label, and the mask the projection built.
fn field<'a>(model: &'a LockViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let empty = model.mask.is_empty();
    let value = if empty {
        model.placeholder.clone()
    } else {
        model.mask.clone()
    };

    let frame = container(
        column![
            caption_line(model.field_label.clone(), tokens, Emphasis::Muted),
            line(value, theme::body_size(tokens), tokens, Emphasis::None),
        ]
        .spacing(f32::from(tokens.spacing.sm)),
    )
    .width(Length::FillPortion(CONTROL_PORTION))
    .padding(theme::control_padding(tokens))
    .style(theme::field(tokens, empty));

    control_row(frame.into())
}

/// The unlock button.
///
/// A button that cannot be pressed is a button with no `on_press`, which is how
/// iced reports it as disabled — so "the control is disabled" and "the control
/// sends nothing" are the same fact rather than two that could disagree.
fn submit<'a>(model: &'a LockViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let label = text(model.submit.label.clone()).size(theme::body_size(tokens));

    let control = button(label)
        .width(Length::FillPortion(CONTROL_PORTION))
        .padding(theme::control_padding(tokens))
        .style(theme::submit(tokens, model.submit.enabled));

    let control: Element<'a, Message> = if model.submit.enabled {
        control.on_press(Message::Submitted).into()
    } else {
        control.into()
    };

    control_row(control)
}

/// The retry control, drawn as the screen's secondary action.
fn retry<'a>(label: &'a str, tokens: &'a Tokens) -> Element<'a, Message> {
    let control = button(text(label.to_owned()).size(theme::body_size(tokens)))
        .width(Length::FillPortion(CONTROL_PORTION))
        .padding(theme::control_padding(tokens))
        .style(theme::action(tokens))
        .on_press(Message::BoundaryRetried);

    control_row(control.into())
}
