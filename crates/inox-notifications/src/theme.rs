//! Design tokens, as QML sees them.
//!
//! Every colour, gap, radius and text size the popup stack draws with is read
//! from [`inox_config::Theme`] here (root AGENTS.md §4, PRD §9.2): neither the
//! projection in [`crate::view`] nor `qml/Main.qml` contains a colour or a pixel
//! size of its own. What this file implements is `docs/quality/ui-guidelines.md`
//! §1 (token discipline), §2 (the four type roles), §4 (card geometry), §5 (the
//! five states of a control) and §6 (a status is a sentence first and a colour
//! second). The L3 component architecture explains why this small mapping is duplicated in each L3
//! component instead of being shared — a shared UI crate would be a new
//! same-layer dependency, which needs a decision of its own.
//!
//! Sizes are derived, never invented. The base text size is
//! `spacing.lg × typography.scale` — the window-shaped surface rule of §2 — and
//! every role is a named multiple of it, so a user asking for 200 % of the text
//! gets it without this file naming a pixel. The one literal left is the
//! hairline: the token set carries a border *colour* and no border width, and one
//! logical pixel is the hairline those colours are written against.
//!
//! Colours cross into QML as `#AARRGGBB` strings, which is the format Qt parses
//! and the format the projection publishes: the conversion happens on the Rust
//! side (docs/qml-migration-contract.md §4).

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
    /// window manager and is set through the projection — but the role is part of
    /// the token contract and is exported so that every surface of this component
    /// is measured against the same four sizes.
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

/// A token colour as the `#AARRGGBB` string QML parses.
///
/// The order is Qt's, not the CSS one: alpha first. Emitting it wrong would make
/// an opaque colour transparent, so the conversion has one test per channel.
pub fn qml_color(rgba: Rgba) -> String {
    format!("#{:02X}{:02X}{:02X}{:02X}", rgba.a, rgba.r, rgba.g, rgba.b)
}

/// The token colours the stack draws with, already in QML's form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colors {
    /// Behind the whole stack: the window's own background.
    pub background: String,
    /// A raised surface: one notification card.
    pub surface: String,
    /// The alternate surface: a notice line, a hovered control.
    pub surface_variant: String,
    /// Separator and outline colour.
    pub border: String,
    /// Default text colour.
    pub text: String,
    /// Secondary text: the application name, the timestamp, the close button.
    pub text_muted: String,
    /// Interactive accent colour: a primary action, a pressed control.
    pub accent: String,
    /// Text drawn on top of the accent.
    pub on_accent: String,
    /// A failure or something that cannot be undone.
    pub danger: String,
    /// A caution: notifications held but not shown.
    pub warning: String,
    /// A success.
    pub success: String,
}

impl Colors {
    /// Resolves every token colour into QML's form.
    pub fn resolve(tokens: &Theme) -> Self {
        let colors = &tokens.colors;

        Self {
            background: qml_color(colors.background),
            surface: qml_color(colors.surface),
            surface_variant: qml_color(colors.surface_variant),
            border: qml_color(colors.border),
            text: qml_color(colors.text),
            text_muted: qml_color(colors.text_muted),
            accent: qml_color(colors.accent),
            on_accent: qml_color(colors.on_accent),
            danger: qml_color(colors.danger),
            warning: qml_color(colors.warning),
            success: qml_color(colors.success),
        }
    }
}

/// One state of one control, as QML draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlState {
    /// Fill colour, or `None` for a control that only draws its text and edge.
    pub fill: Option<String>,
    /// Label colour.
    pub text: String,
    /// Edge colour.
    pub border: String,
}

impl ControlState {
    /// A state with a fill.
    fn filled(fill: &str, text: &str, border: &str) -> Self {
        Self {
            fill: Some(fill.to_owned()),
            text: text.to_owned(),
            border: border.to_owned(),
        }
    }

    /// A state that draws no fill.
    fn plain(text: &str, border: &str) -> Self {
        Self {
            fill: None,
            text: text.to_owned(),
            border: border.to_owned(),
        }
    }
}

