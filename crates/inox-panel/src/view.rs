//! The panel's row of widgets: tasks, tray, clock.
//!
//! Everything here is drawing. Which entries exist, what they say, and why one
//! of them is disabled all arrive already decided in a
//! [`crate::view_model::PanelViewModel`]; this module turns that into iced
//! widgets and takes every colour, size and radius from the tokens through
//! [`crate::theme`].
//!
//! [`surface`] is the entry point the application calls: it picks the widget
//! tree `[panel] surface` asks for and hands each one the section that describes
//! it. The taskbar is [`panel`]; the menu bar is [`menubar::menubar`] and the
//! dock is [`dock::dock`], both of which reuse the entry helpers below so that
//! three surfaces can never draw "disabled" or "hovered" two different ways.
//!
//! The taskbar strip is drawn as **groups separated by a gap wider than the
//! entries inside them** (`docs/quality/ui-guidelines.md` §3), so a user can see
//! at a glance what belongs together:
//!
//! ```text
//! [ notice + retry ]   [ task ][ task ][ task ]        │  [tray]   [clock]
//! └──── group gap ────┘└── entry spacing ──┘            └ divider
//! ```
//!
//! Four layout decisions are worth naming, because they are what acceptance D01
//! and D05 are about:
//!
//! - **The clock and the tray keep their space.** The task buttons sit in a
//!   flexible area and each label is bounded, so an application with a very long
//!   title cannot push a key action off the end of the strip.
//! - **The panel is one row of the configured height.** A taller panel gets
//!   taller entries, not a second row; the entries are inset vertically so a
//!   rounded, pressed or focused entry stays inside the strip.
//! - **Nothing on the strip takes keyboard focus by itself.** The task buttons
//!   are click targets; the clock and the tray items are plain containers, and
//!   the panel's own navigation focus is drawn as a colour, never as a widget
//!   that could hold on to focus (ADR 0013, [`crate::placement`]).
//! - **A disabled entry says why.** Its hover text is the reason the projection
//!   produced, not the application name the label already shows (D05).

pub mod dock;
pub mod menubar;

use iced::widget::text::Wrapping;
use iced::widget::{Row, Space, button, container, text, tooltip};
use iced::{Element, Length, Padding};
use inox_common::i18n::Localizer;
use inox_config::{PanelConfig, PanelPosition, PanelSurface, ShellConfig, Theme as Tokens};

use crate::app::Message;
use crate::theme::{self, TaskEmphasis};
use crate::view_model::{PanelStatus, PanelViewModel, TaskButton, TrayItemView};

/// How many extra-large spacing steps a task label may be wide.
///
/// The token set has no width for a control, so the bound is expressed in the
/// spacing scale: six extra-large steps is 192 logical pixels at the default
/// tokens, which fits a dozen buttons on a 1280-wide panel and still leaves the
/// clock its space. It is a *maximum*, drawn from the tokens rather than written
/// as a number, and the layout clips anything longer (D05).
const LABEL_WIDTH_STEPS: f32 = 6.0;

/// How many extra-large spacing steps the panel's own sentence may be wide.
///
/// The sentence about the panel's state is more important than any single
/// entry, so it is allowed to be wider than one: eight steps is 256 logical
/// pixels, and everything after it is still reachable.
const NOTICE_WIDTH_STEPS: f32 = 8.0;

/// The geometry of the panel strip, derived from the tokens and the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelGeometry {
    /// Height of the strip, in logical pixels.
    pub height: f32,
    /// Padding at the left and right end of the strip.
    pub padding: f32,
    /// Vertical inset of an entry inside the strip, so an entry's rounded
    /// corners and its focus ring are drawn *inside* the strip.
    pub inset: f32,
    /// Gap between two entries of the same group.
    pub spacing: f32,
    /// Gap between two groups: the notice, the tasks, the tray and the clock.
    pub group_gap: f32,
    /// Padding inside one entry.
    pub entry_padding: f32,
    /// Widest a label may be before the layout clips it.
    pub label_max_width: f32,
    /// Text size of a task label, already scaled by the user's font preference
    /// and clamped so scaled text never grows past the strip.
    pub font_size: f32,
    /// Text size of secondary information: the clock, a tray item, the ordinary
    /// states of the panel's own sentence.
    pub caption_size: f32,
    /// Thickness of the divider between the task area and the tray.
    pub divider_width: f32,
    /// Height of that divider: as tall as a line of secondary information.
    pub divider_height: f32,
    /// Which way a tooltip opens, so that it is not drawn past the screen edge.
    pub tooltip_position: tooltip::Position,
}

