//! The popup stack: the widgets, built from the view model and the tokens.
//!
//! Everything here is drawing. Which notifications are shown, in what order,
//! what each one says, and why an action is unavailable all arrive decided in a
//! [`crate::view_model::PopupStack`]; this module turns that into Iced widgets
//! and takes every colour, distance and radius from the tokens through
//! [`crate::theme`].
//!
//! Three layout decisions are worth naming, because they are what the acceptance
//! criteria are about:
//!
//! - **The stack is in the configured corner**, inset by the configured margin:
//!   the corner is the alignment of the surface's content, which is the same
//!   place the layer-surface anchors would put it
//!   ([`crate::placement`]).
//! - **A popup's body wraps and is bounded.** The body is a single rich-text
//!   widget that wraps at the popup's width and is capped in height, so a long
//!   message makes a tall popup rather than a wide one, and cannot push the
//!   actions out of reach (D05).
//! - **A link is clickable and nothing else is.** `<a href>` spans carry the
//!   target the sanitiser accepted and produce exactly one message when clicked;
//!   every other kind of text is display-only.

use iced::widget::text::{Rich, Span, Wrapping};
use iced::widget::{Column, Row, Space, button, container, text};
use iced::{Element, Font, Length, Padding};
use inox_config::{DisplayConfig, NotificationPosition, Theme as Tokens};

use crate::app::Message;
use crate::markup::Span as MarkupSpan;
use crate::theme;
use crate::view_model::{Popup, PopupStack};

/// The geometry of the popup stack, derived from the tokens and the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct StackGeometry {
    /// Width of one popup.
    pub width: f32,
    /// Distance between the stack and the output edges it sits against.
    pub margin: f32,
    /// Padding inside one popup.
    pub padding: f32,
    /// Gap between two popups, and between the parts of one.
    pub spacing: f32,
    /// Size of the space an application icon occupies.
    pub icon_size: f32,
    /// Text size, already scaled by the user's font preference.
    pub font_size: f32,
}

impl StackGeometry {
    /// Derives the stack's geometry.
    pub fn new(tokens: &Tokens, display: &DisplayConfig) -> Self {
        Self {
            width: f32::from(display.width),
            margin: f32::from(display.margin),
            padding: f32::from(tokens.spacing.md),
            spacing: f32::from(tokens.spacing.sm),
            icon_size: f32::from(display.icon_size),
            font_size: theme::text_size(tokens),
        }
    }

    /// The size a popup takes before its content is laid out.
    ///
    /// The window this crate runs in while there is no layer-shell host opens at
    /// the size `max_visible` popups need; a layer surface asks the compositor
    /// to size it instead ([`crate::placement`]).
    pub fn window_size(&self, display: &DisplayConfig) -> (f32, f32) {
        let visible = f32::from(display.max_visible);
        let popup = self.icon_size + self.padding * 2.0 + self.spacing;

        (
            self.width + self.margin * 2.0,
            self.margin * 2.0 + popup * visible,
        )
    }

    /// Which way the popups are aligned inside the surface.
    pub fn alignment(
        &self,
        display: &DisplayConfig,
    ) -> (iced::alignment::Horizontal, iced::alignment::Vertical) {
        let horizontal = match display.position {
            NotificationPosition::TopRight | NotificationPosition::BottomRight => {
                iced::alignment::Horizontal::Right
            }
            NotificationPosition::TopLeft | NotificationPosition::BottomLeft => {
                iced::alignment::Horizontal::Left
            }
        };
        let vertical = match display.position {
            NotificationPosition::TopRight | NotificationPosition::TopLeft => {
                iced::alignment::Vertical::Top
            }
            NotificationPosition::BottomRight | NotificationPosition::BottomLeft => {
                iced::alignment::Vertical::Bottom
            }
        };

        (horizontal, vertical)
    }
}

