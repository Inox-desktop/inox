//! Design tokens, as Iced sees them.
//!
//! Every colour, size, radius and gap in the widget tree comes from here, and
//! every one of them comes from [`inox_config::Theme`] — no view writes a literal
//! colour or pixel size (root `AGENTS.md` §4, PRD §9.2). This is the only module
//! in the crate that turns a token into a colour or a style; the widget tree only
//! asks for one (`docs/quality/ui-guidelines.md` §1).
//!
//! ADR 0018 accepts one small copy of this mapping per L3 component instead of a
//! shared UI crate, because a shared crate would be a new same-layer dependency
//! and therefore an ADR. The cost is that this file is the place a token change
//! has to be applied seven times; the ADR's review condition says to revisit
//! that when it becomes a real source of defects, not before.
//!
//! # Where the numbers come from
//!
//! The token set fixes a *relative* type scale (`typography.scale`, which is what
//! a user's font-size preference moves), a family, a spacing scale, radii and
//! motion durations — but no absolute type size. The UI spec §2 settles what a
//! component does about that: a window-type surface takes its base size from
//! `spacing.lg × typography.scale`, and the four type roles are fixed multiples
//! of that base. So the only literals here are the *factors* between roles, the
//! hairline divisor, the line-height iced lays text out with, and the width of a
//! card in `spacing.xl` units — each named once, in this block, with the rule it
//! implements.

use iced::font::Weight;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Font, Padding};
use inox_config::{Rgba, Theme};

/// `title`: a window or page headline, at most one per surface.
const TITLE_FACTOR: f32 = 1.5;

/// `heading`: a card or section heading.
const HEADING_FACTOR: f32 = 1.25;

/// `body`: prose, control labels, the current value, button text.
const BODY_FACTOR: f32 = 1.0;

/// `caption`: explanations, units, state sentences, secondary metadata.
const CAPTION_FACTOR: f32 = 0.875;

/// The relative line height iced lays a paragraph out with.
///
/// Named because [`control_height`] has to reserve a line box, not just the
/// glyph size: a target sized to the text alone clips it.
const LINE_HEIGHT: f32 = 1.3;

/// A hairline is half of the smallest spacing token (UI spec §1).
const HAIRLINE_DIVISOR: f32 = 2.0;

/// The width of a card in `spacing.xl` units.
///
/// UI spec §4 bounds a control that carries a path at 12 × `spacing.xl`. A card
/// is not a control, but its content is the same kind of thing — an output name,
/// a path, one sentence — so the same bound keeps it from swallowing the preview
/// it is drawn over, and the wrapping in [`crate::view`] keeps a longer path
/// inside it instead of running off the edge.
const CARD_WIDTH_UNITS: f32 = 12.0;

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

/// Emphasis: the token family at the bold weight.
///
/// The UI spec §2 puts emphasis on the weight rather than on a larger size, and
/// asks for no more than one bold weight per screen — so this is the only weight
/// the widget tree asks for.
#[must_use]
pub fn bold(tokens: &Theme) -> Font {
    Font {
        weight: Weight::Bold,
        ..font(tokens)
    }
}

/// The size the four roles are multiples of: `spacing.lg`, scaled by the user's
/// font-size preference (UI spec §2).
#[must_use]
pub fn base_size(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.lg) * tokens.typography.scale
}

/// The `title` size: `1.5 × base` (24 at the default tokens).
#[must_use]
pub fn title_size(tokens: &Theme) -> f32 {
    base_size(tokens) * TITLE_FACTOR
}

/// The `heading` size: `1.25 × base` (20 at the default tokens).
#[must_use]
pub fn heading_size(tokens: &Theme) -> f32 {
    base_size(tokens) * HEADING_FACTOR
}

/// The `body` size: `1 × base` (16 at the default tokens).
#[must_use]
pub fn body_size(tokens: &Theme) -> f32 {
    base_size(tokens) * BODY_FACTOR
}

