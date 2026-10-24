//! The popup stack: the widgets, built from the view model and the tokens.
//!
//! Everything here is drawing. Which notifications are shown, in what order,
//! what each one says, how urgent it says it is, how long ago it arrived, which
//! actions are offered and **why one of them is disabled** all arrive decided in
//! a [`crate::view_model::PopupStack`]; this module turns that into Iced widgets
//! and takes every colour, distance and radius from the tokens through
//! [`crate::theme`]. The rules it is drawn against are
//! `docs/quality/ui-guidelines.md` §2 (the type roles), §3 (the spacing rhythm),
//! §4 (card geometry) and §7 (nothing may overflow the surface).
//!
//! Six layout decisions are worth naming, because they are what the acceptance
//! criteria are about:
//!
//! - **The stack is in the configured corner**, inset by the configured margin:
//!   the corner is the alignment of the surface's content, which is the same
//!   place the layer-surface anchors would put it ([`crate::placement`]).
//! - **A card is a card**: the `surface` token, a hairline edge, the `md` radius
//!   and the `lg` inset, with `lg` between two cards — so a notification is
//!   bounded, and the surface behind it shows through between them.
//! - **The hierarchy is a heading over body text over captions**: the summary is
//!   the card's one `heading`, the message is `body`, and the application name,
//!   the urgency sentence, the timestamp and the reason an action is unavailable
//!   are all `caption`.
//! - **A popup's body wraps and is bounded.** The body is a single rich-text
//!   widget that wraps at the card's content width, and the view model caps what
//!   it hands over, so a long message makes a taller card rather than a wider
//!   one and cannot push the actions out of reach (D05).
//! - **Controls never leave the card.** The actions are laid out in rows of at
//!   most [`MAX_ACTIONS_PER_ROW`], each button sharing its row's width, so even
//!   the sixteen actions a client may legally send wrap instead of overflowing
//!   (ui-guidelines §7).
//! - **A link is clickable and nothing else is.** `<a href>` spans carry the
//!   target the sanitiser accepted and produce exactly one message when clicked;
//!   every other kind of text is display-only. The sanitiser is unchanged: this
//!   module draws the spans the model carries and mints nothing.

use iced::widget::text::{Rich, Span, Wrapping};
use iced::widget::{Column, Row, Space, button, container, text};
use iced::{Element, Font, Length, Padding};
use inox_config::{DisplayConfig, NotificationPosition, Theme as Tokens};

use crate::app::Message;
use crate::markup::Span as MarkupSpan;
use crate::theme::{self, NoticeTone, TextRole};
use crate::view_model::{MAX_ACTIONS_PER_ROW, Popup, PopupStack};

/// The geometry of the popup stack, derived from the tokens and the configuration.
///
/// Every field is either a configuration value or a design token; nothing here
/// is a literal (ui-guidelines §1).
#[derive(Debug, Clone, PartialEq)]
pub struct StackGeometry {
    /// Width of one popup.
    pub width: f32,
    /// Distance between the stack and the output edges it sits against.
    pub margin: f32,
    /// Inset inside one card (ui-guidelines §4: `lg`).
    pub card_padding: f32,
    /// Inset inside one of the daemon's own lines (ui-guidelines §3: `md`).
    pub notice_padding: f32,
    /// Gap between two cards, and between a card and the overflow line (§3: `lg`).
    pub card_gap: f32,
    /// Gap between the parts of one card (§3: the default control step, `md`).
    pub section_gap: f32,
    /// Gap between two controls in one row (§3: `md`).
    pub control_gap: f32,
    /// Gap between items of one line — an icon and its text (§3: `sm`).
    pub inline_gap: f32,
    /// Size of the space an application icon occupies.
    pub icon_size: f32,
    /// Padding inside a control, as [`theme::control_inset`] derives it.
    pub control_inset: Padding,
    /// Size of a card's heading.
    pub heading_size: f32,
    /// Size of body text and control labels.
    pub body_size: f32,
    /// Size of captions: the application name, status sentences, timestamps.
    pub caption_size: f32,
}

impl StackGeometry {
    /// Derives the stack's geometry.
    pub fn new(tokens: &Tokens, display: &DisplayConfig) -> Self {
        Self {
            width: f32::from(display.width),
            margin: f32::from(display.margin),
            card_padding: theme::card_inset(tokens),
            notice_padding: theme::notice_inset(tokens),
            card_gap: theme::block_gap(tokens),
            section_gap: theme::section_gap(tokens),
            control_gap: theme::control_gap(tokens),
            inline_gap: theme::inline_gap(tokens),
            icon_size: f32::from(display.icon_size),
            control_inset: theme::control_inset(tokens),
            heading_size: theme::text_size(tokens, TextRole::Heading),
            body_size: theme::text_size(tokens, TextRole::Body),
            caption_size: theme::text_size(tokens, TextRole::Caption),
        }
    }

    /// The inset inside one card.
    pub fn card_inset(&self) -> Padding {
        Padding::from(self.card_padding)
    }

