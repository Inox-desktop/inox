//! The widget tree.
//!
//! The view makes no decisions: it walks the [`GreeterViewModel`] the projection
//! produced and draws it with the theme's tokens. Every colour, radius and
//! distance comes from [`crate::theme`], every string from the catalogs, and every
//! proportion from a [`Length::FillPortion`] ratio rather than a pixel count —
//! there is no literal colour or size in this file.
//!
//! # How the screen is laid out
//!
//! One card, centred on the window's background and centred in it: at its head the
//! medallion of the account a login would use, then the account's name, then what
//! this build is, then the password field, then what the screen has to report,
//! then the controls, and last what a login would start. Inside the card groups
//! are `lg` apart and lines inside a group `sm`, so the grouping is visible
//! without a rule being drawn (guidelines §3). The card stays inside a
//! `scrollable`, and the content is given the height of the viewport rather than
//! of its own line count, so that 200 % text pushes the layout down rather than
//! clipping it (acceptance Q02) while a screen with less to say stays centred.
//!
//! # Four things worth stating rather than leaving to be discovered
//!
//! - **The identity is chosen, never typed.** There is no text box for a user
//!   name: the account is one of the accounts the system offers, drawn as a
//!   medallion and a name, and the accounts themselves are the screen's second face
//!   ([`FocusTarget::Account`]). Nothing here can invent an account — every row
//!   comes from the projection, which got it from the account database.
//! - **The credential is not a text box.** It is drawn as a mask built from its
//!   *length* ([`crate::view_model::MASK_GLYPH`]), its keystrokes arrive as
//!   messages, and there is no `String` of it anywhere in this process that iced
//!   owns — iced's editor keeps one that this crate cannot zeroize (ADR 0015).
//! - **The focus ring is drawn from the projection, not from the pointer.** Every
//!   control's ring comes from [`GreeterViewModel::focus`], which the update loop
//!   moves with Tab, so "the focus is visible and unique" (acceptance Q01) is the
//!   same fact the keyboard acts on rather than a second one that could disagree.
//!   The account rows carry their own ring for the same reason: a list a keyboard
//!   can walk has to say where the keyboard is.
//! - **A field that cannot be typed into is not drawn as a box.** Guidelines §5
//!   makes a disabled control the muted text with no background and no frame, and
//!   that is not only a style: a framed field under a screen that says it cannot
//!   log anyone in is the same invitation as a prompt, in a form's clothing. What
//!   the person reads instead is the line beside it saying why.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::app::{Message, State};
use crate::theme::{self, Emphasis};
use crate::view_model::{AccountView, Face, FocusTarget, GreeterViewModel};
use inox_config::Theme as Tokens;

/// How much of the window the card takes, as a ratio of the columns beside it.
///
/// A proportion rather than a width: the theme tokens carry no size for a card,
/// and a screen that were a fixed number of pixels wide would be wrong on every
/// window it did not match.
const CARD_PORTION: u16 = 2;

/// The portion of the window left free on each side of the card.
const MARGIN_PORTION: u16 = 1;

/// How much of the card a control takes, as a ratio of the gutters beside it.
///
/// A password field is not a banner: two thirds of the card is a width that says
/// how much goes in it, where the full width would say "write an essay"
/// (guidelines §4). The control and its two gutters are laid out as one row rather
/// than as a child with a portion of its own, because a column that centres its
/// children has no second child to take the rest of the row: the portion would
/// resolve against the whole card and the control would fill it.
const CONTROL_PORTION: u16 = 4;

/// The gutter on each side of a control.
const CONTROL_GUTTER_PORTION: u16 = 1;

/// The login screen's widget tree.
pub fn view(state: &State) -> Element<'_, Message> {
    let model = state.model();
    let tokens = state.tokens();

    let card = container(card_content(model, tokens))
        .width(Length::FillPortion(CARD_PORTION))
        .padding(f32::from(tokens.spacing.lg))
        .style(theme::card(tokens));

    let content = row![
        Space::new().width(Length::FillPortion(MARGIN_PORTION)),
        card,
        Space::new().width(Length::FillPortion(MARGIN_PORTION)),
    ]
    .width(Length::Fill);

    // The content is given the height of the viewport so that it can be centred in
    // it, and it sits inside a `scrollable` so that a longer layout — 200 % text,
    // or a long localized sentence — is scrolled to rather than cut off.
    let content = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_y(Alignment::Center);

    container(scrollable(content).width(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(f32::from(tokens.spacing.xl))
        .style(theme::screen(tokens))
        .into()
}

/// What the card holds: the login, or the accounts to log in as.
fn card_content<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    if model.choosing {
        account_list(model, tokens)
    } else {
        login(model, tokens)
    }
}

