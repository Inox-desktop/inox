//! The widget tree.
//!
//! This module draws and decides nothing: every string, every enabled state and
//! the geometry of every copy of the image were settled in
//! [`crate::view_model`], and every colour, size and radius comes from
//! [`crate::theme`]. What is left is the one thing a view is for — building the
//! tree.
//!
//! # The shape of the window
//!
//! One column at the top-left holds the status card, and the problem card under
//! it when there is one; the windowed-preview notice is a strip at the foot. Both
//! are inset by the same `xl` window padding, so the cards and the strip line up
//! on one edge, and both are drawn with the same card style, radius and inset —
//! a second class of element in the same place with different geometry is what
//! makes a surface look assembled rather than designed (`docs/quality/ui-guidelines.md`
//! §3, §4).
//!
//! # Using the tokens without growing a second design system
//!
//! Every literal a widget would otherwise take — a size, a colour, a radius, a
//! padding, an interactive height — is asked of [`crate::theme`], which asks the
//! token set. The one deliberate exception is geometry that the wallpaper's own
//! placement decided ([`crate::scaling`]): a copy of the image is as large as its
//! rectangle says, because that rectangle *is* the answer the component exists to
//! compute.
//!
//! # How the image is positioned without a custom widget
//!
//! A copy of the image is drawn one rectangle at a time ([`crate::scaling`]),
//! and a rectangle that starts at a positive coordinate is reached with padding,
//! while one that starts before the origin is a copy *larger* than the output and
//! is reached by centring it — which is the same thing, since the rectangle it
//! describes is exactly centred on the output. Nothing here ever scales a copy to
//! a rectangle of a different shape than the copy itself: that would distort the
//! image rather than crop it, and the surface clips whatever runs over its edge.

use iced::alignment::{Horizontal, Vertical};
use iced::widget::text::Wrapping;
use iced::widget::{Column, Row, Space, Stack, button, container, image, text};
use iced::{Color, ContentFit, Element, Length, Padding};
use inox_config::Theme;
use std::sync::Arc;

use crate::app::{Message, State};
use crate::scaling::Rect;
use crate::theme;
use crate::view_model::{
    self, Action, Notice, NoticeKind, Phase, PhaseKind, StatusCard, Wallpaper, WallpaperViewModel,
};

/// The title of the preview window, in the session's language.
#[must_use]
pub fn title(state: &State) -> String {
    state.localizer().message("wallpaper-window-title")
}

/// The whole window.
///
/// The element borrows nothing from the state — everything in it is owned, and
/// the tokens are held through an [`Arc`] — but its lifetime is tied to the
/// state's anyway, because a widget tree is not something a caller gets to keep
/// after the state that produced it has gone.
pub fn view<'a>(state: &'a State) -> Element<'a, Message> {
    render(&view_model::project(state), state.theme_handle())
}

/// Draws a view model with a set of tokens.
///
/// Separate from [`view`] so that the render tests can draw any scenario —
/// light, dark, empty, degraded — without a running application.
///
/// The tokens are an [`Arc`] rather than a reference because a style closure
/// outlives this call: it is kept by the widget tree and asked for a colour on
/// every frame, so it has to own what it reads. Everything else the tree is built
/// from is owned too, which is what lets the element outlive the view model it
/// was projected from.
#[must_use]
pub fn render(model: &WallpaperViewModel, tokens: Arc<Theme>) -> Element<'static, Message> {
    let mut layers: Vec<Element<'static, Message>> = vec![surface(model, &tokens)];

    layers.push(overlay(
        cards(model, &tokens),
        Arc::clone(&tokens),
        Horizontal::Left,
        Vertical::Top,
    ));

    if let Some(footer) = &model.footer {
        layers.push(overlay(
            strip(footer, &tokens),
            Arc::clone(&tokens),
            Horizontal::Left,
            Vertical::Bottom,
        ));
    }

    Stack::with_children(layers)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// The wallpaper itself, or the theme background when there is none.
fn surface(model: &WallpaperViewModel, tokens: &Arc<Theme>) -> Element<'static, Message> {
    match &model.wallpaper {
        Some(wallpaper) => drawn(wallpaper, tokens),
        None => {
            let tokens = Arc::clone(tokens);

            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_theme| theme::background(&tokens))
                .into()
        }
    }
}

