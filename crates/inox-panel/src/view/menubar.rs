//! The menu bar's scene: one glass pane of `[menubar] height`.
//!
//! A hairline highlight runs along its top edge, the focused application's name
//! and the panel's own notice are grouped at the left, and the tray items
//! followed by the clock sit at the right, with the flexible space between the
//! two groups (ui-guidelines §3).
//!
//! **There are no application menus here, and there cannot be.** Inox has no
//! protocol through which an application could publish "File / Edit / View", so
//! the bar shows the application's name and stops ([`crate::menubar`]). Drawing
//! invented titles would be a claim about a protocol that does not exist.
//!
//! Every colour, size and radius comes from the tokens through
//! [`crate::theme`]; the notice, the tray group and the clock are the shared
//! painters of [`crate::view`], so the bar cannot draw a disabled entry or a
//! clock differently from the taskbar (D05).

use inox_common::i18n::Localizer;
use inox_config::{MenubarConfig, PanelSurface, Theme as Tokens};

use crate::menubar::{MenubarGeometry, MenubarView};
use crate::theme;
use crate::view::{
    Align, EntryMetrics, Interaction, NoticeMetrics, Painter, Scene, estimate_width,
};

/// How many extra-large spacing steps the notice sentence may be wide.
///
/// The bar is a strip, so the sentence about the panel's own state has a bound
/// of its own: it can never push the clock off the end (D05).
const NOTICE_WIDTH_STEPS: f32 = 8.0;

/// Projects the menu bar at `width` logical pixels.
pub fn menubar(
    model: &crate::view_model::PanelViewModel,
    tokens: &Tokens,
    config: &MenubarConfig,
    localizer: &Localizer,
    width: f32,
    interaction: &Interaction,
) -> Scene {
    let geometry = MenubarGeometry::new(tokens, config);
    let view = MenubarView::project(model, config, localizer);
    let mut painter = Painter::new(PanelSurface::Menubar, tokens);

    // The surface is exactly `[menubar] height` plus the margin between the bar
    // and the top edge of the output; the pane is inset by that margin, so the
    // glass never reaches the edge the compositor anchored it to.
    let pane_y = geometry.margin;
    let pane = geometry.height;
    let metrics = NoticeMetrics {
        font_size: geometry.font_size,
        caption_size: geometry.caption_size,
        entry_padding: geometry.padding,
        group_gap: geometry.group_gap,
        max_width: geometry.label_max_width * NOTICE_WIDTH_STEPS,
    };

    painter.pane(
        0.0,
        pane_y,
        width,
        pane,
        &theme::pane_style(tokens, false, 0.0, false),
    );

    let top = pane_y + geometry.inset + geometry.highlight_height;
    let content_height = geometry.content_height();

    // The left group: the application's name, then the panel's own state.
    let mut left = geometry.padding;

    if let Some(app_name) = &view.app_name {
        // The application's name is the bar's one title: it is emphasized by
        // weight rather than by a size the four type roles do not have
        // (ui-guidelines §2).
        let name_width = estimate_width(app_name, geometry.font_size);

        painter.text(
            left,
            top,
            name_width,
            content_height,
            app_name,
            &theme::text(tokens),
            geometry.font_size,
            true,
            Align::Left,
        );

        left += name_width + geometry.entry_spacing;
    }

    if let Some(notice) = &view.notice {
        painter.notice(
            notice,
            view.retry.as_deref(),
            model.status,
            tokens,
            metrics,
            left,
            top,
            content_height,
            interaction,
        );
    }

    // The right group: the tray, then the clock, laid out from the right edge.
    let mut right = width - geometry.padding;

    if let Some(clock) = &view.clock {
        let clock_width =
            estimate_width(&clock.text, geometry.caption_size).min(geometry.label_max_width);

        painter.text(
            right - clock_width,
            top,
            clock_width,
            content_height,
            &clock.text,
            &theme::clock_text(tokens),
            geometry.caption_size,
            false,
            Align::Left,
        );

        right -= clock_width + geometry.entry_spacing;
    }

    if !view.tray.is_empty() {
        let entries = EntryMetrics {
            caption_size: geometry.caption_size,
            label_max_width: geometry.label_max_width,
            spacing: geometry.entry_spacing,
            tooltip_position: geometry.tooltip_position,
        };
        let group = Painter::tray_width(&view.tray, entries);

        painter.tray_group(
            &view.tray,
            tokens,
            entries,
            right - group,
            top,
            content_height,
        );
    }

    painter.finish(width, pane_y + pane)
}
