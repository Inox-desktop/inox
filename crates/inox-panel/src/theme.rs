//! Inox design tokens, turned into the values the QML renderer draws with.
//!
//! Root AGENTS.md §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! QML, meanwhile, is a renderer in this crate: it is handed a JSON
//! projection whose colours are `#AARRGGBB` strings and whose sizes are logical
//! pixels. This module is the join between the two, and it is the only place in
//! the crate where a token becomes a value a widget draws with.
//!
//! Every function here takes the tokens as an argument rather than reaching for
//! a global, so the view can be projected with the same theme the rest of the
//! panel is drawn with — and so a test can project the same surface under the
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
//! bottom one, and the blur radius and refraction band it asks of the compositor
//! come from the same group ([`glass_material`], [`glass_blur`] and the
//! refraction accessors). No function in this file picks a tint or a band of its
//! own, so a theme can make the glass warm, cool, dark or light without the panel
//! changing.
//!
//! the L3 component architecture explains why there is a copy of this file in every L3 component
//! instead of one shared crate: the duplication is the price of not adding a
//! same-layer Cargo dependency that would need its own ADR.

use inox_config::{Rgba, Theme as Tokens};
use inox_glass::{Corners, Material, Refraction};

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
pub const SHADOW_BLUR_STEPS: f32 = 2.0;

/// The structural "no colour" value: nothing is painted here.
///
/// This is not a design colour and no theme can set it — it is what a
/// transparent surface, an inactive dock mark or an absent outline is drawn as.
/// It matches `Color::TRANSPARENT`'s premultiplied `#00000000`.
pub const TRANSPARENT: &str = "#00000000";

/// A token colour, as the QML renderer wants it: Qt's `#AARRGGBB`.
///
/// Qt parses `#AARRGGBB` as premultiplied-independent ARGB, which is the same
/// value the token holds: the alpha comes first so a translucent tint keeps its
/// colour instead of being read as `#RRGGBBAA`.
pub fn qml_color(value: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        value.a, value.r, value.g, value.b
    )
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
pub fn text(tokens: &Tokens) -> String {
    qml_color(tokens.colors.text)
}

/// Colour of secondary information, such as the clock or a minimized label.
pub fn muted_text(tokens: &Tokens) -> String {
    qml_color(tokens.colors.text_muted)
}

/// Colour for something the user should look at: an item asking for attention,
/// or a service that is not available.
pub fn warning_text(tokens: &Tokens) -> String {
    qml_color(tokens.colors.warning)
}

/// The accent colour, as the pressed state and the focus ring use it.
pub fn accent(tokens: &Tokens) -> String {
    qml_color(tokens.colors.accent)
}

/// The colour text takes when it sits on the accent colour.
pub fn on_accent(tokens: &Tokens) -> String {
    qml_color(tokens.colors.on_accent)
}

/// The colour of a divider or an outline.
pub fn border_color(tokens: &Tokens) -> String {
    qml_color(tokens.colors.border)
}

/// The corner radius of a control.
pub fn radius(tokens: &Tokens) -> f32 {
    f32::from(tokens.radii.sm)
}

/// The corner radius of a group: a card, a menu, a floating pane.
pub fn radius_md(tokens: &Tokens) -> f32 {
    f32::from(tokens.radii.md)
}

/// The font family the projection names.
pub fn font_family(tokens: &Tokens) -> String {
    tokens.typography.font_family.clone()
}

/// The blur radius the glass asks the compositor for, in logical pixels.
///
/// The panel does not blur anything itself: the backdrop blur is drawn by the
/// compositor in a later step (`docs/quality/ui-guidelines.md` §12), and `0`
/// means the pane is plain translucency with no blur requested.
pub fn glass_blur(tokens: &Tokens) -> u16 {
    tokens.glass.blur
}

/// The refraction band thickness the glass asks the compositor for, in logical
/// pixels.
///
/// The lens itself is drawn by the compositor (ADR 0040): this is the width of
/// the band along the inside of the pane's edge in which the backdrop is
/// displaced. `0` means the pane asks for no refraction at all, which is the
/// explicit "v1 frosted pane" value and not an error.
pub fn glass_refraction(tokens: &Tokens) -> u16 {
    tokens.glass.refraction
}