/// The `caption` size: `0.875 × base` (14 at the default tokens).
#[must_use]
pub fn caption_size(tokens: &Theme) -> f32 {
    base_size(tokens) * CAPTION_FACTOR
}

/// The gap inside one group: a label and its control, two adjacent lines, an
/// icon and its text (UI spec §3, `spacing.sm`).
#[must_use]
pub fn gap(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.sm)
}

/// The gap between blocks: between cards, and between a card's prose and its
/// controls (UI spec §3, `spacing.lg`).
#[must_use]
pub fn section_gap(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.lg)
}

/// The inset of a card (UI spec §4: cards are `surface` + hairline + `md` radius
/// + `lg` padding).
#[must_use]
pub fn card_padding(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.lg)
}

/// The inset of a status strip (UI spec §3, `spacing.md`).
#[must_use]
pub fn strip_padding(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The padding of a button: `sm` above and below, `md` either side (UI spec §3).
#[must_use]
pub fn control_padding(tokens: &Theme) -> Padding {
    Padding {
        top: f32::from(tokens.spacing.sm),
        bottom: f32::from(tokens.spacing.sm),
        left: f32::from(tokens.spacing.md),
        right: f32::from(tokens.spacing.md),
    }
}

/// The inset between the interface and the edge of the surface (UI spec §3:
/// page and window padding is `xl`).
#[must_use]
pub fn window_padding(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.xl)
}

/// The width of a card: twelve times `spacing.xl`, the bound UI spec §4 puts on
/// a control whose content is a path.
#[must_use]
pub fn card_width(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.xl) * CARD_WIDTH_UNITS
}

/// A hairline: `spacing.xs / 2` (UI spec §1).
#[must_use]
pub fn hairline(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.xs) / HAIRLINE_DIVISOR
}

/// The width of a focus ring: `spacing.xs` (UI spec §1, §5).
#[must_use]
pub fn focus_ring(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.xs)
}

/// The radius of an input, a button, a label or a switch.
#[must_use]
pub fn radius_sm(tokens: &Theme) -> f32 {
    f32::from(tokens.radii.sm)
}

/// The radius of a card, a group, a dialog, a menu or a notification.
#[must_use]
pub fn radius_md(tokens: &Theme) -> f32 {
    f32::from(tokens.radii.md)
}

/// The radius of a window or a full-screen surface.
#[must_use]
pub fn radius_lg(tokens: &Theme) -> f32 {
    f32::from(tokens.radii.lg)
}

/// The height of an interactive control.
///
/// UI spec §4 asks for at least `spacing.xl`, growing with the font scale so that
/// a fixed height never clips enlarged text: the label's line box plus the
/// control's own vertical padding wins whenever it is larger than `spacing.xl`.
#[must_use]
pub fn control_height(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.xl)
        .max(body_size(tokens) * LINE_HEIGHT + 2.0 * f32::from(tokens.spacing.sm))
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

/// The style of a card, and of a status strip drawn over the wallpaper.
///
/// UI spec §4: `surface` background, a hairline `border`, an `md` radius and no
/// shadow. A second surface is never stacked on the same colour: a card sits on
/// the `background` or on the wallpaper, and `surface_variant` is reserved for
/// the hovered and disabled states of a control.
#[must_use]
pub fn card(tokens: &Theme) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(color(tokens.colors.text)),
        border: Border {
            color: color(tokens.colors.border),
            width: hairline(tokens),
            radius: radius_md(tokens).into(),
        },
        ..container::Style::default()
    }
}

/// The role a button plays on its surface.
///
/// UI spec §5 keeps `accent` for one action per screen: that action is
/// [`ButtonRole::Primary`], and any further control is [`ButtonRole::Secondary`],
/// which reaches the accent colour only while it is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonRole {
    /// The one action the surface is for.
    Primary,
    /// A control beside the primary one.
    Secondary,
}

