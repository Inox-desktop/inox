//! Inox design tokens, turned into the values the QML tree binds to.
//!
//! Root `AGENTS.md` §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! QML, meanwhile, binds to whatever it is given — so this module is the join
//! between the tokens and the declarative tree: it derives the type roles, the
//! spacing rhythm, the control geometry and the five interaction states, and
//! serialises them as one object the window reads out of its JSON projection.
//!
//! Colours leave this module as `#AARRGGBB` strings — the format Qt's own
//! `QColor::name(QColor::HexArgb)` produces — so no QML file holds a colour
//! literal and no QML file has to know what a token is. Everything here is a
//! pure function of [`inox_config::Theme`], which is what lets the projection
//! tests assert the light and the dark theme without a display.
//!
//! `docs/quality/ui-guidelines.md` §1 draws the line this module sits on: the
//! token set carries colours, spacing, radii and durations but no type scale and
//! no border width, so everything the window draws is either a token or one of
//! the named derivations below — the four type roles (§2), the spacing rhythm
//! (§3), the control geometry (§4) and the five interaction states (§5). A
//! reviewer should be able to read a number here and see where it came from;
//! finding one in a QML file instead is the defect the guidelines name.
//!
//! the L3 component architecture explains why there is a copy of this file in every GUI component
//! instead of one shared crate: the duplication is the price of not adding a
//! same-layer Cargo dependency that would need its own ADR.

use inox_config::{Rgba, Theme as Tokens};
use serde::Serialize;

use crate::view_model::StatusKind;

/// How many large spacing steps make a line of body text.
const BODY_SIZE_STEPS: f32 = 1.0;

/// How many body lines a page title is (guidelines §2).
const TITLE_SIZE_STEPS: f32 = 1.5;

/// How many body lines a card's heading is (guidelines §2).
const HEADING_SIZE_STEPS: f32 = 1.25;

/// How many body lines a caption is. The guidelines set a floor of 0.75, so
/// this is the smallest role the window may draw.
const CAPTION_SIZE_STEPS: f32 = 0.875;

/// The divisor that turns the smallest spacing step into a hairline.
const HAIRLINE_DIVISOR: f32 = 2.0;

/// How many extra-large spacing steps wide a short value is: a number with its
/// unit, or an identifier such as a layout name (guidelines §4).
const SHORT_WIDTH_STEPS: f32 = 6.0;

/// How many extra-large spacing steps wide a long value is: a connector, a
/// format string, a path or a command (guidelines §4).
const WIDE_WIDTH_STEPS: f32 = 12.0;

/// What fraction of a long value the actions of a list row take.
const LIST_ACTION_DIVISOR: f32 = 2.0;

/// How many extra-large spacing steps wide the navigation list is.
const NAV_WIDTH_STEPS: f32 = 5.0;

/// How many extra-large spacing steps wide a field's label column is.
const LABEL_WIDTH_STEPS: f32 = 7.0;

/// How much of the window still shows through the trial dialog's scrim, in
/// percent of the background token's own alpha.
const SCRIM_OPACITY_PERCENT: u8 = 60;

/// The font scale above which a list of several columns stops being a table.
///
/// A column can only name itself while it is wider than the longest word in its
/// name; past this scale the names break in the middle of a word, so the list is
/// drawn one labelled field per row instead (ui-guidelines §7).
const TABLE_SCALE_LIMIT: f32 = 1.5;

/// The weight a title and a heading are drawn at, in Qt's hundred-based scale.
///
/// The guidelines ask for emphasis by weight rather than by another size step,
/// and the token set has no weight, so this is the one place it is written.
const EMPHASIS_WEIGHT: u16 = 700;

/// A token colour, as the `#AARRGGBB` string QML renders.
///
/// The alpha channel comes first because that is the order Qt's
/// `QColor::HexArgb` writes and reads; every opaque token has `0xff` there.
pub fn qml_color(value: Rgba) -> String {
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        value.a, value.r, value.g, value.b
    )
}

// -- Type roles (guidelines §2) -------------------------------------------------

/// The size of a line of body text, in logical pixels.
///
/// The token set carries a family and the user's scale but no size, so the size
/// is one large spacing step times the scale: a user who doubles the scale gets
/// text twice as tall, and the QML layout is what keeps the result inside the
/// window (acceptance Q02).
pub fn font_size(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.lg) * BODY_SIZE_STEPS * tokens.typography.scale
}

/// The size of the body role, which is what [`font_size`] has always been.
pub fn body_size(tokens: &Tokens) -> f32 {
    font_size(tokens)
}

/// The size of a page's title: the one thing a page is called.
pub fn title_size(tokens: &Tokens) -> f32 {
    font_size(tokens) * TITLE_SIZE_STEPS
}

/// The size of a card's or a group's heading.
pub fn heading_size(tokens: &Tokens) -> f32 {
    font_size(tokens) * HEADING_SIZE_STEPS
}

/// The size of a caption: an explanation, a unit, a state sentence.
pub fn caption_size(tokens: &Tokens) -> f32 {
    font_size(tokens) * CAPTION_SIZE_STEPS
}

/// The weight a title or a heading is drawn at.
pub fn emphasis_weight() -> u16 {
    EMPHASIS_WEIGHT
}

// -- Geometry derived from the tokens (guidelines §1) --------------------------