impl PanelGeometry {
    /// Derives the strip's geometry.
    pub fn new(tokens: &Tokens, config: &PanelConfig) -> Self {
        let height = f32::from(config.height);
        let padding = f32::from(tokens.spacing.md);
        let inset = f32::from(tokens.spacing.xs);
        let caption = theme::caption_size(tokens, config.height);
        // Text that is taller than the strip would overflow it, so the scaled
        // size is clamped to what is left after the inset.
        let available = (height - inset * 2.0).max(1.0);

        Self {
            height,
            padding,
            inset,
            spacing: f32::from(tokens.spacing.sm),
            group_gap: f32::from(tokens.spacing.lg),
            entry_padding: padding,
            label_max_width: f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS,
            font_size: theme::font_size(tokens, config.height).min(available),
            caption_size: caption.min(available),
            divider_width: theme::hairline(tokens),
            divider_height: caption.min(available),
            tooltip_position: match config.position {
                PanelPosition::Bottom => tooltip::Position::Top,
                PanelPosition::Top => tooltip::Position::Bottom,
            },
        }
    }

    /// The height one entry has inside the strip.
    ///
    /// This is the room scaled text has to fit in, which is what the unit tests
    /// assert the font size against.
    pub fn content_height(&self) -> f32 {
        (self.height - self.inset * 2.0).max(1.0)
    }

    /// The sizes the strip's group of display-only entries is drawn at.
    fn entries(&self) -> EntryMetrics {
        EntryMetrics {
            caption_size: self.caption_size,
            label_max_width: self.label_max_width,
            spacing: self.entry_padding,
            tooltip_position: self.tooltip_position,
        }
    }

    /// The sizes the strip's own notice is drawn at.
    fn notice(&self) -> NoticeMetrics {
        NoticeMetrics {
            font_size: self.font_size,
            caption_size: self.caption_size,
            entry_padding: self.entry_padding,
            group_gap: self.group_gap,
            max_width: self.label_max_width * NOTICE_WIDTH_STEPS,
        }
    }
}

/// The sizes a group of display-only entries (the tray, the clock) is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct EntryMetrics {
    /// Text size of a secondary line.
    pub caption_size: f32,
    /// Widest a label may be before the layout clips it.
    pub label_max_width: f32,
    /// Gap between two entries of the group.
    pub spacing: f32,
    /// Which way a tooltip opens.
    pub tooltip_position: tooltip::Position,
}

/// The sizes the panel's own notice is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct NoticeMetrics {
    /// Text size of the action the notice offers.
    pub font_size: f32,
    /// Text size of the ordinary states of the sentence.
    pub caption_size: f32,
    /// Padding inside the retry action.
    pub entry_padding: f32,
    /// Gap between the sentence and its action.
    pub group_gap: f32,
    /// Widest the sentence may be before the layout clips it.
    pub max_width: f32,
}

/// Draws the surface `shell.panel.surface` asks for.
///
/// The three surfaces present the same model; what changes is the tree, the
/// section of the configuration that describes it and, for the glass surfaces,
/// the material behind it. The localizer comes along because the menu bar names
/// the focused application and the dock badges it, and both fall back to a
/// catalog message for an application that has no name.
pub fn surface<'a>(
    model: PanelViewModel,
    tokens: &'a Tokens,
    shell: &ShellConfig,
    localizer: &Localizer,
) -> Element<'a, Message> {
    match shell.panel.surface {
        PanelSurface::Taskbar => panel(model, tokens, &shell.panel),
        PanelSurface::Menubar => menubar::menubar(model, tokens, &shell.menubar, localizer),
        PanelSurface::Dock => dock::dock(model, tokens, &shell.panel, &shell.dock, localizer),
    }
}