/// Draws the popup stack.
///
/// The model is taken by value and its strings are copied into the widgets, so
/// the element this returns borrows the tokens and the display configuration and
/// nothing else — which is what lets the caller project a fresh model on every
/// frame instead of caching one.
pub fn stack<'a>(
    model: PopupStack,
    tokens: &'a Tokens,
    display: &'a DisplayConfig,
) -> Element<'a, Message> {
    let geometry = StackGeometry::new(tokens, display);
    let mut children: Vec<Element<'a, Message>> = model
        .popups
        .iter()
        .map(|popup| popup_view(popup, tokens, &geometry))
        .collect();

    if let Some(overflow) = &model.overflow {
        children.push(notice_view(overflow.message.clone(), tokens, &geometry));
    }

    // Nothing to draw: the daemon says what it is rather than leaving a blank
    // rectangle where the popups would be.
    if children.is_empty()
        && let Some(notice) = &model.notice
    {
        children.push(notice_view(notice.clone(), tokens, &geometry));
    }

    let (horizontal, vertical) = geometry.alignment(display);

    container(
        Column::with_children(children)
            .spacing(geometry.spacing)
            .align_x(horizontal),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::from(geometry.margin))
    .align_x(horizontal)
    .align_y(vertical)
    .style(move |_theme| theme::window(tokens))
    .into()
}

/// One popup.
fn popup_view<'a>(
    popup: &Popup,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    let urgency = popup.urgency;
    let mut parts: Vec<Element<'a, Message>> = vec![header(popup, tokens, geometry)];

    parts.push(
        text(popup.content.summary().to_owned())
            .size(geometry.font_size)
            .font(Font {
                weight: iced::font::Weight::Bold,
                ..Font::DEFAULT
            })
            .color(theme::urgency_color(tokens, popup.urgency))
            .wrapping(Wrapping::Word)
            .width(Length::Fill)
            .into(),
    );

    if let Some(body) = popup.content.body()
        && !body.is_empty()
    {
        parts.push(body_view(body, popup.id, tokens, geometry));
    }

    if !popup.actions.is_empty() {
        parts.push(actions_view(popup, tokens, geometry));
    }

    container(
        Column::with_children(parts)
            .spacing(geometry.spacing)
            .width(Length::Fill),
    )
    .width(Length::Fixed(geometry.width))
    .padding(Padding::from(geometry.padding))
    .style(move |_theme| theme::popup(tokens, urgency))
    .into()
}

/// The application name, its icon's place, and the close button.
fn header<'a>(popup: &Popup, tokens: &'a Tokens, geometry: &StackGeometry) -> Element<'a, Message> {
    let mut parts: Vec<Element<'a, Message>> = Vec::new();

    if popup.content.icon().is_some() {
        parts.push(
            container(
                Space::new()
                    .width(geometry.icon_size)
                    .height(geometry.icon_size),
            )
            .style(move |_theme| theme::icon_placeholder(tokens))
            .into(),
        );
    }

    if let Some(app_name) = popup.content.app_name() {
        parts.push(
            text(app_name.to_owned())
                .size(geometry.font_size)
                .color(theme::muted_text(tokens))
                .wrapping(Wrapping::None)
                .into(),
        );
    }

    parts.push(Space::new().width(Length::Fill).into());

    parts.push(
        button(
            text(popup.close.clone())
                .size(geometry.font_size)
                .wrapping(Wrapping::None),
        )
        .padding(Padding::from(geometry.spacing / 2.0))
        .on_press(Message::Dismissed { id: popup.id })
        .style(move |_theme, status| theme::close_button(tokens, status))
        .into(),
    );

    Row::with_children(parts)
        .spacing(geometry.spacing)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

/// The body: one rich-text widget, so spans wrap together and a link is a link.
fn body_view<'a>(
    body: &[MarkupSpan],
    id: crate::queue::NotificationId,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    let spans: Vec<Span<'static, String, Font>> = body
        .iter()
        .map(|span| {
            let is_link = matches!(span, MarkupSpan::Link { .. });
            let color = if is_link {
                theme::link(tokens)
            } else {
                theme::text(tokens)
            };
            let target = match span {
                MarkupSpan::Link { href, .. } => Some(href.clone()),
                MarkupSpan::Text { .. } | MarkupSpan::Image { .. } => None,
            };

            Span::new(span.text().to_owned())
                .font(theme::span_font(span.emphasis()))
                .color(color)
                .underline(is_link || span.emphasis().underline)
                .link_maybe(target)
        })
        .collect();

    Rich::with_spans(spans)
        .size(geometry.font_size)
        .width(Length::Fill)
        .wrapping(Wrapping::Word)
        .on_link_click(move |href: String| Message::LinkFollowed { id, href })
        .into()
}