/// The thickness of a border, in logical pixels.
pub fn hairline(tokens: &Tokens) -> f32 {
    (f32::from(tokens.spacing.xs) / HAIRLINE_DIVISOR).max(0.0)
}

/// The thickness of the outline around the control the keyboard is on.
///
/// Done with a width rather than a second box so the focus cannot be mistaken
/// for a hover: the guidelines give the focus ring its own width
/// (`spacing.xs`), twice the hairline a resting control is drawn with.
pub fn focus_ring(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xs)
}

/// The padding around the window's own contents: `xl` (guidelines §3).
pub fn page_padding(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xl)
}

/// The padding inside a card: `lg` (guidelines §3).
pub fn card_padding(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.lg)
}

/// The padding inside a status or notice strip: `md` (guidelines §3).
pub fn notice_padding(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The padding above and below a control's own text: `sm`.
pub fn control_padding_vertical(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.sm)
}

/// The padding at the sides of a control's own text: `md`.
pub fn control_padding_horizontal(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The padding above and below a list entry: an option, a switch, an entry of
/// the navigation list.
///
/// It is the least vertical padding that makes a one-line entry exactly
/// [`control_height`] tall, which is what the guidelines ask of an interactive
/// target (§4). A control takes its height from its padding and its text rather
/// than from a fixed height, so an entry whose label wraps at a large font scale
/// grows instead of being cut off (Q02).
pub fn list_padding_vertical(tokens: &Tokens) -> f32 {
    ((control_height(tokens) - body_size(tokens)) / 2.0).max(0.0)
}

/// The padding at the sides of a list entry: `md`.
pub fn list_padding_horizontal(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.md)
}

/// The gap between two blocks: `lg` (guidelines §3).
pub fn section_gap(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.lg)
}

/// The gap between two entries of one group: `sm` (guidelines §3).
pub fn row_gap(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.sm)
}

/// The gap between a label and its control, or between a value and its unit:
/// `sm` (guidelines §3).
pub fn inline_gap(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.sm)
}

/// The width of a short value: a number with its unit, or an identifier.
///
/// At most six extra-large spacing steps, so a value of two digits is never
/// drawn as if it were a paragraph (guidelines §4).
pub fn short_width(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xl) * SHORT_WIDTH_STEPS
}

/// The width of a long value: a connector, a format string, a path or a
/// command. At most twelve extra-large spacing steps (guidelines §4).
pub fn wide_width(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xl) * WIDE_WIDTH_STEPS
}

/// The width of the actions a list row carries — add, remove.
///
/// The actions of a list are their own column so that the column names in the
/// header line up with the fields under them, and half a long value is enough
/// for the longest of them at every scale: what does not fit wraps inside the
/// button rather than pushing the row out of the card (Q02).
pub fn list_action_width(tokens: &Tokens) -> f32 {
    wide_width(tokens) / LIST_ACTION_DIVISOR
}

/// The width of the navigation list down the left-hand side.
///
/// It grows with the user's font scale because it holds words: a label a user
/// has doubled has to have somewhere to go (Q02).
pub fn nav_width(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xl) * NAV_WIDTH_STEPS * tokens.typography.scale.max(1.0)
}

/// The width of a field's label column, so the controls line up down the page
/// and across its cards. It grows with the font scale for the same reason the
/// navigation list does.
pub fn label_width(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS * tokens.typography.scale.max(1.0)
}

/// The smallest height a control a user can act on is drawn at.
///
/// At least `xl`, and taller once the text itself needs the room, so a control
/// never cuts off text a user has scaled up (guidelines §4, §7).
pub fn control_height(tokens: &Tokens) -> f32 {
    let text = body_size(tokens) + f32::from(tokens.spacing.sm) * 2.0;

    text.max(f32::from(tokens.spacing.xl))
}

/// The line box a control's own text sits in.
///
/// The text stays vertically centred in the control, so the line box is the
/// control's height less its padding, never smaller than the text itself.
pub fn control_line_height(tokens: &Tokens) -> f32 {
    let line = control_height(tokens) - f32::from(tokens.spacing.sm) * 2.0;

    line.max(body_size(tokens))
}

/// The corner radius of a control.
pub fn radius(tokens: &Tokens) -> f32 {
    f32::from(tokens.radii.sm)
}

/// The corner radius of a card, a panel or the trial dialog.
pub fn card_radius(tokens: &Tokens) -> f32 {
    f32::from(tokens.radii.md)
}

/// The width the display trial's card is drawn at.
///
/// A long value plus the card's own padding, scaled with the text: the dialog
/// holds the longest sentences in the window and has to wrap them rather than
/// grow past the screen at a large font scale (Q02).
pub fn dialog_width(tokens: &Tokens) -> f32 {
    (wide_width(tokens) + card_padding(tokens) * 2.0) * tokens.typography.scale.max(1.0)
}

/// Whether a list of several columns still has room to be a table.
pub fn list_as_table(tokens: &Tokens) -> bool {
    tokens.typography.scale <= TABLE_SCALE_LIMIT
}

// -- Colours -------------------------------------------------------------------

