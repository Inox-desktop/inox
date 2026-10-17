//! Design tokens, turned into iced's theme and style closures.
//!
//! Root AGENTS.md §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! iced, meanwhile, expects a [`iced::Theme`] and a handful of `Style` structs
//! built from it. This module is the join between the two, and the only place in
//! the crate where a colour becomes a widget's.
//!
//! A lock screen has one job for its colours: it must cover everything under it
//! and it must not be confusable with the desktop it is hiding. The card is drawn
//! on the `surface` token over the `background` token, so the whole surface is
//! opaque even where the card is not.
//!
//! Every function takes the tokens as an argument rather than reaching for a
//! global, so the view can hand the styles the same theme the rest of the screen
//! is drawn with — and so a test can render the same widget tree under the light
//! and the dark tokens (acceptance Q02).
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

/// The iced theme the lock screen runs under.
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

/// The text size a theme asks for.
///
/// The token is a *scale*; what it scales is iced's own default text size, so a
/// theme asking for 200 % text gets 200 % of what an unscaled iced window would
/// draw, and this file names no pixel size of its own (acceptance Q02).
pub fn text_size(tokens: &Tokens) -> f32 {
    iced::Settings::default().default_text_size.0 * tokens.typography.scale
}

/// The ratio secondary text is drawn at.
const SMALL_TEXT_RATIO: f32 = 0.75;

/// The size of a line of secondary text.
pub fn small_text_size(tokens: &Tokens) -> f32 {
    text_size(tokens) * SMALL_TEXT_RATIO
}

/// The ratio the mask is drawn at.
///
/// Larger than body text, because the mask is the one thing on the screen the
/// user is looking at while typing.
const MASK_TEXT_RATIO: f32 = 1.25;

/// The size the credential mask is drawn at.
pub fn mask_text_size(tokens: &Tokens) -> f32 {
    text_size(tokens) * MASK_TEXT_RATIO
}

/// Default text colour.
pub fn text(tokens: &Tokens) -> Color {
    color(tokens.colors.text)
}

/// Colour of secondary information.
pub fn muted_text(tokens: &Tokens) -> Color {
    color(tokens.colors.text_muted)
}

/// Colour of a refusal.
pub fn danger_text(tokens: &Tokens) -> Color {
    color(tokens.colors.danger)
}

/// Colour of something the user should notice but which is not their fault.
pub fn warning_text(tokens: &Tokens) -> Color {
    color(tokens.colors.warning)
}

/// The corner radius of a control.
pub fn radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// The theme's border, at the theme's radius.
fn border(tokens: &Tokens, colour: Rgba) -> Border {
    Border {
        color: color(colour),
        width: 1.0,
        radius: radius(tokens),
    }
}

/// The whole surface of the lock screen.
///
/// The `background` token, opaque and full-bleed: a lock screen that let the
/// desktop show through would be showing exactly what it exists to hide.
pub fn screen(tokens: &Tokens) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.background).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The card the prompt sits on.
pub fn card(tokens: &Tokens) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.surface).into()),
        border: border(tokens, tokens.colors.border),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The credential field.
///
/// Empty, it carries the placeholder in the muted colour; typed into, it carries
/// the mask in the text colour at a larger size. Two states, one style,
/// distinguished by what the view puts in it rather than by a second colour
/// invented for it — and the typed border is the muted colour rather than the
/// accent, so the accent belongs to exactly one thing on the screen: the unlock
/// button (which is what lets a test find it, and a person not confuse it).
pub fn field(tokens: &Tokens, empty: bool) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.background).into()),
        border: border(
            tokens,
            if empty {
                tokens.colors.border
            } else {
                tokens.colors.text_muted
            },
        ),
        text_color: Some(if empty {
            muted_text(tokens)
        } else {
            text(tokens)
        }),
        ..container::Style::default()
    }
}

/// The unlock button.
///
/// Enabled, it is the accent and the text that belongs on it. Disabled, it loses
/// its background and takes the muted colour with no border, so it reads as
/// unavailable without a second colour being invented for it (acceptance D05).
pub fn submit(
    tokens: &Tokens,
    enabled: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        if !enabled || status == button::Status::Disabled {
            return button::Style {
                background: None,
                text_color: muted_text(tokens),
                border: Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: radius(tokens),
                },
                ..button::Style::default()
            };
        }

        // The same colour in every interaction state, deliberately: what the
        // pointer does while the session is locked is exactly what the lock
        // screen is hiding, so a hover effect here would be feedback about the
        // pointer rather than about the control.
        button::Style {
            background: Some(color(tokens.colors.accent).into()),
            text_color: color(tokens.colors.on_accent),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: radius(tokens),
            },
            ..button::Style::default()
        }
    }
}