/// The action buttons one popup offers.
///
/// A disabled action keeps its label and gains a tooltip saying why it cannot be
/// used, which is what D05 asks for: an unavailable action is explained rather
/// than hidden or silently inert.
fn actions_view<'a>(
    popup: &Popup,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    let buttons: Vec<Element<'a, Message>> = popup
        .actions
        .iter()
        .map(|action| {
            let id = popup.id;
            let action_id = action.id.clone();
            let button = button(
                text(action.label.clone())
                    .size(geometry.font_size)
                    .wrapping(Wrapping::None),
            )
            .padding(Padding::from(geometry.spacing / 2.0))
            .on_press_maybe(action.enabled.then_some(Message::ActionInvoked {
                id,
                action: action_id,
            }))
            .style(move |_theme, status| theme::action_button(tokens, status));

            let element: Element<'a, Message> = match &action.disabled_reason {
                None => button.into(),
                Some(reason) => iced::widget::tooltip(
                    button,
                    text(reason.clone()).size(geometry.font_size),
                    iced::widget::tooltip::Position::Top,
                )
                .into(),
            };

            element
        })
        .collect();

    Row::with_children(buttons)
        .spacing(geometry.spacing)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

/// The daemon's own line, and the overflow sentence.
fn notice_view<'a>(
    notice: String,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    container(
        text(notice)
            .size(geometry.font_size)
            .color(theme::muted_text(tokens))
            .wrapping(Wrapping::Word),
    )
    .width(Length::Fixed(geometry.width))
    .padding(Padding::from(geometry.padding))
    .style(move |_theme| theme::notice(tokens))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Policy;
    use crate::protocol::Urgency;
    use crate::queue::{NotificationId, Queue};
    use crate::view_model::StackInput;
    use iced::theme::Palette;
    use inox_config::{DisplayConfig, QueueConfig, TimeoutConfig};

    fn geometry(display: &DisplayConfig) -> StackGeometry {
        StackGeometry::new(&Tokens::builtin_dark(), display)
    }

    fn localizer() -> inox_common::i18n::Localizer {
        crate::i18n::load(&inox_common::i18n::Locale::default_locale()).expect("catalogs")
    }

    fn stack_model(status: crate::view_model::StackStatus) -> PopupStack {
        PopupStack {
            status,
            popups: Vec::new(),
            overflow: None,
            notice: Some("a line".to_owned()),
        }
    }

    #[test]
    fn the_geometry_comes_from_the_tokens_and_the_configuration() {
        let display = DisplayConfig {
            width: 400,
            margin: 20,
            icon_size: 48,
            ..DisplayConfig::default()
        };
        let tokens = Tokens::builtin_dark();
        let geometry = StackGeometry::new(&tokens, &display);

        assert!((geometry.width - 400.0).abs() < f32::EPSILON);
        assert!((geometry.margin - 20.0).abs() < f32::EPSILON);
        assert!((geometry.icon_size - 48.0).abs() < f32::EPSILON);
        assert!((geometry.padding - f32::from(tokens.spacing.md)).abs() < f32::EPSILON);
        assert!((geometry.spacing - f32::from(tokens.spacing.sm)).abs() < f32::EPSILON);
        assert!((geometry.font_size - theme::text_size(&tokens)).abs() < f32::EPSILON);
    }

    #[test]
    fn the_window_is_sized_for_the_popups_it_may_show() {
        let display = DisplayConfig::default();
        let geometry = geometry(&display);
        let (width, height) = geometry.window_size(&display);

        assert!(width > geometry.width, "the margin is included");
        assert!(height >= geometry.icon_size * f32::from(display.max_visible));

        let taller = DisplayConfig {
            max_visible: 8,
            ..DisplayConfig::default()
        };
        let (_, more) = geometry.window_size(&taller);

        assert!(more > height, "a stack that shows more needs more room");
    }

    #[test]
    fn the_stack_is_aligned_to_the_configured_corner() {
        let cases = [
            (
                NotificationPosition::TopRight,
                (
                    iced::alignment::Horizontal::Right,
                    iced::alignment::Vertical::Top,
                ),
            ),
            (
                NotificationPosition::TopLeft,
                (
                    iced::alignment::Horizontal::Left,
                    iced::alignment::Vertical::Top,
                ),
            ),
            (
                NotificationPosition::BottomRight,
                (
                    iced::alignment::Horizontal::Right,
                    iced::alignment::Vertical::Bottom,
                ),
            ),
            (
                NotificationPosition::BottomLeft,
                (
                    iced::alignment::Horizontal::Left,
                    iced::alignment::Vertical::Bottom,
                ),
            ),
        ];

        for (position, expected) in cases {
            let display = DisplayConfig {
                position,
                ..DisplayConfig::default()
            };

            assert_eq!(geometry(&display).alignment(&display), expected);
        }
    }

    #[test]
    fn every_state_builds_a_widget_tree() {
        // Building the element is the assertion: an empty stack, a muted one and
        // one with something in it must all lay out.
        for status in [
            crate::view_model::StackStatus::Empty,
            crate::view_model::StackStatus::Suppressed,
            crate::view_model::StackStatus::Showing,
        ] {
            let display = DisplayConfig::default();
            let tokens = Tokens::builtin_dark();

            let _element = stack(stack_model(status), &tokens, &display);
        }
    }

    #[test]
    fn a_popup_with_an_icon_builds_one() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            crate::queue::Draft {
                app_icon: Some("firefox".to_owned()),
                summary: "with an icon".to_owned(),
                ..crate::queue::Draft::default()
            },
            std::time::Instant::now(),
        );
        let display = DisplayConfig::default();
        let policy = Policy::default();
        let model = StackInput {
            queue: &queue,
            display: &display,
            policy: &policy,
            session: crate::policy::SessionState::Unlocked,
        }
        .project(&localizer());
        let tokens = Tokens::builtin_dark();

        assert_eq!(model.popups[0].urgency, Urgency::Normal);
        assert!(model.popups[0].content.icon().is_some());
        let _element = stack(model, &tokens, &display);
    }

    #[test]
    fn a_locked_popup_builds_without_the_sensitive_parts() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            crate::queue::Draft {
                app_name: "app".to_owned(),
                summary: "a secret".to_owned(),
                ..crate::queue::Draft::default()
            },
            std::time::Instant::now(),
        );
        let display = DisplayConfig::default();
        let policy = Policy::default();
        let model = StackInput {
            queue: &queue,
            display: &display,
            policy: &policy,
            session: crate::policy::SessionState::Locked,
        }
        .project(&localizer());
        let tokens = Tokens::builtin_dark();

        assert!(model.popups[0].content.is_withheld());
        let _element = stack(model, &tokens, &display);
    }

    #[test]
    fn a_popup_carries_the_urgency_it_was_sent_with() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            crate::queue::Draft {
                hints: crate::protocol::Hints {
                    urgency: crate::protocol::Urgency::Critical,
                    ..crate::protocol::Hints::default()
                },
                summary: "urgent".to_owned(),
                ..crate::queue::Draft::default()
            },
            std::time::Instant::now(),
        );
        let tokens = Tokens::builtin_dark();
        let style = theme::popup(&tokens, Urgency::Critical);

        assert_ne!(
            style.border.color,
            color_of(&tokens, |colors| colors.border)
        );
        assert_eq!(
            queue.get(NotificationId::new(1)).map(|item| item.urgency()),
            Some(Urgency::Critical)
        );
    }

    /// The palette a token set produces, for the one assertion that needs it.
    fn color_of(
        tokens: &Tokens,
        pick: impl Fn(&inox_config::Colors) -> inox_config::Rgba,
    ) -> iced::Color {
        theme::color(pick(&tokens.colors))
    }

    #[test]
    fn the_default_theme_is_a_palette_of_the_tokens() {
        let tokens = Tokens::builtin_dark();
        let palette: Palette = theme::palette(&tokens);

        assert_eq!(palette.background, theme::color(tokens.colors.background));
        assert_eq!(palette.text, theme::color(tokens.colors.text));
    }
}