/// The peak displacement at the pane's outline, in logical pixels.
///
/// The displacement is strongest on the contour and fades to nothing where the
/// [`glass_refraction`] band reaches the interior. `0` disables the lens just
/// like a band of `0` does.
pub fn glass_refraction_amount(tokens: &Tokens) -> u16 {
    tokens.glass.refraction_amount
}

/// Whether the radial "depth" component of the lens is asked for.
pub fn glass_depth(tokens: &Tokens) -> bool {
    tokens.glass.depth
}

/// Whether the lens splits its refracted edge per channel (the chromatic
/// fringe), which the compositor only draws when it advertises the feature.
pub fn glass_dispersion(tokens: &Tokens) -> bool {
    tokens.glass.dispersion
}

/// The corner radius the compositor must use for the pane's refracting shape.
///
/// ADR 0040 §4 makes the pane's shape the `radii.md` token — the radius the QML
/// pane is drawn with ([`radius_md`]) — because the refracted outline has to be
/// the shape the user sees, not a second number that can drift from it.
pub fn pane_corner_radius(tokens: &Tokens) -> u16 {
    tokens.radii.md
}

/// The complete liquid-glass material, from the tokens alone.
///
/// This is the one place the crate turns `[glass]` into the material the
/// compositor is asked for: the blur, the four refraction tokens and the pane's
/// own corner radius. [`inox_glass::Material::from_tokens`] is the library's
/// translation of the same tokens, and a test pins the two together, so a token
/// that stops being read here cannot go unnoticed.
pub fn glass_material(tokens: &Tokens) -> Material {
    Material::new(
        glass_blur(tokens),
        Corners::uniform(pane_corner_radius(tokens)),
        Refraction::new(
            glass_refraction(tokens),
            glass_refraction_amount(tokens),
            glass_depth(tokens),
            glass_dispersion(tokens),
        ),
    )
}

/// The tint a frosted pane is filled with.
///
/// `strong` selects [`inox_config::Glass::tint_strong`], the tint of an
/// emphasised pane, such as a dock the keyboard navigation is on.
pub fn glass_tint(tokens: &Tokens, strong: bool) -> String {
    qml_color(if strong {
        tokens.glass.tint_strong
    } else {
        tokens.glass.tint
    })
}

/// The colour of the hairline along a pane's top edge: what reads as "glass".
pub fn glass_highlight(tokens: &Tokens) -> String {
    qml_color(tokens.glass.highlight)
}

/// The colour of the hairline along a pane's bottom edge.
pub fn glass_shade(tokens: &Tokens) -> String {
    qml_color(tokens.glass.shade)
}

/// A frosted pane, as the values `GlassPane` binds.
///
/// This is the whole of the panel's contribution to drawing glass. The four
/// tokens plus the hairline width and the corner radius *are* the pane: the
/// component draws the tint, the one-hairline highlight along the top edge and
/// the one-hairline shade along the bottom edge, and it is the same component —
/// compiled from `inox-glass`'s assets by this crate's build script — that every
/// other consumer draws with (ADR 0040 §3).
///
/// Before the component library existed the panel laid the same fill out itself
/// as a stack of solid bands, because a renderer that branched on a gradient
/// would be making a decision. [`GlassPane`] removes that work without moving the
/// decision: the bands and the component draw the same material, and the tokens
/// are still read only here.
///
/// [`GlassPane`]: inox_glass::QML_FILES
#[derive(Debug, Clone, PartialEq)]
pub struct PaneStyle {
    /// The tile of the material: `glass.tint`, or `glass.tint_strong` for a pane
    /// the keyboard is on.
    pub tint: String,
    /// The hairline along the top edge — what reads as "glass".
    pub highlight: String,
    /// The darkening along the bottom edge.
    pub shade: String,
    /// Thickness of the two hairlines, in logical pixels.
    pub hairline: f32,
    /// The corner radius, in logical pixels.
    pub radius: f32,
    /// Whether the caller wants the two edge hairlines drawn at all.
    pub highlighted: bool,
    pub shaded: bool,
}

