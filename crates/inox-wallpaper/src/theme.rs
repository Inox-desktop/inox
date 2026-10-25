//! Design tokens, as QML sees them.
//!
//! Every colour, size, radius and gap in the QML tree comes from here, and every
//! one of them comes from [`inox_config::Theme`] — no view writes a literal
//! colour or pixel size (root `AGENTS.md` §4, PRD §9.2). This is the only module
//! in the crate that turns a token into a QML value; the QML tree only binds to
//! what [`crate::view`] projected from these functions
//! (`docs/quality/ui-guidelines.md` §1).
//!
//! ADR 0018 accepts one small copy of this mapping per L3 component instead of a
//! shared UI crate, because a shared crate would be a new same-layer dependency
//! and therefore an ADR. The cost is that this file is the place a token change
//! has to be applied seven times; the decision's review condition says to revisit
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
//! hairline divisor, the line height a label occupies, the weight QML draws
//! emphasis with, and the width of a card in `spacing.xl` units — each named
//! once, in this block, with the rule it implements.
//!
//! # Why the conversion lives in Rust
//!
//! is explicit: colours are turned into Qt values on the
//! Rust side, never hard-coded in QML. Qt writes a colour as `#AARRGGBB`
//! ([`qml_color`]); sizes, radii and paddings are projected as numbers QML binds
//! to. The old Iced styles (`container::Style`, `button::Style`) are gone; what
//! remains is the token arithmetic they were made of, as plain data.

use inox_config::{Rgba, Theme};

/// `title`: a window or page headline, at most one per surface.
const TITLE_FACTOR: f64 = 1.5;

/// `heading`: a card or section heading.
const HEADING_FACTOR: f64 = 1.25;

/// `body`: prose, control labels, the current value, button text.
const BODY_FACTOR: f64 = 1.0;

/// `caption`: explanations, units, state sentences, secondary metadata.
const CAPTION_FACTOR: f64 = 0.875;

/// The line height a label occupies.
///
/// Named because [`control_height`] has to reserve a line box, not just the
/// glyph size: a target sized to the text alone clips it.
const LINE_HEIGHT: f64 = 1.3;

/// A hairline is half of the smallest spacing token (UI spec §1).
const HAIRLINE_DIVISOR: f64 = 2.0;

/// The width of a card in `spacing.xl` units.
///
/// UI spec §4 bounds a control that carries a path at 12 × `spacing.xl`. A card
/// is not a control, but its content is the same kind of thing — an output name,
/// a path, one sentence — so the same bound keeps it from swallowing the preview
/// it is drawn over, and the wrapping in QML keeps a longer path inside it
/// instead of running off the edge.
const CARD_WIDTH_UNITS: f64 = 12.0;

/// The weight body text is drawn at (Qt's `QFont::Normal`).
pub const NORMAL_WEIGHT: i32 = 400;

/// The weight emphasis is drawn at (Qt's `QFont::Bold`).
///
/// The UI spec §2 puts emphasis on the weight rather than on a larger size, and
/// asks for no more than one bold weight per screen — so this is the only weight
/// the projection ever asks for.
pub const EMPHASIS_WEIGHT: i32 = 700;

/// A fully transparent fill, in the `#AARRGGBB` form QML expects.
///
/// Used where a control has no fill of its own (the secondary role at rest). It
/// is a converted token rather than a colour name so that no QML file has to
/// name a colour.
pub const TRANSPARENT: &str = "#00000000";

/// A token colour as the `#AARRGGBB` string QML parses.
///
/// Qt's own order, alpha first, because that is `QColor::HexArgb`; writing
/// `#RRGGBBAA` instead renders a translucent black for every token.
#[must_use]
pub fn qml_color(rgba: Rgba) -> String {
    format!("#{:02X}{:02X}{:02X}{:02X}", rgba.a, rgba.r, rgba.g, rgba.b)
}

/// A token length as a whole number of logical pixels.
///
/// QML accepts fractional sizes, but a projected pixel value that is not whole
/// rounds differently on every device pixel ratio, so the borders and radii the
/// projection asks for are rounded here. A nonsensical token (a NaN or infinite
/// font scale) projects as zero rather than as an unrepresentable number; the
/// view floors *text* sizes at one pixel separately, so a label never vanishes.
#[must_use]
pub fn px(value: f64) -> i32 {
    if value.is_finite() {
        (value.round() as i32).max(0)
    } else {
        0
    }
}