/// Draws the panel's taskbar strip.
///
/// The model is taken by value and its strings are copied into the widgets, so
/// the element this returns borrows the tokens and nothing else — which is what
/// lets the caller project a fresh model on every frame instead of caching one.
pub fn panel<'a>(
    model: PanelViewModel,
    tokens: &'a Tokens,
    config: &PanelConfig,
) -> Element<'a, Message> {
    let geometry = PanelGeometry::new(tokens, config);
    let mut groups: Vec<Element<'a, Message>> = Vec::new();

    if let Some(notice) = &model.notice {
        groups.push(notice_view(
            notice,
            model.retry.as_deref(),
            model.status,
            tokens,
            geometry.notice(),
        ));
    }

    // The task buttons share one flexible area, which is what keeps the clock
    // and the tray where they belong: iced lays non-fluid children out first, so
    // the tray and the clock ask for their width before the buttons take what is
    // left (D05).
    groups.push(task_area(&model.tasks, model.focused, tokens, &geometry));

    if !model.tray.is_empty() {
        groups.push(divider(
            tokens,
            geometry.divider_width,
            geometry.divider_height,
        ));
        groups.push(tray_group(&model.tray, tokens, geometry.entries()));
    }

    if let Some(clock) = &model.clock {
        groups.push(clock_view(
            clock.text.clone(),
            clock.tooltip.clone(),
            tokens,
            geometry.entries(),
        ));
    }

    container(
        Row::with_children(groups)
            .spacing(geometry.group_gap)
            .align_y(iced::alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fixed(geometry.height))
    .padding(Padding {
        top: geometry.inset,
        right: geometry.padding,
        bottom: geometry.inset,
        left: geometry.padding,
    })
    .style(move |_theme| theme::panel(tokens))
    .into()
}

/// The task buttons, in one flexible area that clips what does not fit.
///
/// The area is fluid and the entries inside it are not, which is the whole
/// arrangement: the tray and the clock are laid out first and keep their width,
/// and a title that is too long is cut off inside this area instead of pushing
/// them off the strip. With no task to draw the area is empty space, which is
/// what keeps the tray and the clock at the panel's other end.
fn task_area<'a>(
    tasks: &[TaskButton],
    focused: Option<crate::tasks::WindowKey>,
    tokens: &'a Tokens,
    geometry: &PanelGeometry,
) -> Element<'a, Message> {
    if tasks.is_empty() {
        return Space::new().width(Length::Fill).into();
    }

    let buttons: Vec<Element<'a, Message>> = tasks
        .iter()
        .map(|task| task_view(task, focused == Some(task.key), tokens, geometry))
        .collect();

    container(
        Row::with_children(buttons)
            .spacing(geometry.spacing)
            .align_y(iced::alignment::Vertical::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .clip(true)
    .into()
}

/// The sentence the panel shows about itself, and its retry action.
///
/// The panel's own state is what a user has to read first, so the state that
/// needs action — the window service is not available — is written at the body
/// size in the warning colour, while the ordinary states (starting, empty) are
/// secondary information at the caption size in the muted colour. The sentence
/// never claims more than the projection knows, and a retry is always offered
/// when there is something to retry (D05: never an endless spinner).
///
/// The taskbar and the menu bar both draw this, so the panel's own state can
/// never be worded, coloured or laid out two different ways.
fn notice_view<'a>(
    notice: &str,
    retry: Option<&str>,
    status: PanelStatus,
    tokens: &'a Tokens,
    metrics: NoticeMetrics,
) -> Element<'a, Message> {
    let colour = theme::notice_text(tokens, status);
    let size = match status {
        PanelStatus::Unavailable => metrics.font_size,
        PanelStatus::Starting | PanelStatus::Empty | PanelStatus::Ready => metrics.caption_size,
    };

    let label = clamp(
        text(notice.to_owned())
            .size(size)
            .color(colour)
            .wrapping(Wrapping::None)
            .into(),
        metrics.max_width,
    );

    match retry {
        None => label,
        Some(retry) => {
            let action = button(text(retry.to_owned()).size(metrics.font_size))
                .height(Length::Fill)
                .padding(Padding {
                    top: 0.0,
                    right: metrics.entry_padding,
                    bottom: 0.0,
                    left: metrics.entry_padding,
                })
                .on_press(Message::Retry)
                .style(move |_theme, status| theme::notice_action(tokens, status));

            Row::with_children([label, action.into()])
                .spacing(metrics.group_gap)
                .align_y(iced::alignment::Vertical::Center)
                .into()
        }
    }
}

