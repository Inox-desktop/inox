//! Design tokens, as Iced sees them.
//!
//! Every colour, size, radius and gap in the widget tree comes from here, and
//! every one of them comes from [`inox_config::Theme`] — no view writes a literal
//! colour or pixel size (root `AGENTS.md` §4, PRD §9.2).
//!
//! ADR 0018 accepts one small copy of this mapping per L3 component instead of a
//! shared UI crate, because a shared crate would be a new same-layer dependency
//! and therefore an ADR. The cost is that this file is the place a token change
//! has to be applied seven times; the ADR's review condition says to revisit
//! that when it becomes a real source of defects, not before.
//!
//! # The two literals in this file, and why they are here
//!
//! `inox-config`'s token set fixes a *relative* type scale
//! (`typography.scale`, which is what a user's font-size preference moves), a
//! family, a spacing scale, radii and motion durations — but no absolute type
//! size. A component therefore has to name its base sizes once, which is what
//! the constants below do, and multiply them by the token. They are in one place,
//! next to the note that they should move into `inox-config` when a type-scale
//! token exists to replace them.

use iced::widget::{button, container};
use iced::{Background, Border, Color, Font};
use inox_config::{Rgba, Theme};

/// The base body text size, in logical pixels.
const BODY_TEXT: f32 = 14.0;

/// The base heading text size, in logical pixels.
const HEADING_TEXT: f32 = 18.0;

/// The base size of secondary text, in logical pixels.
const CAPTION_TEXT: f32 = 12.0;

/// The width of a hairline border, in logical pixels.
///
/// The token set carries a border *colour* and no border width; one logical pixel
/// is the hairline the tokens are written against, including under fractional
/// scaling.
const BORDER_WIDTH: f32 = 1.0;

/// Takes a token colour into Iced's own type.
#[must_use]
pub fn color(rgba: Rgba) -> Color {
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, f32::from(rgba.a) / 255.0)
}

/// The Iced theme for a token set.
///
/// Iced's built-in themes are not used: their palettes are somebody else's
/// choices, and consuming a token set means the widgets that ask iced for a
/// colour get an Inox one.
#[must_use]
pub fn iced_theme(tokens: &Theme) -> iced::Theme {
    iced::Theme::custom(
        tokens.name.clone(),
        iced::theme::Palette {
            background: color(tokens.colors.background),
            text: color(tokens.colors.text),
            primary: color(tokens.colors.accent),
            success: color(tokens.colors.success),
            danger: color(tokens.colors.danger),
            warning: color(tokens.colors.warning),
        },
    )
}

/// The font the token set asks for.
///
/// Iced can only *name* a family that is known at compile time
/// ([`iced::font::Family::Name`] borrows a `&'static str`), while the token is an
/// owned string read from a theme file. What this mapping can act on is the
/// token's kind, so a family whose name says `mono` selects the monospace stack
/// and everything else the sans-serif one; the family the token names is resolved
/// by the font stack underneath, which is also what happens for a web page.
#[must_use]
pub fn font(tokens: &Theme) -> Font {
    if tokens
        .typography
        .font_family
        .to_ascii_lowercase()
        .contains("mono")
        || tokens
            .typography
            .font_mono_family
            .eq_ignore_ascii_case(&tokens.typography.font_family)
    {
        Font::MONOSPACE
    } else {
        Font::DEFAULT
    }
}

/// The body text size, with the user's font-size preference applied.
#[must_use]
pub fn body_size(tokens: &Theme) -> f32 {
    BODY_TEXT * tokens.typography.scale
}

/// The heading text size, with the user's font-size preference applied.
#[must_use]
pub fn heading_size(tokens: &Theme) -> f32 {
    HEADING_TEXT * tokens.typography.scale
}

/// The secondary text size, with the user's font-size preference applied.
#[must_use]
pub fn caption_size(tokens: &Theme) -> f32 {
    CAPTION_TEXT * tokens.typography.scale
}

/// The gap between two pieces of a control.
#[must_use]
pub fn gap(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.sm)
}

