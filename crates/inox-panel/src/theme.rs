//! Inox design tokens, turned into iced's theme and style closures.
//!
//! Root AGENTS.md §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! iced, meanwhile, expects a [`iced::Theme`] and a handful of `Style` structs
//! built from it. This module is the join between the two, and it is the only
//! place in the crate where a colour becomes a widget's.
//!
//! Every function here takes the tokens as an argument rather than reaching for
//! a global, so the view can hand the styles the same theme the rest of the
//! panel is drawn with — and so a test can render the same widget tree under the
//! light and the dark tokens (acceptance Q02).
//!
//! ADR 0018 explains why there is a copy of this file in every L3 component
//! instead of one shared crate: the duplication is the price of not adding a
//! same-layer Cargo dependency that would need its own ADR.

use iced::widget::{button, container};
use iced::{Border, Color, Theme};
use inox_config::{Rgba, Theme as Tokens};

/// A token colour, as iced wants it.
pub fn color(value: Rgba) -> Color {
    Color::from_rgba8(value.r, value.g, value.b, f32::from(value.a) / 255.0)
}

/// The iced theme the panel runs under.
///
/// iced needs a theme of its own for the widgets this crate does not style
/// itself, so the six colours of its palette are filled from the tokens rather
/// than left at iced's defaults.
pub fn from_tokens(tokens: &Tokens) -> Theme {
    Theme::custom(tokens.name.clone(), palette(tokens))
}

/// The palette iced's own widgets fall back to.
pub fn palette(tokens: &Tokens) -> iced::theme::Palette {
    iced::theme::Palette {
        background: color(tokens.colors.background),
        text: color(tokens.colors.text),
        primary: color(tokens.colors.accent),
        success: color(tokens.colors.success),
        danger: color(tokens.colors.danger),
        warning: color(tokens.colors.warning),
    }
}

/// The panel's text size, in logical pixels.
///
/// `inox-config`'s typography tokens carry a family and the user's scale but no
/// size, so the panel derives one from its own height — half the strip, times
/// the user's scale. A user who doubles the scale gets text twice as tall, and
/// the geometry in [`crate::view`] is what keeps the result inside the strip
/// (acceptance Q02).
pub fn font_size(tokens: &Tokens, height: u16) -> f32 {
    f32::from(height) / 2.0 * tokens.typography.scale
}

/// Default text colour.
pub fn text(tokens: &Tokens) -> Color {
    color(tokens.colors.text)
}

/// Colour of secondary information, such as the clock or a minimized label.
pub fn muted_text(tokens: &Tokens) -> Color {
    color(tokens.colors.text_muted)
}

/// Colour for something the user should look at: an item asking for attention,
/// or a service that is not available.
pub fn warning_text(tokens: &Tokens) -> Color {
    color(tokens.colors.warning)
}

/// The corner radius of a control.
pub fn radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.sm).into()
}

/// The panel strip itself.
pub fn panel(tokens: &Tokens) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// How a task button is emphasized.
///
/// The interaction status iced reports ([`button::Status`]) says what the pointer
/// is doing; this says what the *window* is doing, which the panel draws as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEmphasis {
    /// One of the group's windows is the focused one.
    Active,
    /// The group has a window on screen, and it is not the focused one.
    Idle,
    /// Every window of the group is minimized.
    Minimized,
}