/// The column of cards: what the output is drawing, then whatever has to be read.
fn cards(model: &WallpaperViewModel, tokens: &Arc<Theme>) -> Element<'static, Message> {
    let mut column = Column::new()
        .spacing(theme::section_gap(tokens))
        .push(status_card(&model.card, tokens));

    if let Some(notice) = &model.notice {
        column = column.push(notice_card(notice, tokens));
    }

    column.into()
}

/// The card that describes the previewed output.
///
/// The order is fixed by the view model — output, what is on it, where that came
/// from, how it was fitted, the phase, and what would change an empty state — so
/// this function is a transcription of that order and nothing else.
fn status_card(card: &StatusCard, tokens: &Arc<Theme>) -> Element<'static, Message> {
    let mut content = Column::new().spacing(theme::gap(tokens)).push(line(
        &card.heading,
        theme::title_size(tokens),
        theme::bold(tokens),
        theme::text(tokens),
    ));

    if let Some(summary) = &card.summary {
        content = content.push(line(
            summary,
            theme::body_size(tokens),
            theme::font(tokens),
            theme::text(tokens),
        ));
    }
    if let Some(source) = &card.source {
        content = content.push(caption(source, tokens));
    }

    content = content.push(caption(&card.scaling, tokens));

    if let Some(phase) = &card.phase {
        content = content.push(line(
            &phase.text,
            theme::caption_size(tokens),
            theme::font(tokens),
            phase_color(phase, tokens),
        ));
    }
    if let Some(hint) = &card.hint {
        content = content.push(caption(hint, tokens));
    }

    shell(content, tokens)
}

/// The card a notice is drawn in: what happened, why, and the control that
/// answers it.
fn notice_card(notice: &Notice, tokens: &Arc<Theme>) -> Element<'static, Message> {
    let heading_color = match notice.kind {
        NoticeKind::Problem => theme::warning(tokens),
        NoticeKind::Loading => theme::text(tokens),
    };

    let mut story = Column::new().spacing(theme::gap(tokens)).push(line(
        &notice.heading,
        theme::heading_size(tokens),
        theme::bold(tokens),
        heading_color,
    ));

    if let Some(subject) = &notice.subject {
        story = story.push(caption(subject, tokens));
    }
    if let Some(reason) = &notice.reason {
        story = story.push(line(
            reason,
            theme::body_size(tokens),
            theme::font(tokens),
            theme::text(tokens),
        ));
    }
    if let Some(remedy) = &notice.remedy {
        story = story.push(caption(remedy, tokens));
    }

    let mut controls = Column::new()
        .spacing(theme::gap(tokens))
        .push(control(&notice.action, tokens));

    if let Some(reason) = &notice.action.reason {
        controls = controls.push(caption(reason, tokens));
    }

    shell(
        Column::new()
            .spacing(theme::section_gap(tokens))
            .push(story)
            .push(controls),
        tokens,
    )
}

/// The one control a notice offers, and why it cannot be pressed when it cannot.
///
/// It is the surface's primary action, so it is drawn with the accent fill the
/// UI spec reserves for the one action a screen is for (§5); the reason a
/// disabled control cannot be pressed is the caption under it, never the colour
/// alone.
fn control(action: &Action, tokens: &Arc<Theme>) -> Element<'static, Message> {
    let themed = Arc::clone(tokens);

    button(
        text(action.label.clone())
            .size(theme::body_size(tokens))
            .font(theme::font(tokens)),
    )
    .height(Length::Fixed(theme::control_height(tokens)))
    .padding(theme::control_padding(tokens))
    .style(move |_theme, status| theme::button_style(&themed, theme::ButtonRole::Primary, status))
    .on_press_maybe(action.enabled.then_some(Message::Reload))
    .into()
}

/// The strip at the foot of the preview: what kind of surface this is.
///
/// It takes the width it is given because it is an explanatory sentence rather
/// than a field (UI spec §4), and a caption colour because it is context the user
/// reads once, not a value they act on (§7).
fn strip(value: &str, tokens: &Arc<Theme>) -> Element<'static, Message> {
    let themed = Arc::clone(tokens);

    container(caption(value, tokens))
        .width(Length::Fill)
        .padding(theme::strip_padding(tokens))
        .style(move |_theme| theme::card(&themed))
        .into()
}