/// A colour role of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// The window behind the content.
    pub window: Rgba,
    /// Raised surfaces: cards, the navigation list, the trial dialog.
    pub surface: Rgba,
    /// Alternate surface, for a hovered or focused control.
    pub surface_variant: Rgba,
    /// Separator and outline colour.
    pub border: Rgba,
    /// Default text colour.
    pub text: Rgba,
    /// Secondary information.
    pub text_muted: Rgba,
    /// The interactive accent.
    pub accent: Rgba,
    /// Text drawn on top of the accent.
    pub on_accent: Rgba,
    /// Something a user should look at: a refused value, a failed save.
    pub danger: Rgba,
    /// A change that is saved but not yet in effect, or on trial.
    pub warning: Rgba,
    /// A change that is in effect.
    pub success: Rgba,
}

impl Palette {
    /// The palette of the tokens.
    pub fn of(tokens: &Tokens) -> Self {
        let colors = tokens.colors;

        Self {
            window: colors.background,
            surface: colors.surface,
            surface_variant: colors.surface_variant,
            border: colors.border,
            text: colors.text,
            text_muted: colors.text_muted,
            accent: colors.accent,
            on_accent: colors.on_accent,
            danger: colors.danger,
            warning: colors.warning,
            success: colors.success,
        }
    }

    /// What the window is dimmed with while a display trial is open.
    ///
    /// The scrim is the window's own background at less than full opacity, so it
    /// is the theme that decides how much the question stands out — no
    /// component-local colour enters the window (root `AGENTS.md` §4).
    pub fn scrim(&self) -> Rgba {
        Rgba {
            a: (u16::from(self.window.a) * u16::from(SCRIM_OPACITY_PERCENT) / 100) as u8,
            ..self.window
        }
    }

    /// The fully transparent colour a control with no fill is drawn with.
    ///
    /// QML holds no colour literal, so "no fill" is a colour too rather than the
    /// word `transparent`.
    fn none(&self) -> Rgba {
        Rgba {
            a: 0,
            ..self.window
        }
    }
}

/// One drawing of a control: what it is filled with, what its text is, and the
/// outline around it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlStyle {
    /// The fill, transparent when there is none.
    pub background: String,
    /// The text and icon colour.
    pub foreground: String,
    /// The outline colour.
    pub border: String,
    /// The outline width, in logical pixels.
    pub border_width: f32,
}

/// Every state of a control, resolved from the tokens.
///
/// The states are drawn so that no two of them look the same, and every colour
/// is a token:
///
/// | | background | text | outline |
/// |---|---|---|---|
/// | resting | none | text | border, hairline |
/// | hovered | surface variant | text | border, hairline |
/// | pressed | accent | on accent | on accent |
/// | focused | surface variant | accent | accent, focus ring |
/// | selected | accent | on accent | accent, hairline |
/// | selected and focused | accent | on accent | on accent, focus ring |
/// | disabled | none | text muted | border, hairline |
///
/// The pair that would otherwise be ambiguous is the last one: an outline in
/// the accent colour against an accent background is invisible, so a focused
/// selection is outlined in the colour the text already uses. That is what keeps
/// the focus visible on the option that is already current. Hovering the current
/// option keeps the accent background for the same reason: the accent's own text
/// colour on the hover surface would be unreadable.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlColors {
    /// Resting.
    pub plain: ControlStyle,
    /// The keyboard is on it.
    pub plain_focused: ControlStyle,
    /// The pointer is on it.
    pub plain_hovered: ControlStyle,
    /// The pointer and the keyboard are both on it.
    pub plain_hovered_focused: ControlStyle,
    /// It holds the current value.
    pub selected: ControlStyle,
    /// It holds the current value and the keyboard is on it.
    pub selected_focused: ControlStyle,
    /// It is being pressed.
    pub pressed: ControlStyle,
    /// It is being pressed while the keyboard is on it.
    pub pressed_focused: ControlStyle,
    /// It cannot be used.
    pub disabled: ControlStyle,
    /// A destructive action: danger text and outline, never a filled accent.
    pub danger: ControlStyle,
    /// A destructive action the keyboard is on.
    pub danger_focused: ControlStyle,
    /// A destructive action the pointer is on.
    pub danger_hovered: ControlStyle,
    /// A destructive action the pointer and the keyboard are on.
    pub danger_hovered_focused: ControlStyle,
    /// A destructive action being pressed.
    pub danger_pressed: ControlStyle,
}

impl ControlStyle {
    /// A drawing with no fill.
    fn flat(palette: &Palette, foreground: Rgba, border: Rgba, border_width: f32) -> Self {
        Self {
            background: qml_color(palette.none()),
            foreground: qml_color(foreground),
            border: qml_color(border),
            border_width,
        }
    }

    /// A drawing filled with a colour.
    fn filled(background: Rgba, foreground: Rgba, border: Rgba, border_width: f32) -> Self {
        Self {
            background: qml_color(background),
            foreground: qml_color(foreground),
            border: qml_color(border),
            border_width,
        }
    }
}