/// A task button, in its emphasis, whether the panel's own navigation is on it,
/// and its interaction status.
///
/// The active button is drawn on the accent colour, because the one piece of
/// state a taskbar must make obvious is which application the user is looking at.
/// The entry explicit navigation has reached is drawn on the alternate surface
/// with an accent label: a visible, unique focus that needs no border width of
/// its own, so no size has to be invented for it (Q01). It keeps that accent
/// label while the pointer rests on it, because a pointer sitting on the panel
/// is the ordinary case and the focus must not disappear under it. A disabled
/// button loses its background entirely and takes the muted text colour, so it
/// reads as unavailable without a second colour being invented for it (D05).
pub fn task_button(
    tokens: &Tokens,
    emphasis: TaskEmphasis,
    focused: bool,
    status: button::Status,
) -> button::Style {
    let (background, foreground) = match status {
        button::Status::Disabled => (None, muted_text(tokens)),
        button::Status::Pressed => (
            Some(color(tokens.colors.accent).into()),
            color(tokens.colors.on_accent),
        ),
        button::Status::Hovered if focused => (
            Some(color(tokens.colors.surface_variant).into()),
            color(tokens.colors.accent),
        ),
        button::Status::Hovered => (
            Some(color(tokens.colors.surface_variant).into()),
            text(tokens),
        ),
        button::Status::Active if focused => (
            Some(color(tokens.colors.surface_variant).into()),
            color(tokens.colors.accent),
        ),
        button::Status::Active => match emphasis {
            TaskEmphasis::Active => (
                Some(color(tokens.colors.accent).into()),
                color(tokens.colors.on_accent),
            ),
            TaskEmphasis::Idle => (None, text(tokens)),
            TaskEmphasis::Minimized => (None, muted_text(tokens)),
        },
    };

    button::Style {
        background,
        text_color: foreground,
        border: Border {
            radius: radius(tokens),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::theme::Base;
    use inox_config::{BUILTIN_DARK, BUILTIN_LIGHT};

    #[test]
    fn a_token_colour_keeps_its_channels() {
        let colour = color(Rgba {
            r: 0x23,
            g: 0x26,
            b: 0x2e,
            a: 0xff,
        });

        assert!((colour.r - 0x23 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.g - 0x26 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.b - 0x2e as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.a - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_iced_theme_is_named_after_the_tokens_it_came_from() {
        let light = from_tokens(&Tokens::builtin_light());
        let dark = from_tokens(&Tokens::builtin_dark());

        assert_eq!(light.name(), BUILTIN_LIGHT);
        assert_eq!(dark.name(), BUILTIN_DARK);
        assert_ne!(light.palette().background, dark.palette().background);
    }

    #[test]
    fn the_panel_background_is_the_surface_token() {
        let tokens = Tokens::builtin_dark();
        let style = panel(&tokens);

        assert_eq!(style.background, Some(color(tokens.colors.surface).into()));
        assert_eq!(style.text_color, Some(text(&tokens)));
    }

    #[test]
    fn an_active_button_is_drawn_on_the_accent_colour() {
        let tokens = Tokens::builtin_dark();
        let active = task_button(&tokens, TaskEmphasis::Active, false, button::Status::Active);
        let idle = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Active);

        assert_eq!(active.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(active.text_color, color(tokens.colors.on_accent));
        assert_eq!(idle.background, None);
        assert_ne!(active.text_color, idle.text_color);
    }

    #[test]
    fn a_disabled_button_looks_different_from_an_enabled_one() {
        let tokens = Tokens::builtin_dark();
        let enabled = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Active);
        let disabled = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Disabled);

        assert_ne!(enabled.text_color, disabled.text_color);
        assert_eq!(disabled.background, None);
        assert_eq!(disabled.text_color, muted_text(&tokens));
    }

    #[test]
    fn a_minimized_button_is_muted() {
        let tokens = Tokens::builtin_dark();
        let minimized = task_button(
            &tokens,
            TaskEmphasis::Minimized,
            false,
            button::Status::Active,
        );

        assert_eq!(minimized.text_color, muted_text(&tokens));
        assert_eq!(minimized.background, None);
    }

    #[test]
    fn the_entry_navigation_reached_is_visibly_different() {
        let tokens = Tokens::builtin_dark();
        let focused = task_button(&tokens, TaskEmphasis::Idle, true, button::Status::Active);
        let idle = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Active);

        assert_ne!(focused.background, idle.background);
        assert_eq!(focused.text_color, color(tokens.colors.accent));
    }

    #[test]
    fn a_hovered_entry_keeps_its_navigation_focus_visible() {
        let tokens = Tokens::builtin_dark();
        let focused = task_button(&tokens, TaskEmphasis::Idle, true, button::Status::Hovered);
        let hovered = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Hovered);
        let plain = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Active);

        // The pointer rests on a panel all the time: the entry the user reached
        // with explicit navigation has to stay readable there (Q01/D01).
        assert_ne!(focused.text_color, hovered.text_color);
        assert_eq!(focused.text_color, color(tokens.colors.accent));
        assert_eq!(focused.background, hovered.background);
        assert_ne!(hovered.background, plain.background);
    }

    #[test]
    fn a_button_keeps_the_token_radius() {
        let tokens = Tokens::builtin_dark();
        let style = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Active);

        assert_eq!(style.border.radius, radius(&tokens));
        assert_eq!(style.border.width, 0.0, "the panel draws no border");
    }

    #[test]
    fn the_font_size_follows_the_panel_height_and_the_user_scale() {
        let mut tokens = Tokens::builtin_dark();
        assert!((font_size(&tokens, 32) - 16.0).abs() < f32::EPSILON);

        tokens.typography.scale = 2.0;
        assert!(
            (font_size(&tokens, 32) - 32.0).abs() < f32::EPSILON,
            "font scaling is a token, not a hard-coded size"
        );
    }
}
