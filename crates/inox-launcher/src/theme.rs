//! Design tokens to Iced, and the style closures built from them.
//!
//! Every colour, radius and distance the launcher draws with is read from
//! [`inox_config::Theme`] here (root AGENTS.md §4, PRD §9.2): no view and no
//! style function in this crate contains a colour or a pixel size. ADR 0018
//! explains why this small mapping is duplicated in each L3 component instead of
//! being shared — a shared UI crate would be a new same-layer dependency, which
//! needs an ADR of its own.
//!
//! The one thing that is not a token is the *base* text size: the theme carries a
//! typography `scale`, and the base it scales is Iced's own default text size, so
//! a theme that asks for 200 % text gets it without this file naming a pixel.

use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color};
use inox_config::Theme;
use inox_config::theme::Rgba;

/// The Iced theme a whole window is drawn with.
///
/// The seven token colours Iced's palette is built from are the ones with a
/// counterpart in the toolkit; the rest (`surface_variant`, `text_muted`,
/// `on_accent`) reach the screen through the style closures below.
pub fn iced_theme(tokens: &Theme) -> iced::Theme {
    iced::Theme::custom(
        tokens.name.clone(),
        iced::theme::Palette {
            background: colour(tokens.colors.background),
            text: colour(tokens.colors.text),
            primary: colour(tokens.colors.accent),
            success: colour(tokens.colors.success),
            danger: colour(tokens.colors.danger),
            warning: colour(tokens.colors.warning),
        },
    )
}

/// A token colour as an Iced colour.
pub fn colour(rgba: Rgba) -> Color {
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, f32::from(rgba.a) / 255.0)
}

/// The text size a theme asks for.
///
/// The token is the *scale*; what it scales is Iced's own default text size, the
/// one the rest of the toolkit uses. That keeps the multiplication honest — a
/// theme asking for 200 % text gets 200 % of what an unscaled Iced window would
/// draw — without this file naming a pixel size of its own.
pub fn text_size(tokens: &Theme) -> f32 {
    iced::Settings::default().default_text_size.0 * tokens.typography.scale
}

/// The ratio of an average glyph's advance to the font size.
///
/// A proportional sans font's average advance is about half an em. This is a
/// dimensionless ratio, not a size: the size is [`text_size`], and the ratio
/// only decides how many characters a result row's label has room for.
const AVERAGE_ADVANCE_RATIO: f32 = 0.5;

/// The width one character of the theme's text takes, on average.
pub fn average_advance(tokens: &Theme) -> f32 {
    (text_size(tokens) * AVERAGE_ADVANCE_RATIO).max(1.0)
}

/// The overlay's own background: the surface token, with the theme's border
/// radius.
pub fn overlay(tokens: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(colour(tokens.colors.surface).into()),
        border: border(tokens, tokens.colors.border),
        text_color: Some(colour(tokens.colors.text)),
        ..Default::default()
    }
}

/// One line of a surface: the panel the rows and the banner sit on.
pub fn surface(tokens: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(colour(tokens.colors.surface_variant).into()),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: (f32::from(tokens.radii.sm)).into(),
        },
        text_color: Some(colour(tokens.colors.text)),
        ..Default::default()
    }
}

/// The search field.
pub fn input(
    tokens: &Theme,
) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style + '_ {
    move |_, _| text_input::Style {
        background: colour(tokens.colors.background).into(),
        border: border(tokens, tokens.colors.border),
        icon: colour(tokens.colors.text_muted),
        placeholder: colour(tokens.colors.text_muted),
        value: colour(tokens.colors.text),
        selection: colour(tokens.colors.accent),
    }
}

/// A result row.
///
/// The selected row takes the accent's background and the `on_accent` text
/// colour, which is the token that exists for exactly this: text on an accent
/// surface. A row that cannot be launched is muted rather than coloured as an
/// error, because it is not one — the reason is written next to it.
pub fn row(
    tokens: &Theme,
    selected: bool,
    enabled: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        let background = match (selected, enabled, status) {
            (true, true, _) => Some(colour(tokens.colors.accent).into()),
            (false, true, button::Status::Hovered) => {
                Some(colour(tokens.colors.surface_variant).into())
            }
            _ => None,
        };
        button::Style {
            background,
            text_color: if selected && enabled {
                colour(tokens.colors.on_accent)
            } else if enabled {
                colour(tokens.colors.text)
            } else {
                colour(tokens.colors.text_muted)
            },
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: (f32::from(tokens.radii.sm)).into(),
            },
            ..Default::default()
        }
    }
}

/// A secondary action, such as the retry button.
pub fn action(tokens: &Theme) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| button::Style {
        background: Some(
            colour(match status {
                button::Status::Hovered | button::Status::Pressed => tokens.colors.accent,
                _ => tokens.colors.surface_variant,
            })
            .into(),
        ),
        text_color: match status {
            button::Status::Hovered | button::Status::Pressed => colour(tokens.colors.on_accent),
            _ => colour(tokens.colors.text),
        },
        border: border(tokens, tokens.colors.border),
        ..Default::default()
    }
}