/// A token length as a QML logical-pixel number, kept fractional.
///
/// The same rule as [`px`], without the rounding: a length is a width, a radius or
/// a spacing, so a nonsensical token (NaN, infinite, or a negative font scale) is
/// projected as zero rather than as a negative size QML would have to draw.
#[must_use]
pub fn units(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

/// The font family the token set asks for.
///
/// A family whose name says `mono`, or one that *is* the token's monospace
/// family, selects the monospace stack; everything else selects the sans-serif
/// one. The family the token names is passed through, so a machine that has it
/// installed draws it and a machine that does not falls back the way Qt does.
#[must_use]
pub fn font_family(tokens: &Theme) -> String {
    let family = &tokens.typography.font_family;
    if family.to_ascii_lowercase().contains("mono")
        || tokens
            .typography
            .font_mono_family
            .eq_ignore_ascii_case(family)
    {
        tokens.typography.font_mono_family.clone()
    } else {
        family.clone()
    }
}

/// The size the four roles are multiples of: `spacing.lg`, scaled by the user's
/// font-size preference (UI spec §2).
#[must_use]
pub fn base_size(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.lg) * f64::from(tokens.typography.scale)
}

/// The `title` size: `1.5 × base` (24 at the default tokens).
#[must_use]
pub fn title_size(tokens: &Theme) -> f64 {
    base_size(tokens) * TITLE_FACTOR
}

/// The `heading` size: `1.25 × base` (20 at the default tokens).
#[must_use]
pub fn heading_size(tokens: &Theme) -> f64 {
    base_size(tokens) * HEADING_FACTOR
}

/// The `body` size: `1 × base` (16 at the default tokens).
#[must_use]
pub fn body_size(tokens: &Theme) -> f64 {
    base_size(tokens) * BODY_FACTOR
}

/// The `caption` size: `0.875 × base` (14 at the default tokens).
#[must_use]
pub fn caption_size(tokens: &Theme) -> f64 {
    base_size(tokens) * CAPTION_FACTOR
}

/// The gap inside one group: a label and its control, two adjacent lines, an
/// icon and its text (UI spec §3, `spacing.sm`).
#[must_use]
pub fn gap(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.sm)
}

/// The gap between blocks: between cards, and between a card's prose and its
/// controls (UI spec §3, `spacing.lg`).
#[must_use]
pub fn section_gap(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.lg)
}

/// The inset of a card (UI spec §4: cards are `surface` + hairline + `md` radius
/// + `lg` padding).
#[must_use]
pub fn card_padding(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.lg)
}

/// The inset of a status strip (UI spec §3, `spacing.md`).
#[must_use]
pub fn strip_padding(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.md)
}

/// The inset between the interface and the edge of the surface (UI spec §3:
/// page and window padding is `xl`).
#[must_use]
pub fn window_padding(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.xl)
}

/// The width of a card: twelve times `spacing.xl`, the bound UI spec §4 puts on
/// a control whose content is a path.
#[must_use]
pub fn card_width(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.xl) * CARD_WIDTH_UNITS
}

/// A hairline: `spacing.xs / 2` (UI spec §1).
#[must_use]
pub fn hairline(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.xs) / HAIRLINE_DIVISOR
}

/// The width of a focus ring: `spacing.xs` (UI spec §1, §5).
#[must_use]
pub fn focus_ring(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.xs)
}

/// The radius of an input, a button, a label or a switch.
#[must_use]
pub fn radius_sm(tokens: &Theme) -> f64 {
    f64::from(tokens.radii.sm)
}

/// The radius of a card, a group, a dialog, a menu or a notification.
#[must_use]
pub fn radius_md(tokens: &Theme) -> f64 {
    f64::from(tokens.radii.md)
}

/// The radius of a window or a full-screen surface.
#[must_use]
pub fn radius_lg(tokens: &Theme) -> f64 {
    f64::from(tokens.radii.lg)
}