/// The material of a frosted pane: its tint, both hairline tokens and the radius.
///
/// `strong` selects [`inox_config::Glass::tint_strong`] — the tint of an
/// emphasised pane, such as a dock the keyboard navigation is on. `shaded` is
/// false for a pane that reaches the edge of the output (the menu bar), which
/// draws no bottom shade because there is nothing under it to darken against.
///
/// `radius` is passed rather than read here because the two glass surfaces do not
/// share one: the dock is a floating pill and takes `radii.md`, while the menu bar
/// is anchored to the screen edge and is square.
pub fn pane_style(tokens: &Tokens, strong: bool, radius: f32, shaded: bool) -> PaneStyle {
    PaneStyle {
        tint: glass_tint(tokens, strong),
        highlight: glass_highlight(tokens),
        shade: glass_shade(tokens),
        hairline: hairline(tokens),
        radius,
        highlighted: true,
        shaded,
    }
}

/// The shade a floating pane's shadow is drawn in.
pub fn glass_shadow(tokens: &Tokens) -> String {
    glass_shade(tokens)
}

/// The offset of a floating pane's shadow, in logical pixels.
pub fn glass_shadow_offset(tokens: &Tokens) -> f32 {
    hairline(tokens)
}

/// The blur radius of a floating pane's shadow, in logical pixels.
pub fn glass_shadow_blur(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.sm) * SHADOW_BLUR_STEPS
}

/// The panel strip itself.
///
/// The strip is the `surface` token: it is the layer the desktop sits behind,
/// and it draws no outline of its own — a border here would be a size the
/// tokens do not have and would eat into the entries' room.
pub fn panel_fill(tokens: &Tokens) -> String {
    qml_color(tokens.colors.surface)
}

/// The colour that separates two groups on the strip.
///
/// The view gives the divider its width and height; this decides only what
/// colour it is, so a divider can never be a colour nobody chose.
pub fn divider_color(tokens: &Tokens) -> String {
    border_color(tokens)
}

/// The colour of the sentence the panel shows about itself.
///
/// The panel's own state is the first thing a user has to be able to read, so
/// "the service is not available" is drawn in the warning colour; the ordinary
/// states are secondary information and stay muted.
pub fn notice_text(tokens: &Tokens, status: PanelStatus) -> String {
    match status {
        PanelStatus::Unavailable => warning_text(tokens),
        PanelStatus::Starting | PanelStatus::Empty | PanelStatus::Ready => muted_text(tokens),
    }
}

/// The colour of a tray item's label.
pub fn tray_text(tokens: &Tokens, attention: bool) -> String {
    if attention {
        warning_text(tokens)
    } else {
        text(tokens)
    }
}

/// The colour of the clock.
pub fn clock_text(tokens: &Tokens) -> String {
    muted_text(tokens)
}

/// How a task button is emphasized.
///
/// [`InteractionStatus`] says what the pointer is doing; this says what the
/// *window* is doing, which the panel draws as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEmphasis {
    /// One of the group's windows is the focused one.
    Active,
    /// The group has a window on screen, and it is not the focused one.
    Idle,
    /// Every window of the group is minimized.
    Minimized,
}

/// What the pointer is doing to an entry.
///
/// The four states a widget has to tell apart, independent of the toolkit: the
/// panel's pointer events (`src/layer.rs`) decide which one an entry is in, and
/// [`entry_style`] decides what it looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionStatus {
    /// Nothing is pointing at it and nothing is pressed on it.
    Active,
    /// The pointer rests on it.
    Hovered,
    /// A button is pressed on it.
    Pressed,
    /// It cannot be pressed.
    Disabled,
}

