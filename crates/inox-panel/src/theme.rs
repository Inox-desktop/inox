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
//! Three sizes are *derived* rather than taken, because the token set has no
//! type scale and no border width, and are written down here so a reviewer can
//! see the derivation instead of finding a number in a view (see
//! `docs/quality/ui-guidelines.md` §1 and §2):
//!
//! - the body size is half the strip times the user's font scale, and the
//!   caption, heading and title roles are fixed fractions of it;
//! - a hairline is half the smallest spacing step, so a theme that scales its
//!   spacing scales its borders with it;
//! - the focus ring is the smallest spacing step, so a user who asks for more
//!   room also gets a thicker, more visible focus.
//!
//! The **glass material** of the menu bar and the dock is built here too, from
//! the `glass` token group alone (`docs/quality/ui-guidelines.md` §12): a pane
//! is a translucent tint, a highlight along its top edge and a shade along its
//! bottom one, and the blur radius it asks of the compositor is
//! [`glass_blur`]. No function in this file picks a tint of its own, so a theme
//! can make the glass warm, cool, dark or light without the panel changing.
//!
//! ADR 0018 explains why there is a copy of this file in every L3 component
//! instead of one shared crate: the duplication is the price of not adding a
//! same-layer Cargo dependency that would need its own ADR.

use iced::gradient::Linear;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Radians, Shadow, Theme, Vector};
use inox_config::{Rgba, Theme as Tokens};

use crate::view_model::PanelStatus;

/// How much of the body size a caption is.
const CAPTION_SIZE_SCALE: f32 = 0.875;

/// How much of the body size a heading is.
const HEADING_SIZE_SCALE: f32 = 1.25;

/// How much of the body size a title is.
const TITLE_SIZE_SCALE: f32 = 1.5;

/// The divisor that turns the smallest spacing step into a hairline.
const HAIRLINE_DIVISOR: f32 = 2.0;

/// How many `spacing.sm` steps the floating pane's shadow is blurred by.
///
/// A soft shadow is what lifts the dock off the wallpaper; the blur is derived
/// from the spacing scale rather than written down, so a theme with wider
/// spacing also gets a softer shadow.
const SHADOW_BLUR_STEPS: f32 = 2.0;

/// The angle of a pane's edge gradient, in radians: straight down, so offset
/// `0` of the gradient is the pane's top edge and `1` its bottom edge.
const PANE_EDGE_ANGLE: f32 = std::f32::consts::PI;

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

/// The body text size, in logical pixels.
///
/// `inox-config`'s typography tokens carry a family and the user's scale but no
/// size, so the panel derives one from its own height — half the strip, times
/// the user's scale. A user who doubles the scale gets text twice as tall, and
/// the geometry in [`crate::view`] is what keeps the result inside the strip
/// (acceptance Q02).
pub fn font_size(tokens: &Tokens, height: u16) -> f32 {
    f32::from(height) / 2.0 * tokens.typography.scale
}

/// The caption size: secondary information such as the clock or a tray item.
pub fn caption_size(tokens: &Tokens, height: u16) -> f32 {
    font_size(tokens, height) * CAPTION_SIZE_SCALE
}

/// The heading size: the sentence the panel shows about itself.
pub fn heading_size(tokens: &Tokens, height: u16) -> f32 {
    font_size(tokens, height) * HEADING_SIZE_SCALE
}

/// The title size: the largest of the four roles.
///
/// A strip has no room for a title today; the role exists so that every
/// component derives its type scale the same way (ui-guidelines §2).
pub fn title_size(tokens: &Tokens, height: u16) -> f32 {
    font_size(tokens, height) * TITLE_SIZE_SCALE
}

/// The thickness of a border, in logical pixels.
pub fn hairline(tokens: &Tokens) -> f32 {
    (f32::from(tokens.spacing.xs) / HAIRLINE_DIVISOR).max(0.0)
}

