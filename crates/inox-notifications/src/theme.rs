//! Design tokens, as Iced sees them.
//!
//! Every colour, gap, radius and text size the popup stack draws with is read
//! from [`inox_config::Theme`] here (root AGENTS.md §4, PRD §9.2): no view and no
//! style function in this crate contains a colour or a pixel size. What this file
//! implements is `docs/quality/ui-guidelines.md` §1 (token discipline), §2 (the
//! four type roles), §4 (card geometry), §5 (the five states of a control) and
//! §6 (a status is a sentence first and a colour second). ADR 0018 explains why
//! this small mapping is duplicated in each L3 component instead of being shared
//! — a shared UI crate would be a new same-layer dependency, which needs an ADR
//! of its own.
//!
//! Sizes are derived, never invented. The base text size is
//! `spacing.lg × typography.scale` — the window-shaped surface rule of §2 — and
//! every role is a named multiple of it, so a user asking for 200 % of the text
//! gets it without this file naming a pixel. The one literal left is the
//! hairline: the token set carries a border *colour* and no border width, and one
//! logical pixel is the hairline those colours are written against.

use iced::widget::{button, container};
use iced::{Border, Color, Font, Padding};
use inox_config::{Rgba, Theme};

use crate::markup::Emphasis;
use crate::protocol::Urgency;
use crate::view_model::ActionEmphasis;

/// The width of a hairline border, in logical pixels.
///
/// The token set carries a border *colour* and no border width; one logical
/// pixel is the hairline the tokens are written against, including under
/// fractional scaling. Every border this crate draws is exactly this wide —
/// ui-guidelines §4 allows no other.
pub const BORDER_WIDTH: f32 = 1.0;

/// The multiple of the base size a window title is drawn at (ui-guidelines §2).
const TITLE_MULTIPLIER: f32 = 1.5;

/// The multiple of the base size a card heading is drawn at.
const HEADING_MULTIPLIER: f32 = 1.25;

/// The multiple of the base size prose and control labels are drawn at.
const BODY_MULTIPLIER: f32 = 1.0;

/// The multiple of the base size secondary information is drawn at.
const CAPTION_MULTIPLIER: f32 = 0.875;

/// One of the four type roles of ui-guidelines §2.
///
/// The roles are fixed, and so are their uses: a surface has at most one `title`,
/// a card is introduced by a `heading`, prose and control labels are `body`, and
/// everything that explains rather than says is `caption`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    /// The window or page title: at most one per surface.
    ///
    /// Nothing in this stack draws it — the surface's title belongs to the
    /// window manager and is set through [`crate::app::title`] — but the role is
    /// part of the token contract and is exported so that every surface of this
    /// component is measured against the same four sizes.
    Title,
    /// A card's own heading: here, the notification's summary.
    Heading,
    /// Prose, and the label on a control.
    Body,
    /// Secondary information: application names, status sentences, timestamps.
    Caption,
}

impl TextRole {
    /// Every role, for a test that measures all four at once.
    pub const ALL: [Self; 4] = [Self::Title, Self::Heading, Self::Body, Self::Caption];

    /// The multiple of the base size this role is drawn at.
    pub const fn multiplier(self) -> f32 {
        match self {
            Self::Title => TITLE_MULTIPLIER,
            Self::Heading => HEADING_MULTIPLIER,
            Self::Body => BODY_MULTIPLIER,
            Self::Caption => CAPTION_MULTIPLIER,
        }
    }
}

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

/// The size every role is a multiple of: the window surface's base size.
///
/// ui-guidelines §2 sets it to `spacing.lg × typography.scale` for a
/// window-shaped surface, which is what this stack is. It is a token product
/// rather than Iced's default text size, so the whole hierarchy moves together
/// when a user changes the font scale.
pub fn base_text_size(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.lg) * tokens.typography.scale
}

