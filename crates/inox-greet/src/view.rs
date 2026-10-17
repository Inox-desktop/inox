//! The widget tree.
//!
//! The view makes no decisions: it walks the [`GreeterViewModel`] the projection
//! produced and draws it with the theme's tokens. Every colour, radius and
//! distance comes from [`crate::theme`], every string from the catalogs, and
//! every proportion from a [`Length::FillPortion`] ratio rather than a pixel
//! count — there is no literal colour or size in this file.
//!
//! Two things are worth stating here rather than leaving to be discovered.
//!
//! - **The identity is a real text box and the credential never is.** A username
//!   is public data: it is shown on a lock screen, it is read out to the person
//!   sitting there, and typing it wants a caret, selection, an input method and
//!   paste. So the field is an ordinary `iced` text input. What this crate must
//!   never do is put a *credential* in one, because iced's editor keeps a `String`
//!   of its own that this crate cannot zeroize (ADR 0015). Nothing here collects
//!   a password at all: `state::Greeter` has no field to put one in.
//! - **The form is drawn from the top, not centred.** The lock screen centres its
//!   card because it is a full-screen cover with one thing to say; a login screen
//!   is a dialog with a form, a session row and two controls, and it is inside a
//!   window that can be any size. It stays inside a `scrollable` so that 200 % text
//!   or a long localized line pushes the content down rather than clipping it
//!   (acceptance Q02), which a vertically centred tree cannot do — a centred
//!   `Length::Fill` inside a scrollable resolves against the viewport and stops the
//!   content from ever being taller than it.
//! - **The focus ring is drawn from the projection, not from the pointer, and the
//!   field is an input only where that ring is.** Every control's ring comes from
//!   [`GreeterViewModel::focus`], which the update loop moves with Tab, so "the
//!   focus is visible and unique" (acceptance Q01) is the same fact the keyboard
//!   acts on rather than a second one that could disagree. That matters because
//!   `iced` 0.14 has no focusable buttons and no way to take focus away from a
//!   widget, so a text box left in the tree while the ring was on a button would
//!   swallow keystrokes the person believed were going to the button. The field is
//!   therefore drawn as an editable box exactly when the ring is on it — the value
//!   is kept by the model, so nothing is lost by handing the keyboard back — and
//!   the two cannot come apart.

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Border, Color, Element, Length};

use crate::app::{Message, State};
use crate::theme;
use crate::view_model::{Face, FocusTarget, GreeterViewModel};
use inox_config::Theme as Tokens;

/// How much of the screen the card takes, as a ratio of the columns beside it.
///
/// A proportion rather than a width: the theme tokens carry no size for a card,
/// and a form that were a fixed number of pixels wide would be wrong on every
/// window it did not match.
const CARD_PORTION: u16 = 2;

/// The portion of the screen left free on each side of the card.
const MARGIN_PORTION: u16 = 1;

/// The identity input's widget id, which is how the update loop focuses it.
pub const IDENTITY_ID: &str = "identity";

/// The login screen's widget tree.
pub fn view(state: &State) -> Element<'_, Message> {
    let model = state.model();
    let tokens = state.tokens();

    let card = container(column(children(model, tokens)).spacing(f32::from(tokens.spacing.md)))
        .width(Length::FillPortion(CARD_PORTION))
        .padding(f32::from(tokens.spacing.lg))
        .style(theme::card(tokens));

    let content = row![
        Space::new().width(Length::FillPortion(MARGIN_PORTION)),
        card,
        Space::new().width(Length::FillPortion(MARGIN_PORTION)),
    ]
    .width(Length::Fill);

    container(scrollable(content))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(f32::from(tokens.spacing.lg))
        .style(theme::screen(tokens))
        .into()
}

/// Everything inside the card, in the order the user reads it.
fn children<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Vec<Element<'a, Message>> {
    let mut children = vec![
        text(model.headline.clone())
            .size(theme::headline_text_size(tokens))
            .into(),
        line_of(&model.banner, tokens, Line::Warning),
        field(model, tokens),
    ];

    match &model.face {
        Face::Prompt => {}
        Face::Authenticating { line } | Face::Starting { line } => {
            children.push(line_of(line, tokens, Line::Muted));
        }
        Face::Failed { line } => children.push(line_of(line, tokens, Line::Danger)),
        Face::Started { line } => children.push(line_of(line, tokens, Line::Success)),
        Face::Unavailable { line, reason } => {
            children.push(line_of(line, tokens, Line::Danger));
            children.push(line_of(reason, tokens, Line::Muted));
        }
    }

    let identity = &model.identity;
    if let Some(error) = &identity.error {
        children.push(line_of(error, tokens, Line::Danger));
    }

    children.push(
        row![
            text(model.session_label.clone())
                .size(theme::small_text_size(tokens))
                .color(theme::muted_text(tokens)),
            text(model.session.clone()).size(theme::small_text_size(tokens)),
        ]
        .spacing(f32::from(tokens.spacing.sm))
        .into(),
    );

    // A control that cannot be used says why it cannot, once, next to itself
    // (acceptance D05 rather than a grey shape a user has to guess about).
    if let Some(reason) = &model.notice {
        children.push(line_of(reason, tokens, Line::Warning));
    }

    // A hint is a promise about a key, so it only appears where there is a key
    // that does something.
    if let Some(hint) = &model.hint {
        children.push(line_of(hint, tokens, Line::Muted));
    }
    children.push(actions(model, tokens));

    children
}