/// The four drawn states of one control (ui-guidelines §5).
///
/// All four are projected even though this stack is never focused or, for the
/// close button, never disabled: §5 asks for the five states to be
/// distinguishable, and a state that is not projected is a state that would be
/// drawn wrong the day it appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlStyle {
    /// Resting.
    pub rest: ControlState,
    /// Pointer over it.
    pub hover: ControlState,
    /// Pressed.
    pub press: ControlState,
    /// Disabled: no fill, an edge that stops looking like a control.
    pub disabled: ControlState,
}

/// The size every role is a multiple of: the window surface's base size.
///
/// ui-guidelines §2 sets it to `spacing.lg × typography.scale` for a
/// window-shaped surface, which is what this stack is. It is a token product
/// rather than a literal, so the whole hierarchy moves together when a user
/// changes the font scale.
pub fn base_text_size(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.lg) * tokens.typography.scale
}

/// The size of one type role, at the user's scale.
///
/// A nonsensical configured scale must not make text disappear, so the result is
/// floored at one pixel and falls back to the unscaled base size when the scale
/// is not a number.
pub fn text_size(tokens: &Theme, role: TextRole) -> f32 {
    let base = base_text_size(tokens);
    let scaled = if base.is_finite() {
        base
    } else {
        f32::from(tokens.spacing.lg)
    };

    (scaled * role.multiplier()).max(1.0)
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

/// The gap between two parts of one block: the token set's default control step.
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
/// §4's requirement — the stricter one — sets the vertical inset to `md` as well,
/// which makes the inset uniform and is what QML's single padding value carries.
pub fn control_inset(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The height a control needs to be a target a pointer can hit.
///
/// The label's own size is the floor of its line box, so this is the least a
/// button can be — which is what makes it the value ui-guidelines §4's
/// `≥ spacing.xl` can be asserted against.
pub fn control_height(tokens: &Theme) -> f32 {
    text_size(tokens, TextRole::Body) + 2.0 * control_inset(tokens)
}

/// The corner radius of a card, a popup or a notice (ui-guidelines §4: `md`).
pub fn card_radius(tokens: &Theme) -> f32 {
    f32::from(tokens.radii.md)
}

/// The corner radius of a control (ui-guidelines §4: `sm`).
pub fn control_radius(tokens: &Theme) -> f32 {
    f32::from(tokens.radii.sm)
}

/// How wide the urgency rail on the leading edge of a critical card is.
///
/// ui-guidelines §1 forbids a literal, so the rail is one small spacing step: the
/// thinnest mark that still reads as a deliberate edge rather than as a second
/// border. It is drawn in [`urgency_color`], and it is never the only thing that
/// says "critical" — the urgency sentence on the card says it in words (§5, §6).
pub fn urgency_rail_width(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.sm)
}

/// How far a card rises while it enters (ui-guidelines §8).
///
/// One medium step of the spacing rhythm: enough to read as motion, small enough
/// that the resting position is already where the eye expects the card. The
/// animation ends here, and a card that never animated sits here too.
pub fn entry_rise(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.md)
}

/// How thick the focus ring of a control is (ui-guidelines §1 and §5).
///
/// The accent colour, drawn as thick as `spacing.xs`. This stack never takes
/// keyboard focus — its placement is
/// [`crate::placement::KeyboardInteractivity::None`](crate::placement::KeyboardInteractivity)
/// and the popup model has no focus order at all — so nothing here wears it yet.
/// It is part of the token contract §10 asks every component to export, and it is
/// what a host that does hand this surface the keyboard would draw.
pub fn focus_ring_width(tokens: &Theme) -> f32 {
    f32::from(tokens.spacing.xs)
}

/// The colour a link is drawn in.
pub fn link(tokens: &Theme) -> String {
    qml_color(tokens.colors.accent)
}

/// How urgent a notification is, in the token set's colours.
///
/// Low urgency is muted, normal text is ordinary, and critical is the danger
/// colour. This is the one colour the urgency sentence is written in and the one
/// the critical card is edged with: ui-guidelines §5 asks for a colour *and* a
/// sentence, never a stack of colours.
pub fn urgency_color(tokens: &Theme, urgency: Urgency) -> String {
    match urgency {
        Urgency::Low => qml_color(tokens.colors.text_muted),
        Urgency::Normal => qml_color(tokens.colors.text),
        Urgency::Critical => qml_color(tokens.colors.danger),
    }
}

