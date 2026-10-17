//! Design tokens, as Iced sees them.
//!
//! Every colour, gap and radius the popup stack draws with is read from
//! [`inox_config::Theme`] here (root AGENTS.md §4, PRD §9.2): no view and no
//! style function in this crate contains a colour or a pixel size. ADR 0018
//! explains why this small mapping is duplicated in each L3 component instead of
//! being shared — a shared UI crate would be a new same-layer dependency, which
//! needs an ADR of its own.
//!
//! The one thing the token set does not carry is an absolute type size: it
//! carries a *scale*, which is what a user's font-size preference moves. So the
//! text size here is Iced's own default multiplied by that scale, and a theme
//! asking for 200 % of the text gets it without this file naming a pixel.

use iced::widget::{button, container};
use iced::{Border, Color, Font};
use inox_config::{Rgba, Theme};

use crate::markup::Emphasis;
use crate::protocol::Urgency;

/// The width of a hairline border, in logical pixels.
///
/// The token set carries a border *colour* and no border width; one logical
/// pixel is the hairline the tokens are written against, including under
/// fractional scaling.
const BORDER_WIDTH: f32 = 1.0;

/// A token colour, as Iced wants it.
pub fn color(rgba: Rgba) -> Color {
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, f32::from(rgba.a) / 255.0)
}

/// The Iced theme the stack is drawn under.
///
/// Iced needs a theme of its own for the widgets this crate does not style
/// itself, so the six colours of its palette are filled from the tokens rather
/// than left at Iced's defaults.
pub fn iced_theme(tokens: &Theme) -> iced::Theme {
    iced::Theme::custom(tokens.name.clone(), palette(tokens))
}

/// The palette Iced's own widgets fall back to.
pub fn palette(tokens: &Theme) -> iced::theme::Palette {
    iced::theme::Palette {
        background: color(tokens.colors.background),
        text: color(tokens.colors.text),
        primary: color(tokens.colors.accent),
        success: color(tokens.colors.success),
        danger: color(tokens.colors.danger),
        warning: color(tokens.colors.warning),
    }
}

/// The text size a theme asks for.
pub fn text_size(tokens: &Theme) -> f32 {
    iced::Settings::default().default_text_size.0 * tokens.typography.scale
}

/// Default text colour.
pub fn text(tokens: &Theme) -> Color {
    color(tokens.colors.text)
}

/// Colour of secondary information: the application name, the close button.
pub fn muted_text(tokens: &Theme) -> Color {
    color(tokens.colors.text_muted)
}

/// The colour a link is drawn in.
pub fn link(tokens: &Theme) -> Color {
    color(tokens.colors.accent)
}

/// How urgent a notification is, in the token set's colours.
///
/// Low urgency is muted, normal text is ordinary, and critical is the danger
/// colour — the one colour a user's eye is already trained on.
pub fn urgency_color(tokens: &Theme, urgency: Urgency) -> Color {
    match urgency {
        Urgency::Low => muted_text(tokens),
        Urgency::Normal => text(tokens),
        Urgency::Critical => color(tokens.colors.danger),
    }
}

/// The corner radius of a popup or a control.
pub fn radius(tokens: &Theme) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// The font one body span is drawn with.
///
/// `<b>` and `<i>` are weight and style rather than size or colour, so they are
/// the two properties of a font the markup can move; `<u>` is an underline,
/// which Iced 0.14's text cannot draw (a span has no underline decoration), and
/// the view model still carries it.
pub fn span_font(emphasis: Emphasis) -> Font {
    Font {
        weight: if emphasis.bold {
            iced::font::Weight::Bold
        } else {
            iced::font::Weight::Normal
        },
        style: if emphasis.italic {
            iced::font::Style::Italic
        } else {
            iced::font::Style::Normal
        },
        ..Font::DEFAULT
    }
}

/// A hairline border in the theme's own border colour.
fn border(tokens: &Theme) -> Border {
    Border {
        color: color(tokens.colors.border),
        width: BORDER_WIDTH,
        radius: radius(tokens),
    }
}