    /// The inset inside one of the daemon's own lines.
    pub fn notice_inset(&self) -> Padding {
        Padding::from(self.notice_padding)
    }

    /// The height a control needs to be a target a pointer can hit.
    ///
    /// The label's own size is the floor of its line box, so this is the least a
    /// button can be — which is what makes it the value ui-guidelines §4's
    /// `≥ spacing.xl` can be asserted against.
    pub fn control_height(&self) -> f32 {
        self.body_size + self.control_inset.top + self.control_inset.bottom
    }

    /// The room one popup needs before its content is laid out.
    pub fn unit_height(&self) -> f32 {
        2.0 * self.card_padding + self.icon_size.max(self.control_height())
    }

    /// The size a popup stack takes before its content is laid out.
    ///
    /// The window this crate runs in while there is no layer-shell host opens at
    /// the size `max_visible` popups need; a layer surface asks the compositor
    /// to size it instead ([`crate::placement`]).
    pub fn window_size(&self, display: &DisplayConfig) -> (f32, f32) {
        let visible = f32::from(display.max_visible);
        let stack = self.unit_height() * visible + self.card_gap * (visible - 1.0).max(0.0);

        (self.width + self.margin * 2.0, self.margin * 2.0 + stack)
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
        children.push(notice_view(
            overflow.message.clone(),
            NoticeTone::Caution,
            tokens,
            &geometry,
        ));
    }

    // Nothing to draw: the daemon says what it is rather than leaving a blank
    // rectangle where the popups would be.
    if children.is_empty()
        && let Some(notice) = &model.notice
    {
        children.push(notice_view(
            notice.clone(),
            NoticeTone::Quiet,
            tokens,
            &geometry,
        ));
    }

    let (horizontal, vertical) = geometry.alignment(display);

    container(
        Column::with_children(children)
            .spacing(geometry.card_gap)
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

/// One card: its header, its heading, its body, its actions and its status line.
fn popup_view<'a>(
    popup: &Popup,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    let urgency = popup.urgency;
    let mut parts: Vec<Element<'a, Message>> = vec![header(popup, tokens, geometry)];

    parts.push(
        text(popup.content.summary().to_owned())
            .size(geometry.heading_size)
            .font(Font {
                weight: iced::font::Weight::Bold,
                ..Font::DEFAULT
            })
            .color(theme::text(tokens))
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

    parts.push(status_view(popup, tokens, geometry));

    container(
        Column::with_children(parts)
            .spacing(geometry.section_gap)
            .width(Length::Fill),
    )
    .width(Length::Fixed(geometry.width))
    .padding(geometry.card_inset())
    .style(move |_theme| theme::card(tokens, urgency))
    .into()
}

/// The application name, its icon's place, and the close button.
///
/// The name is a `caption` in the muted colour — it is who is speaking, not what
/// they are saying — and the close button is the card's only other control.
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
                .size(geometry.caption_size)
                .color(theme::muted_text(tokens))
                .wrapping(Wrapping::None)
                .into(),
        );
    }

    parts.push(Space::new().width(Length::Fill).into());

    parts.push(
        button(
            text(popup.close.clone())
                .size(geometry.body_size)
                .wrapping(Wrapping::None),
        )
        .padding(geometry.control_inset)
        .on_press(Message::Dismissed { id: popup.id })
        .style(move |_theme, status| theme::close_button(tokens, status))
        .into(),
    );

    Row::with_children(parts)
        .spacing(geometry.inline_gap)
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
        .size(geometry.body_size)
        .width(Length::Fill)
        .wrapping(Wrapping::Word)
        .on_link_click(move |href: String| Message::LinkFollowed { id, href })
        .into()
}

/// The urgency sentence and the timestamp: the card's status line.
///
/// ui-guidelines §5 and §6: how urgent a notification is and how old it is are
/// read as sentences, in the `caption` role, not inferred from a colour. The
/// urgency sentence carries the urgency colour; the timestamp is muted, because
/// it is the least important thing on the card.
fn status_view<'a>(
    popup: &Popup,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    let mut parts: Vec<Element<'a, Message>> = Vec::new();

    if let Some(note) = &popup.urgency_note {
        parts.push(
            text(note.clone())
                .size(geometry.caption_size)
                .font(Font {
                    weight: iced::font::Weight::Bold,
                    ..Font::DEFAULT
                })
                .color(theme::urgency_color(tokens, popup.urgency))
                .wrapping(Wrapping::None)
                .into(),
        );
    }

    parts.push(
        text(popup.age.clone())
            .size(geometry.caption_size)
            .color(theme::muted_text(tokens))
            .wrapping(Wrapping::None)
            .into(),
    );
    parts.push(Space::new().width(Length::Fill).into());

    Row::with_children(parts)
        .spacing(geometry.inline_gap)
        .align_y(iced::alignment::Vertical::Center)
        .into()
}