/// Resolves every state a control can be drawn in.
pub fn control_colors(tokens: &Tokens) -> ControlColors {
    let palette = Palette::of(tokens);
    let hairline = hairline(tokens);
    let ring = focus_ring(tokens);

    // The outline of a control that is filled with the accent has to be the
    // on-accent colour when the keyboard is on it: an accent ring on an accent
    // fill is invisible.
    let on_fill = |focused: bool| {
        if focused {
            (palette.on_accent, ring)
        } else {
            (palette.accent, hairline)
        }
    };
    let pressed = on_fill(false);
    let pressed_focused = on_fill(true);

    ControlColors {
        plain: ControlStyle::flat(&palette, palette.text, palette.border, hairline),
        plain_focused: ControlStyle::filled(
            palette.surface_variant,
            palette.accent,
            palette.accent,
            ring,
        ),
        plain_hovered: ControlStyle::filled(
            palette.surface_variant,
            palette.text,
            palette.border,
            hairline,
        ),
        plain_hovered_focused: ControlStyle::filled(
            palette.surface_variant,
            palette.text,
            palette.accent,
            ring,
        ),
        selected: ControlStyle::filled(palette.accent, palette.on_accent, palette.accent, hairline),
        selected_focused: ControlStyle::filled(
            palette.accent,
            palette.on_accent,
            palette.on_accent,
            ring,
        ),
        pressed: ControlStyle::filled(palette.accent, palette.on_accent, pressed.0, pressed.1),
        pressed_focused: ControlStyle::filled(
            palette.accent,
            palette.on_accent,
            pressed_focused.0,
            pressed_focused.1,
        ),
        disabled: ControlStyle::flat(&palette, palette.text_muted, palette.border, hairline),
        danger: ControlStyle::flat(&palette, palette.danger, palette.danger, hairline),
        danger_focused: ControlStyle::flat(&palette, palette.danger, palette.accent, ring),
        danger_hovered: ControlStyle::filled(
            palette.surface_variant,
            palette.danger,
            palette.danger,
            hairline,
        ),
        danger_hovered_focused: ControlStyle::filled(
            palette.surface_variant,
            palette.danger,
            palette.accent,
            ring,
        ),
        danger_pressed: ControlStyle::filled(
            palette.danger,
            palette.on_accent,
            palette.on_accent,
            ring,
        ),
    }
}

/// The five states of an editable field, resolved from the tokens.
///
/// A field is drawn as a block of its own on the surface variant, so it reads as
/// an input inside a card rather than as a hole in it:
///
/// | | background | outline | value |
/// |---|---|---|---|
/// | resting | surface variant | border, hairline | text |
/// | hovered | surface variant | text, hairline | text |
/// | focused | surface variant | accent, focus ring | text |
/// | disabled | surface | border, hairline | muted |
/// | refused | surface variant | danger, hairline or ring | text |
///
/// A field the schema refuses is drawn in the danger colour and a field the
/// keyboard is on is drawn with the focus ring, so a refused field that is also
/// focused shows both: the colour says *what is wrong*, the width says *where
/// the keyboard is*.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputColors {
    /// The field's fill.
    pub background: String,
    /// The fill of a field that cannot be edited.
    pub background_disabled: String,
    /// The value's colour.
    pub value: String,
    /// The value's colour when it cannot be edited.
    pub value_disabled: String,
    /// The colour of a hint drawn inside an empty field.
    pub placeholder: String,
    /// The selection's colour.
    pub selection: String,
    /// The outline while resting.
    pub border: String,
    /// The outline while the pointer is on it.
    pub border_hovered: String,
    /// The outline while the keyboard is on it.
    pub border_focused: String,
    /// The outline of a value the schema refuses.
    pub border_refused: String,
    /// The resting outline's width.
    pub border_width: f32,
    /// The outline width of a focused or refused field.
    pub focus_width: f32,
}

/// Resolves the states of an editable field.
pub fn input_colors(tokens: &Tokens) -> InputColors {
    let palette = Palette::of(tokens);

    InputColors {
        background: qml_color(palette.surface_variant),
        background_disabled: qml_color(palette.surface),
        value: qml_color(palette.text),
        value_disabled: qml_color(palette.text_muted),
        placeholder: qml_color(palette.text_muted),
        selection: qml_color(palette.accent),
        border: qml_color(palette.border),
        border_hovered: qml_color(palette.text),
        border_focused: qml_color(palette.accent),
        border_refused: qml_color(palette.danger),
        border_width: hairline(tokens),
        focus_width: focus_ring(tokens),
    }
}

/// The border a status line is drawn with, by how far the change has got.
///
/// Nothing here claims more than [`crate::apply`] knows: a change that is saved
/// but not confirmed is drawn in the warning colour, and one that is in effect
/// in the success colour, so the two are distinguishable at a glance as well as
/// in the sentence. A display trial is in effect but not settled, so it takes
/// the warning colour too.
pub fn status_border(tokens: &Tokens, kind: StatusKind) -> String {
    let palette = Palette::of(tokens);

    qml_color(match kind {
        StatusKind::Rejected => palette.danger,
        StatusKind::Pending | StatusKind::SavedOnly | StatusKind::Provisional => palette.warning,
        StatusKind::Applied => palette.success,
        StatusKind::Unchanged | StatusKind::Modified | StatusKind::Saving => palette.border,
    })
}

// -- The projection ------------------------------------------------------------