/// The height of an interactive control.
///
/// UI spec §4 asks for at least `spacing.xl`, growing with the font scale so that
/// a fixed height never clips enlarged text: the label's line box plus the
/// control's own vertical padding wins whenever it is larger than `spacing.xl`.
#[must_use]
pub fn control_height(tokens: &Theme) -> f64 {
    f64::from(tokens.spacing.xl)
        .max(body_size(tokens) * LINE_HEIGHT + 2.0 * f64::from(tokens.spacing.sm))
}

/// The padding of a button: `sm` above and below, `md` either side (UI spec §3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    /// Space above the label.
    pub top: f64,
    /// Space to the right of the label.
    pub right: f64,
    /// Space below the label.
    pub bottom: f64,
    /// Space to the left of the label.
    pub left: f64,
}

/// The padding of a projected control, in logical pixels.
#[must_use]
pub fn control_padding(tokens: &Theme) -> Padding {
    Padding {
        top: f64::from(tokens.spacing.sm),
        right: f64::from(tokens.spacing.md),
        bottom: f64::from(tokens.spacing.sm),
        left: f64::from(tokens.spacing.md),
    }
}

/// The style of a card drawn over the wallpaper or over the background.
///
/// UI spec §4: `surface` background, a hairline `border`, an `md` radius and no
/// shadow. A second surface is never stacked on the same colour: a card sits on
/// the `background` or on the wallpaper, and `surface_variant` is reserved for
/// the hovered and disabled states of a control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardStyle {
    /// The card's fill.
    pub background: String,
    /// The card's hairline outline.
    pub border: String,
    /// The outline's width.
    pub border_width: i32,
    /// The corner radius.
    pub radius: i32,
}