/// The surface the whole stack is drawn on: the window's own background.
pub fn window(tokens: &Theme) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.background).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// One popup.
///
/// The body is the surface token, and the border is the theme's own — except
/// for a critical notification, which takes the danger colour as its border so
/// that it is distinguishable at a glance without a second background colour
/// being invented for it.
pub fn popup(tokens: &Theme, urgency: Urgency) -> container::Style {
    let border = if urgency == Urgency::Critical {
        Border {
            color: color(tokens.colors.danger),
            ..border(tokens)
        }
    } else {
        border(tokens)
    };

    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        border,
        ..container::Style::default()
    }
}

/// The space an application's icon will occupy.
///
/// This daemon does not decode icons yet (the `icon-static` capability is
/// deliberately not claimed, see [`crate::protocol`]), so what is drawn is a
/// neutral swatch of the configured size in the theme's alternate surface —
/// which is what a notification with a missing icon looks like on any desktop,
/// and what keeps the layout from jumping when the icon arrives.
pub fn icon_placeholder(tokens: &Theme) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface_variant).into()),
        border: Border {
            radius: f32::from(tokens.radii.sm).into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The daemon's own line: the empty, muted and overflow notices.
pub fn notice(tokens: &Theme) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface_variant).into()),
        text_color: Some(muted_text(tokens)),
        border: Border {
            radius: radius(tokens),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The button that closes one popup.
pub fn close_button(tokens: &Theme, status: button::Status) -> button::Style {
    let (background, foreground) = match status {
        button::Status::Disabled => (None, muted_text(tokens)),
        button::Status::Hovered | button::Status::Pressed => (
            Some(color(tokens.colors.surface_variant).into()),
            text(tokens),
        ),
        button::Status::Active => (None, muted_text(tokens)),
    };

    button::Style {
        background,
        text_color: foreground,
        border: Border {
            radius: f32::from(tokens.radii.sm).into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

/// A button that invokes one of a notification's actions.
///
/// A disabled action keeps the muted text colour and loses its background, so it
/// reads as unavailable rather than as a button that does nothing (D05).
pub fn action_button(tokens: &Theme, status: button::Status) -> button::Style {
    let (background, foreground, edge) = match status {
        // A disabled action loses the accent colour entirely — label and edge
        // alike — so that nothing about it still looks like something the user
        // could press (D05).
        button::Status::Disabled => (None, muted_text(tokens), color(tokens.colors.border)),
        button::Status::Pressed => (
            Some(color(tokens.colors.accent).into()),
            color(tokens.colors.on_accent),
            color(tokens.colors.accent),
        ),
        button::Status::Hovered => (
            Some(color(tokens.colors.surface_variant).into()),
            color(tokens.colors.accent),
            color(tokens.colors.accent),
        ),
        button::Status::Active => (
            None,
            color(tokens.colors.accent),
            color(tokens.colors.accent),
        ),
    };

    button::Style {
        background,
        text_color: foreground,
        border: Border {
            color: edge,
            width: BORDER_WIDTH,
            radius: f32::from(tokens.radii.sm).into(),
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
            a: 0x80,
        });

        assert!((colour.r - 0x23 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.a - 0x80 as f32 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_iced_theme_is_named_after_the_tokens_it_came_from() {
        let light = iced_theme(&Theme::builtin_light());
        let dark = iced_theme(&Theme::builtin_dark());

        assert_eq!(light.name(), BUILTIN_LIGHT);
        assert_eq!(dark.name(), BUILTIN_DARK);
        assert_ne!(light.palette().background, dark.palette().background);
        assert_eq!(
            dark.palette().primary,
            color(Theme::builtin_dark().colors.accent)
        );
    }

    #[test]
    fn the_text_size_follows_the_user_scale() {
        let mut tokens = Theme::builtin_dark();
        let base = text_size(&tokens);

        tokens.typography.scale = 2.0;

        assert!((text_size(&tokens) - base * 2.0).abs() < f32::EPSILON);
        assert!(base > 0.0);
    }

    #[test]
    fn every_urgency_has_a_colour_of_its_own() {
        let tokens = Theme::builtin_dark();

        let low = urgency_color(&tokens, Urgency::Low);
        let normal = urgency_color(&tokens, Urgency::Normal);
        let critical = urgency_color(&tokens, Urgency::Critical);

        assert_eq!(normal, text(&tokens));
        assert_eq!(low, muted_text(&tokens));
        assert_eq!(critical, color(tokens.colors.danger));
        assert_ne!(low, normal);
        assert_ne!(critical, normal);
    }

    #[test]
    fn a_popup_is_drawn_on_the_surface_token() {
        let tokens = Theme::builtin_dark();
        let style = popup(&tokens, Urgency::Normal);

        assert_eq!(style.background, Some(color(tokens.colors.surface).into()));
        assert_eq!(style.border.color, color(tokens.colors.border));
        assert_eq!(style.border.radius, radius(&tokens));
        assert!((style.border.width - BORDER_WIDTH).abs() < f32::EPSILON);
    }

    #[test]
    fn a_critical_popup_is_edged_in_the_danger_colour() {
        let tokens = Theme::builtin_dark();
        let normal = popup(&tokens, Urgency::Normal);
        let critical = popup(&tokens, Urgency::Critical);

        assert_eq!(critical.border.color, color(tokens.colors.danger));
        assert_ne!(critical.border.color, normal.border.color);
        assert_eq!(
            critical.background, normal.background,
            "the danger colour is an edge, not a second background"
        );
    }

    #[test]
    fn a_disabled_action_looks_different_from_one_that_can_be_used() {
        let tokens = Theme::builtin_dark();
        let enabled = action_button(&tokens, button::Status::Active);
        let disabled = action_button(&tokens, button::Status::Disabled);
        let accent = color(tokens.colors.accent);

        assert_ne!(enabled.text_color, disabled.text_color);
        assert_eq!(disabled.text_color, muted_text(&tokens));
        assert_eq!(disabled.background, None);
        assert_eq!(enabled.text_color, link(&tokens));
        assert_ne!(
            disabled.border.color, accent,
            "nothing about a disabled action is accent-coloured"
        );
        assert_eq!(disabled.border.color, color(tokens.colors.border));
    }

    #[test]
    fn a_pressed_action_is_drawn_on_the_accent_colour() {
        let tokens = Theme::builtin_dark();
        let pressed = action_button(&tokens, button::Status::Pressed);

        assert_eq!(pressed.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(pressed.text_color, color(tokens.colors.on_accent));
    }

    #[test]
    fn a_hovered_close_button_is_visibly_different() {
        let tokens = Theme::builtin_dark();
        let idle = close_button(&tokens, button::Status::Active);
        let hovered = close_button(&tokens, button::Status::Hovered);

        assert_ne!(idle.background, hovered.background);
        assert_ne!(idle.text_color, hovered.text_color);
    }

    #[test]
    fn the_window_wears_the_background_token() {
        let tokens = Theme::builtin_dark();
        let style = window(&tokens);

        assert_eq!(
            style.background,
            Some(color(tokens.colors.background).into())
        );
    }

    #[test]
    fn an_icon_placeholder_uses_the_alternate_surface() {
        let tokens = Theme::builtin_dark();
        let style = icon_placeholder(&tokens);

        assert_eq!(
            style.background,
            Some(color(tokens.colors.surface_variant).into())
        );
    }

    #[test]
    fn a_notice_is_muted_and_on_the_alternate_surface() {
        let tokens = Theme::builtin_dark();
        let style = notice(&tokens);

        assert_eq!(style.text_color, Some(muted_text(&tokens)));
        assert_eq!(
            style.background,
            Some(color(tokens.colors.surface_variant).into())
        );
    }

    #[test]
    fn markup_moves_the_weight_and_the_style_of_a_span() {
        assert_eq!(span_font(Emphasis::PLAIN), Font::DEFAULT);
        assert_eq!(
            span_font(Emphasis {
                bold: true,
                ..Emphasis::PLAIN
            })
            .weight,
            iced::font::Weight::Bold
        );
        assert_eq!(
            span_font(Emphasis {
                italic: true,
                ..Emphasis::PLAIN
            })
            .style,
            iced::font::Style::Italic
        );
        assert_eq!(
            span_font(Emphasis {
                bold: true,
                italic: true,
                underline: true
            }),
            Font {
                weight: iced::font::Weight::Bold,
                style: iced::font::Style::Italic,
                ..Font::DEFAULT
            },
            "an underline is carried by the model and not drawn by iced 0.14"
        );
    }
}
