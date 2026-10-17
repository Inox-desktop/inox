//! The widget tree.
//!
//! The view makes no decisions: it walks the [`ViewModel`] the projection
//! produced and draws it with the theme's tokens. Every colour, radius and
//! distance comes from [`crate::theme`], every string from the catalogs, and the
//! icon column's size from `shell.toml` — there is no literal colour or pixel
//! size in this file.

use iced::widget::{Column, button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length};

use crate::app::{Message, SEARCH_INPUT_ID, State};
use crate::theme;
use crate::view_model::{Banner, Row, Status, ViewModel};

/// The ratio secondary text is drawn at.
const SMALL_TEXT_RATIO: f32 = 0.75;

/// The launcher's widget tree.
pub fn view(state: &State) -> Element<'_, Message> {
    let model = &state.model;

    let tokens = &state.tokens;
    let gap = f32::from(tokens.spacing.sm);
    let gutter = f32::from(tokens.spacing.xl);

    let search = text_input(&model.placeholder, &model.query)
        .id(SEARCH_INPUT_ID)
        .on_input(Message::QueryChanged)
        .size(theme::text_size(tokens))
        .padding(gap)
        .style(theme::input(tokens));

    let body: Element<'_, Message> = match &model.banner {
        Some(banner) => banner_widget(banner, tokens),
        None => {
            let icon_size = f32::from(state.config.icon_size);
            scrollable(list(model, tokens, icon_size))
                .height(Length::Fill)
                .into()
        }
    };

    let content = column![search, body, footer(model, tokens)]
        .spacing(gap)
        .width(Length::Fill)
        .height(Length::Fill);

    container(content)
        .padding(gutter)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::overlay(tokens))
        .into()
}

/// The result rows.
fn list<'a>(
    model: &'a ViewModel,
    tokens: &'a inox_config::Theme,
    icon_size: f32,
) -> Column<'a, Message> {
    let gap = f32::from(tokens.spacing.xs);
    let rows: Vec<Element<'a, Message>> = model
        .rows
        .iter()
        .map(|row| row_widget(row, tokens, icon_size))
        .collect();

    column(rows).spacing(gap).width(Length::Fill)
}

/// One result row.
fn row_widget<'a>(
    row: &'a Row,
    tokens: &'a inox_config::Theme,
    icon_size: f32,
) -> Element<'a, Message> {
    let gap = f32::from(tokens.spacing.sm);
    let small = theme::text_size(tokens) * SMALL_TEXT_RATIO;
    let background = theme::icon_background(tokens, row.icon_placeholder);

    let icon = container(
        text(row.icon.clone())
            .size(small)
            .color(if row.icon_placeholder {
                theme::colour(tokens.colors.text_muted)
            } else {
                theme::colour(tokens.colors.on_accent)
            }),
    )
    .width(Length::Fixed(icon_size))
    .height(Length::Fixed(icon_size))
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(move |_| iced::widget::container::Style {
        background: Some(background),
        border: iced::Border {
            color: iced::Color::TRANSPARENT,
            width: 0.0,
            radius: (f32::from(tokens.radii.sm)).into(),
        },
        ..Default::default()
    });

    let mut labels = column![text(row.label.clone()).size(theme::text_size(tokens))];
    if let Some(detail) = &row.detail {
        labels = labels.push(
            text(detail.clone())
                .size(small)
                .color(theme::colour(tokens.colors.text_muted)),
        );
    }
    if let Some(reason) = &row.disabled_reason {
        labels = labels.push(
            text(reason.clone())
                .size(small)
                .color(theme::colour(tokens.colors.warning)),
        );
    }

    let row_button = button(
        row![icon, labels]
            .spacing(gap)
            .align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .padding(f32::from(tokens.spacing.xs))
    .style(theme::row(tokens, row.selected, row.enabled));

    // A row that cannot be launched is a row that does not accept a click; the
    // reason is printed next to it rather than discovered by pressing it.
    if row.enabled {
        row_button
            .on_press(Message::RowChosen(row.id.clone()))
            .into()
    } else {
        row_button.into()
    }
}

/// The message that takes the list's place.
fn banner_widget<'a>(banner: &'a Banner, tokens: &'a inox_config::Theme) -> Element<'a, Message> {
    let gap = f32::from(tokens.spacing.sm);
    let small = theme::text_size(tokens) * SMALL_TEXT_RATIO;

    let mut content = column![text(banner.title.clone()).size(theme::text_size(tokens))]
        .spacing(gap)
        .align_x(iced::Alignment::Center);

    for line in [&banner.body, &banner.hint].into_iter().flatten() {
        content = content.push(
            text(line.clone())
                .size(small)
                .color(theme::colour(tokens.colors.text_muted)),
        );
    }

    if let Some(retry) = &banner.retry {
        content = content.push(
            button(text(retry.clone()).size(small))
                .on_press(Message::IndexRetried)
                .padding(f32::from(tokens.spacing.xs))
                .style(theme::action(tokens)),
        );
    }

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
}

/// The hints, the note about the list, and the launch status.
fn footer<'a>(model: &'a ViewModel, tokens: &'a inox_config::Theme) -> Element<'a, Message> {
    let gap = f32::from(tokens.spacing.xs);
    let small = theme::text_size(tokens) * SMALL_TEXT_RATIO;

    let mut footer = column![
        text(model.hint.clone())
            .size(small)
            .color(theme::colour(tokens.colors.text_muted))
    ]
    .spacing(gap)
    .width(Length::Fill);

    if let Some(notice) = &model.notice {
        footer = footer.push(
            text(notice.clone())
                .size(small)
                .color(theme::colour(tokens.colors.text_muted)),
        );
    }

    if let Some(status) = &model.status {
        footer = footer.push(status_widget(status, tokens));
    }

    footer.into()
}

/// The launch status line.
fn status_widget<'a>(status: &'a Status, tokens: &'a inox_config::Theme) -> Element<'a, Message> {
    let gap = f32::from(tokens.spacing.sm);
    let small = theme::text_size(tokens) * SMALL_TEXT_RATIO;

    let line = text(status.line.clone())
        .size(small)
        .color(if status.reason.is_some() {
            theme::colour(tokens.colors.danger)
        } else {
            theme::colour(tokens.colors.text)
        });

    let mut content = row![line].spacing(gap).align_y(iced::Alignment::Center);

    if let Some(reason) = &status.reason {
        content = content.push(
            text(reason.clone())
                .size(small)
                .color(theme::colour(tokens.colors.text_muted)),
        );
    }

    if let Some(retry) = &status.retry {
        content = content.push(
            button(text(retry.clone()).size(small))
                .on_press(Message::LaunchRetried)
                .padding(f32::from(tokens.spacing.xs))
                .style(theme::action(tokens)),
        );
    }

    content.into()
}
