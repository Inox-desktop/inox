//! The preview deliberately has no credential field or authentication transport.
use crate::state::{Event, Greeter};
use iced::{
    Border, Color, Element, Length, Subscription, Task, Theme,
    widget::{button, column, container, scrollable, text, text_input},
};
use inox_common::i18n::Localizer;
use inox_config::{Rgba, Theme as Tokens};
use std::{rc::Rc, time::Instant};

/// Preview UI state with its resolved language and design tokens.
pub struct State {
    greeter: Greeter,
    localizer: Rc<Localizer>,
    tokens: Tokens,
    clear_focused: bool,
}
/// User actions; credentials are never part of a UI message.
#[derive(Debug, Clone)]
pub enum Message {
    /// Edit a public account name.
    Username(String),
    /// Clear the identity.
    Cancel,
    /// Move the visible keyboard focus between the editable name and Clear.
    FocusNext,
    /// Activate the focused control; login remains disabled.
    Activate,
}
impl State {
    /// A preview with no authenticator attached.
    pub fn new(localizer: Rc<Localizer>, tokens: Tokens) -> Self {
        Self {
            greeter: Greeter::new(false),
            localizer,
            tokens,
            clear_focused: false,
        }
    }
    /// Localized window title.
    pub fn title(&self) -> String {
        self.localizer.message("title")
    }
    /// Convert the configured semantic palette for Iced's built-in widgets.
    pub fn theme(&self) -> Theme {
        Theme::custom(
            self.tokens.name.clone(),
            iced::theme::Palette {
                background: color(self.tokens.colors.background),
                text: color(self.tokens.colors.text),
                primary: color(self.tokens.colors.accent),
                success: color(self.tokens.colors.success),
                danger: color(self.tokens.colors.danger),
                warning: color(self.tokens.colors.warning),
            },
        )
    }
}
fn color(value: Rgba) -> Color {
    Color::from_rgba8(value.r, value.g, value.b, f32::from(value.a) / 255.0)
}

/// Apply one UI action. There is no outgoing authentication request in preview mode.
pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::FocusNext => {
            state.clear_focused = !state.clear_focused;
            return if state.clear_focused {
                iced::widget::operation::focus("preview-clear")
            } else {
                iced::widget::operation::focus("username")
            };
        }
        Message::Activate if !state.clear_focused => return Task::none(),
        _ => {}
    }
    let event = match message {
        Message::Username(value) => {
            state.clear_focused = false;
            Event::Username(value)
        }
        Message::Cancel | Message::Activate => Event::Cancel,
        Message::FocusNext => return Task::none(),
    };
    let cancelled = matches!(event, Event::Cancel);
    state.greeter.apply(event, Instant::now());
    if cancelled {
        state.clear_focused = false;
        iced::widget::operation::focus("username")
    } else {
        Task::none()
    }
}
/// Keyboard navigation for the two enabled controls. Escape clears the identity.
pub fn subscription(_: &State) -> Subscription<Message> {
    iced::keyboard::listen().filter_map(|event| {
        use iced::keyboard::{Event, Key, key::Named};
        match event {
            Event::KeyPressed {
                key: Key::Named(Named::Tab),
                ..
            } => Some(Message::FocusNext),
            Event::KeyPressed {
                key: Key::Named(Named::Enter | Named::Space),
                ..
            } => Some(Message::Activate),
            Event::KeyPressed {
                key: Key::Named(Named::Escape),
                ..
            } => Some(Message::Cancel),
            _ => None,
        }
    })
}
/// Draw the same preview tree used by the render tests.
pub fn view(state: &State) -> Element<'_, Message> {
    let t = &state.tokens;
    let size = iced::Settings::default().default_text_size.0 * t.typography.scale;
    let content = column![
        text(state.localizer.message("title")).size(size),
        text(state.localizer.message("preview"))
            .size(size)
            .color(color(t.colors.warning)),
        text(state.localizer.message("user")).size(size),
        text_input(&state.localizer.message("user"), state.greeter.username())
            .id("username")
            .on_input(Message::Username)
            .style(move |theme, status| {
                let mut style = text_input::default(theme, status);
                style.background = color(t.colors.surface).into();
                style.border.radius = f32::from(t.radii.md).into();
                style.border.color = if matches!(status, text_input::Status::Focused { .. }) {
                    color(t.colors.accent)
                } else {
                    color(t.colors.border)
                };
                style.value = color(t.colors.text);
                style.placeholder = color(t.colors.text_muted);
                style.selection = color(t.colors.accent);
                style
            })
            .size(size)
            .padding(f32::from(t.spacing.sm)),
        text(state.localizer.message("blocked")).size(size),
        button(text(state.localizer.message("login")).size(size))
            .padding(f32::from(t.spacing.sm))
            .style(move |_, _| button::Style {
                background: Some(color(t.colors.surface_variant).into()),
                text_color: color(t.colors.text_muted),
                border: Border {
                    radius: f32::from(t.radii.md).into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        button(text(state.localizer.message("cancel")).size(size))
            .on_press(Message::Cancel)
            .padding(f32::from(t.spacing.sm))
            .style(move |_, status| button::Style {
                background: Some(
                    color(
                        if matches!(status, button::Status::Hovered | button::Status::Pressed)
                            || state.clear_focused
                        {
                            t.colors.accent
                        } else {
                            t.colors.surface_variant
                        }
                    )
                    .into()
                ),
                text_color: color(
                    if matches!(status, button::Status::Hovered | button::Status::Pressed)
                        || state.clear_focused
                    {
                        t.colors.on_accent
                    } else {
                        t.colors.text
                    }
                ),
                border: Border {
                    radius: f32::from(t.radii.md).into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
    ]
    .spacing(f32::from(t.spacing.md))
    .padding(f32::from(t.spacing.lg));
    container(scrollable(content))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(color(t.colors.background).into()),
            text_color: Some(color(t.colors.text)),
            ..Default::default()
        })
        .into()
}