/// One task button.
fn task_view<'a>(
    task: &TaskButton,
    focused: bool,
    tokens: &'a Tokens,
    geometry: &PanelGeometry,
) -> Element<'a, Message> {
    let emphasis = if task.active {
        TaskEmphasis::Active
    } else if task.minimized {
        TaskEmphasis::Minimized
    } else {
        TaskEmphasis::Idle
    };

    // A click on the application the user is already looking at is the request
    // to put it away, which is what a taskbar's active button does (D03).
    let on_press = if task.active {
        Message::TaskToggled { key: task.key }
    } else {
        Message::TaskActivated { key: task.key }
    };

    // A disabled entry says why: the reason is a sentence the user can act on,
    // and the label already carries the application's name (D05).
    let hint = hint(task);

    let button = button(clamp(
        text(task.label.clone())
            .size(geometry.font_size)
            .wrapping(Wrapping::None)
            .into(),
        geometry.label_max_width,
    ))
    .height(Length::Fill)
    .padding(Padding {
        top: 0.0,
        right: geometry.entry_padding,
        bottom: 0.0,
        left: geometry.entry_padding,
    })
    .on_press_maybe(task.enabled.then_some(on_press))
    .style(move |_theme, status| theme::task_button(tokens, emphasis, focused, status));

    tooltip(
        button,
        text(hint).size(geometry.caption_size),
        geometry.tooltip_position,
    )
    .into()
}

/// What a hover on an entry says: its tooltip, or the reason it is disabled.
///
/// A disabled entry says why, and the reason is a sentence the user can act on
/// rather than the application name the label already carries (D05).
pub(super) fn hint(task: &TaskButton) -> String {
    if task.enabled {
        task.tooltip.clone()
    } else {
        task.disabled_reason
            .clone()
            .unwrap_or_else(|| task.tooltip.clone())
    }
}

/// The tray items, as one group: display-only, so they show a title and their
/// status and take no pointer or keyboard input.
pub(super) fn tray_group<'a>(
    items: &[TrayItemView],
    tokens: &'a Tokens,
    metrics: EntryMetrics,
) -> Element<'a, Message> {
    let children: Vec<Element<'a, Message>> = items
        .iter()
        .map(|item| tray_view(item, tokens, metrics))
        .collect();

    Row::with_children(children)
        .spacing(metrics.spacing)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

/// One tray item.
pub(super) fn tray_view<'a>(
    item: &TrayItemView,
    tokens: &'a Tokens,
    metrics: EntryMetrics,
) -> Element<'a, Message> {
    let label = clamp(
        text(item.label.clone())
            .size(metrics.caption_size)
            .color(theme::tray_text(tokens, item.attention))
            .wrapping(Wrapping::None)
            .into(),
        metrics.label_max_width,
    );

    tooltip(
        label,
        text(item.tooltip.clone()).size(metrics.caption_size),
        metrics.tooltip_position,
    )
    .into()
}

/// The clock, display-only, with the full date in its tooltip.
pub(super) fn clock_view<'a>(
    clock: String,
    tooltip_text: String,
    tokens: &'a Tokens,
    metrics: EntryMetrics,
) -> Element<'a, Message> {
    tooltip(
        text(clock)
            .size(metrics.caption_size)
            .color(theme::clock_text(tokens)),
        text(tooltip_text).size(metrics.caption_size),
        metrics.tooltip_position,
    )
    .into()
}

/// A hairline that separates two groups.
///
/// The tasks are what the user acts on and the tray is what the panel reports,
/// so the two groups are separated by a divider as well as by a wider gap. It
/// takes its colour from [`theme::divider`] and both of its sizes from the
/// geometry it is drawn on, so a divider can never be a size nobody chose.
pub(super) fn divider<'a>(tokens: &'a Tokens, width: f32, height: f32) -> Element<'a, Message> {
    container(Space::new())
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .style(move |_theme| theme::divider(tokens))
        .into()
}

