//! The widget tree.
//!
//! The view makes no decisions: it walks the [`ViewModel`] the projection
//! produced and draws it with the theme's tokens. Every colour, radius and
//! distance comes from [`crate::theme`], every string from the catalogs, and the
//! icon column's size from `shell.toml` — there is no literal colour or pixel
//! size in this file.
//!
//! ## The layout, and why it is grouped
//!
//! Three groups, separated by `spacing.lg` inside a window padded with
//! `spacing.xl` (design guidelines §3):
//!
//! | group | draws | carries |
//! |-------|-------|---------|
//! | the search field | the query, at `title` weight | the focus ring |
//! | the result list | a `heading` and the rows, or the state that replaces them | the selection |
//! | the status bar | the key hints, notices, the launch in progress or its failure | the actions |
//!
//! Each group is a `surface` card with a hairline boundary (§4) — a three-card
//! column rather than a full-width field over an unbounded list, which is the
//! arrangement §9 rejects. Inside a row, the name is `body`, the generic name and
//! every explanation are `caption`, and the icon column is a square tile of the
//! configured `icon_size`, so every row in the list is the same height.
//!
//! The launcher is a keyboard surface, so nothing here animates (§8): typing,
//! moving the selection and launching are answered on the next frame, and every
//! state a key can reach is visible without a transition. There is no reduced
//! motion preference to consult either, so no state is expressed by motion alone.

use iced::widget::text::Wrapping;
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};
use inox_config::Theme;

use crate::app::{Message, SEARCH_INPUT_ID, State};
use crate::theme::{self, Metrics, TextRole};
use crate::view_model::{Banner, Row, Status, ViewModel};

/// The launcher's widget tree.
pub fn view(state: &State) -> Element<'_, Message> {
    let metrics = Metrics::from_tokens(&state.tokens);

    let groups = column![
        search_field(state, &metrics),
        list_group(state, &metrics),
        status_bar(state, &metrics),
    ]
    .spacing(metrics.space_lg)
    .width(Length::Fill)
    .height(Length::Fill);

    // The surface the host maps is `placement.size`. A window manager, or the
    // headless render harness, may hand the view a larger window than that — so
    // the groups keep the width the launcher will actually be, which is what
    // keeps the search field from stretching across a whole screen (§4: width
    // follows the semantics of the field, not the size of the screen). The
    // window's own padding is already applied around this column, so what is
    // capped here is the width the cards get.
    let width = metrics.content_width(state.placement.size.width);

    container(
        container(groups)
            .width(Length::Fill)
            .height(Length::Fill)
            .max_width(width),
    )
    .padding(metrics.space_xl)
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .style(theme::window(&state.tokens))
    .into()
}

/// The search field: the one thing the surface is about (guidelines §2).
fn search_field<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let tokens = &state.tokens;

    text_input(&state.model.placeholder, &state.model.query)
        .id(SEARCH_INPUT_ID)
        .on_input(Message::QueryChanged)
        .size(theme::role_size(tokens, TextRole::Title))
        .padding(metrics.control_padding())
        .style(theme::field(tokens))
        .into()
}

/// The result list, or the state that stands in for it.
fn list_group<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let model = &state.model;
    let tokens = &state.tokens;

    let content: Element<'a, Message> = match &model.banner {
        Some(banner) => banner_content(banner, tokens, metrics),
        None => list_content(model, tokens, metrics, f32::from(state.config.icon_size)),
    };

    container(content)
        .padding(metrics.card_padding())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::card(tokens))
        .into()
}

/// The rows, under the group's heading.
fn list_content<'a>(
    model: &'a ViewModel,
    tokens: &'a Theme,
    metrics: &Metrics,
    icon_size: f32,
) -> Element<'a, Message> {
    let heading = text(model.title.clone())
        .size(theme::role_size(tokens, TextRole::Heading))
        .color(theme::colour(tokens.colors.text));

    let mut header = column![heading]
        .spacing(metrics.space_sm)
        .width(Length::Fill);

    if let Some(prompt) = &model.prompt {
        header = header.push(caption(prompt, tokens));
    }

    let rows: Vec<Element<'a, Message>> = model
        .rows
        .iter()
        .map(|entry| row_widget(entry, tokens, metrics, icon_size))
        .collect();

    let list = scrollable(column(rows).spacing(metrics.space_sm).width(Length::Fill))
        .height(Length::Fill)
        .width(Length::Fill);

    column![header, list]
        .spacing(metrics.space_md)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// One result row: a square icon tile, the name, and what describes it.
