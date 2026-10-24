//! Inox design tokens, turned into iced's theme and style closures.
//!
//! Root `AGENTS.md` §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! iced, meanwhile, expects an [`iced::Theme`] and a handful of `Style` structs
//! built from it. This module is the join between the two, and it is the only
//! place in the crate where a colour becomes a widget's.
//!
//! Every function takes the tokens as an argument rather than reaching for a
//! global, so the view can hand the styles the same theme the rest of the
//! window is drawn with — and so a test can render the same widget tree under
//! the light and the dark tokens (acceptance Q02).
//!
//! `docs/quality/ui-guidelines.md` §1 draws the line this module sits on: the
//! token set carries colours, spacing, radii and durations but no type scale and
//! no border width, so everything the window draws is either a token or one of
//! the named derivations below — the four type roles (§2), the spacing rhythm
//! (§3), the control geometry (§4) and the five interaction states (§5). A
//! reviewer should be able to read a number here and see where it came from;
//! finding one in [`crate::view`] instead is the defect the guidelines name.
//!
//! ADR 0018 explains why there is a copy of this file in every L3 component
//! instead of one shared crate: the duplication is the price of not adding a
//! same-layer Cargo dependency that would need its own ADR.

use iced::widget::button;
use iced::widget::container;
use iced::{Border, Color, Font, Padding, Theme};
use inox_config::{Rgba, Theme as Tokens};

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

/// How much of the window still shows through the trial dialog's scrim, in
/// percent of the background token's own alpha.
const SCRIM_OPACITY_PERCENT: u8 = 60;

/// The weight a title and a heading are drawn at.
///
/// The guidelines ask for emphasis by weight rather than by another size step,
/// and the token set has no weight, so this is the one place it is written.
const EMPHASIS_WEIGHT: iced::font::Weight = iced::font::Weight::Bold;

/// A token colour, as iced wants it.
pub fn color(value: Rgba) -> Color {
    Color::from_rgba8(value.r, value.g, value.b, f32::from(value.a) / 255.0)
}

/// The iced theme the settings window runs under.
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

// -- Type roles (guidelines §2) -------------------------------------------------

/// The size of a line of body text, in logical pixels.
///
/// The token set carries a family and the user's scale but no size, so the size
/// is one large spacing step times the scale: a user who doubles the scale gets
/// text twice as tall, and the layout in [`crate::view`] is what keeps the
/// result inside the window (acceptance Q02).
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

/// The font a title or a heading is drawn in.
pub fn emphasis_font() -> Font {
    Font {
        weight: EMPHASIS_WEIGHT,
        ..Font::DEFAULT
    }
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
pub fn page_padding(tokens: &Tokens) -> Padding {
    Padding::from(f32::from(tokens.spacing.xl))
}

/// The padding inside a card: `lg` (guidelines §3).
pub fn card_padding(tokens: &Tokens) -> Padding {
    Padding::from(f32::from(tokens.spacing.lg))
}

/// The padding inside a status or notice strip: `md` (guidelines §3).
pub fn notice_padding(tokens: &Tokens) -> Padding {
    Padding::from(f32::from(tokens.spacing.md))
}

/// The padding inside a control: `sm` above and below, `md` at the sides.
pub fn control_padding(tokens: &Tokens) -> Padding {
    Padding {
        top: f32::from(tokens.spacing.sm),
        right: f32::from(tokens.spacing.md),
        bottom: f32::from(tokens.spacing.sm),
        left: f32::from(tokens.spacing.md),
    }
}

/// The padding inside a list entry: an option, a switch, an entry of the
/// navigation list.
///
/// It is the least vertical padding that makes a one-line entry exactly
/// [`control_height`] tall, which is what the guidelines ask of an interactive
/// target (§4). A button takes its height from its padding and its text rather
/// than from a fixed height, so an entry whose label wraps at a large font scale
/// grows instead of being cut off (Q02).
pub fn list_padding(tokens: &Tokens) -> Padding {
    let vertical = (control_height(tokens) - body_size(tokens)) / 2.0;

    Padding {
        top: vertical,
        right: f32::from(tokens.spacing.md),
        bottom: vertical,
        left: f32::from(tokens.spacing.md),
    }
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
/// iced sizes a text field from its padding and its line box, so widening the
/// line box is how a field reaches [`control_height`] without a second padding
/// role: the text stays vertically centred in it.
pub fn control_line_height(tokens: &Tokens) -> iced::Pixels {
    let line = control_height(tokens) - f32::from(tokens.spacing.sm) * 2.0;

    iced::Pixels(line.max(body_size(tokens)))
}

/// The corner radius of a control.
pub fn radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.sm).into()
}