/// The size of one type role, at the user's scale.
pub fn text_size(tokens: &Theme, role: TextRole) -> f32 {
    base_text_size(tokens) * role.multiplier()
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
/// colour. This is the one colour the urgency sentence is written in and the one
/// the critical card is edged with: ui-guidelines §5 asks for a colour *and* a
/// sentence, never a stack of colours.
pub fn urgency_color(tokens: &Theme, urgency: Urgency) -> Color {
    match urgency {
        Urgency::Low => muted_text(tokens),
        Urgency::Normal => text(tokens),
        Urgency::Critical => color(tokens.colors.danger),
    }
}

/// The inset inside a card (ui-guidelines §4: the large step).
pub fn card_inset(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.lg)
}

/// The inset inside one of the daemon's own lines (ui-guidelines §3: a status
/// bar is inset by the medium step).
pub fn notice_inset(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The gap between two blocks — two cards, or a card and a status line
/// (ui-guidelines §3: blocks are separated by the large step).
pub fn block_gap(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.lg)
}

/// The gap between two parts of one block.
///
/// ui-guidelines §3 builds its rhythm out of two distances — blocks are `lg`
/// apart, the things inside a group are `sm` apart — and this is the step
/// between one thing and the next inside a card, which is the token set's own
/// default gap between controls.
pub fn section_gap(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The gap between two controls in one row.
pub fn control_gap(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The gap between two items of one line: an icon and the name beside it, or a
/// status sentence and the timestamp after it.
pub fn inline_gap(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.sm)
}

/// The padding inside a control.
///
/// ui-guidelines §3 asks for `sm` above and below a control and `md` to its
/// sides, and §4 also requires an interactive target at least `xl` tall. With the
/// `body` role as a label the `sm` inset leaves a button shorter than that, so
/// §4's requirement — the stricter one — sets the vertical inset to `md` as well.
/// [`crate::view::StackGeometry::control_height`] is where that minimum is
/// measured.
pub fn control_inset(tokens: &Theme) -> Padding {
    let step = f32::from(tokens.spacing.md);

    Padding {
        top: step,
        bottom: step,
        left: step,
        right: step,
    }
}

/// The corner radius of a card, a popup or a notice (ui-guidelines §4: `md`).
pub fn card_radius(tokens: &Theme) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// The corner radius of a control (ui-guidelines §4: `sm`).
pub fn control_radius(tokens: &Theme) -> iced::border::Radius {
    f32::from(tokens.radii.sm).into()
}

/// The focus ring of a control (ui-guidelines §1 and §5).
///
/// The accent colour, drawn as thick as `spacing.xs`. This stack never takes
/// keyboard focus — its placement is
/// [`crate::placement::KeyboardInteractivity::None`](crate::placement::KeyboardInteractivity)
/// and the popup model has no focus order at all — so nothing here wears it yet.
/// It is part of the token contract §10 asks every component to export, and it is
/// what a host that does hand this surface the keyboard would draw.
pub fn focus_ring(tokens: &Theme) -> Border {
    hairline(
        color(tokens.colors.accent),
        f32::from(tokens.spacing.xs),
        control_radius(tokens),
    )
}

/// A hairline edge in `colour`, with `radius` rounded corners.
fn hairline(colour: Color, width: f32, radius: iced::border::Radius) -> Border {
    Border {
        color: colour,
        width,
        radius,
    }
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

/// The surface the whole stack is drawn on: the window's own background.
pub fn window(tokens: &Theme) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.background).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// One notification card.
///
/// ui-guidelines §4: the body is the `surface` token, the edge is a `border`
/// hairline, the radius is `md` and the padding (which the view applies) is `lg`.
/// The one card that differs is a critical notification, which takes the danger
/// colour as its edge — the same colour its urgency sentence is written in, so
/// urgency is one colour plus one sentence rather than a second surface.
pub fn card(tokens: &Theme, urgency: Urgency) -> container::Style {
    let edge = match urgency {
        Urgency::Critical => color(tokens.colors.danger),
        Urgency::Low | Urgency::Normal => color(tokens.colors.border),
    };

    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        border: hairline(edge, BORDER_WIDTH, card_radius(tokens)),
        ..container::Style::default()
    }
}

/// What one of the daemon's own lines is saying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeTone {
    /// A state the user is in: nothing is held, or do not disturb is on.
    Quiet,
    /// Something the user cannot see and should know about: the overflow line,
    /// which is written in the warning colour because it is about notifications
    /// that arrived and are not on screen (D05).
    Caution,
}