/// The sizes the QML tree lays the window out with.
///
/// Every padding, gap, size and bound the window draws with is a field here, so
/// a reviewer can read the layout without following a declarative tree and so a
/// test can assert the bounds the guidelines set (ui-guidelines §4).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    /// The size of a page's title.
    pub title_size: f32,
    /// The size of a card's heading.
    pub heading_size: f32,
    /// The size of body text, labels and button text.
    pub body_size: f32,
    /// The size of an explanation, a unit or a state sentence.
    pub caption_size: f32,
    /// The weight a title or a heading is drawn at.
    pub emphasis_weight: u16,
    /// The window's own padding.
    pub page_padding: f32,
    /// The padding inside a card.
    pub card_padding: f32,
    /// The padding inside a status or notice strip.
    pub notice_padding: f32,
    /// The padding above and below a control's own text.
    pub control_padding_vertical: f32,
    /// The padding at the sides of a control's own text.
    pub control_padding_horizontal: f32,
    /// The padding above and below a list entry.
    pub list_padding_vertical: f32,
    /// The padding at the sides of a list entry.
    pub list_padding_horizontal: f32,
    /// The gap between two cards and between the window's blocks.
    pub section_gap: f32,
    /// The gap between two entries of one card.
    pub row_gap: f32,
    /// The gap between a value and its unit, or a label and its control.
    pub inline_gap: f32,
    /// The width of the navigation list.
    pub nav_width: f32,
    /// The width of a field's label column.
    pub label_width: f32,
    /// The width of a short value.
    pub short_width: f32,
    /// The width of a long value.
    pub wide_width: f32,
    /// The width of the actions a list row carries.
    pub list_action_width: f32,
    /// The smallest height a control a user can act on is drawn at.
    pub control_height: f32,
    /// The line box a field's own text sits in.
    pub line_height: f32,
    /// The width of the display trial's card.
    pub dialog_width: f32,
    /// The thickness of a border.
    pub hairline: f32,
    /// The thickness of the focus outline.
    pub focus_ring: f32,
    /// The corner radius of a control.
    pub radius: f32,
    /// The corner radius of a card, a panel or the dialog.
    pub card_radius: f32,
    /// Whether a list of several columns still has room to be a table.
    pub list_as_table: bool,
}

impl Geometry {
    /// Derives the window's geometry from the tokens.
    pub fn of(tokens: &Tokens) -> Self {
        Self {
            title_size: title_size(tokens),
            heading_size: heading_size(tokens),
            body_size: body_size(tokens),
            caption_size: caption_size(tokens),
            emphasis_weight: emphasis_weight(),
            page_padding: page_padding(tokens),
            card_padding: card_padding(tokens),
            notice_padding: notice_padding(tokens),
            control_padding_vertical: control_padding_vertical(tokens),
            control_padding_horizontal: control_padding_horizontal(tokens),
            list_padding_vertical: list_padding_vertical(tokens),
            list_padding_horizontal: list_padding_horizontal(tokens),
            section_gap: section_gap(tokens),
            row_gap: row_gap(tokens),
            inline_gap: inline_gap(tokens),
            nav_width: nav_width(tokens),
            label_width: label_width(tokens),
            short_width: short_width(tokens),
            wide_width: wide_width(tokens),
            list_action_width: list_action_width(tokens),
            control_height: control_height(tokens),
            line_height: control_line_height(tokens),
            dialog_width: dialog_width(tokens),
            hairline: hairline(tokens),
            focus_ring: focus_ring(tokens),
            radius: radius(tokens),
            card_radius: card_radius(tokens),
            list_as_table: list_as_table(tokens),
        }
    }
}

/// The colours the window is drawn with.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Colors {
    /// The window's own background.
    pub window: String,
    /// A card, the navigation list and the trial dialog.
    pub surface: String,
    /// A hovered or focused control's fill.
    pub surface_variant: String,
    /// Separators and resting outlines.
    pub border: String,
    /// Default text.
    pub text: String,
    /// Secondary text.
    pub text_muted: String,
    /// The one accent of the screen.
    pub accent: String,
    /// Text on the accent.
    pub on_accent: String,
    /// A refused value or a failed save.
    pub danger: String,
    /// Saved but not in effect, or on trial.
    pub warning: String,
    /// In effect.
    pub success: String,
    /// What the window is dimmed with while a display trial is open.
    pub scrim: String,
    /// The outline of the trial dialog, which is not a settled change.
    pub dialog_border: String,
}

/// Everything the QML tree needs from the design tokens.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeView {
    /// The theme's own name, for the status line and for tests.
    pub name: String,
    /// The font family prose is drawn in.
    pub font_family: String,
    /// The font family code is drawn in.
    pub font_mono_family: String,
    /// Colours.
    pub colors: Colors,
    /// Control states.
    pub controls: ControlColors,
    /// Editable field states.
    pub input: InputColors,
    /// Sizes.
    pub geometry: Geometry,
}