/// How a line of text is emphasized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// Secondary text.
    Muted,
    /// A refusal or a failure.
    Danger,
    /// Something to notice.
    Warning,
    /// Something that went the right way.
    Success,
}

/// One line of text, in the colour its emphasis asks for.
fn line_of<'a>(content: &'a str, tokens: &'a Tokens, line: Line) -> Element<'a, Message> {
    let colour = match line {
        Line::Muted => theme::muted_text(tokens),
        Line::Danger => theme::danger_text(tokens),
        Line::Warning => theme::warning_text(tokens),
        Line::Success => theme::success_text(tokens),
    };

    text(content.to_owned())
        .size(theme::small_text_size(tokens))
        .color(colour)
        .into()
}

/// The identity field: a label, and either an editable box or what was typed.
///
/// The frame is drawn by the container in both cases, so a field that cannot be
/// typed into does not look like a different kind of thing from one that can —
/// the difference is the reason printed under it.
fn field<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let identity = &model.identity;
    let focused = model.focus == Some(FocusTarget::Identity);
    let wrong = identity.error.is_some();
    let size = theme::text_size(tokens);

    // Editable *and* focused: see the module docs for why the second one is not
    // optional.
    let control: Element<'a, Message> = if identity.editable && focused {
        text_input(&identity.placeholder, &identity.value)
            .id(IDENTITY_ID)
            .on_input(Message::Identity)
            .size(size)
            .padding(f32::from(tokens.spacing.sm))
            .style(bare(tokens))
            .into()
    } else if identity.value.is_empty() {
        // A field that shows its placeholder in the placeholder's colour says
        // "there is nothing here" in the same words a typable one would.
        text(identity.placeholder.clone())
            .size(size)
            .color(theme::muted_text(tokens))
            .into()
    } else {
        text(identity.value.clone()).size(size).into()
    };

    container(
        column![
            text(identity.label.clone())
                .size(theme::small_text_size(tokens))
                .color(theme::muted_text(tokens)),
            control,
        ]
        .spacing(f32::from(tokens.spacing.xs)),
    )
    .width(Length::Fill)
    .padding(f32::from(tokens.spacing.sm))
    .style(theme::field(tokens, focused, wrong))
    .into()
}

/// A text-input style with no frame of its own, so the container's field style is
/// the one frame on screen.
fn bare(tokens: &Tokens) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style + '_ {
    move |_, _| text_input::Style {
        background: Color::TRANSPARENT.into(),
        border: Border::default(),
        icon: theme::muted_text(tokens),
        placeholder: theme::muted_text(tokens),
        value: theme::text(tokens),
        selection: theme::color(tokens.colors.accent),
    }
}

/// The two controls at the foot of the form.
fn actions<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let size = theme::text_size(tokens);

    // Enabled and operable are the same fact: a button without `on_press` is how
    // iced reports a control as disabled, so a control that says it cannot be
    // used is one that sends nothing.
    let submit = button(text(model.submit.label.clone()).size(size))
        .width(Length::Fill)
        .padding(f32::from(tokens.spacing.sm))
        .style(theme::submit(
            tokens,
            model.submit.enabled,
            model.focus == Some(FocusTarget::Submit),
        ));

    let submit: Element<'a, Message> = if model.submit.enabled {
        submit.on_press(Message::Submitted).into()
    } else {
        submit.into()
    };

    let cancel = button(text(model.cancel.label.clone()).size(size))
        .padding(f32::from(tokens.spacing.sm))
        .style(theme::action(
            tokens,
            model.focus == Some(FocusTarget::Cancel),
        ));

    let cancel: Element<'a, Message> = if model.cancel.enabled {
        cancel.on_press(Message::Cancelled).into()
    } else {
        cancel.into()
    };

    row![submit, cancel]
        .spacing(f32::from(tokens.spacing.sm))
        .width(Length::Fill)
        .into()
}