/// The daemon's own line: the empty and muted states, and the overflow sentence.
///
/// A notice is a status bar rather than a card (ui-guidelines §3 gives it the
/// `md` inset), but it keeps the hairline edge of §4 so that it still reads as
/// something the daemon is saying rather than as slack in the layout.
pub fn notice(tokens: &Theme, tone: NoticeTone) -> container::Style {
    let edge = match tone {
        NoticeTone::Quiet => color(tokens.colors.border),
        NoticeTone::Caution => color(tokens.colors.warning),
    };

    container::Style {
        background: Some(color(tokens.colors.surface_variant).into()),
        text_color: Some(notice_text(tokens, tone)),
        border: hairline(edge, BORDER_WIDTH, card_radius(tokens)),
        ..container::Style::default()
    }
}

/// The colour the daemon's own line is written in.
pub fn notice_text(tokens: &Theme, tone: NoticeTone) -> Color {
    match tone {
        NoticeTone::Quiet => muted_text(tokens),
        NoticeTone::Caution => color(tokens.colors.warning),
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
            radius: control_radius(tokens),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The button that closes one popup.
///
/// ui-guidelines §5, all five rows of it: the resting button is the muted text
/// colour with a hairline edge, hovering is the alternate surface, pressing fills
/// it with the accent and writes on it in `on_accent`, and a disabled close
/// button loses both its fill and its edge — so it stops looking like a control
/// rather than looking like a quiet one.
///
/// A close button is in fact never disabled by this stack: every popup carries
/// one and it is always pressable, which is why there is no reason sentence for
/// it to show. The state is drawn anyway because §5 asks for all five to be
/// distinguishable, and a state that is not drawn is a state that would be drawn
/// wrong the day it appears.
pub fn close_button(tokens: &Theme, status: button::Status) -> button::Style {
    let border = color(tokens.colors.border);
    let (background, foreground, edge) = match status {
        button::Status::Active => (None, muted_text(tokens), border),
        button::Status::Hovered => (
            Some(color(tokens.colors.surface_variant).into()),
            text(tokens),
            border,
        ),
        button::Status::Pressed => (
            Some(color(tokens.colors.accent).into()),
            color(tokens.colors.on_accent),
            color(tokens.colors.accent),
        ),
        // No edge at all: a disabled close button should not look like a quiet
        // control, it should stop looking like one.
        button::Status::Disabled => (None, muted_text(tokens), Color::TRANSPARENT),
    };

    button::Style {
        background,
        text_color: foreground,
        border: Border {
            color: edge,
            width: BORDER_WIDTH,
            radius: control_radius(tokens),
        },
        ..button::Style::default()
    }
}

/// A button that invokes one of a notification's actions.
///
/// The tier decides the colour, and only the colour: a primary action is drawn
/// in the accent, a secondary one in ordinary text, and an action that cannot be
/// undone in the danger colour (ui-guidelines §5: `accent` for the one primary
/// action, `danger` as an edge, never a whole accent-filled block for something
/// destructive). The four states are the shared ones: resting is plain, hovering
/// is the alternate surface, pressing fills with the accent, and a disabled
/// action loses its tier colour entirely — muted text, plain edge — so that
/// nothing about it still looks like something the user could press (D05).
pub fn action_button(
    tokens: &Theme,
    emphasis: ActionEmphasis,
    status: button::Status,
) -> button::Style {
    let border = color(tokens.colors.border);
    let (background, foreground, edge) = match status {
        button::Status::Disabled => (None, muted_text(tokens), border),
        button::Status::Active => (
            None,
            tier_color(tokens, emphasis),
            tier_color(tokens, emphasis),
        ),
        button::Status::Hovered => (
            Some(color(tokens.colors.surface_variant).into()),
            tier_color(tokens, emphasis),
            tier_color(tokens, emphasis),
        ),
        button::Status::Pressed => (
            Some(color(tokens.colors.accent).into()),
            color(tokens.colors.on_accent),
            color(tokens.colors.accent),
        ),
    };

    button::Style {
        background,
        text_color: foreground,
        border: Border {
            color: edge,
            width: BORDER_WIDTH,
            radius: control_radius(tokens),
        },
        ..button::Style::default()
    }
}

/// The colour one action tier is drawn in at rest.
fn tier_color(tokens: &Theme, emphasis: ActionEmphasis) -> Color {
    match emphasis {
        ActionEmphasis::Primary => color(tokens.colors.accent),
        ActionEmphasis::Secondary => text(tokens),
        ActionEmphasis::Danger => color(tokens.colors.danger),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::theme::Base;
    use inox_config::{BUILTIN_DARK, BUILTIN_LIGHT};

    /// The four states of one action button, for the pairwise assertions.
    fn states(
        tokens: &Theme,
        emphasis: ActionEmphasis,
    ) -> [(Option<iced::Background>, Color, Color, f32); 4] {
        [
            button::Status::Active,
            button::Status::Hovered,
            button::Status::Pressed,
            button::Status::Disabled,
        ]
        .map(|status| describe(&action_button(tokens, emphasis, status)))
    }

    /// The four things that make one button state tell itself apart from another:
    /// its fill, its text, its edge and how thick that edge is.
    fn describe(style: &button::Style) -> (Option<iced::Background>, Color, Color, f32) {
        (
            style.background,
            style.text_color,
            style.border.color,
            style.border.width,
        )
    }

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
    fn the_base_size_is_a_token_product_and_follows_the_user_scale() {
        let mut tokens = Theme::builtin_dark();
        let base = base_text_size(&tokens);

        assert!(
            (base - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON,
            "the base size is spacing.lg at scale 1, not a literal"
        );

        tokens.typography.scale = 2.0;

        assert!((base_text_size(&tokens) - base * 2.0).abs() < f32::EPSILON);
        assert!(base > 0.0);
    }

    #[test]
    fn every_role_is_a_fixed_multiple_of_the_base_size() {
        let tokens = Theme::builtin_dark();
        let base = base_text_size(&tokens);

        let expected = [
            (TextRole::Title, 1.5_f32),
            (TextRole::Heading, 1.25),
            (TextRole::Body, 1.0),
            (TextRole::Caption, 0.875),
        ];

        for (role, multiplier) in expected {
            assert!(
                (role.multiplier() - multiplier).abs() < f32::EPSILON,
                "{role:?} is not {multiplier}× the base size"
            );
            assert!(
                (text_size(&tokens, role) - base * multiplier).abs() < f32::EPSILON,
                "{role:?} does not follow the user's scale"
            );
        }

        assert_eq!(TextRole::ALL.len(), expected.len(), "four roles, no more");
        assert!(
            TextRole::Caption.multiplier() >= 0.75,
            "ui-guidelines §2: a caption is never smaller than 0.75× the base"
        );
        assert!(TextRole::Title.multiplier() > TextRole::Heading.multiplier());
        assert!(TextRole::Heading.multiplier() > TextRole::Body.multiplier());
        assert!(TextRole::Body.multiplier() > TextRole::Caption.multiplier());
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
    fn a_card_is_the_surface_token_with_a_hairline_and_the_card_radius() {
        let tokens = Theme::builtin_dark();
        let style = card(&tokens, Urgency::Normal);

        assert_eq!(style.background, Some(color(tokens.colors.surface).into()));
        assert_eq!(style.border.color, color(tokens.colors.border));
        assert_eq!(style.border.radius, card_radius(&tokens));
        assert_eq!(card_radius(&tokens), f32::from(tokens.radii.md).into());
        assert!((style.border.width - BORDER_WIDTH).abs() < f32::EPSILON);
    }

    #[test]
    fn a_critical_card_is_edged_in_the_danger_colour() {
        let tokens = Theme::builtin_dark();
        let normal = card(&tokens, Urgency::Normal);
        let critical = card(&tokens, Urgency::Critical);

        assert_eq!(critical.border.color, color(tokens.colors.danger));
        assert_ne!(critical.border.color, normal.border.color);
        assert_eq!(
            critical.background, normal.background,
            "the danger colour is an edge, not a second background"
        );
        assert_eq!(
            card(&tokens, Urgency::Low).border.color,
            normal.border.color,
            "low urgency is said in a sentence, not in a second edge colour"
        );
    }

    #[test]
    fn the_spacing_rhythm_comes_from_the_tokens() {
        let tokens = Theme::builtin_dark();

        assert_eq!(card_inset(&tokens), f32::from(tokens.spacing.lg));
        assert_eq!(notice_inset(&tokens), f32::from(tokens.spacing.md));
        assert_eq!(block_gap(&tokens), f32::from(tokens.spacing.lg));
        assert_eq!(section_gap(&tokens), f32::from(tokens.spacing.md));
        assert_eq!(control_gap(&tokens), f32::from(tokens.spacing.md));
        assert_eq!(inline_gap(&tokens), f32::from(tokens.spacing.sm));
        assert_eq!(
            control_inset(&tokens),
            Padding {
                top: f32::from(tokens.spacing.md),
                bottom: f32::from(tokens.spacing.md),
                left: f32::from(tokens.spacing.md),
                right: f32::from(tokens.spacing.md),
            }
        );
        assert!(
            inline_gap(&tokens) < block_gap(&tokens),
            "the rhythm is a rhythm: a group is tighter than a block boundary"
        );
    }

    #[test]
    fn a_changed_token_moves_the_distance_derived_from_it() {
        let tokens = Theme {
            spacing: inox_config::Spacing {
                md: 12,
                ..Theme::builtin_dark().spacing
            },
            ..Theme::builtin_dark()
        };

        assert_eq!(section_gap(&tokens), 12.0);
        assert_eq!(control_gap(&tokens), 12.0);
        assert_eq!(notice_inset(&tokens), 12.0);
        assert_eq!(control_inset(&tokens).left, 12.0);
        assert_eq!(
            card_inset(&tokens),
            f32::from(Theme::builtin_dark().spacing.lg),
            "a distance is derived from the token it names, not from a default"
        );
    }

    #[test]
    fn a_control_radius_is_the_small_token_and_a_card_radius_is_the_medium_one() {
        let tokens = Theme::builtin_dark();

        assert_eq!(control_radius(&tokens), f32::from(tokens.radii.sm).into());
        assert_ne!(control_radius(&tokens), card_radius(&tokens));
    }

    #[test]
    fn the_focus_ring_is_the_accent_colour_as_thick_as_the_spacing_step() {
        let tokens = Theme::builtin_dark();
        let ring = focus_ring(&tokens);

        assert_eq!(ring.color, color(tokens.colors.accent));
        assert_eq!(ring.width, f32::from(tokens.spacing.xs));
        assert!(
            ring.width > BORDER_WIDTH,
            "a focus ring is thicker than the hairline it sits on"
        );
        assert_eq!(ring.radius, control_radius(&tokens));
    }

    #[test]
    fn every_action_tier_is_a_colour_of_its_own() {
        let tokens = Theme::builtin_dark();
        let accent = color(tokens.colors.accent);
        let primary = action_button(&tokens, ActionEmphasis::Primary, button::Status::Active);
        let secondary = action_button(&tokens, ActionEmphasis::Secondary, button::Status::Active);
        let danger = action_button(&tokens, ActionEmphasis::Danger, button::Status::Active);

        assert_eq!(primary.text_color, accent);
        assert_eq!(primary.border.color, accent);
        assert_eq!(secondary.text_color, text(&tokens));
        assert_eq!(secondary.background, None, "a secondary action has no fill");
        assert_eq!(danger.text_color, color(tokens.colors.danger));
        assert_eq!(danger.border.color, color(tokens.colors.danger));
        assert_ne!(primary.text_color, secondary.text_color);
        assert_ne!(danger.text_color, secondary.text_color);
        assert_ne!(danger.border.color, accent, "danger is not accent-filled");
    }

    #[test]
    fn the_four_states_of_an_action_button_are_pairwise_distinguishable() {
        let tokens = Theme::builtin_dark();

        for emphasis in [
            ActionEmphasis::Primary,
            ActionEmphasis::Secondary,
            ActionEmphasis::Danger,
        ] {
            let states = states(&tokens, emphasis);
            let names = ["resting", "hovered", "pressed", "disabled"];

            for (left, first) in states.iter().enumerate() {
                for (right, second) in states.iter().enumerate().skip(left + 1) {
                    assert_ne!(
                        first, second,
                        "the {} and {} states of a {emphasis:?} action look the same",
                        names[left], names[right]
                    );
                }
            }
        }
    }

    #[test]
    fn the_four_states_of_a_close_button_are_pairwise_distinguishable() {
        let tokens = Theme::builtin_dark();
        let states = [
            button::Status::Active,
            button::Status::Hovered,
            button::Status::Pressed,
            button::Status::Disabled,
        ]
        .map(|status| describe(&close_button(&tokens, status)));

        for (left, first) in states.iter().enumerate() {
            for (_, second) in states.iter().enumerate().skip(left + 1) {
                assert_ne!(first, second, "two close-button states look the same");
            }
        }
    }

    #[test]
    fn a_pressed_control_is_filled_with_the_accent_and_written_on_it() {
        let tokens = Theme::builtin_dark();
        let pressed = action_button(&tokens, ActionEmphasis::Primary, button::Status::Pressed);
        let close = close_button(&tokens, button::Status::Pressed);

        assert_eq!(pressed.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(pressed.text_color, color(tokens.colors.on_accent));
        assert_eq!(close.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(close.text_color, color(tokens.colors.on_accent));
    }

    #[test]
    fn a_hovered_control_is_drawn_on_the_alternate_surface() {
        let tokens = Theme::builtin_dark();
        let hovered = action_button(&tokens, ActionEmphasis::Primary, button::Status::Hovered);
        let idle = action_button(&tokens, ActionEmphasis::Primary, button::Status::Active);

        assert_eq!(
            hovered.background,
            Some(color(tokens.colors.surface_variant).into())
        );
        assert_ne!(idle.background, hovered.background);
        assert_eq!(
            close_button(&tokens, button::Status::Hovered).background,
            Some(color(tokens.colors.surface_variant).into())
        );
    }

    #[test]
    fn a_disabled_action_looks_different_from_one_that_can_be_used() {
        let tokens = Theme::builtin_dark();
        let enabled = action_button(&tokens, ActionEmphasis::Primary, button::Status::Active);
        let disabled = action_button(&tokens, ActionEmphasis::Primary, button::Status::Disabled);
        let accent = color(tokens.colors.accent);

        assert_ne!(enabled.text_color, disabled.text_color);
        assert_eq!(disabled.text_color, muted_text(&tokens));
        assert_eq!(disabled.background, None);
        assert_ne!(
            disabled.border.color, accent,
            "nothing about a disabled action is accent-coloured"
        );
        assert_eq!(disabled.border.color, color(tokens.colors.border));
        assert_eq!(
            action_button(&tokens, ActionEmphasis::Danger, button::Status::Disabled).text_color,
            muted_text(&tokens),
            "a disabled action loses its tier colour, danger included"
        );
    }

    #[test]
    fn a_caution_notice_is_written_in_the_warning_colour_and_a_quiet_one_in_muted_text() {
        let tokens = Theme::builtin_dark();
        let quiet = notice(&tokens, NoticeTone::Quiet);
        let caution = notice(&tokens, NoticeTone::Caution);

        assert_eq!(quiet.text_color, Some(muted_text(&tokens)));
        assert_eq!(caution.text_color, Some(color(tokens.colors.warning)));
        assert_ne!(quiet.text_color, caution.text_color);
        assert_eq!(caution.border.color, color(tokens.colors.warning));
        assert_eq!(quiet.border.color, color(tokens.colors.border));
        assert_eq!(
            quiet.background, caution.background,
            "a caution is a sentence and an edge, not a second surface"
        );
        assert_eq!(
            quiet.background,
            Some(color(tokens.colors.surface_variant).into())
        );
        assert_eq!(quiet.border.radius, card_radius(&tokens));
        assert!((quiet.border.width - BORDER_WIDTH).abs() < f32::EPSILON);
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
        assert_eq!(style.border.radius, control_radius(&tokens));
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