/// The gap between controls, and their inner padding.
#[must_use]
pub fn padding(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The margin between the interface and the edge of the surface.
#[must_use]
pub fn margin(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.lg)
}

/// The corner radius of a panel.
#[must_use]
pub fn radius(tokens: &Theme) -> f32 {
    f32::from(tokens.radii.md)
}

/// The style of the wallpaper surface itself: the theme's background colour.
#[must_use]
pub fn background(tokens: &Theme) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.background).into()),
        text_color: Some(color(tokens.colors.text)),
        ..container::Style::default()
    }
}

/// The style of a panel drawn over the wallpaper.
#[must_use]
pub fn panel(tokens: &Theme) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(color(tokens.colors.text)),
        border: Border {
            color: color(tokens.colors.border),
            width: BORDER_WIDTH,
            radius: radius(tokens).into(),
        },
        ..container::Style::default()
    }
}

/// The style of a button, in one of its four states.
///
/// A disabled button is a token colour of its own rather than a faded copy of the
/// enabled one: on a wallpaper, which may be any colour at all, "faded" is not
/// something a drawing can be relied on to show.
#[must_use]
pub fn button_style(tokens: &Theme, status: button::Status) -> button::Style {
    let (background, text) = match status {
        button::Status::Active => (tokens.colors.accent, tokens.colors.on_accent),
        button::Status::Hovered => (tokens.colors.accent, tokens.colors.on_accent),
        button::Status::Pressed => (tokens.colors.accent, tokens.colors.on_accent),
        button::Status::Disabled => (tokens.colors.surface_variant, tokens.colors.text_muted),
    };

    button::Style {
        background: Some(color(background).into()),
        text_color: color(text),
        border: Border {
            color: color(tokens.colors.border),
            width: BORDER_WIDTH,
            radius: radius(tokens).into(),
        },
        ..button::Style::default()
    }
}

/// The colour of secondary text.
#[must_use]
pub fn muted(tokens: &Theme) -> Color {
    color(tokens.colors.text_muted)
}

/// The colour of a warning the user has to read.
#[must_use]
pub fn warning(tokens: &Theme) -> Color {
    color(tokens.colors.warning)
}

/// The colour of ordinary text.
#[must_use]
pub fn text(tokens: &Theme) -> Color {
    color(tokens.colors.text)
}