/// A secondary action, such as the retry in the unavailable state.
pub fn action(tokens: &Tokens) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| button::Style {
        background: Some(
            color(match status {
                button::Status::Hovered | button::Status::Pressed => tokens.colors.accent,
                _ => tokens.colors.surface_variant,
            })
            .into(),
        ),
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => color(tokens.colors.on_accent),
            _ => text(tokens),
        },
        border: border(tokens, tokens.colors.border),
        ..button::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::theme::Base;
    use inox_config::{BUILTIN_DARK, BUILTIN_LIGHT};

    fn tokens() -> Tokens {
        Tokens::builtin_dark()
    }

    #[test]
    fn a_token_colour_keeps_its_channels_and_its_alpha() {
        let rgba = Rgba {
            r: 0x23,
            g: 0x26,
            b: 0x2e,
            a: 0x80,
        };
        let colour = color(rgba);

        assert!((colour.r - 0x23 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.g - 0x26 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.b - 0x2e as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.a - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_iced_theme_is_named_after_the_tokens_it_came_from() {
        assert_eq!(from_tokens(&Tokens::builtin_light()).name(), BUILTIN_LIGHT);
        assert_eq!(from_tokens(&Tokens::builtin_dark()).name(), BUILTIN_DARK);
    }

    #[test]
    fn the_screen_is_fully_opaque() {
        let tokens = tokens();
        let style = screen(&tokens)(&from_tokens(&tokens));

        assert_eq!(
            style.background,
            Some(color(tokens.colors.background).into())
        );
        assert_eq!(
            color(tokens.colors.background).a,
            1.0,
            "a lock screen that is not opaque shows what it is hiding"
        );
    }

    #[test]
    fn the_card_sits_on_the_surface_token() {
        let tokens = tokens();
        let style = card(&tokens)(&from_tokens(&tokens));

        assert_eq!(style.background, Some(color(tokens.colors.surface).into()));
        assert_eq!(style.text_color, Some(text(&tokens)));
        assert_eq!(style.border.radius, radius(&tokens));
        assert_eq!(style.border.width, 1.0);
    }

    #[test]
    fn an_empty_field_and_a_typed_one_look_different() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let empty = field(&tokens, true)(&theme);
        let typed = field(&tokens, false)(&theme);

        assert_ne!(empty.text_color, typed.text_color);
        assert_ne!(empty.border.color, typed.border.color);
        assert_eq!(typed.text_color, Some(color(tokens.colors.text)));
        assert_ne!(
            typed.border.color,
            color(tokens.colors.accent),
            "the accent belongs to the unlock button alone"
        );
    }

    #[test]
    fn a_disabled_submit_is_muted_rather_than_emphasized() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let enabled = submit(&tokens, true)(&theme, button::Status::Active);
        let disabled = submit(&tokens, false)(&theme, button::Status::Active);

        assert_eq!(enabled.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(enabled.text_color, color(tokens.colors.on_accent));
        assert_eq!(disabled.background, None);
        assert_eq!(disabled.text_color, muted_text(&tokens));
        assert_ne!(enabled.text_color, disabled.text_color);
    }

    #[test]
    fn a_button_taken_away_by_iced_is_drawn_disabled_too() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let style = submit(&tokens, true)(&theme, button::Status::Disabled);

        assert_eq!(style.background, None);
        assert_eq!(style.text_color, muted_text(&tokens));
    }

    #[test]
    fn a_control_keeps_the_token_radius() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        assert_eq!(
            submit(&tokens, true)(&theme, button::Status::Active)
                .border
                .radius,
            radius(&tokens)
        );
        assert_eq!(
            action(&tokens)(&theme, button::Status::Active)
                .border
                .radius,
            radius(&tokens)
        );
    }

    #[test]
    fn the_two_builtin_themes_produce_different_palettes() {
        let light = from_tokens(&Tokens::builtin_light());
        let dark = from_tokens(&Tokens::builtin_dark());

        assert_ne!(light.palette().background, dark.palette().background);
        assert_ne!(light.palette().text, dark.palette().text);
    }

    #[test]
    fn text_sizes_follow_the_token_scale() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert!(text_size(&tokens) > 0.0, "a zero text size draws nothing");
        assert_eq!(text_size(&doubled), text_size(&tokens) * 2.0);
        assert_eq!(small_text_size(&doubled), small_text_size(&tokens) * 2.0);
        assert_eq!(mask_text_size(&doubled), mask_text_size(&tokens) * 2.0);
        assert!(small_text_size(&tokens) < text_size(&tokens));
        assert!(mask_text_size(&tokens) > text_size(&tokens));
    }

    #[test]
    fn the_submit_button_does_not_react_to_the_pointer() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let idle = submit(&tokens, true)(&theme, button::Status::Active);
        let hovered = submit(&tokens, true)(&theme, button::Status::Hovered);

        assert_eq!(idle.background, hovered.background);
        assert_eq!(idle.text_color, hovered.text_color);
    }
}
