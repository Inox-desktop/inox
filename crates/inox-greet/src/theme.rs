//! Design tokens, turned into iced's theme and style closures.
//!
//! Root AGENTS.md §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! iced, meanwhile, expects a [`iced::Theme`] and a handful of `Style` structs
//! built from it. This module is the join between the two, and the only place in
//! the crate where a colour becomes a widget's.
//!
//! The login screen has one job for its colours: a person must be able to see
//! which control the keyboard is on (acceptance Q01). Every control therefore
//! draws its focus ring in the one token that is legible against its own fill —
//! the accent on a plain control, and the on-accent colour on the filled log-in
//! button — and nothing else on the screen uses the accent.
//!
//! ADR 0018 explains why there is a copy of this file in every component instead
//! of one shared crate: the duplication is the price of not adding a same-layer
//! Cargo dependency that would need its own ADR.

use iced::widget::{button, container};
use iced::{Border, Color, Theme};
use inox_config::{Rgba, Theme as Tokens};

/// A token colour, as iced wants it.
pub fn color(value: Rgba) -> Color {
    Color::from_rgba8(value.r, value.g, value.b, f32::from(value.a) / 255.0)
}

/// The iced theme the login screen runs under.
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

/// The ratio the headline is drawn at.
const HEADLINE_TEXT_RATIO: f32 = 1.5;

/// The size of the headline.
pub fn headline_text_size(tokens: &Tokens) -> f32 {
    text_size(tokens) * HEADLINE_TEXT_RATIO
}

/// The width of a focus ring, in logical pixels.
///
/// A width rather than a token: [`inox_config::Theme`] carries radii and spacing
/// but no line width, and a ring that were zero would be an invisible focus.
const FOCUS_RING_WIDTH: f32 = 2.0;

/// The width of an ordinary outline.
const HAIRLINE_WIDTH: f32 = 1.0;

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

/// Colour of a state that went the right way.
pub fn success_text(tokens: &Tokens) -> Color {
    color(tokens.colors.success)
}

/// The corner radius of a control.
pub fn radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// A border of `colour` at the theme's radius.
fn border(tokens: &Tokens, colour: Rgba, width: f32) -> Border {
    Border {
        color: color(colour),
        width,
        radius: radius(tokens),
    }
}

/// The focus ring, in the one token that is legible on `fill`.
fn focus_ring(tokens: &Tokens, fill: Rgba) -> Border {
    let ring = if fill == tokens.colors.accent {
        tokens.colors.on_accent
    } else {
        tokens.colors.accent
    };

    border(tokens, ring, FOCUS_RING_WIDTH)
}

/// The whole surface of the login screen.
pub fn screen(tokens: &Tokens) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.background).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The card the form sits on.
pub fn card(tokens: &Tokens) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.surface).into()),
        border: border(tokens, tokens.colors.border, HAIRLINE_WIDTH),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The identity field.
///
/// Focused it carries the focus ring, in error it carries the danger colour with
/// the error text beside it, and otherwise it is an ordinary `border` outline on
/// the `background` token. The three are exclusive, so "focused" and "wrong" are
/// never drawn as the same thing.
pub fn field(
    tokens: &Tokens,
    focused: bool,
    error: bool,
) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.background).into()),
        border: if focused {
            focus_ring(tokens, tokens.colors.background)
        } else if error {
            border(tokens, tokens.colors.danger, HAIRLINE_WIDTH)
        } else {
            border(tokens, tokens.colors.border, HAIRLINE_WIDTH)
        },
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The log-in button.
///
/// Enabled it is the accent and the text that belongs on it; disabled it loses its
/// background and takes the muted colour with no border, so it reads as
/// unavailable without a second colour being invented for it (acceptance D05).
/// Focused, the ring takes the one token legible on the fill it is drawn over.
pub fn submit(
    tokens: &Tokens,
    enabled: bool,
    focused: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        if !enabled {
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

        // The fill does not change with the pointer: a login screen that reacts
        // to a hover is telling the person something about the pointer rather
        // than about the control they are operating from the keyboard.
        let _ = status;

        button::Style {
            background: Some(color(tokens.colors.accent).into()),
            text_color: color(tokens.colors.on_accent),
            border: if focused {
                focus_ring(tokens, tokens.colors.accent)
            } else {
                Border {
                    color: Color::TRANSPARENT,
                    width: 0.0,
                    radius: radius(tokens),
                }
            },
            ..button::Style::default()
        }
    }
}