/// The colour of a surface drawn over the wallpaper.
#[must_use]
pub fn surface(tokens: &Theme) -> Background {
    color(tokens.colors.surface).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::{Appearance, Theme as Tokens};

    fn light() -> Tokens {
        Tokens::builtin_light()
    }

    fn dark() -> Tokens {
        Tokens::builtin_dark()
    }

    #[test]
    fn a_token_colour_becomes_the_same_colour_in_iced() {
        let tokens = light();
        let converted = color(tokens.colors.accent);

        assert_eq!(
            converted.r,
            0.0_f32.max(f32::from(tokens.colors.accent.r) / 255.0)
        );
        assert_eq!(
            converted,
            Color::from_rgba8(
                tokens.colors.accent.r,
                tokens.colors.accent.g,
                tokens.colors.accent.b,
                f32::from(tokens.colors.accent.a) / 255.0
            )
        );
    }

    #[test]
    fn the_iced_theme_is_built_from_the_token_palette() {
        let tokens = dark();
        let theme = iced_theme(&tokens);
        let palette = theme.palette();

        assert_eq!(palette.background, color(tokens.colors.background));
        assert_eq!(palette.text, color(tokens.colors.text));
        assert_eq!(palette.primary, color(tokens.colors.accent));
        assert_eq!(palette.danger, color(tokens.colors.danger));
        assert_eq!(palette.success, color(tokens.colors.success));
        assert_eq!(palette.warning, color(tokens.colors.warning));
        assert_eq!(theme.to_string(), tokens.name);
    }

    #[test]
    fn the_background_style_is_the_background_token() {
        let tokens = light();
        let style = background(&tokens);

        assert_eq!(
            style.background,
            Some(Background::Color(color(tokens.colors.background)))
        );
        assert_eq!(style.text_color, Some(color(tokens.colors.text)));
        assert_eq!(
            style.border,
            Border::default(),
            "the wallpaper has no border"
        );
    }

    #[test]
    fn the_panel_style_uses_the_surface_border_and_radius_tokens() {
        let tokens = dark();
        let style = panel(&tokens);

        assert_eq!(
            style.background,
            Some(Background::Color(color(tokens.colors.surface)))
        );
        assert_eq!(style.border.color, color(tokens.colors.border));
        assert_eq!(style.border.width, BORDER_WIDTH);
        assert_eq!(style.border.radius, radius(&tokens).into());
    }

    #[test]
    fn a_disabled_button_is_not_drawn_like_an_enabled_one() {
        let tokens = light();
        let enabled = button_style(&tokens, button::Status::Active);
        let disabled = button_style(&tokens, button::Status::Disabled);

        assert_ne!(enabled.background, disabled.background);
        assert_ne!(enabled.text_color, disabled.text_color);
        assert_eq!(
            disabled.background,
            Some(Background::Color(color(tokens.colors.surface_variant)))
        );
        assert_eq!(disabled.text_color, color(tokens.colors.text_muted));
    }

    #[test]
    fn every_state_of_the_button_is_drawn_from_tokens() {
        let tokens = dark();
        let allowed = [
            color(tokens.colors.accent),
            color(tokens.colors.surface_variant),
        ];

        for status in [
            button::Status::Active,
            button::Status::Hovered,
            button::Status::Pressed,
            button::Status::Disabled,
        ] {
            let style = button_style(&tokens, status);
            let background = match style.background {
                Some(Background::Color(color)) => color,
                other => panic!("expected a colour, got {other:?}"),
            };

            assert!(
                allowed.contains(&background),
                "{status:?} is {background:?}"
            );
        }
    }

    #[test]
    fn the_type_scale_token_moves_every_text_size() {
        let mut tokens = light();
        let before = (
            body_size(&tokens),
            heading_size(&tokens),
            caption_size(&tokens),
        );

        tokens.typography.scale = 2.0;

        assert_eq!(body_size(&tokens), before.0 * 2.0);
        assert_eq!(heading_size(&tokens), before.1 * 2.0);
        assert_eq!(caption_size(&tokens), before.2 * 2.0);
        assert!(body_size(&tokens) > before.0, "200% font scaling grows");
    }

    #[test]
    fn the_spacing_and_radius_tokens_are_what_the_layout_uses() {
        let mut tokens = light();
        tokens.spacing.sm = 7;
        tokens.spacing.md = 9;
        tokens.spacing.lg = 11;
        tokens.radii.md = 13;

        assert_eq!(gap(&tokens), 7.0);
        assert_eq!(padding(&tokens), 9.0);
        assert_eq!(margin(&tokens), 11.0);
        assert_eq!(radius(&tokens), 13.0);
    }

    #[test]
    fn a_monospaced_family_selects_the_monospace_stack() {
        let mut tokens = light();
        tokens.typography.font_family = "JetBrains Mono".to_owned();

        assert_eq!(font(&tokens), Font::MONOSPACE);

        tokens.typography.font_family = "Noto Sans".to_owned();
        assert_eq!(font(&tokens), Font::DEFAULT);
    }

    #[test]
    fn the_light_and_dark_token_sets_draw_differently() {
        assert_ne!(
            background(&light()).background,
            background(&dark()).background
        );
        assert_ne!(
            text(&light()),
            text(&dark()),
            "the two appearances must not collapse into one"
        );
        assert_ne!(surface(&light()), surface(&dark()));
        assert_ne!(muted(&light()), muted(&dark()));
        assert_ne!(warning(&light()), warning(&dark()));
    }

    #[test]
    fn the_appearance_of_the_token_set_is_carried_through() {
        assert_eq!(light().appearance, Appearance::Light);
        assert_eq!(dark().appearance, Appearance::Dark);
    }
}