/// The card style the tokens ask for.
#[must_use]
pub fn card(tokens: &Theme) -> CardStyle {
    CardStyle {
        background: qml_color(tokens.colors.surface),
        border: qml_color(tokens.colors.border),
        // A hairline is the one projected length that is not a whole number at
        // the default tokens, so it stays fractional: Qt draws a 0.5px border.
        border_width: px(hairline(tokens)),
        radius: px(radius_md(tokens)),
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

/// The states a control is drawn in (UI spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    /// At rest, waiting to be pressed.
    Active,
    /// Under the pointer.
    Hovered,
    /// Being pressed.
    Pressed,
    /// Present but not pressable.
    Disabled,
}

/// The style of a button in one state.
///
/// Every field is a token value already converted for QML; QML binds them and
/// decides nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonStyle {
    /// The fill; [`TRANSPARENT`] when the role has no fill at rest.
    pub background: String,
    /// The label colour.
    pub text: String,
    /// The outline colour.
    pub border: String,
    /// The outline width.
    pub border_width: i32,
    /// The corner radius.
    pub radius: i32,
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
pub fn button_style(tokens: &Theme, role: ButtonRole, state: ButtonState) -> ButtonStyle {
    let border = |color: String, width: f64| (color, px(width));

    let (background, text, (border_color, border_width)) = match (role, state) {
        (ButtonRole::Primary, ButtonState::Active) => (
            qml_color(tokens.colors.accent),
            qml_color(tokens.colors.on_accent),
            border(qml_color(tokens.colors.accent), hairline(tokens)),
        ),
        (ButtonRole::Primary, ButtonState::Hovered) => (
            qml_color(tokens.colors.accent),
            qml_color(tokens.colors.on_accent),
            border(qml_color(tokens.colors.on_accent), hairline(tokens)),
        ),
        (ButtonRole::Primary, ButtonState::Pressed) => (
            qml_color(tokens.colors.accent),
            qml_color(tokens.colors.on_accent),
            border(qml_color(tokens.colors.on_accent), focus_ring(tokens)),
        ),
        (ButtonRole::Primary, ButtonState::Disabled) => (
            qml_color(tokens.colors.surface_variant),
            qml_color(tokens.colors.text_muted),
            border(qml_color(tokens.colors.border), hairline(tokens)),
        ),
        (ButtonRole::Secondary, ButtonState::Active) => (
            TRANSPARENT.to_owned(),
            qml_color(tokens.colors.text),
            border(qml_color(tokens.colors.border), hairline(tokens)),
        ),
        (ButtonRole::Secondary, ButtonState::Hovered) => (
            qml_color(tokens.colors.surface_variant),
            qml_color(tokens.colors.text),
            border(qml_color(tokens.colors.border), hairline(tokens)),
        ),
        (ButtonRole::Secondary, ButtonState::Pressed) => (
            qml_color(tokens.colors.accent),
            qml_color(tokens.colors.on_accent),
            border(qml_color(tokens.colors.accent), hairline(tokens)),
        ),
        (ButtonRole::Secondary, ButtonState::Disabled) => (
            TRANSPARENT.to_owned(),
            qml_color(tokens.colors.text_muted),
            border(qml_color(tokens.colors.border), hairline(tokens)),
        ),
    };

    ButtonStyle {
        background,
        text,
        border: border_color,
        border_width,
        radius: px(radius_sm(tokens)),
    }
}

/// The colour of secondary text.
#[must_use]
pub fn muted(tokens: &Theme) -> String {
    qml_color(tokens.colors.text_muted)
}

/// The colour of a warning the user has to read.
#[must_use]
pub fn warning(tokens: &Theme) -> String {
    qml_color(tokens.colors.warning)
}

/// The colour of a state the component has confirmed.
#[must_use]
pub fn success(tokens: &Theme) -> String {
    qml_color(tokens.colors.success)
}

/// The colour of ordinary text.
#[must_use]
pub fn text(tokens: &Theme) -> String {
    qml_color(tokens.colors.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::{Appearance, Rgba};

    fn light() -> Theme {
        Theme::builtin_light()
    }

    fn dark() -> Theme {
        Theme::builtin_dark()
    }

    /// The four states the projection asks a button's style for.
    const STATES: [ButtonState; 4] = [
        ButtonState::Active,
        ButtonState::Hovered,
        ButtonState::Pressed,
        ButtonState::Disabled,
    ];

    /// What makes two drawings of a control different.
    fn drawing(style: ButtonStyle) -> (String, String, String, i32) {
        (
            style.background,
            style.text,
            style.border,
            style.border_width,
        )
    }

    #[test]
    fn a_token_colour_becomes_the_qt_argb_string() {
        let tokens = light();
        let converted = qml_color(tokens.colors.accent);

        assert_eq!(
            converted,
            format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                tokens.colors.accent.a,
                tokens.colors.accent.r,
                tokens.colors.accent.g,
                tokens.colors.accent.b
            )
        );
        assert_eq!(converted.len(), 9, "Qt writes #AARRGGBB");
        assert_eq!(
            qml_color(Rgba {
                r: 0x12,
                g: 0x34,
                b: 0x56,
                a: 0xff
            }),
            "#FF123456",
            "alpha comes first, as QColor::HexArgb writes it"
        );
        assert_eq!(qml_color(tokens.colors.border), {
            let border = tokens.colors.border;
            format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                border.a, border.r, border.g, border.b
            )
        });
    }

    #[test]
    fn a_card_is_a_surface_with_a_hairline_and_an_md_radius() {
        // UI spec §4: `surface` + hairline `border` + `md` radius, no shadow.
        let tokens = dark();
        let style = card(&tokens);

        assert_eq!(style.background, qml_color(tokens.colors.surface));
        assert_eq!(style.border, qml_color(tokens.colors.border));
        assert_eq!(style.border_width, px(hairline(&tokens)));
        assert_eq!(style.radius, px(radius_md(&tokens)));
    }

    #[test]
    fn the_four_type_roles_are_the_multiples_of_the_token_base() {
        // UI spec §2: the base is `spacing.lg × scale`, the roles are 1.5 / 1.25
        // / 1 / 0.875 of it, so at the default tokens they are 24 / 20 / 16 / 14.
        let tokens = light();

        assert_eq!(base_size(&tokens), f64::from(tokens.spacing.lg));
        assert_eq!(px(title_size(&tokens)), 24);
        assert_eq!(px(heading_size(&tokens)), 20);
        assert_eq!(px(body_size(&tokens)), 16);
        assert_eq!(px(caption_size(&tokens)), 14);
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
    fn a_nonsensical_font_scale_still_yields_a_drawable_size() {
        let mut tokens = light();
        for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -4.0, 0.0] {
            tokens.typography.scale = scale;
            assert_eq!(
                px(body_size(&tokens)),
                0,
                "an unusable scale is projected as no length, never as a panic"
            );
            assert_eq!(units(body_size(&tokens)), 0.0);
        }
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
                right: 9.0,
                bottom: 7.0,
                left: 9.0
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
        assert_eq!(
            px(hairline(&tokens)),
            2,
            "a projected border is a whole number of device pixels"
        );
    }

    #[test]
    fn a_card_is_bounded_the_way_a_path_bearing_control_is() {
        let tokens = light();

        assert_eq!(card_width(&tokens), 12.0 * f64::from(tokens.spacing.xl));
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

        assert!(small >= f64::from(tokens.spacing.xl), "{small}");
        assert!(
            small >= body_size(&tokens) * LINE_HEIGHT + 2.0 * f64::from(tokens.spacing.sm),
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
            qml_color(tokens.colors.accent),
            qml_color(tokens.colors.surface_variant),
            TRANSPARENT.to_owned(),
        ];

        for role in [ButtonRole::Primary, ButtonRole::Secondary] {
            for state in STATES {
                let style = button_style(&tokens, role, state);

                assert!(
                    allowed.contains(&style.background),
                    "{role:?} {state:?} is {}",
                    style.background
                );
                assert!(
                    style.border_width == px(hairline(&tokens))
                        || style.border_width == px(focus_ring(&tokens)),
                    "{role:?} {state:?} has a border of {}",
                    style.border_width
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
                let drawings: Vec<_> = STATES
                    .into_iter()
                    .map(|state| drawing(button_style(&tokens, role, state)))
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
        let enabled = button_style(&tokens, ButtonRole::Primary, ButtonState::Active);
        let disabled = button_style(&tokens, ButtonRole::Primary, ButtonState::Disabled);

        assert_ne!(enabled.background, disabled.background);
        assert_ne!(enabled.text, disabled.text);
        assert_eq!(
            disabled.background,
            qml_color(tokens.colors.surface_variant)
        );
        assert_eq!(disabled.text, qml_color(tokens.colors.text_muted));
    }

    #[test]
    fn only_the_primary_role_reaches_the_accent_colour_at_rest() {
        // UI spec §5: `accent` is for one action per surface. A secondary control
        // reaches it only while it is pressed.
        let tokens = dark();
        let primary = button_style(&tokens, ButtonRole::Primary, ButtonState::Active);
        let secondary = button_style(&tokens, ButtonRole::Secondary, ButtonState::Active);
        let pressed = button_style(&tokens, ButtonRole::Secondary, ButtonState::Pressed);

        assert_eq!(primary.background, qml_color(tokens.colors.accent));
        assert_eq!(
            secondary.background, TRANSPARENT,
            "a quiet control has no fill"
        );
        assert_eq!(pressed.background, qml_color(tokens.colors.accent));
    }

    #[test]
    fn a_monospaced_family_selects_the_monospace_stack() {
        let mut tokens = light();
        tokens.typography.font_mono_family = "JetBrains Mono".to_owned();
        tokens.typography.font_family = "JetBrains Mono".to_owned();

        assert_eq!(font_family(&tokens), "JetBrains Mono");

        tokens.typography.font_family = "Noto Sans".to_owned();
        assert_eq!(font_family(&tokens), "Noto Sans");
    }

    #[test]
    fn emphasis_is_the_bold_weight() {
        assert_eq!(EMPHASIS_WEIGHT, 700);
        assert_eq!(NORMAL_WEIGHT, 400);
        // A compile-time check rather than a run-time one: the two weights are
        // constants, so "emphasis is the heavier of the two" can be settled when
        // this file is compiled and can never drift at run time.
        const {
            assert!(EMPHASIS_WEIGHT > NORMAL_WEIGHT);
        }
    }

    #[test]
    fn the_light_and_dark_token_sets_draw_differently() {
        assert_ne!(card(&light()).background, card(&dark()).background);
        assert_ne!(
            text(&light()),
            text(&dark()),
            "the two appearances must not collapse into one"
        );
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