/// The corner radius of a card, a panel or the trial dialog.
pub fn card_radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// A border in the given colour, at the token's control radius.
fn border(tokens: &Tokens, color: Color) -> Border {
    Border {
        color,
        width: hairline(tokens),
        radius: radius(tokens),
    }
}

/// The outline of the control the keyboard is on.
///
/// The width is the focus ring's, so a focused control differs from a hovered
/// one even when both are outlined in the accent colour (guidelines §5).
fn focus_border(tokens: &Tokens, color: Color) -> Border {
    Border {
        color,
        width: focus_ring(tokens),
        radius: radius(tokens),
    }
}

// -- Colours -------------------------------------------------------------------

/// Default text colour.
pub fn text(tokens: &Tokens) -> Color {
    color(tokens.colors.text)
}

/// Colour of secondary information.
pub fn muted_text(tokens: &Tokens) -> Color {
    color(tokens.colors.text_muted)
}

/// Colour for something a user should look at: a value the schema refuses, or
/// a change that did not take effect.
pub fn danger_text(tokens: &Tokens) -> Color {
    color(tokens.colors.danger)
}

/// Colour for a change that is saved but not yet in effect.
pub fn warning_text(tokens: &Tokens) -> Color {
    color(tokens.colors.warning)
}

/// Colour for a change that is in effect.
pub fn success_text(tokens: &Tokens) -> Color {
    color(tokens.colors.success)
}

/// Colour of the text on the accent colour.
pub fn on_accent(tokens: &Tokens) -> Color {
    color(tokens.colors.on_accent)
}

// -- Surfaces ------------------------------------------------------------------

/// The window itself.
pub fn window(tokens: &Tokens) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.background).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The navigation list down the left-hand side.
pub fn sidebar(tokens: &Tokens) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        border: Border {
            radius: iced::border::Radius::default(),
            ..border(tokens, color(tokens.colors.border))
        },
        ..container::Style::default()
    }
}

/// A card: one topic's fields, on the surface colour inside a hairline.
///
/// The guidelines (§4) define a card as `surface` + hairline + `md` radius, and
/// the window adds `lg` padding in [`crate::view`]. Nothing else is layered on
/// top: a settings window is not a place for shadows.
pub fn card(tokens: &Tokens) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        border: Border {
            radius: card_radius(tokens),
            ..border(tokens, color(tokens.colors.border))
        },
        ..container::Style::default()
    }
}

/// What the window is dimmed with while a display trial is open.
///
/// The scrim is the window's own background at less than full opacity, so it is
/// the theme that decides how much the question stands out — no component-local
/// colour enters the window (root `AGENTS.md` §4).
pub fn scrim(tokens: &Tokens) -> container::Style {
    let background = tokens.colors.background;
    let veil = Rgba {
        a: (u16::from(background.a) * u16::from(SCRIM_OPACITY_PERCENT) / 100) as u8,
        ..background
    };

    container::Style {
        background: Some(color(veil).into()),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The card a display trial is asked in.
///
/// It sits above the scrim on the surface colour, with the border the theme uses
/// for a raised panel, and the warning colour on the edge because the change it
/// describes is not settled yet (ADR 0028).
pub fn dialog(tokens: &Tokens) -> container::Style {
    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        border: Border {
            radius: card_radius(tokens),
            ..border(tokens, color(tokens.colors.warning))
        },
        shadow: iced::Shadow {
            color: color(tokens.colors.border),
            offset: iced::Vector::new(0.0, hairline(tokens)),
            blur_radius: hairline(tokens) * 4.0,
        },
        ..container::Style::default()
    }
}

/// The status line, coloured by how far the change has got.
///
/// Nothing here claims more than [`crate::apply`] knows: a change that is saved
/// but not confirmed is drawn in the warning colour, and one that is in effect
/// in the success colour, so the two are distinguishable at a glance as well as
/// in the sentence (ADR 0014, decision 4). A display trial is in effect but not
/// settled, so it takes the warning colour too (ADR 0028).
pub fn status(tokens: &Tokens, kind: StatusKind) -> container::Style {
    let border_colour = match kind {
        StatusKind::Rejected => color(tokens.colors.danger),
        StatusKind::Pending | StatusKind::SavedOnly | StatusKind::Provisional => {
            color(tokens.colors.warning)
        }
        StatusKind::Applied => color(tokens.colors.success),
        StatusKind::Unchanged | StatusKind::Modified | StatusKind::Saving => {
            color(tokens.colors.border)
        }
    };

    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        border: Border {
            radius: card_radius(tokens),
            ..border(tokens, border_colour)
        },
        ..container::Style::default()
    }
}