/// The clear button.
pub fn action(
    tokens: &Tokens,
    focused: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);

        button::Style {
            background: Some(
                color(if hovered {
                    tokens.colors.surface_variant
                } else {
                    tokens.colors.surface
                })
                .into(),
            ),
            text_color: text(tokens),
            border: if focused {
                focus_ring(tokens, tokens.colors.surface)
            } else {
                border(tokens, tokens.colors.border, HAIRLINE_WIDTH)
            },
            ..button::Style::default()
        }
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
    fn the_iced_theme_is_named_after_the_tokens_it_came_from() {
        assert_eq!(from_tokens(&Tokens::builtin_light()).name(), BUILTIN_LIGHT);
        assert_eq!(from_tokens(&Tokens::builtin_dark()).name(), BUILTIN_DARK);
    }

    #[test]
    fn the_screen_is_fully_opaque() {
        let tokens = tokens();
        let style = screen(&tokens)(&from_tokens(&tokens));

        assert_eq!(
            color(tokens.colors.background).a,
            1.0,
            "a login screen that is not opaque shows whatever is behind it"
        );
        assert_eq!(
            style.background,
            Some(color(tokens.colors.background).into())
        );
    }

    #[test]
    fn the_focus_ring_is_visible_on_every_control_it_is_drawn_on() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        for (control, fill) in [
            ("field", tokens.colors.background),
            ("clear", tokens.colors.surface),
            ("log in", tokens.colors.accent),
        ] {
            let ring = focus_ring(&tokens, fill);

            assert_eq!(ring.width, FOCUS_RING_WIDTH, "{control} has no ring width");
            assert_ne!(
                ring.color,
                Color::TRANSPARENT,
                "{control}'s focus ring is invisible"
            );
            assert_ne!(
                ring.color,
                color(fill),
                "{control}'s focus ring is the same colour as the control"
            );
            assert!(ring.width > 0.0);
        }

        // The two fills really do resolve to two different rings, so "the ring is
        // legible against the fill" is a rule and not a coincidence.
        assert_ne!(
            focus_ring(&tokens, tokens.colors.background).color,
            focus_ring(&tokens, tokens.colors.accent).color
        );

        let style = field(&tokens, true, false)(&theme);
        assert_eq!(style.border, focus_ring(&tokens, tokens.colors.background));
    }

    #[test]
    fn a_focused_field_is_not_drawn_as_a_wrong_one() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let focused = field(&tokens, true, false)(&theme);
        let wrong = field(&tokens, false, true)(&theme);
        let plain = field(&tokens, false, false)(&theme);

        assert_ne!(focused.border.color, wrong.border.color);
        assert_ne!(focused.border.color, plain.border.color);
        assert_ne!(wrong.border.color, plain.border.color);
        assert_eq!(wrong.border.color, color(tokens.colors.danger));
    }

    #[test]
    fn a_disabled_submit_is_muted_rather_than_emphasized() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let enabled = submit(&tokens, true, false)(&theme, button::Status::Active);
        let disabled = submit(&tokens, false, false)(&theme, button::Status::Active);

        assert_eq!(enabled.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(enabled.text_color, color(tokens.colors.on_accent));
        assert_eq!(disabled.background, None);
        assert_eq!(disabled.text_color, muted_text(&tokens));
    }

    #[test]
    fn a_disabled_control_has_no_ring_to_promise_it_can_be_used() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        // Focus implies enabled for every button in this crate; the ring a caller
        // could otherwise draw on a disabled control is deliberately absent.
        let style = submit(&tokens, false, true)(&theme, button::Status::Active);

        assert_eq!(style.border.width, 0.0);
        assert_eq!(style.background, None);
    }

    #[test]
    fn the_log_in_button_does_not_react_to_the_pointer() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let idle = submit(&tokens, true, false)(&theme, button::Status::Active);
        let hovered = submit(&tokens, true, false)(&theme, button::Status::Hovered);

        assert_eq!(idle.background, hovered.background);
        assert_eq!(idle.text_color, hovered.text_color);
        assert_eq!(idle.border.color, hovered.border.color);
    }

    #[test]
    fn a_control_keeps_the_token_radius() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        assert_eq!(
            submit(&tokens, true, false)(&theme, button::Status::Active)
                .border
                .radius,
            radius(&tokens)
        );
        assert_eq!(
            action(&tokens, false)(&theme, button::Status::Active)
                .border
                .radius,
            radius(&tokens)
        );
        assert_eq!(card(&tokens)(&theme).border.radius, radius(&tokens));
    }

    #[test]
    fn text_sizes_follow_the_token_scale() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert!(text_size(&tokens) > 0.0, "a zero text size draws nothing");
        assert_eq!(text_size(&doubled), text_size(&tokens) * 2.0);
        assert_eq!(small_text_size(&doubled), small_text_size(&tokens) * 2.0);
        assert_eq!(
            headline_text_size(&doubled),
            headline_text_size(&tokens) * 2.0
        );
        assert!(small_text_size(&tokens) < text_size(&tokens));
        assert!(headline_text_size(&tokens) > text_size(&tokens));
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
    fn the_two_builtin_themes_produce_different_palettes() {
        let light = from_tokens(&Tokens::builtin_light());
        let dark = from_tokens(&Tokens::builtin_dark());

        assert_ne!(light.palette().background, dark.palette().background);
        assert_ne!(light.palette().text, dark.palette().text);
    }
}