/// The login itself: who, what this build does, the password, what is in the way,
/// and what can be done about it.
fn login<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let mut groups = vec![
        who(model, tokens),
        caption_line(model.banner.clone(), tokens, Emphasis::Warning),
        field(model, tokens),
    ];

    let status = status_lines(model, tokens);

    if !status.is_empty() {
        groups.push(column(status).spacing(f32::from(tokens.spacing.sm)).into());
    }

    groups.push(actions(model, tokens));
    groups.push(session_line(model, tokens));

    column(groups)
        .spacing(f32::from(tokens.spacing.lg))
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .into()
}

/// The head of the card: the account's medallion, its name, and — when there is
/// nobody to log in as — why not.
fn who<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let mut lines: Vec<Element<'a, Message>> = Vec::new();

    if let Some(account) = &model.selected {
        lines.push(medallion(
            account,
            theme::medallion_size(tokens),
            theme::title_size(tokens),
            tokens,
        ));
        lines.push(
            text(account.label.clone())
                .size(theme::title_size(tokens))
                .color(theme::text(tokens))
                .into(),
        );
    }

    if let Some(reason) = &model.accounts_reason {
        // The screen draws no account, so it says why it has none: an empty card
        // over a disabled field would be a puzzle rather than a screen.
        lines.push(body_line(reason.clone(), tokens, Emphasis::Warning));
    }

    column(lines)
        .spacing(f32::from(tokens.spacing.sm))
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .into()
}

/// One account's medallion: its initial, in a circle as wide as it is tall.
fn medallion<'a>(
    account: &AccountView,
    diameter: f32,
    size: f32,
    tokens: &'a Tokens,
) -> Element<'a, Message> {
    container(text(account.initial.clone()).size(size))
        .width(Length::Fixed(diameter))
        .height(Length::Fixed(diameter))
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(theme::medallion(tokens, diameter))
        .into()
}

/// The password field: its label, and either the mask or what the screen says
/// instead.
///
/// The frame is drawn by the container, and it carries the focus ring while the
/// keyboard is on the field — the field is not an iced widget that can hold focus
/// (the credential is a mask, ADR 0015), so the ring is the projection's answer to
/// "where is the keyboard?" rather than iced's. What changes with the state is
/// whether there is a box at all, and that is a decision the projection made and
/// the person can read beside it.
fn field<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let field = &model.credential;
    let focused = model.focus == Some(FocusTarget::Credential);
    let size = theme::body_size(tokens);

    let body: Option<Element<'a, Message>> = if !field.mask.is_empty() {
        Some(line(
            field.mask.clone(),
            size,
            tokens,
            emphasis(field.editable),
        ))
    } else if field.editable {
        // A field that shows its placeholder in the placeholder's colour says
        // "there is nothing here" in the same words a typable one would.
        Some(line(
            field.placeholder.clone(),
            size,
            tokens,
            Emphasis::Muted,
        ))
    } else {
        // A field that cannot be typed into shows what it holds, and it holds
        // nothing: a placeholder there would be an invitation to fill it in.
        None
    };

    let frame_style = theme::field_style(tokens, focused, field.error.is_some(), field.editable);

    let mut content = column![caption_line(field.label.clone(), tokens, Emphasis::Muted)]
        .spacing(f32::from(tokens.spacing.sm));

    if let Some(body) = body {
        content = content.push(body);
    }

    let frame = container(content)
        .width(Length::Fill)
        .padding(theme::control_padding(tokens))
        .style(move |_| frame_style);

    let mut children = vec![control_row(frame.into())];

    if let Some(error) = &field.error {
        children.push(caption_line(error.clone(), tokens, Emphasis::Danger));
    }

    column(children)
        .spacing(f32::from(tokens.spacing.sm))
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .into()
}

/// What the screen has to report: the lifecycle, and the reason nothing else says.
fn status_lines<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Vec<Element<'a, Message>> {
    let mut lines = Vec::new();

    match &model.face {
        Face::Prompt => {}
        Face::Authenticating { line } | Face::Starting { line } => {
            lines.push(body_line(line.clone(), tokens, Emphasis::Muted));
        }
        Face::Failed { line } => lines.push(body_line(line.clone(), tokens, Emphasis::Danger)),
        Face::Started { line } => lines.push(body_line(line.clone(), tokens, Emphasis::Success)),
        Face::Unavailable { line, reason } => {
            lines.push(body_line(line.clone(), tokens, Emphasis::Danger));
            lines.push(caption_line(reason.clone(), tokens, Emphasis::Muted));
        }
    }

    // A control that cannot be used says why it cannot, once, next to itself
    // (acceptance D05 rather than a grey shape a user has to guess about). The
    // projection decides which sentences belong here, so that the same fact is
    // never drawn twice.
    if let Some(reason) = &model.notice {
        lines.push(caption_line(reason.clone(), tokens, Emphasis::Warning));
    }

    lines
}