/// The icon column's backdrop.
///
/// The icon theme is not loaded yet, so a row whose entry names an icon is drawn
/// with the letter of its name on the accent, and a row whose entry names none is
/// drawn muted with the placeholder. Keeping the two apart is what makes a
/// missing icon visible rather than silent (acceptance D05).
pub fn icon_background(tokens: &Theme, placeholder: bool) -> Background {
    let tint = if placeholder {
        tokens.colors.surface_variant
    } else {
        tokens.colors.accent
    };
    colour(tint).into()
}

/// The theme's border, at the theme's radius.
fn border(tokens: &Theme, colour_token: Rgba) -> Border {
    Border {
        color: colour(colour_token),
        width: 1.0,
        radius: (f32::from(tokens.radii.md)).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::Theme as Tokens;

    fn tokens() -> Tokens {
        Tokens::builtin_dark()
    }

    #[test]
    fn the_theme_carries_the_named_palette_tokens() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        assert_eq!(theme.palette().background, colour(tokens.colors.background));
        assert_eq!(theme.palette().text, colour(tokens.colors.text));
        assert_eq!(theme.palette().primary, colour(tokens.colors.accent));
        assert_eq!(theme.palette().success, colour(tokens.colors.success));
        assert_eq!(theme.palette().danger, colour(tokens.colors.danger));
        assert_eq!(theme.palette().warning, colour(tokens.colors.warning));
    }

    #[test]
    fn the_two_builtin_themes_produce_different_palettes() {
        let dark = iced_theme(&Tokens::builtin_dark());
        let light = iced_theme(&Tokens::builtin_light());
        assert_ne!(dark.palette().background, light.palette().background);
        assert_ne!(dark.palette().text, light.palette().text);
    }

    #[test]
    fn a_token_colour_keeps_its_channels_and_its_alpha() {
        let rgba = tokens().colors.accent;
        let colour = colour(rgba);
        assert_eq!(
            colour,
            Color::from_rgba8(rgba.r, rgba.g, rgba.b, f32::from(rgba.a) / 255.0)
        );
    }

    #[test]
    fn the_selected_row_is_drawn_in_the_accent_colour() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        let selected = row(&tokens, true, true)(&theme, button::Status::Active);
        assert_eq!(
            selected.background,
            Some(colour(tokens.colors.accent).into())
        );
        assert_eq!(selected.text_color, colour(tokens.colors.on_accent));

        let idle = row(&tokens, false, true)(&theme, button::Status::Active);
        assert_eq!(idle.background, None);
        assert_eq!(idle.text_color, colour(tokens.colors.text));
    }

    #[test]
    fn a_row_that_cannot_be_launched_is_muted_rather_than_highlighted() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);

        let disabled = row(&tokens, true, false)(&theme, button::Status::Active);
        assert_eq!(
            disabled.text_color,
            colour(tokens.colors.text_muted),
            "a row that cannot be launched must not look like the selection",
        );
        assert_eq!(disabled.background, None);
    }

    #[test]
    fn the_icon_placeholder_distinguishes_the_two_icon_states() {
        let tokens = tokens();
        assert_ne!(
            icon_background(&tokens, true),
            icon_background(&tokens, false)
        );
    }

    #[test]
    fn the_overlay_background_comes_from_the_surface_token() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);
        let style = overlay(&tokens)(&theme);
        assert_eq!(style.background, Some(colour(tokens.colors.surface).into()));
        assert_eq!(style.text_color, Some(colour(tokens.colors.text)));
        assert_eq!(style.border.radius, (f32::from(tokens.radii.md)).into());
    }

    #[test]
    fn the_text_size_is_the_token_scale_applied_to_iceds_default() {
        let tokens = tokens();
        assert!(text_size(&tokens) > 0.0, "a zero text size draws nothing");
        assert_eq!(
            text_size(&tokens),
            iced::Settings::default().default_text_size.0 * tokens.typography.scale
        );

        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;
        assert_eq!(text_size(&doubled), text_size(&tokens) * 2.0);
    }

    #[test]
    fn the_average_advance_follows_the_font_scale() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert!(average_advance(&tokens) >= 1.0);
        assert_eq!(average_advance(&doubled), average_advance(&tokens) * 2.0);
    }

    #[test]
    fn the_input_style_uses_the_theme_tokens() {
        let tokens = tokens();
        let theme = iced_theme(&tokens);
        let style = input(&tokens)(&theme, text_input::Status::Active);
        assert_eq!(style.placeholder, colour(tokens.colors.text_muted));
        assert_eq!(style.value, colour(tokens.colors.text));
        assert_eq!(style.selection, colour(tokens.colors.accent));
    }
}
