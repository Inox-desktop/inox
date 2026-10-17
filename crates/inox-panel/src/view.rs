//! The panel's row of widgets: tasks, tray, clock.
//!
//! Everything here is drawing. Which entries exist, what they say, and why one
//! of them is disabled all arrive already decided in a
//! [`crate::view_model::PanelViewModel`]; this module turns that into iced
//! widgets and takes every colour, size and radius from the tokens through
//! [`crate::theme`].
//!
//! Three layout decisions are worth naming, because they are what acceptance
//! D01 and D05 are about:
//!
//! - **The clock and the tray keep their space.** The task buttons sit in a
//!   flexible area and each label is bounded, so an application with a very long
//!   title cannot push a key action off the end of the strip.
//! - **The panel is one row of the configured height.** A taller panel gets
//!   taller entries, not a second row.
//! - **Nothing on the strip takes keyboard focus by itself.** The task buttons
//!   are click targets; the clock and the tray items are plain containers, and
//!   the panel's own navigation focus is drawn as a colour, never as a widget
//!   that could hold on to focus (ADR 0013, [`crate::placement`]).

use iced::widget::text::Wrapping;
use iced::widget::{Row, Space, button, container, text, tooltip};
use iced::{Element, Length, Padding};
use inox_config::{PanelConfig, PanelPosition, Theme as Tokens};

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

/// The geometry of the panel strip, derived from the tokens and the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelGeometry {
    /// Height of the strip, in logical pixels.
    pub height: f32,
    /// Padding at the left and right end of the strip.
    pub padding: f32,
    /// Gap between two entries.
    pub spacing: f32,
    /// Padding inside one entry.
    pub entry_padding: f32,
    /// Widest a label may be before the layout clips it.
    pub label_max_width: f32,
    /// Text size, already scaled by the user's font preference.
    pub font_size: f32,
    /// Which way a tooltip opens, so that it is not drawn past the screen edge.
    pub tooltip_position: tooltip::Position,
}

impl PanelGeometry {
    /// Derives the strip's geometry.
    pub fn new(tokens: &Tokens, config: &PanelConfig) -> Self {
        let height = f32::from(config.height);
        let padding = f32::from(tokens.spacing.sm);
        // Text that is taller than the strip would overflow it, so the scaled
        // size is clamped to what is left after the padding.
        let available = (height - padding * 2.0).max(1.0);

        Self {
            height,
            padding,
            spacing: f32::from(tokens.spacing.xs),
            entry_padding: f32::from(tokens.spacing.sm),
            label_max_width: f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS,
            font_size: theme::font_size(tokens, config.height).min(available),
            tooltip_position: match config.position {
                PanelPosition::Bottom => tooltip::Position::Top,
                PanelPosition::Top => tooltip::Position::Bottom,
            },
        }
    }
}

/// Draws the panel.
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
    let mut entries: Vec<Element<'a, Message>> = Vec::new();

    if let Some(notice) = &model.notice {
        entries.push(notice_view(notice, &model, tokens, &geometry));
    }

    // The task buttons share one flexible area, which is what keeps the clock
    // and the tray where they belong: iced lays non-fluid children out first, so
    // the clock asks for its width before the buttons take what is left (D05).
    entries.push(task_area(&model.tasks, model.focused, tokens, &geometry));

    for item in &model.tray {
        entries.push(tray_view(item, tokens, &geometry));
    }

    if let Some(clock) = &model.clock {
        entries.push(clock_view(
            clock.text.clone(),
            clock.tooltip.clone(),
            tokens,
            &geometry,
        ));
    }

    container(
        Row::with_children(entries)
            .spacing(geometry.spacing)
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
    .style(move |_theme| theme::panel(tokens))
    .into()
}

/// The task buttons, in one flexible area that clips what does not fit.
///
/// The area is fluid and the entries inside it are not, which is the whole
/// arrangement: the clock and the tray are laid out first and keep their width,
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
fn notice_view<'a>(
    notice: &str,
    model: &PanelViewModel,
    tokens: &'a Tokens,
    geometry: &PanelGeometry,
) -> Element<'a, Message> {
    let colour = match model.status {
        PanelStatus::Unavailable => theme::warning_text(tokens),
        PanelStatus::Starting | PanelStatus::Empty | PanelStatus::Ready => {
            theme::muted_text(tokens)
        }
    };

    let label = clamp(
        text(notice.to_owned())
            .size(geometry.font_size)
            .color(colour)
            .wrapping(Wrapping::None)
            .into(),
        geometry.label_max_width * 2.0,
    );

    match &model.retry {
        None => label,
        Some(retry) => {
            let action = button(text(retry.clone()).size(geometry.font_size))
                .height(Length::Fill)
                .padding(Padding {
                    top: 0.0,
                    right: geometry.entry_padding,
                    bottom: 0.0,
                    left: geometry.entry_padding,
                })
                .on_press(Message::Retry)
                .style(move |_theme, status| {
                    theme::task_button(tokens, TaskEmphasis::Idle, false, status)
                });

            Row::with_children([label, action.into()])
                .spacing(geometry.spacing)
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
        text(task.tooltip.clone()).size(geometry.font_size),
        geometry.tooltip_position,
    )
    .into()
}

/// One tray item, which is display-only: it shows a title and its status, and
/// takes no pointer or keyboard input.
fn tray_view<'a>(
    item: &TrayItemView,
    tokens: &'a Tokens,
    geometry: &PanelGeometry,
) -> Element<'a, Message> {
    let colour = if item.attention {
        theme::warning_text(tokens)
    } else {
        theme::text(tokens)
    };

    let label: Element<'a, Message> = clamp(
        text(item.label.clone())
            .size(geometry.font_size)
            .color(colour)
            .wrapping(Wrapping::None)
            .into(),
        geometry.label_max_width,
    );

    tooltip(
        label,
        text(item.tooltip.clone()).size(geometry.font_size),
        geometry.tooltip_position,
    )
    .into()
}

/// The clock, display-only, with the full date in its tooltip.
fn clock_view<'a>(
    clock: String,
    tooltip_text: String,
    tokens: &'a Tokens,
    geometry: &PanelGeometry,
) -> Element<'a, Message> {
    tooltip(
        text(clock)
            .size(geometry.font_size)
            .color(theme::muted_text(tokens)),
        text(tooltip_text).size(geometry.font_size),
        geometry.tooltip_position,
    )
    .into()
}

/// Bounds a label so that the rest of the strip keeps its space.
///
/// `Wrapping::None` keeps a label on one line; the maximum width is what stops
/// that line from growing without end.
fn clamp<'a>(content: Element<'a, Message>, max_width: f32) -> Element<'a, Message> {
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
            geometry.font_size <= geometry.height - geometry.padding * 2.0,
            "the text has to fit inside the strip"
        );
        assert!(geometry.font_size > 0.0);
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
}
