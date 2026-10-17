//! The widget tree.
//!
//! This module draws and decides nothing: every string, every enabled state and
//! the geometry of every copy of the image were settled in
//! [`crate::view_model`], and every colour, size and radius comes from
//! [`crate::theme`]. What is left is the one thing a view is for — building the
//! tree.
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
use iced::widget::{Column, Row, Space, Stack, button, container, image, text};
use iced::{ContentFit, Element, Length, Padding};
use inox_config::Theme;
use std::sync::Arc;

use crate::app::{Message, State};
use crate::scaling::Rect;
use crate::theme;
use crate::view_model::{self, Action, Notice, Wallpaper, WallpaperViewModel};

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

    if let Some(notice) = &model.notice {
        layers.push(overlay(
            panel(notice, &tokens),
            Arc::clone(&tokens),
            Horizontal::Left,
            Vertical::Top,
        ));
    }
    if let Some(status) = &model.status {
        layers.push(overlay(
            caption(status, &tokens),
            Arc::clone(&tokens),
            Horizontal::Right,
            Vertical::Top,
        ));
    }
    if let Some(footer) = &model.footer {
        layers.push(overlay(
            caption(footer, &tokens),
            Arc::clone(&tokens),
            Horizontal::Right,
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

/// The panel a notice is drawn in.
fn panel(notice: &Notice, tokens: &Arc<Theme>) -> Element<'static, Message> {
    let mut content = Column::new().spacing(theme::gap(tokens)).push(
        text(notice.heading.clone())
            .size(theme::heading_size(tokens))
            .font(theme::font(tokens))
            .color(theme::warning(tokens)),
    );

    if let Some(subject) = &notice.subject {
        content = content.push(caption(subject, tokens));
    }
    if let Some(reason) = &notice.reason {
        content = content.push(
            text(reason.clone())
                .size(theme::body_size(tokens))
                .font(theme::font(tokens))
                .color(theme::text(tokens)),
        );
    }
    if let Some(remedy) = &notice.remedy {
        content = content.push(caption(remedy, tokens));
    }

    content = content.push(control(&notice.action, tokens));

    let themed = Arc::clone(tokens);

    container(content)
        .padding(theme::padding(tokens))
        .style(move |_theme| theme::panel(&themed))
        .into()
}

/// The one control a notice offers, and why it cannot be pressed when it cannot.
fn control(action: &Action, tokens: &Arc<Theme>) -> Element<'static, Message> {
    let themed = Arc::clone(tokens);
    let mut content = Column::new().spacing(theme::gap(tokens)).push(
        button(
            text(action.label.clone())
                .size(theme::body_size(tokens))
                .font(theme::font(tokens)),
        )
        .padding(theme::padding(tokens))
        .style(move |_theme, status| theme::button_style(&themed, status))
        .on_press_maybe(action.enabled.then_some(Message::Reload)),
    );

    if let Some(reason) = &action.reason {
        content = content.push(caption(reason, tokens));
    }

    content.into()
}

/// Secondary text, in the theme's muted colour.
fn caption(value: &str, tokens: &Theme) -> Element<'static, Message> {
    text(value.to_owned())
        .size(theme::caption_size(tokens))
        .font(theme::font(tokens))
        .color(theme::muted(tokens))
        .into()
}

/// Places a layer in one corner of the surface, inset by the theme's margin.
fn overlay(
    content: Element<'static, Message>,
    tokens: Arc<Theme>,
    horizontal: Horizontal,
    vertical: Vertical,
) -> Element<'static, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(theme::margin(&tokens))
        .align_x(horizontal)
        .align_y(vertical)
        .into()
}