// -- Controls and their five states (guidelines §5) ----------------------------

/// How a control is emphasized.
///
/// Two independent facts, because a control can be both: the window's own
/// navigation can be sitting on the option that already holds the current
/// value, and the user still has to see where the keyboard is (Q01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Emphasis {
    /// The control holds the current value.
    pub selected: bool,
    /// The window's own navigation is on this control.
    pub focused: bool,
}

impl Emphasis {
    /// Neither selected nor focused.
    pub const PLAIN: Self = Self {
        selected: false,
        focused: false,
    };

    /// Holds the current value, and the keyboard is somewhere else.
    pub const SELECTED: Self = Self {
        selected: true,
        focused: false,
    };

    /// The keyboard is on it, and it holds some other value.
    pub const FOCUSED: Self = Self {
        selected: false,
        focused: true,
    };

    /// The keyboard is on the option that holds the current value.
    pub const FOCUSED_SELECTED: Self = Self {
        selected: true,
        focused: true,
    };
}

/// How an action is emphasized.
///
/// The three roles are what keeps a settings window calm: exactly one accent
/// action per screen, everything else quiet, and anything that throws work away
/// drawn in the danger colour rather than in the accent (guidelines §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The one action the window wants (Apply) — the only accent-filled control.
    Primary,
    /// An ordinary action: Cancel, Reload, Add entry.
    Secondary,
    /// An action that discards something: Reset, Reset all, Remove entry.
    Danger,
}

/// The style of one control: a page entry, an option, a switch, a step button.
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
pub fn control(tokens: &Tokens, emphasis: Emphasis, status: button::Status) -> button::Style {
    let accent = color(tokens.colors.accent);
    let border_colour = color(tokens.colors.border);
    let focused = emphasis.focused;
    let focus_outline = |on_accent_background: bool| {
        if focused && !on_accent_background {
            focus_border(tokens, accent)
        } else if focused {
            focus_border(tokens, on_accent(tokens))
        } else if on_accent_background {
            border(tokens, accent)
        } else {
            border(tokens, border_colour)
        }
    };

    let (background, foreground, outline) = match status {
        button::Status::Disabled => (None, muted_text(tokens), border(tokens, border_colour)),
        button::Status::Pressed => (Some(accent.into()), on_accent(tokens), focus_outline(true)),
        button::Status::Hovered if emphasis.selected => {
            (Some(accent.into()), on_accent(tokens), focus_outline(true))
        }
        button::Status::Hovered => (
            Some(color(tokens.colors.surface_variant).into()),
            text(tokens),
            focus_outline(false),
        ),
        button::Status::Active => match (emphasis.selected, focused) {
            (true, _) => (Some(accent.into()), on_accent(tokens), focus_outline(true)),
            (false, false) => (None, text(tokens), focus_outline(false)),
            (false, true) => (
                Some(color(tokens.colors.surface_variant).into()),
                accent,
                focus_outline(false),
            ),
        },
    };

    button::Style {
        background,
        text_color: foreground,
        border: outline,
        ..button::Style::default()
    }
}

/// The style of one option of a segmented control.
///
/// A segment is a control that holds one of a few values, so its five states are
/// [`control`]'s with the current value drawn as the selection.
pub fn segment(
    tokens: &Tokens,
    selected: bool,
    focused: bool,
    status: button::Status,
) -> button::Style {
    control(tokens, Emphasis { selected, focused }, status)
}