/// The edge of one card.
///
/// ui-guidelines §4: the edge is the `border` token, except for a critical
/// notification, which takes the danger colour — the same colour its urgency
/// sentence is written in, so urgency is one colour plus one sentence rather than
/// a second surface.
pub fn card_edge(tokens: &Theme, urgency: Urgency) -> String {
    match urgency {
        Urgency::Critical => qml_color(tokens.colors.danger),
        Urgency::Low | Urgency::Normal => qml_color(tokens.colors.border),
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

/// The colour the daemon's own line is written in.
pub fn notice_text(tokens: &Theme, tone: NoticeTone) -> String {
    match tone {
        NoticeTone::Quiet => qml_color(tokens.colors.text_muted),
        NoticeTone::Caution => qml_color(tokens.colors.warning),
    }
}

/// The edge of the daemon's own line.
///
/// A notice is a status bar rather than a card (ui-guidelines §3 gives it the
/// `md` inset), but it keeps the hairline edge of §4 so that it still reads as
/// something the daemon is saying rather than as slack in the layout.
pub fn notice_edge(tokens: &Theme, tone: NoticeTone) -> String {
    match tone {
        NoticeTone::Quiet => qml_color(tokens.colors.border),
        NoticeTone::Caution => qml_color(tokens.colors.warning),
    }
}

/// The button that closes one popup (ui-guidelines §5, all four drawn states).
///
/// The resting button is the muted text colour with a hairline edge, hovering is
/// the alternate surface, pressing fills it with the accent and writes on it in
/// `on_accent`, and a disabled close button loses both its fill and its edge — so
/// it stops looking like a control rather than looking like a quiet one.
///
/// A close button is in fact never disabled by this stack: every popup carries
/// one and it is always pressable, which is why there is no reason sentence for
/// it to show. The state is projected anyway because §5 asks for all of them to
/// be distinguishable.
pub fn close_control(tokens: &Theme) -> ControlStyle {
    let colors = Colors::resolve(tokens);

    ControlStyle {
        rest: ControlState::plain(&colors.text_muted, &colors.border),
        hover: ControlState::filled(&colors.surface_variant, &colors.text, &colors.border),
        press: ControlState::filled(&colors.accent, &colors.on_accent, &colors.accent),
        // No edge at all: a disabled close button should not look like a quiet
        // control, it should stop looking like one.
        disabled: ControlState::plain(&colors.text_muted, "transparent"),
    }
}

/// A button that invokes one of a notification's actions.
///
/// The tier decides the colour, and only the colour: a primary action is drawn in
/// the accent, a secondary one in ordinary text, and an action that cannot be
/// undone in the danger colour (ui-guidelines §5: `accent` for the one primary
/// action, `danger` as an edge, never a whole accent-filled block for something
/// destructive). A disabled action loses its tier colour entirely — muted text,
/// plain edge — so that nothing about it still looks like something the user
/// could press (D05).
pub fn action_control(tokens: &Theme, emphasis: ActionEmphasis) -> ControlStyle {
    let colors = Colors::resolve(tokens);
    let tier = tier_color(tokens, emphasis);

    ControlStyle {
        rest: ControlState::plain(&tier, &tier),
        hover: ControlState::filled(&colors.surface_variant, &tier, &tier),
        press: ControlState::filled(&colors.accent, &colors.on_accent, &colors.accent),
        disabled: ControlState::plain(&colors.text_muted, &colors.border),
    }
}

/// The colour one action tier is drawn in at rest.
fn tier_color(tokens: &Theme, emphasis: ActionEmphasis) -> String {
    match emphasis {
        ActionEmphasis::Primary => qml_color(tokens.colors.accent),
        ActionEmphasis::Secondary => qml_color(tokens.colors.text),
        ActionEmphasis::Danger => qml_color(tokens.colors.danger),
    }
}

/// How one body span is drawn.
///
/// `<b>` and `<i>` are weight and style rather than size or colour, so they are
/// the properties of a span QML can move; `<u>` is an underline. The colour is the
/// link colour for an `<a href>` span and the text colour for everything else —
/// the sanitiser, not this file, decides what is a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanStyle {
    /// Drawn bold.
    pub bold: bool,
    /// Drawn italic.
    pub italic: bool,
    /// Drawn underlined.
    pub underline: bool,
}

impl SpanStyle {
    /// The style of one span's emphasis.
    pub fn from_emphasis(emphasis: Emphasis) -> Self {
        Self {
            bold: emphasis.bold,
            italic: emphasis.italic,
            underline: emphasis.underline,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::{BUILTIN_DARK, BUILTIN_LIGHT};

    #[test]
    fn a_token_colour_keeps_its_channels_in_qml_order() {
        let colour = qml_color(Rgba {
            r: 0x23,
            g: 0x26,
            b: 0x2e,
            a: 0x80,
        });

        assert_eq!(colour, "#8023262E", "alpha comes first, as Qt reads it");
        assert_eq!(colour.len(), 9);
    }

    #[test]
    fn an_opaque_colour_is_not_turned_transparent() {
        let colour = qml_color(Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        });

        assert_eq!(colour, "#FF000000");
        assert!(
            !colour.contains("#00"),
            "the alpha channel is written first"
        );
    }

    #[test]
    fn every_palette_colour_is_the_token_it_came_from() {
        let tokens = Theme::builtin_dark();
        let colors = Colors::resolve(&tokens);

        assert_eq!(colors.background, qml_color(tokens.colors.background));
        assert_eq!(colors.text, qml_color(tokens.colors.text));
        assert_eq!(colors.accent, qml_color(tokens.colors.accent));
        assert_eq!(colors.on_accent, qml_color(tokens.colors.on_accent));
        assert_ne!(colors.background, colors.surface);
    }

    #[test]
    fn the_light_and_dark_themes_have_their_own_palettes() {
        let light = Colors::resolve(&Theme::builtin_light());
        let dark = Colors::resolve(&Theme::builtin_dark());

        assert_ne!(light.background, dark.background);
        assert_ne!(light.text, dark.text);
        assert_ne!(light.background, light.text);
        assert_eq!(Theme::builtin_light().name, BUILTIN_LIGHT);
        assert_eq!(Theme::builtin_dark().name, BUILTIN_DARK);
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
    fn a_nonsensical_font_scale_still_yields_visible_text() {
        let mut tokens = Theme::builtin_dark();

        for scale in [f32::NAN, f32::INFINITY, -4.0, 0.0] {
            tokens.typography.scale = scale;

            for role in TextRole::ALL {
                let size = text_size(&tokens, role);
                assert!(size >= 1.0, "scale {scale} produced {size} for {role:?}");
                assert!(size.is_finite(), "scale {scale} produced {size}");
            }
        }
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
        assert_eq!(control_inset(&tokens), f32::from(tokens.spacing.md));
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
        assert_eq!(control_inset(&tokens), 12.0);
        assert_eq!(
            card_inset(&tokens),
            f32::from(Theme::builtin_dark().spacing.lg),
            "a distance is derived from the token it names, not from a default"
        );
    }

    #[test]
    fn a_control_radius_is_the_small_token_and_a_card_radius_is_the_medium_one() {
        let tokens = Theme::builtin_dark();

        assert_eq!(control_radius(&tokens), f32::from(tokens.radii.sm));
        assert_eq!(card_radius(&tokens), f32::from(tokens.radii.md));
        assert_ne!(control_radius(&tokens), card_radius(&tokens));
    }

    #[test]
    fn a_control_is_at_least_the_tall_target_the_guidelines_ask_for() {
        let mut tokens = Theme::builtin_dark();

        for scale in [1.0_f32, 2.0] {
            tokens.typography.scale = scale;
            assert!(
                control_height(&tokens) >= f32::from(tokens.spacing.xl),
                "at {scale}× scale a control is {} px tall, under the {} px target",
                control_height(&tokens),
                tokens.spacing.xl
            );
        }
    }

    #[test]
    fn the_focus_ring_is_as_thick_as_the_spacing_step() {
        let tokens = Theme::builtin_dark();

        assert_eq!(focus_ring_width(&tokens), f32::from(tokens.spacing.xs));
        assert!(focus_ring_width(&tokens) > BORDER_WIDTH);
    }

    #[test]
    fn the_urgency_rail_and_the_entry_rise_are_token_steps() {
        let tokens = Theme::builtin_dark();

        assert_eq!(urgency_rail_width(&tokens), f32::from(tokens.spacing.sm));
        assert_eq!(entry_rise(&tokens), f32::from(tokens.spacing.md));
        assert!(
            urgency_rail_width(&tokens) > BORDER_WIDTH,
            "a rail is a mark of its own, not a second hairline"
        );
        assert!(
            urgency_rail_width(&tokens) < card_inset(&tokens),
            "the rail stays out of the card's inset"
        );
    }

    #[test]
    fn the_motion_tokens_are_the_durations_the_surface_animates_with() {
        let tokens = Theme::builtin_dark();

        assert_eq!(tokens.motion.normal_ms, 200);
        assert!(
            tokens.motion.fast_ms < tokens.motion.normal_ms
                && tokens.motion.normal_ms < tokens.motion.slow_ms,
            "the three durations are ordered"
        );
        assert!(
            tokens.motion.slow_ms > 0,
            "an entrance has to take some time"
        );
    }

    #[test]
    fn every_urgency_has_a_colour_of_its_own() {
        let tokens = Theme::builtin_dark();

        let low = urgency_color(&tokens, Urgency::Low);
        let normal = urgency_color(&tokens, Urgency::Normal);
        let critical = urgency_color(&tokens, Urgency::Critical);

        assert_eq!(normal, qml_color(tokens.colors.text));
        assert_eq!(low, qml_color(tokens.colors.text_muted));
        assert_eq!(critical, qml_color(tokens.colors.danger));
        assert_ne!(low, normal);
        assert_ne!(critical, normal);
    }

    #[test]
    fn a_critical_card_is_edged_in_the_danger_colour() {
        let tokens = Theme::builtin_dark();

        let normal = card_edge(&tokens, Urgency::Normal);
        let critical = card_edge(&tokens, Urgency::Critical);

        assert_eq!(normal, qml_color(tokens.colors.border));
        assert_eq!(critical, qml_color(tokens.colors.danger));
        assert_ne!(critical, normal);
        assert_eq!(
            card_edge(&tokens, Urgency::Low),
            normal,
            "low urgency is said in a sentence, not in a second edge colour"
        );
    }

    #[test]
    fn a_caution_notice_is_written_in_the_warning_colour_and_a_quiet_one_in_muted_text() {
        let tokens = Theme::builtin_dark();

        assert_eq!(
            notice_text(&tokens, NoticeTone::Quiet),
            qml_color(tokens.colors.text_muted)
        );
        assert_eq!(
            notice_text(&tokens, NoticeTone::Caution),
            qml_color(tokens.colors.warning)
        );
        assert_eq!(
            notice_edge(&tokens, NoticeTone::Caution),
            qml_color(tokens.colors.warning)
        );
        assert_eq!(
            notice_edge(&tokens, NoticeTone::Quiet),
            qml_color(tokens.colors.border)
        );
    }

    #[test]
    fn a_link_is_drawn_in_the_accent_colour() {
        let tokens = Theme::builtin_dark();

        assert_eq!(link(&tokens), qml_color(tokens.colors.accent));
        assert_ne!(link(&tokens), qml_color(tokens.colors.text));
    }

    #[test]
    fn every_action_tier_is_a_colour_of_its_own() {
        let tokens = Theme::builtin_dark();
        let colors = Colors::resolve(&tokens);

        let primary = action_control(&tokens, ActionEmphasis::Primary);
        let secondary = action_control(&tokens, ActionEmphasis::Secondary);
        let danger = action_control(&tokens, ActionEmphasis::Danger);

        assert_eq!(primary.rest.text, colors.accent);
        assert_eq!(primary.rest.border, colors.accent);
        assert_eq!(secondary.rest.text, colors.text);
        assert_eq!(secondary.rest.fill, None, "a secondary action has no fill");
        assert_eq!(danger.rest.text, colors.danger);
        assert_eq!(danger.rest.border, colors.danger);
        assert_ne!(primary.rest.text, secondary.rest.text);
        assert_ne!(danger.rest.text, secondary.rest.text);
        assert_ne!(
            danger.rest.border, colors.accent,
            "danger is not accent-filled"
        );
    }

    /// The four states of one control, for the pairwise assertions.
    fn states(style: &ControlStyle) -> [&ControlState; 4] {
        [&style.rest, &style.hover, &style.press, &style.disabled]
    }

    #[test]
    fn the_four_states_of_an_action_button_are_pairwise_distinguishable() {
        let tokens = Theme::builtin_dark();

        for emphasis in [
            ActionEmphasis::Primary,
            ActionEmphasis::Secondary,
            ActionEmphasis::Danger,
        ] {
            let style = action_control(&tokens, emphasis);
            let states = states(&style);
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
        let style = close_control(&tokens);
        let states = states(&style);

        for (left, first) in states.iter().enumerate() {
            for (_, second) in states.iter().enumerate().skip(left + 1) {
                assert_ne!(first, second, "two close-button states look the same");
            }
        }
    }

    #[test]
    fn a_pressed_control_is_filled_with_the_accent_and_written_on_it() {
        let tokens = Theme::builtin_dark();
        let colors = Colors::resolve(&tokens);

        let pressed = action_control(&tokens, ActionEmphasis::Primary).press;
        let close = close_control(&tokens).press;

        assert_eq!(pressed.fill.as_deref(), Some(colors.accent.as_str()));
        assert_eq!(pressed.text, colors.on_accent);
        assert_eq!(close.fill.as_deref(), Some(colors.accent.as_str()));
        assert_eq!(close.text, colors.on_accent);
    }

    #[test]
    fn a_hovered_control_is_drawn_on_the_alternate_surface() {
        let tokens = Theme::builtin_dark();
        let colors = Colors::resolve(&tokens);

        let hovered = action_control(&tokens, ActionEmphasis::Primary).hover;
        let idle = action_control(&tokens, ActionEmphasis::Primary).rest;

        assert_eq!(
            hovered.fill.as_deref(),
            Some(colors.surface_variant.as_str())
        );
        assert_ne!(idle.fill, hovered.fill);
        assert_eq!(
            close_control(&tokens).hover.fill.as_deref(),
            Some(colors.surface_variant.as_str())
        );
    }

    #[test]
    fn a_disabled_action_looks_different_from_one_that_can_be_used() {
        let tokens = Theme::builtin_dark();
        let colors = Colors::resolve(&tokens);

        let enabled = action_control(&tokens, ActionEmphasis::Primary).rest;
        let disabled = action_control(&tokens, ActionEmphasis::Primary).disabled;

        assert_ne!(enabled.text, disabled.text);
        assert_eq!(disabled.text, colors.text_muted);
        assert_eq!(disabled.fill, None);
        assert_ne!(
            disabled.border, colors.accent,
            "nothing about a disabled action is accent-coloured"
        );
        assert_eq!(disabled.border, colors.border);
        assert_eq!(
            action_control(&tokens, ActionEmphasis::Danger)
                .disabled
                .text,
            colors.text_muted,
            "a disabled action loses its tier colour, danger included"
        );
    }

    #[test]
    fn markup_moves_the_weight_the_style_and_the_underline_of_a_span() {
        assert_eq!(
            SpanStyle::from_emphasis(Emphasis::PLAIN),
            SpanStyle {
                bold: false,
                italic: false,
                underline: false
            }
        );
        assert_eq!(
            SpanStyle::from_emphasis(Emphasis {
                bold: true,
                italic: true,
                underline: true
            }),
            SpanStyle {
                bold: true,
                italic: true,
                underline: true
            }
        );
    }
}
