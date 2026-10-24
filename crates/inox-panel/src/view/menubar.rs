//! The menu bar's widget tree.
//!
//! The bar is one glass pane of `[menubar] height`: a hairline highlight along
//! its top edge, the focused application's name and the panel's own notice
//! grouped at the left, and the tray items followed by the clock at the right,
//! with the flexible space between the two groups (ui-guidelines §3).
//!
//! **There are no application menus here, and there cannot be.** Inox has no
//! protocol through which an application could publish "File / Edit / View",
//! so the bar shows the application's name and stops ([`crate::menubar`]).
//! Drawing invented titles would be a claim about a protocol that does not
//! exist.
//!
//! Every colour, size and radius comes from the tokens through
//! [`crate::theme`]; the entries themselves are the shared helpers of
//! [`crate::view`], so the bar cannot draw a disabled entry differently from the
//! taskbar (D05).

use iced::widget::{Row, Space, container, text};
use iced::{Element, Length, Padding};
use inox_common::i18n::Localizer;
use inox_config::{MenubarConfig, Theme as Tokens};

use crate::app::Message;
use crate::menubar::{MenubarGeometry, MenubarView};
use crate::theme;
use crate::view::{EntryMetrics, NoticeMetrics, clock_view, notice_view, tray_group};
use crate::view_model::PanelViewModel;

/// How many extra-large spacing steps the notice sentence may be wide.
///
/// The bar is a strip, so the sentence about the panel's own state has a bound
/// of its own: it can never push the clock off the end (D05).
const NOTICE_WIDTH_STEPS: f32 = 8.0;

/// Draws the menu bar.
pub fn menubar<'a>(
    model: PanelViewModel,
    tokens: &'a Tokens,
    config: &MenubarConfig,
    localizer: &Localizer,
) -> Element<'a, Message> {
    let geometry = MenubarGeometry::new(tokens, config);
    let view = MenubarView::project(&model, config, localizer);

    let mut left: Vec<Element<'a, Message>> = Vec::new();
    if let Some(app_name) = &view.app_name {
        // The application's name is the bar's one title: it is emphasized by
        // weight rather than by a size the four type roles do not have
        // (ui-guidelines §2).
        left.push(
            text(app_name.clone())
                .size(geometry.font_size)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..iced::Font::DEFAULT
                })
                .color(theme::text(tokens))
                .into(),
        );
    }
    if let Some(notice) = &view.notice {
        left.push(notice_view(
            notice,
            view.retry.as_deref(),
            model.status,
            tokens,
            NoticeMetrics {
                font_size: geometry.font_size,
                caption_size: geometry.caption_size,
                entry_padding: geometry.padding,
                group_gap: geometry.group_gap,
                max_width: geometry.label_max_width * NOTICE_WIDTH_STEPS,
            },
        ));
    }

    let mut right: Vec<Element<'a, Message>> = Vec::new();
    if !view.tray.is_empty() {
        right.push(tray_group(&view.tray, tokens, entries(&geometry)));
    }
    if let Some(clock) = &view.clock {
        right.push(clock_view(
            clock.text.clone(),
            clock.tooltip.clone(),
            tokens,
            entries(&geometry),
        ));
    }

    // The flexible space is what puts the two groups at the two ends of the bar,
    // and it is never narrower than one group gap — which is wider than the
    // entries inside either group, so the grouping is visible at any width.
    let mut groups: Vec<Element<'a, Message>> = vec![
        Row::with_children(left)
            .spacing(geometry.entry_spacing)
            .align_y(iced::alignment::Vertical::Center)
            .into(),
    ];
    if !right.is_empty() {
        groups.push(Space::new().width(Length::Fill).into());
        groups.push(
            Row::with_children(right)
                .spacing(geometry.entry_spacing)
                .align_y(iced::alignment::Vertical::Center)
                .into(),
        );
    }

    // The bar is exactly `[menubar] height` tall: the pane's own box is the
    // configured height, its content is centred inside it, and the vertical room
    // the text is allowed is what `MenubarGeometry::content_height` clamps it to.
    // Putting the inset in the pane's padding instead would make the pane taller
    // than the height it was configured with, because iced adds a container's
    // padding around the length it was told.
    let pane = container(
        Row::with_children(groups)
            .spacing(geometry.group_gap)
            .align_y(iced::alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(geometry.height))
    .padding(Padding {
        top: 0.0,
        right: geometry.padding,
        bottom: 0.0,
        left: geometry.padding,
    })
    .align_y(iced::alignment::Vertical::Center)
    .style(move |_theme| theme::glass_bar(tokens, geometry.height));

    // The configured margin is the distance between the bar and the top edge of
    // the output: the surface is anchored with that margin, and the pane is inset
    // by it inside the surface.
    container(pane)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: geometry.margin,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        })
        .style(move |_theme| theme::glass_surface(tokens))
        .into()
}

/// The sizes the bar's group of display-only entries is drawn at.
fn entries(geometry: &MenubarGeometry) -> EntryMetrics {
    EntryMetrics {
        caption_size: geometry.caption_size,
        label_max_width: geometry.label_max_width,
        spacing: geometry.entry_spacing,
        tooltip_position: geometry.tooltip_position,
    }
}
