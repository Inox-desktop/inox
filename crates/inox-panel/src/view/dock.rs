//! The dock's scene: one glass pane, centred and floating `[dock] margin`
//! above the bottom edge.
//!
//! A hairline highlight runs along the pane's top edge, the panel's own notice
//! (when it has one), one square tile per task group and — when there are status
//! notifier items — a hairline separator followed by the tray group all sit in
//! the same pane, and a hairline shade closes it along the bottom edge.
//!
//! A tile is a click target carrying the same meaning the taskbar's button does
//! ([`EntryAction::Activate`] / [`EntryAction::Toggle`]) and the same
//! enabled/disabled rules: a disabled tile accepts no click and its tooltip is
//! the reason (D05). Its active mark is drawn only for a group the compositor
//! reports as focused, but the mark's slot is laid out for every tile so the
//! tiles never move when focus changes.
//!
//! Every colour, size and radius comes from the tokens through
//! [`crate::theme`]; the tray items and the notice are the shared painters of
//! [`crate::view`].

use inox_common::i18n::Localizer;
use inox_config::{DockConfig, PanelConfig, PanelSurface, Theme as Tokens};

use crate::dock::{DockGeometry, DockTile, DockView};
use crate::theme;
use crate::view::{
    Align, Entry, EntryAction, EntryMetrics, Interaction, NoticeMetrics, Painter, Scene,
    estimate_width,
};
use crate::view_model::PanelViewModel;

/// Projects the dock at `width` logical pixels.
#[allow(clippy::too_many_arguments)]
pub fn dock(
    model: &PanelViewModel,
    tokens: &Tokens,
    panel: &PanelConfig,
    config: &DockConfig,
    localizer: &Localizer,
    width: f32,
    interaction: &Interaction,
) -> Scene {
    let geometry = DockGeometry::new(tokens, config);
    let view = DockView::project(model, config, panel.max_task_buttons, localizer);
    let mut painter = Painter::new(PanelSurface::Dock, tokens);

    let pane_height = geometry.pane_height();
    let window_height = geometry.window_height();
    let pane_y = (window_height - geometry.margin - pane_height).max(0.0);

    // The pane is emphasised while the keyboard navigation is on it, which is
    // what the stronger tint token is for.
    let emphasised = view.tiles.iter().any(|tile| tile.focused);

    painter.pane(
        0.0,
        pane_y,
        width,
        pane_height,
        &theme::glass_stops(tokens, emphasised, pane_height, true),
        theme::radius_md(tokens),
    );

    // The row the pane holds, laid out exactly as `DockGeometry::row_width`
    // sums it: every entry, the gaps between them and the pane's padding. The
    // surface is as wide as this row, so centring it is what centres the pill
    // in its own surface.
    let tray_entries = EntryMetrics {
        caption_size: geometry.tray_size,
        label_max_width: geometry.label_max_width,
        spacing: geometry.spacing,
        tooltip_position: geometry.tooltip_position,
    };
    let notice_slot = view.notice.as_ref().map_or(0.0, |notice| {
        geometry.notice_width(notice, view.retry.as_deref())
    });
    let tray_width = Painter::tray_width(&view.tray, tray_entries);
    let tiles_width = if view.tiles.is_empty() {
        0.0
    } else {
        view.tiles.len() as f32 * geometry.icon_size
            + (view.tiles.len() as f32 - 1.0) * geometry.spacing
    };

    let mut row = 0.0;
    let mut gaps = 0usize;
    for entry in [
        notice_slot,
        tiles_width,
        if view.has_separator() {
            geometry.separator_width
        } else {
            0.0
        },
        tray_width,
    ] {
        if entry > 0.0 {
            row += entry;
            gaps += 1;
        }
    }
    let row_width = geometry.padding * 2.0 + row + gaps.saturating_sub(1) as f32 * geometry.spacing;
    let mut cursor = ((width - row_width) / 2.0).max(0.0) + geometry.padding;

    let top = pane_y + geometry.edge_height + geometry.padding;
    let tiles_top = pane_y + geometry.edge_height + geometry.padding;
    let tiles_height = geometry.icon_size + geometry.indicator_gap + geometry.indicator_height;

    if let Some(notice) = &view.notice {
        // The sentence is laid out in exactly the slot `DockGeometry` reserved
        // for it — the estimate of the sentence, floored at what the longest
        // catalog sentence needs — so the pane is as wide as the placement asks
        // and the compositor frosts the pane rather than a band around it. The
        // sentence itself is centred in that slot.
        let metrics = NoticeMetrics {
            font_size: geometry.tray_size,
            caption_size: geometry.tray_size,
            entry_padding: geometry.padding,
            group_gap: geometry.spacing,
            max_width: geometry.notice_max_width(),
        };
        let row = notice_row_width(notice, view.retry.as_deref(), metrics);
        let offset = ((notice_slot - row) / 2.0).max(0.0);

        painter.notice(
            notice,
            view.retry.as_deref(),
            model.status,
            tokens,
            metrics,
            cursor + offset,
            top,
            geometry.icon_size,
            interaction,
        );

        cursor += notice_slot + geometry.spacing;
    }

    for tile in &view.tiles {
        tile_view(
            &mut painter,
            tile,
            tokens,
            &geometry,
            cursor,
            tiles_top,
            tiles_height,
            interaction,
        );

        cursor += geometry.icon_size + geometry.spacing;
    }

    if view.has_separator() {
        painter.rect(
            cursor,
            top + (geometry.icon_size - geometry.separator_height) / 2.0,
            geometry.separator_width,
            geometry.separator_height,
            &theme::divider_color(tokens),
            0.0,
        );

        cursor += geometry.separator_width + geometry.spacing;
    }

    if !view.tray.is_empty() {
        painter.tray_group(
            &view.tray,
            tokens,
            tray_entries,
            cursor,
            top,
            geometry.icon_size,
        );
    }

    painter.finish(width, window_height)
}