/// A card: `surface`, a hairline outline, an `md` radius, `lg` padding, and the
/// width the UI spec bounds a path-bearing body to.
fn shell(
    content: impl Into<Element<'static, Message>>,
    tokens: &Arc<Theme>,
) -> Element<'static, Message> {
    let themed = Arc::clone(tokens);

    container(content)
        .width(Length::Fixed(theme::card_width(tokens)))
        .padding(theme::card_padding(tokens))
        .style(move |_theme| theme::card(&themed))
        .into()
}

/// Ordinary text.
fn line(value: &str, size: f32, font: iced::Font, color: Color) -> iced::widget::Text<'static> {
    text(value.to_owned())
        .size(size)
        .font(font)
        .color(color)
        .wrapping(Wrapping::WordOrGlyph)
}

/// Secondary text: a path, a mode, a state sentence, an explanation.
fn caption(value: &str, tokens: &Theme) -> Element<'static, Message> {
    line(
        value,
        theme::caption_size(tokens),
        theme::font(tokens),
        theme::muted(tokens),
    )
    .into()
}

/// The colour of a phase sentence.
///
/// The sentence is the state; the colour only agrees with it (UI spec §6: colour
/// is never the only thing that carries the state).
fn phase_color(phase: &Phase, tokens: &Theme) -> Color {
    match phase.kind {
        PhaseKind::Loading | PhaseKind::Idle => theme::text(tokens),
        PhaseKind::Ready => theme::success(tokens),
        PhaseKind::Degraded => theme::warning(tokens),
    }
}

/// The image, one widget per copy of it.
fn drawn(wallpaper: &Wallpaper, tokens: &Arc<Theme>) -> Element<'static, Message> {
    let first = wallpaper.placement.first();

    let content: Element<'static, Message> = if wallpaper.placement.is_tiled() {
        tiled(wallpaper)
    } else {
        copy(wallpaper, first)
    };

    let themed = Arc::clone(tokens);
    let mut surface = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(move |_theme| theme::background(&themed));

    if first.x > 0 || first.y > 0 {
        surface = surface.padding(
            Padding::default()
                .top(first.y.max(0) as f32)
                .left(first.x.max(0) as f32),
        );
    }
    if first.x < 0 {
        surface = surface.align_x(Horizontal::Center);
    }
    if first.y < 0 {
        surface = surface.align_y(Vertical::Center);
    }

    surface.into()
}

/// One whole copy of the image, stretched to the rectangle the geometry chose.
///
/// `ContentFit::Fill` is what draws the copy exactly as large as the rectangle:
/// the scaling decision has already been taken, and the image is not made to
/// choose again.
fn copy(wallpaper: &Wallpaper, rect: Rect) -> Element<'static, Message> {
    image(wallpaper.image.clone())
        .width(Length::Fixed(rect.width as f32))
        .height(Length::Fixed(rect.height as f32))
        .content_fit(ContentFit::Fill)
        .into()
}

/// The grid of copies a tiling needs, row by row from the origin.
fn tiled(wallpaper: &Wallpaper) -> Element<'static, Message> {
    let tile = wallpaper.placement.first();
    let columns = (wallpaper.placement.bounds().width / tile.width.max(1)).max(1) as usize;
    let rects: Vec<Rect> = wallpaper.placement.rects().collect();
    let mut rows = Column::new();

    for row in rects.chunks(columns) {
        let mut widgets = Row::new();
        for rect in row {
            widgets = widgets.push(copy(wallpaper, *rect));
        }
        rows = rows.push(widgets);
    }

    rows.into()
}

/// Places a layer in one corner of the surface, inset by the window padding the
/// tokens ask for.
fn overlay(
    content: Element<'static, Message>,
    tokens: Arc<Theme>,
    horizontal: Horizontal,
    vertical: Vertical,
) -> Element<'static, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(theme::window_padding(&tokens))
        .align_x(horizontal)
        .align_y(vertical)
        .into()
}