fn row_widget<'a>(
    entry: &'a Row,
    tokens: &'a Theme,
    metrics: &Metrics,
    icon_size: f32,
) -> Element<'a, Message> {
    // A square tile, in both directions: the width is the configured icon size
    // and not the row's, so the labels start at the same place on every row.
    let badge = container(
        text(entry.icon.clone())
            .size(theme::role_size(tokens, TextRole::Caption))
            .color(theme::badge_glyph_colour(tokens, entry.icon_placeholder)),
    )
    .width(Length::Fixed(icon_size))
    .height(Length::Fixed(icon_size))
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(theme::badge(tokens, entry.icon_placeholder));

    // The name inherits the row's text colour from the button style below, which
    // is what makes it `on_accent` on the selected row and muted on an unusable
    // one. The lines that are explicitly secondary are told their colour here.
    let mut labels = column![
        text(entry.label.clone())
            .size(theme::role_size(tokens, TextRole::Body))
            .wrapping(Wrapping::Word)
    ]
    .spacing(metrics.space_xs)
    .width(Length::Fill);

    if let Some(detail) = &entry.detail {
        labels = labels.push(
            text(detail.clone())
                .size(theme::role_size(tokens, TextRole::Caption))
                .color(theme::row_detail_colour(
                    tokens,
                    entry.selected,
                    entry.enabled,
                ))
                .wrapping(Wrapping::Word),
        );
    }

    if let Some(reason) = &entry.disabled_reason {
        labels = labels.push(
            text(reason.clone())
                .size(theme::role_size(tokens, TextRole::Caption))
                .color(theme::colour(tokens.colors.warning))
                .wrapping(Wrapping::Word),
        );
    }

    let control = button(
        row![badge, labels]
            .spacing(metrics.space_sm)
            .align_y(Alignment::Center)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .padding(metrics.control_padding())
    .style(theme::row(tokens, entry.selected, entry.enabled));

    // A row that cannot be launched is a row that does not accept a click; the
    // reason is printed next to it rather than discovered by pressing it.
    if entry.enabled {
        control
            .on_press(Message::RowChosen(entry.id.clone()))
            .into()
    } else {
        control.into()
    }
}

/// The message that takes the list's place.
///
/// One headline and, when the state has one, one explanation, one suggestion and
/// one action — the action is the one the state can actually carry out, and none
/// is offered for a state that has nothing to do.
fn banner_content<'a>(
    banner: &'a Banner,
    tokens: &'a Theme,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut content = column![
        text(banner.title.clone())
            .size(theme::role_size(tokens, TextRole::Heading))
            .color(theme::colour(tokens.colors.text))
    ]
    .spacing(metrics.space_sm)
    .width(Length::Fill);

    for line in [&banner.body, &banner.hint].into_iter().flatten() {
        content = content.push(caption(line, tokens));
    }

    if let (Some(label), Some(message)) = (&banner.action, banner.kind.action_message()) {
        content = content.push(action_button(label, message, tokens, metrics));
    }

    container(content)
        .width(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

/// The key hints, the note about the list, and the launch status.
fn status_bar<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let model = &state.model;
    let tokens = &state.tokens;

    let mut lines = column![caption(&model.hint, tokens)]
        .spacing(metrics.space_sm)
        .width(Length::Fill);

    if let Some(notice) = &model.notice {
        lines = lines.push(caption(notice, tokens));
    }

    if let Some(status) = &model.status {
        lines = lines.push(status_line(status, tokens, metrics));
    }

    container(lines)
        .padding(metrics.card_padding())
        .width(Length::Fill)
        .style(theme::card(tokens))
        .into()
}

/// The launch in progress, or what became of it.
fn status_line<'a>(
    status: &'a Status,
    tokens: &'a Theme,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let failed = status.reason.is_some();

    let mut content = column![
        text(status.line.clone())
            .size(theme::role_size(tokens, TextRole::Body))
            .color(theme::colour(if failed {
                tokens.colors.danger
            } else {
                tokens.colors.text
            }))
            .wrapping(Wrapping::Word)
    ]
    .spacing(metrics.space_sm)
    .width(Length::Fill);

    if let Some(reason) = &status.reason {
        content = content.push(caption(reason, tokens));
    }

    if let Some(retry) = &status.retry {
        content = content.push(action_button(
            retry,
            Message::LaunchRetried,
            tokens,
            metrics,
        ));
    }

    content.into()
}

/// A secondary line: the `caption` role, in the muted text colour (§2, §3).
fn caption<'a>(line: &'a str, tokens: &'a Theme) -> Element<'a, Message> {
    text(line)
        .size(theme::role_size(tokens, TextRole::Caption))
        .color(theme::colour(tokens.colors.text_muted))
        .wrapping(Wrapping::Word)
        .into()
}

/// The one action a state offers.
///
/// The label inherits the button's text colour, which is what lets the same
/// widget be `text` while it rests and `on_accent` while it is pressed (§5).
fn action_button<'a>(
    label: &'a str,
    message: Message,
    tokens: &'a Theme,
    metrics: &Metrics,
) -> Element<'a, Message> {
    button(text(label).size(theme::role_size(tokens, TextRole::Body)))
        .on_press(message)
        .padding(metrics.control_padding())
        .style(theme::action(tokens, true))
        .into()
}