/// The width of the ring that shows which entry the keyboard is on.
pub fn focus_ring(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xs).max(0.0)
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

/// The accent colour, as the pressed state and the focus ring use it.
pub fn accent(tokens: &Tokens) -> Color {
    color(tokens.colors.accent)
}

/// The colour text takes when it sits on the accent colour.
pub fn on_accent(tokens: &Tokens) -> Color {
    color(tokens.colors.on_accent)
}

/// The colour of a divider or an outline.
pub fn border_color(tokens: &Tokens) -> Color {
    color(tokens.colors.border)
}

/// The corner radius of a control.
pub fn radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.sm).into()
}

/// The corner radius of a group: a card, a menu, a floating pane.
pub fn radius_md(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// The corner radius of a window-sized surface.
pub fn radius_lg(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.lg).into()
}

/// The blur radius the glass asks the compositor for, in logical pixels.
///
/// The panel does not blur anything itself: the backdrop blur is drawn by the
/// compositor in a later step (`docs/quality/ui-guidelines.md` §12), and `0`
/// means the pane is plain translucency with no blur requested.
pub fn glass_blur(tokens: &Tokens) -> u16 {
    tokens.glass.blur
}

/// The tint a frosted pane is filled with.
///
/// `strong` selects [`inox_config::Glass::tint_strong`], the tint of an
/// emphasised pane, such as a dock the keyboard navigation is on.
pub fn glass_tint(tokens: &Tokens, strong: bool) -> Color {
    color(if strong {
        tokens.glass.tint_strong
    } else {
        tokens.glass.tint
    })
}

/// The colour of the hairline along a pane's top edge: what reads as "glass".
pub fn glass_highlight(tokens: &Tokens) -> Color {
    color(tokens.glass.highlight)
}

/// The colour of the hairline along a pane's bottom edge.
pub fn glass_shade(tokens: &Tokens) -> Color {
    color(tokens.glass.shade)
}

/// The fill of a frosted pane: its tint, with a hairline highlight along the top
/// edge and — for a floating pane — a shade along the bottom one.
///
/// The two edges are part of the pane's own fill rather than two extra rows of
/// widgets. A row that spanned the pane would make the pane itself span the
/// surface it is drawn on, and a dock is only as wide as the tiles it holds; the
/// gradient is `height` tall, so `height` is what its hairline offsets are
/// derived from.
pub fn glass_fill(tokens: &Tokens, strong: bool, height: f32, shaded: bool) -> Background {
    let edge = (hairline(tokens) / height.max(1.0)).clamp(0.0, 0.5);
    let tint = glass_tint(tokens, strong);
    let mut gradient = Linear::new(Radians::from(PANE_EDGE_ANGLE))
        .add_stop(0.0, glass_highlight(tokens))
        .add_stop(edge, tint);

    if shaded {
        gradient = gradient
            .add_stop(1.0 - edge, tint)
            .add_stop(1.0, glass_shade(tokens));
    }

    Background::Gradient(gradient.into())
}

