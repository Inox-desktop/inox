//! The widget tree.
//!
//! The view makes no decisions: it walks the [`LockViewModel`] the projection
//! produced and draws it with the theme's tokens. Every colour, radius and
//! distance comes from [`crate::theme`], every string from the catalogs, and every
//! proportion from a [`Length::FillPortion`] ratio rather than a pixel count —
//! there is no literal colour or size in this file.
//!
//! Two things about the credential field are worth stating here rather than
//! leaving to be discovered.
//!
//! - **The field is not a text box.** The credential is drawn as a mask built from
//!   its *length*, and keystrokes arrive as messages from the keyboard
//!   subscription ([`crate::app::key_binding`]). That is deliberate: iced's own
//!   text editor keeps a `String` of its own that this crate cannot zeroize, it
//!   offers copy and paste, and a future widget change could put the plaintext
//!   somewhere nobody is looking. Drawing the mask means the credential never
//!   enters the widget tree at all — and it is why a screenshot of the lock screen
//!   has nothing to redact (acceptance Q02, ADR 0015). The cost is that the field
//!   is not an iced `text_input`: there is no caret, no selection, no IME and no
//!   copy/paste inside it. What is needed to make that a real decision — focus
//!   handling, IME and the accessible role — belongs with the surface, which does
//!   not exist yet.
//! - **The card is drawn at a two-to-one proportion of its screen**, not at a fixed
//!   width: a lock screen is shown at whatever the output's size is, and the tokens
//!   carry no width for it.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Element, Length};

use crate::app::{Message, State};
use crate::theme;
use crate::view_model::{Face, LockViewModel};
use inox_config::Theme as Tokens;

/// How much of the screen the card takes, as a ratio of the columns beside it.
///
/// A proportion rather than a width: the theme tokens carry no size for a card,
/// and a lock screen that were a fixed number of pixels wide would be wrong on
/// every output it did not match.
const CARD_PORTION: u16 = 2;

/// The portion of the screen left free on each side of the card.
const MARGIN_PORTION: u16 = 1;

/// The lock screen's widget tree.
pub fn view(state: &State) -> Element<'_, Message> {
    let model = state.model();
    let tokens = state.tokens();

    let card =
        container(column(card_children(model, tokens)).spacing(f32::from(tokens.spacing.md)))
            .width(Length::FillPortion(CARD_PORTION))
            .padding(f32::from(tokens.spacing.xl))
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
        .style(theme::screen(tokens))
        .into()
}

/// Everything inside the card, in the order the user reads it.
fn card_children<'a>(model: &'a LockViewModel, tokens: &'a Tokens) -> Vec<Element<'a, Message>> {
    let mut children = vec![
        text(model.headline.clone())
            .size(theme::text_size(tokens))
            .into(),
    ];

    // A password box on a screen that says it cannot check a password is the same
    // invitation as a prompt, in a smaller font.
    if model.prompts_for_credential {
        children.push(Field::new(model, tokens).into_element());
    }

    match &model.face {
        Face::Prompt => {}
        Face::Verifying { line } => children.push(line_of(line, tokens, Line::Muted)),
        Face::Failed { line, reason } => {
            children.push(line_of(line, tokens, Line::Danger));
            children.push(line_of(reason, tokens, Line::Muted));
        }
        Face::Waiting { line } => children.push(line_of(line, tokens, Line::Warning)),
        // Neither of these is a prompt: the headline already says what the screen
        // is, so what goes under the field is only what the user has to know.
        Face::HeldWithoutUi { detail } => {
            children.push(line_of(detail, tokens, Line::Muted));
        }
        Face::Unavailable { reason, retry } => {
            children.push(line_of(reason, tokens, Line::Muted));

            if let Some(label) = retry {
                children.push(
                    button(text(label.clone()).size(theme::small_text_size(tokens)))
                        .on_press(Message::BoundaryRetried)
                        .padding(f32::from(tokens.spacing.xs))
                        .style(theme::action(tokens))
                        .into(),
                );
            }
        }
    }

    // A disabled control says why it is disabled, next to itself (acceptance
    // D05 rather than a grey shape a user has to guess about).
    if let Some(reason) = &model.disabled_reason {
        children.push(line_of(reason, tokens, Line::Warning));
    }

    // Absent where the keyboard has nothing to do — a hint about pressing Enter
    // on a screen with nothing to press is worse than no hint.
    if let Some(hint) = &model.hint {
        children.push(line_of(hint, tokens, Line::Muted));
    }
    children.push(submit(model, tokens));

    children
}

/// How a line of text is emphasized.
///
/// There is no "plain" variant: every line the card draws is one of these four,
/// and a variant nothing constructs would be a colour nobody can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// Secondary text.
    Muted,
    /// A refusal or a failure.
    Danger,
    /// Something to notice.
    Warning,
}

/// One line of text, in the colour its emphasis asks for.
fn line_of<'a>(content: &'a str, tokens: &'a Tokens, line: Line) -> Element<'a, Message> {
    let colour = match line {
        Line::Muted => theme::muted_text(tokens),
        Line::Danger => theme::danger_text(tokens),
        Line::Warning => theme::warning_text(tokens),
    };

    text(content.to_owned())
        .size(theme::small_text_size(tokens))
        .color(colour)
        .into()
}

/// The credential field: a label, and the mask the projection built.
struct Field<'a> {
    /// What the field holds — a placeholder, or one glyph per character typed.
    content: &'a str,
    /// Whether the field is empty, which is what picks the style.
    empty: bool,
    /// The field's label, which is also its accessible name.
    label: &'a str,
    /// The size the content is drawn at.
    size: f32,
    /// The tokens.
    tokens: &'a Tokens,
}

impl<'a> Field<'a> {
    fn new(model: &'a LockViewModel, tokens: &'a Tokens) -> Self {
        let empty = model.mask.is_empty();

        Self {
            content: if empty {
                &model.placeholder
            } else {
                &model.mask
            },
            empty,
            label: &model.field_label,
            size: if empty {
                theme::text_size(tokens)
            } else {
                theme::mask_text_size(tokens)
            },
            tokens,
        }
    }

    /// The field, as a widget.
    fn into_element(self) -> Element<'a, Message> {
        let tokens = self.tokens;

        container(
            column![
                text(self.label.to_owned())
                    .size(theme::small_text_size(tokens))
                    .color(theme::muted_text(tokens)),
                text(self.content.to_owned()).size(self.size),
            ]
            .spacing(f32::from(tokens.spacing.xs)),
        )
        .width(Length::Fill)
        .padding(f32::from(tokens.spacing.sm))
        .style(theme::field(tokens, self.empty))
        .into()
    }
}

/// The unlock button.
///
/// A button that cannot be pressed is a button with no `on_press`, which is how
/// iced reports it as disabled — so "the control is disabled" and "the control
/// sends nothing" are the same fact rather than two that could disagree.
fn submit<'a>(model: &'a LockViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let label = text(model.submit.label.clone()).size(theme::text_size(tokens));
    let style = theme::submit(tokens, model.submit.enabled);

    let control = button(label)
        .width(Length::Fill)
        .padding(f32::from(tokens.spacing.sm))
        .style(style);

    if model.submit.enabled {
        control.on_press(Message::Submitted).into()
    } else {
        control.into()
    }
}
