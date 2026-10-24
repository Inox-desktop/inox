//! The dock's widget tree.
//!
//! One glass pane, centred horizontally and floating `[dock] margin` above the
//! bottom edge: a hairline highlight along its top edge, the panel's own notice
//! when it has one, one square tile per task group, and — when there are status
//! notifier items — a hairline separator followed by the tray group, all inside
//! the same pane. A hairline shade closes the pane along its bottom edge.
//!
//! A tile is a `button` carrying the same messages the taskbar uses
//! ([`Message::TaskActivated`] / [`Message::TaskToggled`]) and the same
//! enabled/disabled rules: a disabled tile cannot be pressed and its tooltip is
//! the reason (D05). Its active mark is drawn only for a group the compositor
//! reports as focused.
//!
//! Every colour, size and radius comes from the tokens through
//! [`crate::theme`]; the tray items and the notice are the shared helpers of
//! [`crate::view`].

use iced::widget::{Column, Row, Space, button, container, text, tooltip};
use iced::{Element, Length, Padding};
use inox_common::i18n::Localizer;
use inox_config::{DockConfig, PanelConfig, Theme as Tokens};

use crate::app::Message;
use crate::dock::{DockGeometry, DockTile, DockView};
use crate::theme;
use crate::view::{EntryMetrics, NoticeMetrics, notice_view, tray_group};
use crate::view_model::PanelViewModel;

/// Draws the dock.
pub fn dock<'a>(
    model: PanelViewModel,
    tokens: &'a Tokens,
    panel: &PanelConfig,
    config: &DockConfig,
    localizer: &Localizer,
) -> Element<'a, Message> {
    let geometry = DockGeometry::new(tokens, config);
    let view = DockView::project(&model, config, panel.max_task_buttons, localizer);

    let mut entries: Vec<Element<'a, Message>> = Vec::new();
    if let Some(notice) = &view.notice {
        // The sentence is laid out in exactly the slot `DockGeometry` reserved
        // for it — the estimate of the sentence, floored at what the longest
        // catalog sentence needs — so the pane is as wide as the placement asks
        // and the compositor frosts the pane rather than a band around it. The
        // sentence itself is centred in that slot.
        let slot = geometry.notice_width(notice, view.retry.as_deref());
        let notice = notice_view(
            notice,
            view.retry.as_deref(),
            model.status,
            tokens,
            NoticeMetrics {
                font_size: geometry.tray_size,
                caption_size: geometry.tray_size,
                entry_padding: geometry.padding,
                group_gap: geometry.spacing,
                max_width: geometry.notice_max_width(),
            },
        );

        entries.push(
            container(notice)
                .width(Length::Fixed(slot))
                .align_x(iced::alignment::Horizontal::Center)
                .into(),
        );
    }
    if !view.tiles.is_empty() {
        entries.push(tiles(&view.tiles, tokens, &geometry));
    }
    if view.has_separator() {
        entries.push(
            container(Space::new())
                .width(Length::Fixed(geometry.separator_width))
                .height(Length::Fixed(geometry.separator_height))
                .style(move |_theme| theme::divider(tokens))
                .into(),
        );
        entries.push(tray_group(&view.tray, tokens, entries_metrics(&geometry)));
    }

    // The pane is exactly the geometry's pane height with its content centred,
    // and exactly as wide as the row it holds — which is the width
    // `DockGeometry::surface_width` asks the compositor for, so the surface and
    // the pane agree. The tiles are inset from the pane's edge by the token
    // padding and the edge hairlines together, which is the room `DockGeometry`
    // already accounts for. Putting that inset in the pane's own padding would
    // make the pane taller than the geometry says, because iced adds a
    // container's padding around the length it was told.
    let pane_height = geometry.pane_height();

    // The pane is emphasised while the keyboard navigation is on it, which is
    // what the stronger tint token is for.
    let emphasised = view.tiles.iter().any(|tile| tile.focused);
    let pane = container(
        Row::with_children(entries)
            .spacing(geometry.spacing)
            .align_y(iced::alignment::Vertical::Center),
    )
    .height(Length::Fixed(pane_height))
    .padding(Padding {
        top: 0.0,
        right: geometry.padding,
        bottom: 0.0,
        left: geometry.padding,
    })
    .align_y(iced::alignment::Vertical::Center)
    .clip(true)
    .style(move |_theme| theme::glass_dock(tokens, emphasised, pane_height));

    container(pane)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: 0.0,
            right: 0.0,
            bottom: geometry.margin,
            left: 0.0,
        })
        .align_x(iced::alignment::Horizontal::Center)
        .align_y(iced::alignment::Vertical::Bottom)
        .clip(true)
        .style(move |_theme| theme::glass_surface(tokens))
        .into()
}

/// The tiles, as one row of square badges.
fn tiles<'a>(
    tiles: &[DockTile],
    tokens: &'a Tokens,
    geometry: &DockGeometry,
) -> Element<'a, Message> {
    let children: Vec<Element<'a, Message>> = tiles
        .iter()
        .map(|tile| tile_view(tile, tokens, geometry))
        .collect();

    Row::with_children(children)
        .spacing(geometry.spacing)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

/// One tile: the badge, then the active mark's slot below it.
///
/// The slot is laid out for every tile — an inactive one draws nothing there —
/// so the tiles never move when the focused application changes.
fn tile_view<'a>(
    tile: &DockTile,
    tokens: &'a Tokens,
    geometry: &DockGeometry,
) -> Element<'a, Message> {
    let key = tile.key;
    let active = tile.active;
    let enabled = tile.enabled;
    let focused = tile.focused;

    // A click on the application the user is already looking at is the request
    // to put it away, which is what an active tile does (D03).
    let on_press = if active {
        Message::TaskToggled { key }
    } else {
        Message::TaskActivated { key }
    };

    let badge = button(
        container(text(tile.glyph.clone()).size(geometry.glyph_size))
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center),
    )
    .width(Length::Fixed(geometry.icon_size))
    .height(Length::Fixed(geometry.icon_size))
    .padding(Padding {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    })
    .on_press_maybe(enabled.then_some(on_press))
    .style(move |_theme, status| theme::dock_tile(tokens, focused, status));

    let badge: Element<'a, Message> = tooltip(
        badge,
        text(tile.tooltip.clone()).size(geometry.tray_size),
        geometry.tooltip_position,
    )
    .into();

    let mark = container(Space::new())
        .width(Length::Fixed(geometry.indicator_width))
        .height(Length::Fixed(geometry.indicator_height))
        .style(move |_theme| theme::dock_indicator(tokens, active));

    Column::with_children([badge, mark.into()])
        .spacing(geometry.indicator_gap)
        .align_x(iced::alignment::Horizontal::Center)
        .width(Length::Shrink)
        .into()
}

/// The sizes the dock's group of display-only entries is drawn at.
fn entries_metrics(geometry: &DockGeometry) -> EntryMetrics {
    EntryMetrics {
        caption_size: geometry.tray_size,
        label_max_width: geometry.label_max_width,
        spacing: geometry.spacing,
        tooltip_position: geometry.tooltip_position,
    }
}