/// The style of a switch and the word it is showing.
///
/// A switch is a control that is on or off rather than one of a set, and "on" is
/// drawn as the selection: accent background, on-accent text. The pressed and
/// disabled states are the ones every other control uses, so a switch a user
/// cannot work has the same look as any other disabled control.
pub fn switch(tokens: &Tokens, on: bool, focused: bool, status: button::Status) -> button::Style {
    control(
        tokens,
        Emphasis {
            selected: on,
            focused,
        },
        status,
    )
}

/// The style of a destructive action.
///
/// The guidelines keep `accent` for the one primary action per screen and draw
/// what throws work away in `danger` — as text and as an outline, never as a
/// filled accent button. The pressed state is filled in the danger colour so the
/// press is answered where the pointer is, and the focus is the accent ring
/// every other control is focused with.
fn destructive(tokens: &Tokens, focused: bool, status: button::Status) -> button::Style {
    let danger = danger_text(tokens);
    let border_colour = color(tokens.colors.border);

    let (background, foreground, outline) = match status {
        button::Status::Disabled => (None, muted_text(tokens), border(tokens, border_colour)),
        button::Status::Pressed => (
            Some(danger.into()),
            on_accent(tokens),
            focus_border(tokens, on_accent(tokens)),
        ),
        button::Status::Hovered => (
            Some(color(tokens.colors.surface_variant).into()),
            danger,
            if focused {
                focus_border(tokens, color(tokens.colors.accent))
            } else {
                border(tokens, danger)
            },
        ),
        button::Status::Active => (
            None,
            danger,
            if focused {
                focus_border(tokens, color(tokens.colors.accent))
            } else {
                border(tokens, danger)
            },
        ),
    };

    button::Style {
        background,
        text_color: foreground,
        border: outline,
        ..button::Style::default()
    }
}

/// The style of one action at the foot of the window, or beside a field.
///
/// `Role::Primary` is the action the window wants the user to take — Apply — and
/// it is the only one drawn on the accent colour, so the button that writes a
/// file is never the one a user presses by accident. The focus is drawn on top
/// of that, not instead of it.
pub fn action(tokens: &Tokens, role: Role, focused: bool, status: button::Status) -> button::Style {
    match role {
        Role::Primary => control(
            tokens,
            Emphasis {
                selected: true,
                focused,
            },
            status,
        ),
        Role::Secondary => control(
            tokens,
            Emphasis {
                selected: false,
                focused,
            },
            status,
        ),
        Role::Danger => destructive(tokens, focused, status),
    }
}