/// The controls at the foot of the form, and the hint about the keys that reach
/// them.
fn actions<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    // Enabled and operable are the same fact: a button without `on_press` is how
    // iced reports a control as disabled, so a control that says it cannot be
    // used is one that sends nothing.
    let submit = button(text(model.submit.label.clone()).size(theme::body_size(tokens)))
        .width(Length::Fill)
        .padding(theme::control_padding(tokens))
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

    // The secondary controls: switching account where there is a choice, and
    // clearing the password. They share one row, and it is two thirds of the card
    // wide like the field above it, because neither is what the screen asks for.
    let mut secondary: Vec<Element<'a, Message>> = Vec::new();

    if let Some(switch) = &model.switch_user {
        secondary.push(
            button(text(switch.label.clone()).size(theme::body_size(tokens)))
                .width(Length::Fill)
                .padding(theme::control_padding(tokens))
                .style(theme::action(
                    tokens,
                    model.focus == Some(FocusTarget::SwitchUser),
                ))
                .on_press(Message::SwitchUser)
                .into(),
        );
    }

    secondary.push(
        button(text(model.cancel.label.clone()).size(theme::body_size(tokens)))
            .width(Length::Fill)
            .padding(theme::control_padding(tokens))
            .style(theme::action(
                tokens,
                model.focus == Some(FocusTarget::Cancel),
            ))
            .on_press(Message::Cancelled)
            .into(),
    );

    let secondary = control_row(
        row(secondary)
            .spacing(f32::from(tokens.spacing.sm))
            .width(Length::Fill)
            .into(),
    );

    let mut children = vec![control_row(submit), secondary];

    // A hint is a promise about a key, so it only appears where there is a key
    // that does something.
    if let Some(hint) = &model.hint {
        children.push(caption_line(hint.clone(), tokens, Emphasis::Muted));
    }

    column(children)
        .spacing(f32::from(tokens.spacing.sm))
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .into()
}

/// What a login would start. The label and the session read as one line because
/// they are one fact.
fn session_line<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    row![
        caption_line(model.session_label.clone(), tokens, Emphasis::Muted),
        caption_line(model.session.clone(), tokens, Emphasis::None),
    ]
    .spacing(f32::from(tokens.spacing.sm))
    .align_y(Alignment::Center)
    .into()
}

/// The accounts to log in as: every account the system offers, one row each, and
/// the way back to the login.
fn account_list<'a>(model: &'a GreeterViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let mut rows = column![].spacing(f32::from(tokens.spacing.sm));

    for (index, account) in model.accounts.iter().enumerate() {
        rows = rows.push(account_row(account, index, model, tokens));
    }

    let back = button(text(model.cancel.label.clone()).size(theme::body_size(tokens)))
        .width(Length::Fill)
        .padding(theme::control_padding(tokens))
        .style(theme::action(
            tokens,
            model.focus == Some(FocusTarget::Cancel),
        ))
        .on_press(Message::Cancelled);

    column![
        text(model.accounts_heading.clone())
            .size(theme::heading_size(tokens))
            .color(theme::text(tokens)),
        rows,
        control_row(back.into()),
    ]
    .spacing(f32::from(tokens.spacing.lg))
    .width(Length::Fill)
    .align_x(Alignment::Center)
    .into()
}

/// One account: its medallion and its name, and whether it is the account a login
/// would use — which it says by being the filled row (guidelines §5).
fn account_row<'a>(
    account: &'a AccountView,
    index: usize,
    model: &'a GreeterViewModel,
    tokens: &'a Tokens,
) -> Element<'a, Message> {
    let focused = model.focus == Some(FocusTarget::account(index));
    // No explicit colour on the label: the row's own style is what says whether
    // the row is the chosen one, and it draws the label accordingly.
    let content = row![
        medallion(
            account,
            theme::row_medallion_size(tokens),
            theme::body_size(tokens),
            tokens
        ),
        text(account.label.clone()).size(theme::body_size(tokens)),
    ]
    .spacing(f32::from(tokens.spacing.md))
    .align_y(Alignment::Center);

    button(content)
        .width(Length::Fill)
        .padding(theme::control_padding(tokens))
        .style(theme::account(tokens, account.selected, focused))
        .on_press(Message::AccountChosen(account.name.clone()))
        .into()
}

/// A control, with the gutters that make it two thirds of the card rather than
/// all of it.
///
/// The gutters are `FillPortion`s beside the control inside one row, which is the
/// only place a portion has siblings to be a portion *of*. The control is wrapped
/// rather than given a portion of its own, because a control builder sets the width
/// it fills and the wrapper is what decides how much that is.
fn control_row<'a>(control: Element<'a, Message>) -> Element<'a, Message> {
    row![
        Space::new().width(Length::FillPortion(CONTROL_GUTTER_PORTION)),
        container(control).width(Length::FillPortion(CONTROL_PORTION)),
        Space::new().width(Length::FillPortion(CONTROL_GUTTER_PORTION)),
    ]
    .width(Length::Fill)
    .into()
}

/// The colour a value is drawn in: the text colour where it can be changed, and
/// the muted one where the field is disabled.
fn emphasis(editable: bool) -> Emphasis {
    if editable {
        Emphasis::None
    } else {
        Emphasis::Muted
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