/// The surface a glass pane floats on: nothing at all.
///
/// The menu bar and the dock are drawn on a transparent surface so that the
/// wallpaper and the windows behind them show through the pane. This is the
/// structural "no design colour" case, not a token: there is deliberately no
/// colour here for a theme to set.
pub fn glass_surface(tokens: &Tokens) -> container::Style {
    container::Style {
        background: Some(Color::TRANSPARENT.into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// A frosted pane with square corners: the menu bar, which spans the screen
/// edge and so has no corner of its own.
pub fn glass_bar(tokens: &Tokens, height: f32) -> container::Style {
    container::Style {
        background: Some(glass_fill(tokens, false, height, false)),
        text_color: Some(text(tokens)),
        // Square corners are the default border: a bar that reaches both sides
        // of the output must not round the ends it does not own.
        ..container::Style::default()
    }
}

/// A frosted pane with the group radius, both edges and a soft shadow: the
/// floating dock.
pub fn glass_dock(tokens: &Tokens, strong: bool, height: f32) -> container::Style {
    container::Style {
        background: Some(glass_fill(tokens, strong, height, true)),
        text_color: Some(text(tokens)),
        border: Border {
            radius: radius_md(tokens),
            ..Border::default()
        },
        shadow: Shadow {
            color: glass_shade(tokens),
            offset: Vector::new(0.0, hairline(tokens)),
            blur_radius: f32::from(tokens.spacing.sm) * SHADOW_BLUR_STEPS,
        },
        ..container::Style::default()
    }
}

/// The mark under a dock tile: the accent colour when the group is active, and
/// nothing at all when it is not.
///
/// The inactive mark is still laid out — the view reserves the same slot for
/// every tile — so the mark's position never depends on which application is in
/// use, and the structural "no colour" value is the transparent sentinel rather
/// than a token.
pub fn dock_indicator(tokens: &Tokens, active: bool) -> container::Style {
    container::Style {
        background: Some(
            if active {
                accent(tokens)
            } else {
                Color::TRANSPARENT
            }
            .into(),
        ),
        ..container::Style::default()
    }
}

/// The panel strip itself.
///
/// The strip is the `surface` token: it is the layer the desktop sits behind,
/// and it draws no outline of its own — a border here would be a size the
/// tokens do not have and would eat into the entries' room.
pub fn panel(tokens: &Tokens) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// A hairline that separates two groups on the strip.
///
/// The view gives it its width and height; this decides only what colour it is,
/// so a divider can never be a colour nobody chose.
pub fn divider(tokens: &Tokens) -> container::Style {
    container::Style {
        background: Some(border_color(tokens).into()),
        ..container::Style::default()
    }
}

/// The colour of the sentence the panel shows about itself.
///
/// The panel's own state is the first thing a user has to be able to read, so
/// "the service is not available" is drawn in the warning colour; the ordinary
/// states are secondary information and stay muted.
pub fn notice_text(tokens: &Tokens, status: PanelStatus) -> Color {
    match status {
        PanelStatus::Unavailable => warning_text(tokens),
        PanelStatus::Starting | PanelStatus::Empty | PanelStatus::Ready => muted_text(tokens),
    }
}

/// The colour of a tray item's label.
pub fn tray_text(tokens: &Tokens, attention: bool) -> Color {
    if attention {
        warning_text(tokens)
    } else {
        text(tokens)
    }
}

/// The colour of the clock.
pub fn clock_text(tokens: &Tokens) -> Color {
    muted_text(tokens)
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
/// Five states have to be told apart at a glance, and each one is drawn with a
/// combination of background, text colour and outline so that no two of them
/// look the same (ui-guidelines §5):
///
/// | state | background | text | outline |
/// |---|---|---|---|
/// | resting | none | `text` | `border`, one hairline |
/// | hovered | `surface_variant` | `text` | `border`, one hairline |
/// | pressed | `accent` | `on_accent` | `on_accent`, one focus ring |
/// | focused | `surface_variant` | `accent` | `accent`, one focus ring |
/// | disabled | none | `text_muted` | none |
///
/// Three decisions in that table are worth naming:
///
/// - **The focus takes over the background.** An entry the keyboard reached is
///   drawn on the alternate surface with an accent label and an accent outline,
///   and it keeps all three while the pointer rests on it — the ordinary state
///   of a panel — which is what D01/Q01 ask for. Where the background is already
///   the accent colour, the outline is taken from the text colour: an accent ring
///   on an accent background is invisible.
/// - **A minimized entry keeps its outline, a disabled one does not.** Both are
///   muted, because neither is on screen, but a minimized window is still there
///   to be restored while a disabled entry is inert. Without the outline the two
///   would be the same pixels (ui-guidelines §5: no two states may look alike).
/// - **The entry the user is looking at stays on the accent colour under the
///   pointer**, with an `on_accent` hairline as the hover cue, so hovering the
///   application in use does not make it look like any other entry.
///
/// What a disabled entry *means* is the view's job: the reason shown is
/// [`crate::view_model::TaskButton::disabled_reason`], a sentence the user can
/// act on (D05).
pub fn task_button(
    tokens: &Tokens,
    emphasis: TaskEmphasis,
    focused: bool,
    status: button::Status,
) -> button::Style {
    entry_style(tokens, emphasis, focused, status, radius(tokens))
}

/// One dock tile: the same five states as a task entry, drawn as a square badge
/// of the group's first letter because the panel has no icon decoder.
///
/// The tile is always the *accent badge* — the identity colour of the
/// application, which ui-guidelines §5 allows as long as it is not the same
/// shape as a selection — and it takes the control radius of the token set. The
/// five interaction states are exactly a task entry's, so a tile hovered,
/// pressed, focused or disabled looks the way the rest of the panel looks in
/// that state.
pub fn dock_tile(tokens: &Tokens, focused: bool, status: button::Status) -> button::Style {
    entry_style(
        tokens,
        TaskEmphasis::Active,
        focused,
        status,
        radius(tokens),
    )
}

/// The state table shared by a task entry and a dock tile.
///
/// Only the corner radius differs between the two: the table itself is one
/// place so that the two surfaces can never drift apart on what "hovered",
/// "pressed", "focused" or "disabled" means (ui-guidelines §5).
fn entry_style(
    tokens: &Tokens,
    emphasis: TaskEmphasis,
    focused: bool,
    status: button::Status,
    radius: iced::border::Radius,
) -> button::Style {
    let accent = accent(tokens);
    let on_accent = on_accent(tokens);
    let variant = color(tokens.colors.surface_variant);
    let outline = |width: f32, color: Color| Border {
        radius,
        width,
        color,
    };
    let none = outline(0.0, Color::TRANSPARENT);
    let hairline_outline = outline(hairline(tokens), border_color(tokens));
    let focus_outline = outline(focus_ring(tokens), accent);

    let (background, foreground, border) = match status {
        button::Status::Disabled => (None, muted_text(tokens), none),
        button::Status::Pressed => (
            Some(accent.into()),
            on_accent,
            outline(focus_ring(tokens), on_accent),
        ),
        button::Status::Hovered if focused => (Some(variant.into()), accent, focus_outline),
        button::Status::Hovered => match emphasis {
            TaskEmphasis::Active => (
                Some(accent.into()),
                on_accent,
                outline(hairline(tokens), on_accent),
            ),
            TaskEmphasis::Idle => (Some(variant.into()), text(tokens), hairline_outline),
            TaskEmphasis::Minimized => (Some(variant.into()), muted_text(tokens), hairline_outline),
        },
        button::Status::Active if focused => (Some(variant.into()), accent, focus_outline),
        button::Status::Active => match emphasis {
            TaskEmphasis::Active => (Some(accent.into()), on_accent, none),
            TaskEmphasis::Idle => (None, text(tokens), hairline_outline),
            TaskEmphasis::Minimized => (None, muted_text(tokens), hairline_outline),
        },
    };

    button::Style {
        background,
        text_color: foreground,
        border,
        ..button::Style::default()
    }
}

/// The action that answers the panel's own notice, such as "Retry".
///
/// It is drawn as an outline in the warning colour rather than as a filled
/// button: it belongs to the sentence about a service that is not available, and
/// the panel's one filled control is the entry of the application the user is
/// looking at. Pressing it fills it, with the text taken from the window's own
/// background token so the label stays readable on the warning colour.
pub fn notice_action(tokens: &Tokens, status: button::Status) -> button::Style {
    let warning = warning_text(tokens);
    let border = Border {
        radius: radius(tokens),
        width: hairline(tokens),
        color: warning,
    };

    let (background, foreground) = match status {
        button::Status::Disabled => (None, muted_text(tokens)),
        button::Status::Pressed => (Some(warning.into()), color(tokens.colors.background)),
        button::Status::Hovered => (Some(color(tokens.colors.surface_variant).into()), warning),
        button::Status::Active => (None, warning),
    };

    button::Style {
        background,
        text_color: foreground,
        border,
        ..button::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Gradient;
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
        assert_eq!(
            style.border,
            Border::default(),
            "the strip draws no outline: the top-left pixel has to stay the surface token"
        );
    }

    #[test]
    fn the_four_type_roles_are_ordered() {
        let tokens = Tokens::builtin_dark();
        let height = 32;

        let caption = caption_size(&tokens, height);
        let body = font_size(&tokens, height);
        let heading = heading_size(&tokens, height);
        let title = title_size(&tokens, height);

        assert!(caption < body, "{caption} < {body}");
        assert!(body < heading, "{body} < {heading}");
        assert!(heading < title, "{heading} < {title}");
        assert!(
            (caption / body - CAPTION_SIZE_SCALE).abs() < f32::EPSILON,
            "the caption is a fixed fraction of the body size"
        );
    }

    #[test]
    fn every_type_role_follows_the_users_scale() {
        let mut tokens = Tokens::builtin_dark();
        let plain = font_size(&tokens, 32);

        tokens.typography.scale = 2.0;

        assert!((font_size(&tokens, 32) - plain * 2.0).abs() < f32::EPSILON);
        assert!(
            (caption_size(&tokens, 32) - plain * 2.0 * CAPTION_SIZE_SCALE).abs() < f32::EPSILON
        );
        assert!((title_size(&tokens, 32) - plain * 2.0 * TITLE_SIZE_SCALE).abs() < f32::EPSILON);
    }

    #[test]
    fn the_hairline_and_the_focus_ring_come_from_the_smallest_spacing_step() {
        let mut tokens = Tokens::builtin_dark();

        assert!((hairline(&tokens) - 1.0).abs() < f32::EPSILON);
        assert!((focus_ring(&tokens) - 2.0).abs() < f32::EPSILON);

        tokens.spacing.xs = 8;
        assert!((hairline(&tokens) - 4.0).abs() < f32::EPSILON);
        assert!((focus_ring(&tokens) - 8.0).abs() < f32::EPSILON);
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
        assert_eq!(disabled.border.width, 0.0);
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

        assert_eq!(focused.text_color, color(tokens.colors.accent));
        assert_eq!(focused.border.color, color(tokens.colors.accent));
        assert_eq!(focused.border.width, focus_ring(&tokens));
        assert_ne!(focused.text_color, idle.text_color);
        assert_ne!(focused.border.color, idle.border.color);
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
        assert_eq!(focused.border.width, focus_ring(&tokens));
    }

    #[test]
    fn the_five_interaction_states_are_pairwise_distinct() {
        let tokens = Tokens::builtin_dark();

        // Every emphasis is an entry a user can point at and press, so all
        // three of them have to carry five states nobody has to guess at
        // (ui-guidelines §5).
        for emphasis in [
            TaskEmphasis::Active,
            TaskEmphasis::Idle,
            TaskEmphasis::Minimized,
        ] {
            let states = [
                (
                    "resting",
                    task_button(&tokens, emphasis, false, button::Status::Active),
                ),
                (
                    "hovered",
                    task_button(&tokens, emphasis, false, button::Status::Hovered),
                ),
                (
                    "pressed",
                    task_button(&tokens, emphasis, false, button::Status::Pressed),
                ),
                (
                    "focused",
                    task_button(&tokens, emphasis, true, button::Status::Active),
                ),
                (
                    "disabled",
                    task_button(&tokens, emphasis, false, button::Status::Disabled),
                ),
            ];

            for (index, (name, style)) in states.iter().enumerate() {
                for (other_name, other) in &states[index + 1..] {
                    assert_ne!(
                        (
                            style.background,
                            style.text_color,
                            style.border.width,
                            style.border.color
                        ),
                        (
                            other.background,
                            other.text_color,
                            other.border.width,
                            other.border.color
                        ),
                        "{emphasis:?}: {name} and {other_name} have to be told apart"
                    );
                }
            }
        }
    }

    #[test]
    fn a_resting_entry_has_the_hairline_outline_and_a_disabled_one_does_not() {
        let tokens = Tokens::builtin_dark();
        let resting = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Active);
        let disabled = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Disabled);

        assert_eq!(resting.border.width, hairline(&tokens));
        assert_eq!(resting.border.color, border_color(&tokens));
        assert_eq!(disabled.border.width, 0.0);
        assert_ne!(resting.border.width, disabled.border.width);
    }

    #[test]
    fn a_minimized_entry_is_not_drawn_like_a_disabled_one() {
        let tokens = Tokens::builtin_dark();
        let minimized = task_button(
            &tokens,
            TaskEmphasis::Minimized,
            false,
            button::Status::Active,
        );
        let disabled = task_button(
            &tokens,
            TaskEmphasis::Minimized,
            false,
            button::Status::Disabled,
        );

        // Both are muted — neither is on screen — but a minimized window can
        // still be restored, so it keeps the outline a disabled entry loses.
        assert_eq!(minimized.text_color, disabled.text_color);
        assert_eq!(minimized.background, disabled.background);
        assert_ne!(minimized.border.width, disabled.border.width);
        assert!(minimized.border.width > 0.0);
    }

    #[test]
    fn the_entry_in_use_keeps_its_accent_colour_under_the_pointer() {
        let tokens = Tokens::builtin_dark();
        let resting = task_button(&tokens, TaskEmphasis::Active, false, button::Status::Active);
        let hovered = task_button(
            &tokens,
            TaskEmphasis::Active,
            false,
            button::Status::Hovered,
        );

        assert_eq!(resting.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(hovered.background, resting.background);
        assert_ne!(
            hovered.border.width, resting.border.width,
            "the hover still has to be visible on the entry the user is looking at"
        );
    }

    #[test]
    fn a_pressed_entry_fills_with_the_accent_colour() {
        let tokens = Tokens::builtin_dark();
        let pressed = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Pressed);
        let hovered = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Hovered);

        assert_eq!(pressed.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(pressed.text_color, color(tokens.colors.on_accent));
        assert_ne!(pressed.background, hovered.background);
    }

    #[test]
    fn a_button_keeps_the_token_radius() {
        let tokens = Tokens::builtin_dark();
        let style = task_button(&tokens, TaskEmphasis::Idle, false, button::Status::Active);

        assert_eq!(style.border.radius, radius(&tokens));
    }

    #[test]
    fn the_notice_action_is_an_outline_in_the_warning_colour() {
        let tokens = Tokens::builtin_dark();
        let rest = notice_action(&tokens, button::Status::Active);
        let hovered = notice_action(&tokens, button::Status::Hovered);
        let pressed = notice_action(&tokens, button::Status::Pressed);
        let disabled = notice_action(&tokens, button::Status::Disabled);

        assert_eq!(rest.text_color, warning_text(&tokens));
        assert_eq!(rest.border.color, warning_text(&tokens));
        assert_eq!(rest.border.width, hairline(&tokens));
        assert_eq!(rest.background, None);
        assert_ne!(hovered.background, rest.background);
        assert_eq!(pressed.background, Some(warning_text(&tokens).into()));
        assert_eq!(disabled.text_color, muted_text(&tokens));
    }

    #[test]
    fn the_clock_is_muted_and_a_tray_item_asking_for_attention_is_not() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(clock_text(&tokens), muted_text(&tokens));
        assert_eq!(tray_text(&tokens, false), text(&tokens));
        assert_eq!(tray_text(&tokens, true), warning_text(&tokens));
    }

    #[test]
    fn the_panel_state_decides_the_notice_colour() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(
            notice_text(&tokens, PanelStatus::Unavailable),
            warning_text(&tokens)
        );
        for status in [
            PanelStatus::Starting,
            PanelStatus::Empty,
            PanelStatus::Ready,
        ] {
            assert_eq!(notice_text(&tokens, status), muted_text(&tokens));
        }
    }

    #[test]
    fn a_divider_is_the_border_token() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(
            divider(&tokens).background,
            Some(border_color(&tokens).into())
        );
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

    /// The colour stops of a pane's fill, in the order they are drawn.
    fn stops(background: &Background) -> Vec<iced::gradient::ColorStop> {
        match background {
            Background::Gradient(Gradient::Linear(linear)) => {
                linear.stops.into_iter().flatten().collect()
            }
            other => panic!("expected a gradient pane, got {other:?}"),
        }
    }

    fn stop_colours(background: &Background) -> Vec<Color> {
        stops(background)
            .into_iter()
            .map(|stop| stop.color)
            .collect()
    }

    #[test]
    fn a_menu_bar_is_a_tint_with_a_highlight_along_its_top_edge_only() {
        let tokens = Tokens::builtin_dark();
        let height = 28.0;
        let bar = glass_bar(&tokens, height);
        let stops = stops(&glass_fill(&tokens, false, height, false));

        assert_eq!(bar.text_color, Some(text(&tokens)));
        assert_eq!(
            bar.border,
            Border::default(),
            "the bar is square: it reaches both sides of the output"
        );
        assert_eq!(
            bar.shadow,
            Shadow::default(),
            "a bar along the screen edge carries no shadow"
        );
        assert_eq!(
            stop_colours(&bar.background.expect("a bar has a fill")),
            vec![glass_highlight(&tokens), glass_tint(&tokens, false)],
            "the fill starts with the highlight and is the tint from there on"
        );
        assert_eq!(stops[0].offset, 0.0);
        assert!(
            (stops[1].offset - hairline(&tokens) / height).abs() < f32::EPSILON,
            "the highlight is exactly one hairline of the pane's height"
        );
        assert!(
            !stop_colours(&glass_fill(&tokens, false, height, false))
                .contains(&glass_shade(&tokens)),
            "a bar that reaches the screen edge draws no bottom shade"
        );
    }

    #[test]
    fn a_floating_pane_takes_the_group_radius_a_highlight_a_shade_and_a_soft_shadow() {
        let tokens = Tokens::builtin_dark();
        let height = 70.0;
        let dock = glass_dock(&tokens, false, height);
        let stops = stops(&glass_fill(&tokens, false, height, true));

        assert_eq!(dock.border.radius, radius_md(&tokens));
        assert_eq!(dock.border.width, 0.0);
        assert_eq!(dock.shadow.color, glass_shade(&tokens));
        assert!(
            dock.shadow.blur_radius > 0.0,
            "a floating pane is lifted off the wallpaper by a soft shadow"
        );
        assert_ne!(
            dock.shadow.color,
            Color::TRANSPARENT,
            "the shadow is derived from the glass shade token, not written down"
        );
        assert_eq!(dock.shadow.offset.y, hairline(&tokens));

        assert_eq!(
            stop_colours(&dock.background.expect("a dock has a fill")),
            vec![
                glass_highlight(&tokens),
                glass_tint(&tokens, false),
                glass_tint(&tokens, false),
                glass_shade(&tokens),
            ],
            "the pane is highlighted at the top, tinted in the middle and shaded \
             at the bottom"
        );
        assert_eq!(stops[0].offset, 0.0);
        assert_eq!(stops[3].offset, 1.0);
        assert!((stops[2].offset - (1.0 - hairline(&tokens) / height)).abs() < f32::EPSILON);

        let strong = glass_dock(&tokens, true, height);
        assert_ne!(dock.background, strong.background);
        assert!(
            stop_colours(&strong.background.expect("a dock has a fill"))
                .contains(&glass_tint(&tokens, true)),
            "the emphasised pane uses the stronger tint"
        );
        assert_eq!(glass_tint(&tokens, false), color(tokens.glass.tint));
        assert_eq!(glass_tint(&tokens, true), color(tokens.glass.tint_strong));
        assert_ne!(
            glass_tint(&tokens, false),
            glass_tint(&tokens, true),
            "the emphasised pane has its own tint"
        );
    }

    #[test]
    fn the_two_edges_of_a_pane_come_from_the_two_glass_edge_tokens() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(glass_highlight(&tokens), color(tokens.glass.highlight));
        assert_eq!(glass_shade(&tokens), color(tokens.glass.shade));
        assert_ne!(glass_highlight(&tokens), glass_shade(&tokens));
    }

    #[test]
    fn a_glass_surface_is_structurally_transparent() {
        let tokens = Tokens::builtin_dark();
        let surface = glass_surface(&tokens);

        assert_eq!(
            surface.background,
            Some(Color::TRANSPARENT.into()),
            "the backdrop has to show through the surface the pane floats on"
        );
        assert_ne!(
            surface.background,
            Some(color(tokens.colors.surface).into()),
            "a glass surface is not the opaque `surface` token"
        );
        assert_eq!(surface.text_color, Some(text(&tokens)));
    }

    #[test]
    fn the_blur_the_glass_asks_for_is_the_token() {
        let mut tokens = Tokens::builtin_dark();

        assert_eq!(glass_blur(&tokens), tokens.glass.blur);
        assert!(glass_blur(&tokens) > 0);

        tokens.glass.blur = 0;
        assert_eq!(
            glass_blur(&tokens),
            0,
            "a theme may ask for plain translucency and no backdrop blur"
        );
    }

    #[test]
    fn a_dock_tile_draws_the_same_five_states_as_a_task_entry() {
        let tokens = Tokens::builtin_dark();
        let states = [
            ("resting", dock_tile(&tokens, false, button::Status::Active)),
            (
                "hovered",
                dock_tile(&tokens, false, button::Status::Hovered),
            ),
            (
                "pressed",
                dock_tile(&tokens, false, button::Status::Pressed),
            ),
            ("focused", dock_tile(&tokens, true, button::Status::Active)),
            (
                "disabled",
                dock_tile(&tokens, false, button::Status::Disabled),
            ),
        ];

        for (index, (name, style)) in states.iter().enumerate() {
            for (other_name, other) in &states[index + 1..] {
                assert_ne!(
                    (
                        style.background,
                        style.text_color,
                        style.border.width,
                        style.border.color
                    ),
                    (
                        other.background,
                        other.text_color,
                        other.border.width,
                        other.border.color
                    ),
                    "{name} and {other_name} have to be told apart"
                );
            }
        }

        assert_eq!(
            states[0].1.border.radius,
            radius(&tokens),
            "a tile is a control, so it takes the control radius"
        );
        assert_eq!(
            states[0].1.background,
            Some(accent(&tokens).into()),
            "the tile is the application's badge, drawn on the accent colour"
        );
        assert_eq!(
            states[4].1.background, None,
            "a disabled tile carries no fill"
        );
    }

    #[test]
    fn the_dock_indicator_is_the_accent_colour_only_for_an_active_group() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(
            dock_indicator(&tokens, true).background,
            Some(accent(&tokens).into())
        );
        assert_eq!(
            dock_indicator(&tokens, false).background,
            Some(Color::TRANSPARENT.into()),
            "the inactive mark still occupies its slot, it just draws nothing"
        );
    }
}