/// Everything an entry draws with: its fill, its label colour and its outline.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryStyle {
    /// The fill, or `None` for "no fill at all" (the resting panel look).
    pub background: Option<String>,
    /// The colour of the label.
    pub text_color: String,
    /// The outline's width, in logical pixels; `0` means no outline.
    pub border_width: f32,
    /// The outline's colour.
    pub border_color: String,
    /// The corner radius of the entry.
    pub radius: f32,
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
    status: InteractionStatus,
) -> EntryStyle {
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
pub fn dock_tile(tokens: &Tokens, focused: bool, status: InteractionStatus) -> EntryStyle {
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
    status: InteractionStatus,
    radius: f32,
) -> EntryStyle {
    let accent = accent(tokens);
    let on_accent = on_accent(tokens);
    let variant = qml_color(tokens.colors.surface_variant);
    let none = (0.0, TRANSPARENT.to_owned());
    let hairline_outline = (hairline(tokens), border_color(tokens));
    let focus_outline = (focus_ring(tokens), accent.clone());

    let (background, foreground, (border_width, border_color)) = match status {
        InteractionStatus::Disabled => (None, muted_text(tokens), none),
        InteractionStatus::Pressed => (
            Some(accent.clone()),
            on_accent.clone(),
            (focus_ring(tokens), on_accent),
        ),
        InteractionStatus::Hovered if focused => (Some(variant), accent.clone(), focus_outline),
        InteractionStatus::Hovered => match emphasis {
            TaskEmphasis::Active => (
                Some(accent.clone()),
                on_accent.clone(),
                (hairline(tokens), on_accent),
            ),
            TaskEmphasis::Idle => (Some(variant), text(tokens), hairline_outline),
            TaskEmphasis::Minimized => (Some(variant), muted_text(tokens), hairline_outline),
        },
        InteractionStatus::Active if focused => (Some(variant), accent, focus_outline),
        InteractionStatus::Active => match emphasis {
            TaskEmphasis::Active => (Some(accent), on_accent, none),
            TaskEmphasis::Idle => (None, text(tokens), hairline_outline),
            TaskEmphasis::Minimized => (None, muted_text(tokens), hairline_outline),
        },
    };

    EntryStyle {
        background,
        text_color: foreground,
        border_width,
        border_color,
        radius,
    }
}

/// The action that answers the panel's own notice, such as "Retry".
///
/// It is drawn as an outline in the warning colour rather than as a filled
/// button: it belongs to the sentence about a service that is not available, and
/// the panel's one filled control is the entry of the application the user is
/// looking at. Pressing it fills it, with the text taken from the window's own
/// background token so the label stays readable on the warning colour.
pub fn notice_action(tokens: &Tokens, status: InteractionStatus) -> EntryStyle {
    let warning = warning_text(tokens);

    let (background, foreground) = match status {
        InteractionStatus::Disabled => (None, muted_text(tokens)),
        InteractionStatus::Pressed => (Some(warning.clone()), qml_color(tokens.colors.background)),
        InteractionStatus::Hovered => (Some(qml_color(tokens.colors.surface_variant)), warning),
        InteractionStatus::Active => (None, warning),
    };

    EntryStyle {
        background,
        text_color: foreground,
        border_width: hairline(tokens),
        border_color: warning_text(tokens),
        radius: radius(tokens),
    }
}

/// The workspace indicator that opens the task area.
///
/// It is a control, not a decoration: it says which workspace the windows on the
/// strip belong to, and a click asks for the next one (ADR 0045 §4). It is drawn
/// as an *outline* rather than as a filled entry, because the strip's one filled
/// control is the application the user is looking at — a second filled box would
/// compete with it for the same "this is what is in use" reading.
///
/// The four interaction states are the panel's, not the indicator's: hovering it
/// takes the alternate surface, pressing it takes the accent, and a disabled one
/// has no fill and no outline (D05: a disabled control says why in its tooltip).
pub fn workspace_indicator(tokens: &Tokens, status: InteractionStatus) -> EntryStyle {
    let (background, foreground) = match status {
        InteractionStatus::Disabled => (None, muted_text(tokens)),
        InteractionStatus::Pressed => (Some(accent(tokens)), on_accent(tokens)),
        InteractionStatus::Hovered => {
            (Some(qml_color(tokens.colors.surface_variant)), text(tokens))
        }
        InteractionStatus::Active => (None, muted_text(tokens)),
    };

    EntryStyle {
        background,
        text_color: foreground,
        border_width: hairline(tokens),
        border_color: border_color(tokens),
        radius: radius(tokens),
    }
}

/// The mark under a dock tile: the accent colour when the group is active, and
/// nothing at all when it is not.
///
/// The inactive mark is still laid out — the view reserves the same slot for
/// every tile — so the mark's position never depends on which application is in
/// use, and the structural "no colour" value is the transparent sentinel rather
/// than a token.
pub fn dock_indicator(tokens: &Tokens, active: bool) -> String {
    if active {
        accent(tokens)
    } else {
        TRANSPARENT.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::{Rgba, Theme as Tokens};

    #[test]
    fn a_token_colour_keeps_its_channels() {
        let colour = qml_color(Rgba {
            r: 0x23,
            g: 0x26,
            b: 0x2e,
            a: 0xff,
        });

        assert_eq!(colour, "#FF23262E");
    }

    #[test]
    fn the_alpha_comes_first_so_a_translucent_token_keeps_its_colour() {
        // `#RRGGBBAA` would make Qt read the alpha as a blue channel; the glass
        // tints are translucent, so the order is not cosmetic.
        assert_eq!(
            qml_color(Rgba {
                r: 0x10,
                g: 0x20,
                b: 0x30,
                a: 0x80,
            }),
            "#80102030"
        );
        assert_eq!(
            qml_color(Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            }),
            TRANSPARENT
        );
    }

    #[test]
    fn the_light_and_dark_themes_do_not_draw_the_same_strip() {
        let light = panel_fill(&Tokens::builtin_light());
        let dark = panel_fill(&Tokens::builtin_dark());

        assert_ne!(light, dark, "the two shipped themes are different themes");
        assert_ne!(dark, TRANSPARENT, "a strip is painted, not left blank");
    }

    #[test]
    fn the_panel_background_is_the_surface_token() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(panel_fill(&tokens), qml_color(tokens.colors.surface));
        assert_ne!(panel_fill(&tokens), TRANSPARENT);
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
    fn the_font_size_follows_the_panel_height_and_the_user_scale() {
        let mut tokens = Tokens::builtin_dark();
        assert!((font_size(&tokens, 32) - 16.0).abs() < f32::EPSILON);

        tokens.typography.scale = 2.0;
        assert!(
            (font_size(&tokens, 32) - 32.0).abs() < f32::EPSILON,
            "font scaling is a token, not a hard-coded size"
        );
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
        let active = task_button(
            &tokens,
            TaskEmphasis::Active,
            false,
            InteractionStatus::Active,
        );
        let idle = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Active,
        );

        assert_eq!(active.background, Some(accent(&tokens)));
        assert_eq!(active.text_color, on_accent(&tokens));
        assert_eq!(idle.background, None);
        assert_ne!(active.text_color, idle.text_color);
    }

    #[test]
    fn a_disabled_button_looks_different_from_an_enabled_one() {
        let tokens = Tokens::builtin_dark();
        let enabled = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Active,
        );
        let disabled = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Disabled,
        );

        assert_ne!(enabled.text_color, disabled.text_color);
        assert_eq!(disabled.background, None);
        assert_eq!(disabled.text_color, muted_text(&tokens));
        assert_eq!(disabled.border_width, 0.0);
    }

    #[test]
    fn a_minimized_button_is_muted() {
        let tokens = Tokens::builtin_dark();
        let minimized = task_button(
            &tokens,
            TaskEmphasis::Minimized,
            false,
            InteractionStatus::Active,
        );

        assert_eq!(minimized.text_color, muted_text(&tokens));
        assert_eq!(minimized.background, None);
    }

    #[test]
    fn the_entry_navigation_reached_is_visibly_different() {
        let tokens = Tokens::builtin_dark();
        let focused = task_button(&tokens, TaskEmphasis::Idle, true, InteractionStatus::Active);
        let idle = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Active,
        );

        assert_eq!(focused.text_color, accent(&tokens));
        assert_eq!(focused.border_color, accent(&tokens));
        assert_eq!(focused.border_width, focus_ring(&tokens));
        assert_ne!(focused.text_color, idle.text_color);
        assert_ne!(focused.border_color, idle.border_color);
    }

    #[test]
    fn a_hovered_entry_keeps_its_navigation_focus_visible() {
        let tokens = Tokens::builtin_dark();
        let focused = task_button(
            &tokens,
            TaskEmphasis::Idle,
            true,
            InteractionStatus::Hovered,
        );
        let hovered = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Hovered,
        );
        let plain = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Active,
        );

        // The pointer rests on a panel all the time: the entry the user reached
        // with explicit navigation has to stay readable there (Q01/D01).
        assert_ne!(focused.text_color, hovered.text_color);
        assert_eq!(focused.text_color, accent(&tokens));
        assert_eq!(focused.background, hovered.background);
        assert_ne!(hovered.background, plain.background);
        assert_eq!(focused.border_width, focus_ring(&tokens));
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
                    task_button(&tokens, emphasis, false, InteractionStatus::Active),
                ),
                (
                    "hovered",
                    task_button(&tokens, emphasis, false, InteractionStatus::Hovered),
                ),
                (
                    "pressed",
                    task_button(&tokens, emphasis, false, InteractionStatus::Pressed),
                ),
                (
                    "focused",
                    task_button(&tokens, emphasis, true, InteractionStatus::Active),
                ),
                (
                    "disabled",
                    task_button(&tokens, emphasis, false, InteractionStatus::Disabled),
                ),
            ];

            for (index, (name, style)) in states.iter().enumerate() {
                for (other_name, other) in &states[index + 1..] {
                    assert_ne!(
                        (
                            &style.background,
                            &style.text_color,
                            style.border_width,
                            &style.border_color
                        ),
                        (
                            &other.background,
                            &other.text_color,
                            other.border_width,
                            &other.border_color
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
        let resting = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Active,
        );
        let disabled = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Disabled,
        );

        assert_eq!(resting.border_width, hairline(&tokens));
        assert_eq!(resting.border_color, border_color(&tokens));
        assert_eq!(disabled.border_width, 0.0);
        assert_ne!(resting.border_width, disabled.border_width);
    }

    #[test]
    fn a_minimized_entry_is_not_drawn_like_a_disabled_one() {
        let tokens = Tokens::builtin_dark();
        let minimized = task_button(
            &tokens,
            TaskEmphasis::Minimized,
            false,
            InteractionStatus::Active,
        );
        let disabled = task_button(
            &tokens,
            TaskEmphasis::Minimized,
            false,
            InteractionStatus::Disabled,
        );

        // Both are muted — neither is on screen — but a minimized window can
        // still be restored, so it keeps the outline a disabled entry loses.
        assert_eq!(minimized.text_color, disabled.text_color);
        assert_eq!(minimized.background, disabled.background);
        assert_ne!(minimized.border_width, disabled.border_width);
        assert!(minimized.border_width > 0.0);
    }

    #[test]
    fn the_entry_in_use_keeps_its_accent_colour_under_the_pointer() {
        let tokens = Tokens::builtin_dark();
        let resting = task_button(
            &tokens,
            TaskEmphasis::Active,
            false,
            InteractionStatus::Active,
        );
        let hovered = task_button(
            &tokens,
            TaskEmphasis::Active,
            false,
            InteractionStatus::Hovered,
        );

        assert_eq!(resting.background, Some(accent(&tokens)));
        assert_eq!(hovered.background, resting.background);
        assert_ne!(
            hovered.border_width, resting.border_width,
            "the hover still has to be visible on the entry the user is looking at"
        );
    }

    #[test]
    fn a_pressed_entry_fills_with_the_accent_colour() {
        let tokens = Tokens::builtin_dark();
        let pressed = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Pressed,
        );
        let hovered = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Hovered,
        );

        assert_eq!(pressed.background, Some(accent(&tokens)));
        assert_eq!(pressed.text_color, on_accent(&tokens));
        assert_ne!(pressed.background, hovered.background);
    }

    #[test]
    fn a_button_keeps_the_token_radius() {
        let tokens = Tokens::builtin_dark();
        let style = task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Active,
        );

        assert!((style.radius - radius(&tokens)).abs() < f32::EPSILON);
    }

    #[test]
    fn the_notice_action_is_an_outline_in_the_warning_colour() {
        let tokens = Tokens::builtin_dark();
        let rest = notice_action(&tokens, InteractionStatus::Active);
        let hovered = notice_action(&tokens, InteractionStatus::Hovered);
        let pressed = notice_action(&tokens, InteractionStatus::Pressed);
        let disabled = notice_action(&tokens, InteractionStatus::Disabled);

        assert_eq!(rest.text_color, warning_text(&tokens));
        assert_eq!(rest.border_color, warning_text(&tokens));
        assert_eq!(rest.border_width, hairline(&tokens));
        assert_eq!(rest.background, None);
        assert_ne!(hovered.background, rest.background);
        assert_eq!(pressed.background, Some(warning_text(&tokens)));
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

        assert_eq!(divider_color(&tokens), border_color(&tokens));
    }

    #[test]
    fn a_menu_bar_is_a_tint_with_a_highlight_along_its_top_edge_only() {
        let tokens = Tokens::builtin_dark();
        // The bar is square: it is anchored to the screen edge, so it has no
        // corner to round and nothing under its bottom edge to darken against.
        let style = pane_style(&tokens, false, 0.0, false);

        assert_eq!(style.tint, glass_tint(&tokens, false));
        assert_eq!(style.highlight, glass_highlight(&tokens));
        assert_eq!(style.shade, glass_shade(&tokens));
        assert_eq!(style.hairline, hairline(&tokens));
        assert!((style.radius - 0.0).abs() < f32::EPSILON);
        assert!(style.highlighted);
        assert!(
            !style.shaded,
            "a bar that reaches the screen edge draws no bottom shade"
        );
    }

    #[test]
    fn a_floating_pane_takes_the_group_radius_a_highlight_a_shade_and_a_soft_shadow() {
        let tokens = Tokens::builtin_dark();
        let radius = radius_md(&tokens);
        let style = pane_style(&tokens, false, radius, true);

        assert!((radius - f32::from(tokens.radii.md)).abs() < f32::EPSILON);
        assert_eq!(glass_shadow(&tokens), glass_shade(&tokens));
        assert!(
            glass_shadow_blur(&tokens) > 0.0,
            "a floating pane is lifted off the wallpaper by a soft shadow"
        );
        assert_ne!(
            glass_shadow(&tokens),
            TRANSPARENT,
            "the shadow is derived from the glass shade token, not written down"
        );
        assert!((glass_shadow_offset(&tokens) - hairline(&tokens)).abs() < f32::EPSILON);

        assert_eq!(
            style.tint,
            glass_tint(&tokens, false),
            "the pane is tinted in the middle"
        );
        assert_eq!(
            style.highlight,
            glass_highlight(&tokens),
            "the pane is highlighted at the top"
        );
        assert_eq!(
            style.shade,
            glass_shade(&tokens),
            "the pane is shaded at the bottom"
        );
        assert!(style.highlighted, "a floating pane has both edges");
        assert!(style.shaded);
        assert!(
            (style.radius - radius).abs() < f32::EPSILON,
            "the pill's corner is the radius the caller passes, not one of its own"
        );

        let strong = pane_style(&tokens, true, radius, true);
        assert_ne!(style, strong);
        assert_eq!(
            strong.tint,
            glass_tint(&tokens, true),
            "the emphasised pane uses the stronger tint"
        );
        assert_eq!(glass_tint(&tokens, false), qml_color(tokens.glass.tint));
        assert_eq!(
            glass_tint(&tokens, true),
            qml_color(tokens.glass.tint_strong)
        );
        assert_ne!(
            glass_tint(&tokens, false),
            glass_tint(&tokens, true),
            "the emphasised pane has its own tint"
        );
    }

    #[test]
    fn the_two_edges_of_a_pane_come_from_the_two_glass_edge_tokens() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(glass_highlight(&tokens), qml_color(tokens.glass.highlight));
        assert_eq!(glass_shade(&tokens), qml_color(tokens.glass.shade));
        assert_ne!(glass_highlight(&tokens), glass_shade(&tokens));
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
    fn the_lens_the_glass_asks_for_is_the_token_group() {
        let mut tokens = Tokens::builtin_dark();

        assert_eq!(glass_refraction(&tokens), tokens.glass.refraction);
        assert_eq!(
            glass_refraction_amount(&tokens),
            tokens.glass.refraction_amount
        );
        assert!(glass_refraction(&tokens) > 0 && glass_refraction_amount(&tokens) > 0);
        assert!(
            !glass_depth(&tokens),
            "the reference glass has no depth component"
        );
        assert!(
            glass_dispersion(&tokens),
            "the reference glass splits the refracted edge per channel"
        );

        // The refracting shape is the pane's own radius, not a second number.
        assert_eq!(pane_corner_radius(&tokens), tokens.radii.md);
        assert!(
            (f32::from(pane_corner_radius(&tokens)) - radius_md(&tokens)).abs() < f32::EPSILON,
            "the compositor and the QML pane are drawn with the same radius"
        );

        tokens.glass.refraction = 0;
        tokens.glass.refraction_amount = 0;
        tokens.glass.depth = true;
        assert_eq!(glass_refraction(&tokens), 0);
        assert_eq!(glass_refraction_amount(&tokens), 0);
        assert!(glass_depth(&tokens));
        assert!(
            glass_dispersion(&tokens),
            "dispersion is independent of the band being on"
        );
    }

    #[test]
    fn the_material_is_the_library_translation_of_the_same_tokens() {
        for tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
            assert_eq!(
                glass_material(&tokens),
                Material::from_tokens(&tokens.glass, tokens.radii.md),
                "the panel's material and the library's translation cannot drift"
            );
            assert!(
                glass_material(&tokens).has_effect(),
                "a built-in theme asks the compositor for the material"
            );
        }

        let mut tokens = Tokens::builtin_dark();
        tokens.glass.blur = 0;
        tokens.glass.refraction = 0;

        assert!(
            !glass_material(&tokens).has_effect(),
            "both halves off is plain translucency: no glass object is asked for"
        );
    }

    #[test]
    fn a_dock_tile_draws_the_same_five_states_as_a_task_entry() {
        let tokens = Tokens::builtin_dark();
        let states = [
            (
                "resting",
                dock_tile(&tokens, false, InteractionStatus::Active),
            ),
            (
                "hovered",
                dock_tile(&tokens, false, InteractionStatus::Hovered),
            ),
            (
                "pressed",
                dock_tile(&tokens, false, InteractionStatus::Pressed),
            ),
            (
                "focused",
                dock_tile(&tokens, true, InteractionStatus::Active),
            ),
            (
                "disabled",
                dock_tile(&tokens, false, InteractionStatus::Disabled),
            ),
        ];

        for (index, (name, style)) in states.iter().enumerate() {
            for (other_name, other) in &states[index + 1..] {
                assert_ne!(
                    (
                        &style.background,
                        &style.text_color,
                        style.border_width,
                        &style.border_color
                    ),
                    (
                        &other.background,
                        &other.text_color,
                        other.border_width,
                        &other.border_color
                    ),
                    "{name} and {other_name} have to be told apart"
                );
            }
        }

        assert!(
            (states[0].1.radius - radius(&tokens)).abs() < f32::EPSILON,
            "a tile is a control, so it takes the control radius"
        );
        assert_eq!(
            states[0].1.background,
            Some(accent(&tokens)),
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

        assert_eq!(dock_indicator(&tokens, true), accent(&tokens));
        assert_eq!(
            dock_indicator(&tokens, false),
            TRANSPARENT,
            "the inactive mark still occupies its slot, it just draws nothing"
        );
    }
}