/// One tile: the badge, then the active mark's slot below it.
///
/// The slot is laid out for every tile — an inactive one draws nothing there —
/// so the tiles never move when the focused application changes.
#[allow(clippy::too_many_arguments)]
fn tile_view(
    painter: &mut Painter,
    tile: &DockTile,
    tokens: &Tokens,
    geometry: &DockGeometry,
    x: f32,
    y: f32,
    height: f32,
    interaction: &Interaction,
) {
    let key = tile.key;

    // A click on the application the user is already looking at is the request
    // to put it away, which is what an active tile does (D03).
    let action = if tile.active {
        EntryAction::Toggle(key)
    } else {
        EntryAction::Activate(key)
    };
    let style = theme::dock_tile(
        tokens,
        tile.focused,
        interaction.status(action, tile.enabled),
    );

    painter.entry(&style, x, y, geometry.icon_size, geometry.icon_size);
    painter.text(
        x,
        y,
        geometry.icon_size,
        geometry.icon_size,
        &tile.glyph,
        &style.text_color,
        geometry.glyph_size,
        false,
        Align::Center,
    );

    let mark_y = y + geometry.icon_size + geometry.indicator_gap;

    painter.rect(
        x + (geometry.icon_size - geometry.indicator_width) / 2.0,
        mark_y,
        geometry.indicator_width,
        geometry.indicator_height,
        &theme::dock_indicator(tokens, tile.active),
        geometry.indicator_height.min(geometry.indicator_width) / 2.0,
    );

    // The click target is the badge and the mark's slot under it: the whole
    // column the user sees as one tile. Starting it at the mark instead would
    // leave the badge itself outside its own button.
    painter.entries.push(Entry {
        action,
        enabled: tile.enabled,
        x,
        y,
        width: geometry.icon_size,
        height,
    });
}

/// How wide the sentence and its retry action take in the dock.
fn notice_row_width(notice: &str, retry: Option<&str>, metrics: NoticeMetrics) -> f32 {
    let sentence = estimate_width(notice, metrics.caption_size).min(metrics.max_width);

    match retry {
        None => sentence,
        Some(retry) => {
            sentence
                + metrics.group_gap
                + metrics.entry_padding * 2.0
                + estimate_width(retry, metrics.font_size)
        }
    }
}