/// Bounds a label so that the rest of the surface keeps its space.
///
/// `Wrapping::None` keeps a label on one line; the maximum width is what stops
/// that line from growing without end.
pub(super) fn clamp<'a>(content: Element<'a, Message>, max_width: f32) -> Element<'a, Message> {
    container(content).max_width(max_width).clip(true).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_model::PanelViewModel;

    fn geometry(config: &PanelConfig) -> PanelGeometry {
        PanelGeometry::new(&Tokens::builtin_dark(), config)
    }

    fn model() -> PanelViewModel {
        PanelViewModel {
            status: PanelStatus::Ready,
            tasks: Vec::new(),
            tray: Vec::new(),
            clock: None,
            notice: None,
            retry: None,
            focus_order: Vec::new(),
            focused: None,
        }
    }

    #[test]
    fn the_strip_is_as_tall_as_the_configuration_says() {
        let geometry = geometry(&PanelConfig {
            height: 48,
            ..PanelConfig::default()
        });

        assert!((geometry.height - 48.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_label_bound_comes_from_the_spacing_tokens() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&PanelConfig::default());

        assert!(
            (geometry.label_max_width - f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS).abs()
                < f32::EPSILON
        );
        assert!(geometry.label_max_width > f32::from(tokens.spacing.lg));
    }

    #[test]
    fn scaled_text_never_grows_past_the_strip() {
        let mut tokens = Tokens::builtin_dark();
        tokens.typography.scale = 4.0;
        let config = PanelConfig {
            height: 24,
            ..PanelConfig::default()
        };

        let geometry = PanelGeometry::new(&tokens, &config);

        assert!(
            geometry.font_size <= geometry.content_height(),
            "the text has to fit inside the strip"
        );
        assert!(geometry.font_size > 0.0);
    }

    #[test]
    fn the_entries_are_inset_so_their_corners_stay_inside_the_strip() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&PanelConfig::default());

        assert!((geometry.inset - f32::from(tokens.spacing.xs)).abs() < f32::EPSILON);
        assert!(geometry.inset > 0.0);
        assert!(
            geometry.content_height() < geometry.height,
            "an entry is shorter than the strip it sits in"
        );
    }

    #[test]
    fn every_size_comes_from_a_token() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&PanelConfig::default());

        assert!((geometry.padding - f32::from(tokens.spacing.md)).abs() < f32::EPSILON);
        assert!((geometry.entry_padding - geometry.padding).abs() < f32::EPSILON);
        assert!((geometry.spacing - f32::from(tokens.spacing.sm)).abs() < f32::EPSILON);
        assert!((geometry.group_gap - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON);
        assert!((geometry.divider_width - theme::hairline(&tokens)).abs() < f32::EPSILON);
    }

    #[test]
    fn groups_are_further_apart_than_the_entries_inside_them() {
        let geometry = geometry(&PanelConfig::default());

        assert!(
            geometry.group_gap > geometry.spacing,
            "grouping is what tells a user what belongs together (ui-guidelines §3)"
        );
    }

    #[test]
    fn the_secondary_information_is_smaller_than_a_task_label() {
        let geometry = geometry(&PanelConfig::default());

        assert!(geometry.caption_size < geometry.font_size);
        assert!(
            (geometry.divider_height - geometry.caption_size).abs() < f32::EPSILON,
            "the divider is as tall as a line of secondary information"
        );
    }

    #[test]
    fn a_tooltip_opens_away_from_the_edge_the_panel_sits_on() {
        let bottom = geometry(&PanelConfig {
            position: PanelPosition::Bottom,
            ..PanelConfig::default()
        });
        let top = geometry(&PanelConfig {
            position: PanelPosition::Top,
            ..PanelConfig::default()
        });

        assert_eq!(bottom.tooltip_position, tooltip::Position::Top);
        assert_eq!(top.tooltip_position, tooltip::Position::Bottom);
    }

    #[test]
    fn an_empty_model_still_lays_out() {
        let tokens = Tokens::builtin_dark();
        let config = PanelConfig {
            height: 34,
            ..PanelConfig::default()
        };

        // Building the element is the assertion: a model with nothing in it
        // must still produce a strip of the configured height.
        let _element = panel(model(), &tokens, &config);
        assert!((geometry(&config).height - 34.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_surface_picker_draws_the_section_the_configuration_names() {
        let tokens = Tokens::builtin_dark();
        let locale = inox_common::i18n::Locale::default_locale();
        let localizer = crate::i18n::load(&locale).expect("the catalogs parse");

        for mode in [
            PanelSurface::Taskbar,
            PanelSurface::Menubar,
            PanelSurface::Dock,
        ] {
            let shell = ShellConfig {
                panel: PanelConfig {
                    surface: mode,
                    ..PanelConfig::default()
                },
                ..ShellConfig::default()
            };

            // Building the element is the assertion: the picker has to find a
            // tree for every surface, and each tree only borrows `shell`.
            let _element = surface(model(), &tokens, &shell, &localizer);
            assert_eq!(shell.panel.surface, mode);
        }
    }
}