/// The five states of an editable field.
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
pub fn input(
    tokens: &Tokens,
    status: iced::widget::text_input::Status,
    refused: bool,
) -> iced::widget::text_input::Style {
    let disabled = status == iced::widget::text_input::Status::Disabled;
    let hovered = status == iced::widget::text_input::Status::Hovered;
    let focused = matches!(status, iced::widget::text_input::Status::Focused { .. });

    let outline = if refused {
        if focused {
            focus_border(tokens, color(tokens.colors.danger))
        } else {
            border(tokens, color(tokens.colors.danger))
        }
    } else if focused {
        focus_border(tokens, color(tokens.colors.accent))
    } else if hovered {
        border(tokens, text(tokens))
    } else {
        border(tokens, color(tokens.colors.border))
    };

    iced::widget::text_input::Style {
        background: if disabled {
            color(tokens.colors.surface).into()
        } else {
            color(tokens.colors.surface_variant).into()
        },
        border: outline,
        icon: text(tokens),
        placeholder: muted_text(tokens),
        value: if disabled {
            muted_text(tokens)
        } else {
            text(tokens)
        },
        selection: color(tokens.colors.accent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn the_window_wears_the_background_token() {
        let tokens = Tokens::builtin_dark();
        let style = window(&tokens);

        assert_eq!(
            style.background,
            Some(color(tokens.colors.background).into())
        );
        assert_eq!(style.text_color, Some(text(&tokens)));
    }

    #[test]
    fn the_sidebar_wears_the_surface_token() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(
            sidebar(&tokens).background,
            Some(color(tokens.colors.surface).into())
        );
    }

    #[test]
    fn a_selected_control_is_drawn_on_the_accent_colour() {
        let tokens = Tokens::builtin_dark();
        let selected = control(&tokens, Emphasis::SELECTED, button::Status::Active);
        let plain = control(&tokens, Emphasis::PLAIN, button::Status::Active);

        assert_eq!(
            selected.background,
            Some(color(tokens.colors.accent).into())
        );
        assert_eq!(selected.text_color, on_accent(&tokens));
        assert_eq!(plain.background, None);
        assert_ne!(selected.text_color, plain.text_color);
    }

    #[test]
    fn the_focus_is_visible_in_every_state() {
        let tokens = Tokens::builtin_dark();
        let accent = color(tokens.colors.accent);

        let plain = control(&tokens, Emphasis::PLAIN, button::Status::Active);
        let focused = control(&tokens, Emphasis::FOCUSED, button::Status::Active);
        let selected = control(&tokens, Emphasis::SELECTED, button::Status::Active);
        let both = control(&tokens, Emphasis::FOCUSED_SELECTED, button::Status::Active);

        assert_eq!(focused.border.color, accent);
        assert_ne!(focused.text_color, plain.text_color);

        // Selected: on the accent colour, and the outline disappears into it.
        assert_eq!(selected.background, Some(accent.into()));
        assert_eq!(selected.border.color, accent);

        // Selected *and* focused: the outline is taken from the text colour,
        // because an accent outline on an accent background is invisible.
        assert_eq!(both.background, Some(accent.into()));
        assert_eq!(both.border.color, on_accent(&tokens));
        assert_ne!(both.border.color, selected.border.color);
        assert_ne!(plain.text_color, both.text_color);
    }

    #[test]
    fn a_disabled_control_looks_different_from_an_enabled_one() {
        let tokens = Tokens::builtin_dark();
        let enabled = control(&tokens, Emphasis::PLAIN, button::Status::Active);
        let disabled = control(&tokens, Emphasis::PLAIN, button::Status::Disabled);

        assert_ne!(enabled.text_color, disabled.text_color);
        assert_eq!(disabled.background, None);
        assert_eq!(disabled.text_color, muted_text(&tokens));
    }

    #[test]
    fn the_primary_action_is_the_only_one_on_the_accent_colour() {
        let tokens = Tokens::builtin_dark();
        let apply = action(&tokens, Role::Primary, false, button::Status::Active);
        let cancel = action(&tokens, Role::Secondary, false, button::Status::Active);
        let reset = action(&tokens, Role::Danger, false, button::Status::Active);

        assert_eq!(apply.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(cancel.background, None);
        assert_eq!(
            reset.background, None,
            "the accent is kept for the one primary action"
        );
        assert_eq!(reset.text_color, danger_text(&tokens));
        assert_eq!(reset.border.color, danger_text(&tokens));
        assert_ne!(cancel.text_color, reset.text_color);
    }

    #[test]
    fn the_primary_action_still_shows_the_focus() {
        let tokens = Tokens::builtin_dark();
        let focused = action(&tokens, Role::Primary, true, button::Status::Active);
        let selected = action(&tokens, Role::Primary, false, button::Status::Active);

        assert_ne!(
            focused.border.color, selected.border.color,
            "the focus has to be visible on the primary action too"
        );
    }

    #[test]
    fn a_control_keeps_the_token_radius_and_the_derived_hairline() {
        let tokens = Tokens::builtin_dark();
        let style = control(&tokens, Emphasis::PLAIN, button::Status::Active);

        assert_eq!(style.border.radius, radius(&tokens));
        assert!((style.border.width - hairline(&tokens)).abs() < f32::EPSILON);
        assert!(style.border.width > 0.0);
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
    fn the_status_line_says_what_the_change_has_done() {
        let tokens = Tokens::builtin_dark();

        assert_eq!(
            status(&tokens, StatusKind::Applied).border.color,
            color(tokens.colors.success)
        );
        assert_eq!(
            status(&tokens, StatusKind::Rejected).border.color,
            color(tokens.colors.danger)
        );
        assert_eq!(
            status(&tokens, StatusKind::Pending).border.color,
            color(tokens.colors.warning)
        );
        assert_eq!(
            status(&tokens, StatusKind::SavedOnly).border.color,
            status(&tokens, StatusKind::Pending).border.color,
        );
        assert_ne!(
            status(&tokens, StatusKind::Modified).border.color,
            status(&tokens, StatusKind::Rejected).border.color
        );
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

        assert_eq!(
            page_padding(&tokens),
            Padding::from(f32::from(tokens.spacing.xl))
        );
        assert_eq!(
            card_padding(&tokens),
            Padding::from(f32::from(tokens.spacing.lg))
        );
        assert_eq!(
            notice_padding(&tokens),
            Padding::from(f32::from(tokens.spacing.md))
        );
        assert_eq!(section_gap(&tokens), f32::from(tokens.spacing.lg));
        assert_eq!(row_gap(&tokens), f32::from(tokens.spacing.sm));
        assert_eq!(inline_gap(&tokens), f32::from(tokens.spacing.sm));

        let control = control_padding(&tokens);
        assert_eq!(control.top, f32::from(tokens.spacing.sm));
        assert_eq!(control.right, f32::from(tokens.spacing.md));
        assert_eq!(control.bottom, f32::from(tokens.spacing.sm));
        assert_eq!(control.left, f32::from(tokens.spacing.md));

        // A list entry is a full control tall without a fixed height, so a
        // wrapped label grows instead of being cut off.
        let list = list_padding(&tokens);
        assert_eq!(list.right, f32::from(tokens.spacing.md));
        assert_eq!(list.left, f32::from(tokens.spacing.md));
        assert!(
            (list.top + body_size(&tokens) + list.bottom - control_height(&tokens)).abs()
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
        let padding = control_padding(&tokens);
        let height = control_line_height(&tokens).0 + padding.top + padding.bottom;

        assert!(
            (height - control_height(&tokens)).abs() < f32::EPSILON,
            "padding plus line box is the control's height"
        );

        tokens.typography.scale = 2.0;
        let padding = control_padding(&tokens);
        let height = control_line_height(&tokens).0 + padding.top + padding.bottom;

        assert!((height - control_height(&tokens)).abs() < f32::EPSILON);
        assert!(control_line_height(&tokens).0 >= body_size(&tokens));
    }

    #[test]
    fn the_focus_ring_is_wider_than_a_hairline() {
        let tokens = Tokens::builtin_dark();
        let focused = control(&tokens, Emphasis::FOCUSED, button::Status::Active);
        let resting = control(&tokens, Emphasis::PLAIN, button::Status::Active);
        let hovered = control(&tokens, Emphasis::PLAIN, button::Status::Hovered);

        assert_eq!(focused.border.width, focus_ring(&tokens));
        assert!(focused.border.width > resting.border.width);
        assert_ne!(
            focused.border.width, hovered.border.width,
            "the focus is told apart from a hover by its width as well as its colour"
        );
        assert_eq!(resting.background, None);
        assert_eq!(
            hovered.background,
            Some(color(tokens.colors.surface_variant).into()),
            "a hover is visible"
        );
    }

    #[test]
    fn hovering_the_current_value_keeps_its_text_readable() {
        let tokens = Tokens::builtin_dark();
        let hovered = control(&tokens, Emphasis::SELECTED, button::Status::Hovered);

        assert_eq!(
            hovered.background,
            Some(color(tokens.colors.accent).into()),
            "the accent background stays, so the on-accent text is readable"
        );
        assert_eq!(hovered.text_color, on_accent(&tokens));
    }

    #[test]
    fn a_switch_and_a_segment_share_the_control_states() {
        let tokens = Tokens::builtin_dark();

        for on in [true, false] {
            for status in [
                button::Status::Active,
                button::Status::Hovered,
                button::Status::Pressed,
                button::Status::Disabled,
            ] {
                assert_eq!(
                    switch(&tokens, on, false, status),
                    control(
                        &tokens,
                        Emphasis {
                            selected: on,
                            focused: false
                        },
                        status
                    )
                );
                assert_eq!(
                    segment(&tokens, on, false, status),
                    control(
                        &tokens,
                        Emphasis {
                            selected: on,
                            focused: false
                        },
                        status
                    )
                );
            }
        }
    }

    #[test]
    fn a_field_shows_every_state_differently() {
        use iced::widget::text_input::Status;

        let tokens = Tokens::builtin_dark();
        let resting = input(&tokens, Status::Active, false);
        let hovered = input(&tokens, Status::Hovered, false);
        let focused = input(&tokens, Status::Focused { is_hovered: false }, false);
        let disabled = input(&tokens, Status::Disabled, false);
        let refused = input(&tokens, Status::Active, true);

        assert_eq!(
            resting.background,
            color(tokens.colors.surface_variant).into(),
            "a field is a block of its own inside a card"
        );
        assert_eq!(resting.border.color, color(tokens.colors.border));
        assert_ne!(hovered.border.color, resting.border.color);
        assert_eq!(focused.border.color, color(tokens.colors.accent));
        assert_eq!(focused.border.width, focus_ring(&tokens));
        assert_eq!(disabled.value, muted_text(&tokens));
        assert_eq!(disabled.background, color(tokens.colors.surface).into());
        assert_eq!(refused.border.color, danger_text(&tokens));
        assert_ne!(refused.value, disabled.value);

        for (left, right) in [
            (resting, hovered),
            (resting, disabled),
            (resting, focused),
            (hovered, focused),
            (disabled, refused),
        ] {
            assert_ne!(
                (
                    left.background,
                    left.border.color,
                    left.border.width,
                    left.value
                ),
                (
                    right.background,
                    right.border.color,
                    right.border.width,
                    right.value
                ),
                "two states of a field must be told apart"
            );
        }
    }

    #[test]
    fn a_refused_field_that_is_focused_shows_both_facts() {
        let tokens = Tokens::builtin_dark();
        let refused = input(&tokens, iced::widget::text_input::Status::Active, true);
        let refused_focused = input(
            &tokens,
            iced::widget::text_input::Status::Focused { is_hovered: false },
            true,
        );

        assert_eq!(refused_focused.border.color, danger_text(&tokens));
        assert_eq!(refused_focused.border.width, focus_ring(&tokens));
        assert_ne!(refused_focused.border.width, refused.border.width);
    }

    #[test]
    fn a_card_is_the_surface_colour_inside_a_hairline() {
        let tokens = Tokens::builtin_dark();
        let style = card(&tokens);

        assert_eq!(style.background, Some(color(tokens.colors.surface).into()));
        assert_eq!(style.border.color, color(tokens.colors.border));
        assert_eq!(style.border.width, hairline(&tokens));
        assert_eq!(style.border.radius, card_radius(&tokens));
        assert_eq!(style.border.radius, f32::from(tokens.radii.md).into());
        assert_eq!(card_radius(&tokens), f32::from(tokens.radii.md).into());
        assert_eq!(radius(&tokens), f32::from(tokens.radii.sm).into());
        assert_eq!(style.border.radius, iced::border::Radius::from(8.0));
    }

    #[test]
    fn a_destructive_action_never_fills_the_accent_colour() {
        let tokens = Tokens::builtin_dark();
        let accent = color(tokens.colors.accent);

        for (status, text_colour) in [
            (button::Status::Active, danger_text(&tokens)),
            (button::Status::Hovered, danger_text(&tokens)),
            (button::Status::Disabled, muted_text(&tokens)),
            // A press is answered where the pointer is, in the action's own
            // colour rather than in the accent's.
            (button::Status::Pressed, on_accent(&tokens)),
        ] {
            let style = action(&tokens, Role::Danger, false, status);

            assert_ne!(style.background, Some(accent.into()));
            assert_eq!(style.text_color, text_colour, "{status:?}");
        }

        let pressed = action(&tokens, Role::Danger, false, button::Status::Pressed);
        assert_eq!(pressed.background, Some(danger_text(&tokens).into()));

        let resting = action(&tokens, Role::Danger, false, button::Status::Active);
        assert_eq!(
            resting.border.color,
            danger_text(&tokens),
            "a destructive action is outlined in the danger colour"
        );
    }

    #[test]
    fn the_emphasis_font_is_the_bold_weight() {
        assert_eq!(emphasis_font().weight, iced::font::Weight::Bold);
    }
}