/// Projects the tokens into the object the QML tree binds to.
pub fn project(tokens: &Tokens) -> ThemeView {
    let palette = Palette::of(tokens);

    ThemeView {
        name: tokens.name.clone(),
        font_family: tokens.typography.font_family.clone(),
        font_mono_family: tokens.typography.font_mono_family.clone(),
        colors: Colors {
            window: qml_color(palette.window),
            surface: qml_color(palette.surface),
            surface_variant: qml_color(palette.surface_variant),
            border: qml_color(palette.border),
            text: qml_color(palette.text),
            text_muted: qml_color(palette.text_muted),
            accent: qml_color(palette.accent),
            on_accent: qml_color(palette.on_accent),
            danger: qml_color(palette.danger),
            warning: qml_color(palette.warning),
            success: qml_color(palette.success),
            scrim: qml_color(palette.scrim()),
            dialog_border: qml_color(palette.warning),
        },
        controls: control_colors(tokens),
        input: input_colors(tokens),
        geometry: Geometry::of(tokens),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_leaves_as_qt_writes_it() {
        let colour = Rgba {
            r: 0x12,
            g: 0x34,
            b: 0x56,
            a: 0xff,
        };

        assert_eq!(qml_color(colour), "#ff123456");

        let translucent = Rgba { a: 0x80, ..colour };
        assert_eq!(qml_color(translucent), "#80123456");
    }

    #[test]
    fn the_geometry_comes_from_the_tokens() {
        let tokens = Tokens::builtin_dark();
        let geometry = Geometry::of(&tokens);

        assert!((geometry.page_padding - f32::from(tokens.spacing.xl)).abs() < f32::EPSILON);
        assert!((geometry.card_padding - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON);
        assert!((geometry.notice_padding - f32::from(tokens.spacing.md)).abs() < f32::EPSILON);
        assert!((geometry.section_gap - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON);
        assert!((geometry.row_gap - f32::from(tokens.spacing.sm)).abs() < f32::EPSILON);
        assert!(
            (geometry.nav_width - f32::from(tokens.spacing.xl) * NAV_WIDTH_STEPS).abs()
                < f32::EPSILON
        );
        assert!(geometry.label_width > geometry.nav_width / 2.0);
        assert!(geometry.control_height >= f32::from(tokens.spacing.xl));
        assert!(geometry.dialog_width > geometry.wide_width);
        assert!((geometry.radius - f32::from(tokens.radii.sm)).abs() < f32::EPSILON);
        assert!((geometry.card_radius - f32::from(tokens.radii.md)).abs() < f32::EPSILON);

        // One screen carries one title, its cards carry headings and everything
        // that explains something is a caption (ui-guidelines §2).
        assert_eq!(geometry.title_size, title_size(&tokens));
        assert_eq!(geometry.heading_size, heading_size(&tokens));
        assert_eq!(geometry.body_size, body_size(&tokens));
        assert_eq!(geometry.caption_size, caption_size(&tokens));
        assert!(geometry.title_size > geometry.heading_size);
        assert!(geometry.heading_size > geometry.body_size);
        assert!(geometry.body_size > geometry.caption_size);
        assert!(geometry.list_as_table);
    }

    #[test]
    fn scaled_text_grows_with_the_tokens_and_the_columns_with_it() {
        let mut tokens = Tokens::builtin_dark();
        let plain = Geometry::of(&tokens);

        tokens.typography.scale = 2.0;
        let scaled = Geometry::of(&tokens);

        assert!(scaled.body_size > plain.body_size);
        assert!(scaled.nav_width > plain.nav_width);
        assert!(scaled.label_width > plain.label_width);
        assert!(scaled.control_height > plain.control_height);
        assert!(scaled.heading_size > plain.heading_size);
        assert!(
            !scaled.list_as_table,
            "a table whose columns are narrower than the words in its headings is \
             drawn one field per row instead"
        );
    }

    #[test]
    fn every_colour_of_the_projection_is_a_token() {
        let tokens = Tokens::builtin_dark();
        let view = project(&tokens);
        let palette = Palette::of(&tokens);

        assert_eq!(view.colors.window, qml_color(palette.window));
        assert_eq!(view.colors.text, qml_color(palette.text));
        assert_eq!(view.colors.text_muted, qml_color(palette.text_muted));
        assert_eq!(view.colors.accent, qml_color(palette.accent));
        assert_eq!(view.colors.on_accent, qml_color(palette.on_accent));
        assert_eq!(view.colors.danger, qml_color(palette.danger));
        assert_eq!(view.colors.warning, qml_color(palette.warning));
        assert_eq!(view.colors.success, qml_color(palette.success));
        assert_eq!(view.colors.surface, qml_color(palette.surface));
        assert_eq!(
            view.colors.surface_variant,
            qml_color(palette.surface_variant)
        );
        assert_eq!(view.colors.border, qml_color(palette.border));
        assert_eq!(view.colors.scrim, qml_color(palette.scrim()));
        assert_eq!(view.input.background, qml_color(palette.surface_variant));

        // A colour is a string Qt can parse, never a name or a number.
        for colour in [
            &view.colors.window,
            &view.colors.accent,
            &view.colors.scrim,
            &view.controls.plain.foreground,
            &view.controls.disabled.border,
            &view.input.placeholder,
        ] {
            assert!(
                colour.starts_with('#') && colour.len() == 9,
                "`{colour}` is not an #AARRGGBB string"
            );
        }
    }

    #[test]
    fn a_control_that_cannot_be_used_is_drawn_differently_from_one_that_can() {
        let tokens = Tokens::builtin_dark();
        let controls = control_colors(&tokens);

        assert_ne!(controls.plain, controls.disabled);
        assert_ne!(controls.plain, controls.plain_focused);
        assert_ne!(controls.plain_focused, controls.selected);
        assert_ne!(
            controls.selected, controls.selected_focused,
            "the focus has to stay visible on the option that is already current"
        );
        assert_ne!(controls.plain, controls.pressed);
        assert_ne!(
            controls.danger, controls.danger_focused,
            "a destructive action shows the focus ring too"
        );
        assert_eq!(
            controls.disabled.border_width, controls.plain.border_width,
            "a disabled control is not drawn with the focus ring"
        );
        assert!(
            controls.plain_focused.border_width > controls.plain.border_width,
            "the focus ring is wider than a hairline"
        );
    }

    #[test]
    fn a_control_with_no_fill_is_transparent_rather_than_named() {
        let tokens = Tokens::builtin_dark();
        let controls = control_colors(&tokens);

        // "No fill" is a colour, not the word `transparent` (which QML would take,
        // but which would leave the value untyped): an alpha of zero, and the
        // window's own colour underneath it, so nothing new enters the window.
        let expected = qml_color(Rgba {
            a: 0,
            ..tokens.colors.background
        });

        assert_eq!(controls.plain.background, expected);
        assert_eq!(controls.disabled.background, expected);
        assert_eq!(controls.plain.background.len(), 9);
        assert!(
            controls.plain.background.starts_with("#00"),
            "a control with no fill is drawn with zero alpha: {}",
            controls.plain.background
        );
    }

    #[test]
    fn a_selected_control_is_drawn_on_the_accent_colour() {
        let tokens = Tokens::builtin_dark();
        let controls = control_colors(&tokens);

        assert_eq!(
            controls.selected.background,
            qml_color(tokens.colors.accent)
        );
        assert_eq!(
            controls.selected.foreground,
            qml_color(tokens.colors.on_accent)
        );
        assert_ne!(controls.selected.foreground, controls.plain.foreground);
    }

    #[test]
    fn a_destructive_action_never_fills_the_accent_colour() {
        let tokens = Tokens::builtin_dark();
        let controls = control_colors(&tokens);

        for style in [
            &controls.danger,
            &controls.danger_focused,
            &controls.danger_hovered,
        ] {
            assert_ne!(style.background, qml_color(tokens.colors.accent));
            assert_eq!(style.foreground, qml_color(tokens.colors.danger));
        }

        assert_eq!(
            controls.danger_pressed.background,
            qml_color(tokens.colors.danger),
            "a press is answered where the pointer is, in the action's own colour"
        );
    }

    #[test]
    fn the_status_line_takes_the_colour_of_the_state_it_reports() {
        let tokens = Tokens::builtin_dark();
        let palette = Palette::of(&tokens);

        assert_eq!(
            status_border(&tokens, StatusKind::Applied),
            qml_color(palette.success)
        );
        assert_eq!(
            status_border(&tokens, StatusKind::Rejected),
            qml_color(palette.danger)
        );
        assert_eq!(
            status_border(&tokens, StatusKind::SavedOnly),
            qml_color(palette.warning)
        );
        assert_eq!(
            status_border(&tokens, StatusKind::Provisional),
            qml_color(palette.warning)
        );
        assert_eq!(
            status_border(&tokens, StatusKind::Unchanged),
            qml_color(palette.border)
        );
    }

    #[test]
    fn the_hairline_is_half_the_smallest_spacing_step() {
        let mut tokens = Tokens::builtin_dark();
        assert!((hairline(&tokens) - 1.0).abs() < f32::EPSILON);

        tokens.spacing.xs = 8;
        assert!((hairline(&tokens) - 4.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_text_size_follows_the_users_scale() {
        let mut tokens = Tokens::builtin_dark();
        assert!((font_size(&tokens) - 16.0).abs() < f32::EPSILON);

        tokens.typography.scale = 2.0;
        assert!(
            (font_size(&tokens) - 32.0).abs() < f32::EPSILON,
            "font scaling is a token, not a hard-coded size"
        );
        assert!(heading_size(&tokens) > font_size(&tokens));
    }

    #[test]
    fn the_four_type_roles_are_ordered_and_derived_from_the_scale() {
        let mut tokens = Tokens::builtin_dark();

        assert!((title_size(&tokens) - font_size(&tokens) * 1.5).abs() < f32::EPSILON);
        assert!((heading_size(&tokens) - font_size(&tokens) * 1.25).abs() < f32::EPSILON);
        assert!((body_size(&tokens) - font_size(&tokens)).abs() < f32::EPSILON);
        assert!((caption_size(&tokens) - font_size(&tokens) * 0.875).abs() < f32::EPSILON);
        assert!(title_size(&tokens) > heading_size(&tokens));
        assert!(heading_size(&tokens) > body_size(&tokens));
        assert!(body_size(&tokens) > caption_size(&tokens));
        assert!(
            caption_size(&tokens) >= font_size(&tokens) * 0.75,
            "the guidelines set a floor for the caption role"
        );

        tokens.typography.scale = 2.0;
        assert!((title_size(&tokens) - 48.0).abs() < f32::EPSILON);
        assert!((caption_size(&tokens) - 28.0).abs() < f32::EPSILON);
    }

    #[test]
    fn every_spacing_role_comes_from_a_step_of_the_scale() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(page_padding(&tokens), f32::from(tokens.spacing.xl));
        assert_eq!(card_padding(&tokens), f32::from(tokens.spacing.lg));
        assert_eq!(notice_padding(&tokens), f32::from(tokens.spacing.md));
        assert_eq!(section_gap(&tokens), f32::from(tokens.spacing.lg));
        assert_eq!(row_gap(&tokens), f32::from(tokens.spacing.sm));
        assert_eq!(inline_gap(&tokens), f32::from(tokens.spacing.sm));

        assert_eq!(
            control_padding_vertical(&tokens),
            f32::from(tokens.spacing.sm)
        );
        assert_eq!(
            control_padding_horizontal(&tokens),
            f32::from(tokens.spacing.md)
        );

        // A list entry is a full control tall without a fixed height, so a
        // wrapped label grows instead of being cut off.
        let vertical = list_padding_vertical(&tokens);
        assert_eq!(
            list_padding_horizontal(&tokens),
            f32::from(tokens.spacing.md)
        );
        assert!(
            (vertical + body_size(&tokens) + vertical - control_height(&tokens)).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn a_short_value_is_not_drawn_as_a_wide_one() {
        let tokens = Tokens::builtin_dark();
        let xl = f32::from(tokens.spacing.xl);

        assert!(short_width(&tokens) <= xl * 6.0);
        assert!(wide_width(&tokens) <= xl * 12.0);
        assert!(short_width(&tokens) < wide_width(&tokens));
        assert!((list_action_width(&tokens) - wide_width(&tokens) / 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_control_is_at_least_one_extra_large_step_tall() {
        let mut tokens = Tokens::builtin_dark();

        assert!((control_height(&tokens) - f32::from(tokens.spacing.xl)).abs() < f32::EPSILON);

        tokens.typography.scale = 2.0;
        assert!(
            control_height(&tokens) > f32::from(tokens.spacing.xl),
            "a scaled-up control grows rather than cutting its text off"
        );
        assert!(control_height(&tokens) >= body_size(&tokens));
    }

    #[test]
    fn a_fields_line_box_is_what_makes_it_tall_enough() {
        let mut tokens = Tokens::builtin_dark();
        let height = control_line_height(&tokens) + control_padding_vertical(&tokens) * 2.0;

        assert!(
            (height - control_height(&tokens)).abs() < f32::EPSILON,
            "padding plus line box is the control's height"
        );

        tokens.typography.scale = 2.0;
        let height = control_line_height(&tokens) + control_padding_vertical(&tokens) * 2.0;

        assert!((height - control_height(&tokens)).abs() < f32::EPSILON);
        assert!(control_line_height(&tokens) >= body_size(&tokens));
    }

    #[test]
    fn the_focus_ring_is_wider_than_a_hairline() {
        let tokens = Tokens::builtin_dark();
        let controls = control_colors(&tokens);

        assert_eq!(controls.plain_focused.border_width, focus_ring(&tokens));
        assert!(controls.plain_focused.border_width > controls.plain.border_width);
        assert_ne!(
            controls.plain_focused.border_width, controls.plain_hovered.border_width,
            "the focus is told apart from a hover by its width as well as its colour"
        );
        assert_ne!(
            controls.plain_hovered.background, controls.plain.background,
            "a hover is visible"
        );
    }

    #[test]
    fn hovering_the_current_value_keeps_its_text_readable() {
        let tokens = Tokens::builtin_dark();
        let controls = control_colors(&tokens);

        assert_eq!(
            controls.selected.background,
            qml_color(tokens.colors.accent),
            "the accent background stays, so the on-accent text is readable"
        );
        assert_eq!(
            controls.selected.foreground,
            qml_color(tokens.colors.on_accent)
        );
    }

    #[test]
    fn a_field_shows_every_state_differently() {
        let tokens = Tokens::builtin_dark();
        let input = input_colors(&tokens);

        assert_eq!(
            input.background,
            qml_color(tokens.colors.surface_variant),
            "a field is a block of its own inside a card"
        );
        assert_eq!(input.border, qml_color(tokens.colors.border));
        assert_ne!(input.border_hovered, input.border);
        assert_eq!(input.border_focused, qml_color(tokens.colors.accent));
        assert_eq!(input.focus_width, focus_ring(&tokens));
        assert_eq!(input.value_disabled, qml_color(tokens.colors.text_muted));
        assert_eq!(input.background_disabled, qml_color(tokens.colors.surface));
        assert_eq!(input.border_refused, qml_color(tokens.colors.danger));
        assert_ne!(input.value, input.value_disabled);
    }

    #[test]
    fn a_card_is_the_surface_colour_inside_a_hairline() {
        let tokens = Tokens::builtin_dark();
        let view = project(&tokens);

        assert_eq!(view.colors.surface, qml_color(tokens.colors.surface));
        assert_eq!(view.colors.border, qml_color(tokens.colors.border));
        assert_eq!(view.geometry.hairline, hairline(&tokens));
        assert_eq!(view.geometry.card_radius, f32::from(tokens.radii.md));
        assert_eq!(view.geometry.radius, f32::from(tokens.radii.sm));
    }

    #[test]
    fn the_emphasis_weight_is_the_bold_weight() {
        assert_eq!(emphasis_weight(), 700);
    }

    #[test]
    fn the_light_theme_keeps_its_own_palette() {
        let dark = project(&Tokens::builtin_dark());
        let light = project(&Tokens::builtin_light());

        assert_ne!(dark.colors.window, light.colors.window);
        assert_ne!(dark.colors.text, light.colors.text);
        assert_ne!(dark.name, light.name);
    }
}