/// The style of a button, in its role and one of its four states.
///
/// Every state is drawn with token colours and a token border, and no two states
/// of the same role share a drawing (UI spec §5, §10).
///
/// The disabled primary keeps `surface_variant` as its background rather than
/// fading the accent: on a wallpaper, which may be any colour at all, "a faded
/// accent" is not something a drawing can be relied on to show. The reason a
/// control is disabled is text next to it, never colour alone (UI spec §5, §6).
#[must_use]
pub fn button_style(tokens: &Theme, role: ButtonRole, status: button::Status) -> button::Style {
    let border = |color: Color, width: f32| Border {
        color,
        width,
        radius: radius_sm(tokens).into(),
    };

    let (background, text_color, border) = match (role, status) {
        (ButtonRole::Primary, button::Status::Active) => (
            Some(color(tokens.colors.accent)),
            color(tokens.colors.on_accent),
            border(color(tokens.colors.accent), hairline(tokens)),
        ),
        (ButtonRole::Primary, button::Status::Hovered) => (
            Some(color(tokens.colors.accent)),
            color(tokens.colors.on_accent),
            border(color(tokens.colors.on_accent), hairline(tokens)),
        ),
        (ButtonRole::Primary, button::Status::Pressed) => (
            Some(color(tokens.colors.accent)),
            color(tokens.colors.on_accent),
            border(color(tokens.colors.on_accent), focus_ring(tokens)),
        ),
        (ButtonRole::Primary, button::Status::Disabled) => (
            Some(color(tokens.colors.surface_variant)),
            color(tokens.colors.text_muted),
            border(color(tokens.colors.border), hairline(tokens)),
        ),
        (ButtonRole::Secondary, button::Status::Active) => (
            None,
            color(tokens.colors.text),
            border(color(tokens.colors.border), hairline(tokens)),
        ),
        (ButtonRole::Secondary, button::Status::Hovered) => (
            Some(color(tokens.colors.surface_variant)),
            color(tokens.colors.text),
            border(color(tokens.colors.border), hairline(tokens)),
        ),
        (ButtonRole::Secondary, button::Status::Pressed) => (
            Some(color(tokens.colors.accent)),
            color(tokens.colors.on_accent),
            border(color(tokens.colors.accent), hairline(tokens)),
        ),
        (ButtonRole::Secondary, button::Status::Disabled) => (
            None,
            color(tokens.colors.text_muted),
            border(color(tokens.colors.border), hairline(tokens)),
        ),
    };

    button::Style {
        background: background.map(Background::Color),
        text_color,
        border,
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

/// The colour of a state the component has confirmed.
#[must_use]
pub fn success(tokens: &Theme) -> Color {
    color(tokens.colors.success)
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

    /// The four states iced asks a button's style for.
    const STATUSES: [button::Status; 4] = [
        button::Status::Active,
        button::Status::Hovered,
        button::Status::Pressed,
        button::Status::Disabled,
    ];

    /// What makes two drawings of a control different.
    fn drawing(style: button::Style) -> (Option<Background>, Color, Color, f32) {
        (
            style.background,
            style.text_color,
            style.border.color,
            style.border.width,
        )
    }

    #[test]
    fn a_token_colour_becomes_the_same_colour_in_iced() {
        let tokens = light();
        let converted = color(tokens.colors.accent);

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
    fn a_card_is_a_surface_with_a_hairline_and_an_md_radius() {
        // UI spec §4: `surface` + hairline `border` + `md` radius, no shadow.
        let tokens = dark();
        let style = card(&tokens);

        assert_eq!(
            style.background,
            Some(Background::Color(color(tokens.colors.surface)))
        );
        assert_eq!(style.border.color, color(tokens.colors.border));
        assert_eq!(style.border.width, hairline(&tokens));
        assert_eq!(style.border.radius, radius_md(&tokens).into());
        assert_eq!(style.shadow, iced::Shadow::default(), "no stacked shadow");
    }

    #[test]
    fn the_four_type_roles_are_the_multiples_of_the_token_base() {
        // UI spec §2: the base is `spacing.lg × scale`, the roles are 1.5 / 1.25
        // / 1 / 0.875 of it, so at the default tokens they are 24 / 20 / 16 / 14.
        let tokens = light();

        assert_eq!(base_size(&tokens), f32::from(tokens.spacing.lg));
        assert_eq!(title_size(&tokens), 24.0);
        assert_eq!(heading_size(&tokens), 20.0);
        assert_eq!(body_size(&tokens), 16.0);
        assert_eq!(caption_size(&tokens), 14.0);
        assert!(
            caption_size(&tokens) >= 0.75 * base_size(&tokens),
            "a caption never falls below 0.75 × base"
        );
        assert!(
            title_size(&tokens) > heading_size(&tokens)
                && heading_size(&tokens) > body_size(&tokens)
                && body_size(&tokens) > caption_size(&tokens),
            "the hierarchy has to be readable at a glance"
        );
    }

    #[test]
    fn the_type_scale_token_moves_every_text_size() {
        let mut tokens = light();
        let before = (
            base_size(&tokens),
            title_size(&tokens),
            heading_size(&tokens),
            body_size(&tokens),
            caption_size(&tokens),
        );

        tokens.typography.scale = 2.0;

        assert_eq!(base_size(&tokens), before.0 * 2.0);
        assert_eq!(title_size(&tokens), before.1 * 2.0);
        assert_eq!(heading_size(&tokens), before.2 * 2.0);
        assert_eq!(body_size(&tokens), before.3 * 2.0);
        assert_eq!(caption_size(&tokens), before.4 * 2.0);
        assert!(body_size(&tokens) > before.3, "200% font scaling grows");
    }

    #[test]
    fn the_spacing_tokens_are_what_the_layout_uses() {
        let mut tokens = light();
        tokens.spacing.xs = 2;
        tokens.spacing.sm = 7;
        tokens.spacing.md = 9;
        tokens.spacing.lg = 11;
        tokens.spacing.xl = 13;
        tokens.radii.sm = 3;
        tokens.radii.md = 13;
        tokens.radii.lg = 21;

        assert_eq!(gap(&tokens), 7.0, "within a group: sm");
        assert_eq!(section_gap(&tokens), 11.0, "between blocks: lg");
        assert_eq!(card_padding(&tokens), 11.0, "card inset: lg");
        assert_eq!(strip_padding(&tokens), 9.0, "strip inset: md");
        assert_eq!(window_padding(&tokens), 13.0, "window inset: xl");
        assert_eq!(
            control_padding(&tokens),
            Padding {
                top: 7.0,
                bottom: 7.0,
                left: 9.0,
                right: 9.0
            },
            "a control is padded sm vertically and md horizontally"
        );
        assert_eq!(radius_sm(&tokens), 3.0);
        assert_eq!(radius_md(&tokens), 13.0);
        assert_eq!(radius_lg(&tokens), 21.0);
    }

    #[test]
    fn a_hairline_and_a_focus_ring_are_derived_from_the_smallest_token() {
        let mut tokens = light();
        tokens.spacing.xs = 3;

        assert_eq!(hairline(&tokens), 1.5, "a hairline is half of xs");
        assert_eq!(focus_ring(&tokens), 3.0, "a focus ring is xs");
    }

    #[test]
    fn a_card_is_bounded_the_way_a_path_bearing_control_is() {
        let tokens = light();

        assert_eq!(card_width(&tokens), 12.0 * f32::from(tokens.spacing.xl));
        assert!(
            card_width(&tokens) < 1280.0,
            "a card leaves the preview it describes visible"
        );
    }

    #[test]
    fn a_control_is_at_least_the_minimum_target_and_grows_with_the_text() {
        // UI spec §4: at least `spacing.xl`, and never a fixed height that clips
        // enlarged text.
        let mut tokens = light();
        let small = control_height(&tokens);

        assert!(small >= f32::from(tokens.spacing.xl), "{small}");
        assert!(
            small >= body_size(&tokens) * LINE_HEIGHT + 2.0 * f32::from(tokens.spacing.sm),
            "the label's line box fits inside the target"
        );

        tokens.typography.scale = 2.0;

        assert!(
            control_height(&tokens) > small,
            "200% font scaling grows the target instead of clipping the label"
        );
    }

    #[test]
    fn every_state_of_a_button_is_drawn_from_tokens() {
        let tokens = dark();
        let allowed = [
            color(tokens.colors.accent),
            color(tokens.colors.surface_variant),
        ];

        for role in [ButtonRole::Primary, ButtonRole::Secondary] {
            for status in STATUSES {
                let style = button_style(&tokens, role, status);
                let Some(fill) = style.background else {
                    continue;
                };
                let background = match fill {
                    Background::Color(color) => color,
                    other => panic!("expected a colour, got {other:?}"),
                };

                assert!(
                    allowed.contains(&background),
                    "{role:?} {status:?} is {background:?}"
                );
                assert!(
                    [hairline(&tokens), focus_ring(&tokens)].contains(&style.border.width),
                    "{role:?} {status:?} has a border of {}",
                    style.border.width
                );
            }
        }
    }

    #[test]
    fn the_states_of_a_control_are_pairwise_distinguishable() {
        // UI spec §5 and §10: inactive / hovered / pressed / disabled have to be
        // told apart, for the primary action and for the quieter ones alike.
        for tokens in [light(), dark()] {
            for role in [ButtonRole::Primary, ButtonRole::Secondary] {
                let drawings: Vec<_> = STATUSES
                    .into_iter()
                    .map(|status| drawing(button_style(&tokens, role, status)))
                    .collect();

                for (index, first) in drawings.iter().enumerate() {
                    for (other, second) in drawings.iter().enumerate().skip(index + 1) {
                        assert_ne!(
                            first, second,
                            "{:?} is drawn the same way in states {index} and {other}",
                            tokens.name
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_disabled_button_is_not_drawn_like_an_enabled_one() {
        let tokens = light();
        let enabled = button_style(&tokens, ButtonRole::Primary, button::Status::Active);
        let disabled = button_style(&tokens, ButtonRole::Primary, button::Status::Disabled);

        assert_ne!(enabled.background, disabled.background);
        assert_ne!(enabled.text_color, disabled.text_color);
        assert_eq!(
            disabled.background,
            Some(Background::Color(color(tokens.colors.surface_variant)))
        );
        assert_eq!(disabled.text_color, color(tokens.colors.text_muted));
    }

    #[test]
    fn only_the_primary_role_reaches_the_accent_colour_at_rest() {
        // UI spec §5: `accent` is for one action per surface. A secondary control
        // reaches it only while it is pressed.
        let tokens = dark();
        let primary = button_style(&tokens, ButtonRole::Primary, button::Status::Active);
        let secondary = button_style(&tokens, ButtonRole::Secondary, button::Status::Active);
        let pressed = button_style(&tokens, ButtonRole::Secondary, button::Status::Pressed);

        assert_eq!(
            primary.background,
            Some(Background::Color(color(tokens.colors.accent)))
        );
        assert_eq!(secondary.background, None, "a quiet control has no fill");
        assert_eq!(
            pressed.background,
            Some(Background::Color(color(tokens.colors.accent)))
        );
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
    fn emphasis_is_the_bold_weight_of_the_token_family() {
        let tokens = light();
        let bold = bold(&tokens);

        assert_eq!(bold.weight, Weight::Bold);
        assert_eq!(bold.family, font(&tokens).family);
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
        assert_ne!(success(&light()), success(&dark()));
    }

    #[test]
    fn the_appearance_of_the_token_set_is_carried_through() {
        assert_eq!(light().appearance, Appearance::Light);
        assert_eq!(dark().appearance, Appearance::Dark);
    }
}