/// The action buttons one popup offers, and the reason any of them is unavailable.
///
/// A disabled action keeps its label, loses its tier colour and gains a line
/// saying why it cannot be used, which is what D05 asks for: an unavailable
/// action is explained to everyone, not only to a pointer that happens to hover
/// it. The reason is written once for the card, because every button of one
/// notification is disabled for the same reason.
fn actions_view<'a>(
    popup: &Popup,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    let rows: Vec<Element<'a, Message>> = popup
        .actions
        .chunks(MAX_ACTIONS_PER_ROW)
        .map(|row| {
            let buttons: Vec<Element<'a, Message>> = row
                .iter()
                .map(|action| action_button(action, popup.id, tokens, geometry))
                .collect();

            Row::with_children(buttons)
                .spacing(geometry.control_gap)
                .align_y(iced::alignment::Vertical::Center)
                .into()
        })
        .collect();

    let buttons: Element<'a, Message> = Column::with_children(rows)
        .spacing(geometry.inline_gap)
        .width(Length::Fill)
        .into();

    let Some(reason) = popup
        .actions
        .iter()
        .find_map(|action| action.disabled_reason.clone())
    else {
        return buttons;
    };

    Column::with_children([
        buttons,
        text(reason)
            .size(geometry.caption_size)
            .color(theme::muted_text(tokens))
            .wrapping(Wrapping::Word)
            .width(Length::Fill)
            .into(),
    ])
    .spacing(geometry.inline_gap)
    .width(Length::Fill)
    .into()
}

/// One action button, filling its share of the row it is drawn in.
fn action_button<'a>(
    action: &crate::view_model::ActionButton,
    id: crate::queue::NotificationId,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    let action_id = action.id.clone();
    let emphasis = action.emphasis;

    button(
        text(action.label.clone())
            .size(geometry.body_size)
            .wrapping(Wrapping::Word),
    )
    .width(Length::Fill)
    .padding(geometry.control_inset)
    .on_press_maybe(action.enabled.then_some(Message::ActionInvoked {
        id,
        action: action_id,
    }))
    .style(move |_theme, status| theme::action_button(tokens, emphasis, status))
    .into()
}

/// The daemon's own line: the empty and muted states, and the overflow sentence.
fn notice_view<'a>(
    notice: String,
    tone: NoticeTone,
    tokens: &'a Tokens,
    geometry: &StackGeometry,
) -> Element<'a, Message> {
    container(
        text(notice)
            .size(geometry.caption_size)
            .color(theme::notice_text(tokens, tone))
            .wrapping(Wrapping::Word),
    )
    .width(Length::Fixed(geometry.width))
    .padding(geometry.notice_inset())
    .style(move |_theme| theme::notice(tokens, tone))
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
    use std::time::Instant;

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
        assert!(
            (geometry.card_padding - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON,
            "a card is inset by the large step (ui-guidelines §4)"
        );
        assert!(
            (geometry.notice_padding - f32::from(tokens.spacing.md)).abs() < f32::EPSILON,
            "a status line is inset by the medium step (ui-guidelines §3)"
        );
        assert!((geometry.card_gap - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON);
        assert!((geometry.section_gap - f32::from(tokens.spacing.md)).abs() < f32::EPSILON);
        assert!((geometry.control_gap - f32::from(tokens.spacing.md)).abs() < f32::EPSILON);
        assert!((geometry.inline_gap - f32::from(tokens.spacing.sm)).abs() < f32::EPSILON);
        assert_eq!(geometry.card_inset(), Padding::from(geometry.card_padding));
        assert_eq!(
            geometry.notice_inset(),
            Padding::from(geometry.notice_padding)
        );
    }

    #[test]
    fn the_card_carries_the_four_text_roles_it_draws() {
        let tokens = Tokens::builtin_dark();
        let geometry = StackGeometry::new(&tokens, &DisplayConfig::default());

        assert!(
            (geometry.heading_size - theme::text_size(&tokens, TextRole::Heading)).abs()
                < f32::EPSILON
        );
        assert!(
            (geometry.body_size - theme::text_size(&tokens, TextRole::Body)).abs() < f32::EPSILON
        );
        assert!(
            (geometry.caption_size - theme::text_size(&tokens, TextRole::Caption)).abs()
                < f32::EPSILON
        );
        assert!(
            geometry.caption_size < geometry.body_size
                && geometry.body_size < geometry.heading_size,
            "the hierarchy has to be a hierarchy: {} / {} / {}",
            geometry.caption_size,
            geometry.body_size,
            geometry.heading_size
        );
    }

    #[test]
    fn a_control_is_at_least_the_tall_target_the_guidelines_ask_for() {
        let tokens = Tokens::builtin_dark();

        for scale in [1.0_f32, 2.0] {
            let tokens = Tokens {
                typography: inox_config::Typography {
                    scale,
                    ..tokens.typography.clone()
                },
                ..tokens.clone()
            };
            let geometry = StackGeometry::new(&tokens, &DisplayConfig::default());

            assert!(
                geometry.control_height() >= f32::from(tokens.spacing.xl),
                "at {scale}× scale a control is {} px tall, under the {} px target",
                geometry.control_height(),
                tokens.spacing.xl
            );
        }
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
            now: Instant::now(),
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
            now: Instant::now(),
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
        let style = theme::card(&tokens, Urgency::Critical);

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
