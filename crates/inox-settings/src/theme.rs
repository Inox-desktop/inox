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
//! Two sizes are *derived* rather than taken, because the token set has no
//! type scale and no border width, and are written down here so a reviewer can
//! see the derivation instead of finding a number in a view:
//!
//! - the text size is one large spacing step times the user's font scale;
//! - a hairline is half the smallest spacing step, so a theme that scales its
//!   spacing scales its borders with it.
//!
//! ADR 0018 explains why there is a copy of this file in every L3 component
//! instead of one shared crate: the duplication is the price of not adding a
//! same-layer Cargo dependency that would need its own ADR.

use iced::widget::{button, container};
use iced::{Border, Color, Theme};
use inox_config::{Rgba, Theme as Tokens};

use crate::view_model::StatusKind;

/// How many large spacing steps make a line of text.
const TEXT_SIZE_STEPS: f32 = 1.0;

/// The divisor that turns the smallest spacing step into a hairline.
const HAIRLINE_DIVISOR: f32 = 2.0;

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

/// The size of a line of text, in logical pixels.
///
/// The token set carries a family and the user's scale but no size, so the size
/// is one large spacing step times the scale: a user who doubles the scale gets
/// text twice as tall, and the layout in [`crate::view`] is what keeps the
/// result inside the window (acceptance Q02).
pub fn font_size(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.lg) * TEXT_SIZE_STEPS * tokens.typography.scale
}

/// The thickness of a border, in logical pixels.
pub fn hairline(tokens: &Tokens) -> f32 {
    (f32::from(tokens.spacing.xs) / HAIRLINE_DIVISOR).max(0.0)
}

/// The size of a page's heading, in logical pixels.
pub fn heading_size(tokens: &Tokens) -> f32 {
    font_size(tokens) * 1.25
}

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

/// The corner radius of a control.
pub fn radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.sm).into()
}

/// The corner radius of the window's own surfaces.
pub fn surface_radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// A border in the given colour.
fn border(tokens: &Tokens, color: Color) -> Border {
    Border {
        color,
        width: hairline(tokens),
        radius: radius(tokens),
    }
}

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

/// The status line, coloured by how far the change has got.
///
/// Nothing here claims more than [`crate::apply`] knows: a change that is saved
/// but not confirmed is drawn in the warning colour, and one that is in effect
/// in the success colour, so the two are distinguishable at a glance as well as
/// in the sentence (ADR 0014, decision 4).
pub fn status(tokens: &Tokens, kind: StatusKind) -> container::Style {
    let border_colour = match kind {
        StatusKind::Rejected => color(tokens.colors.danger),
        StatusKind::Pending | StatusKind::SavedOnly => color(tokens.colors.warning),
        StatusKind::Applied => color(tokens.colors.success),
        StatusKind::Unchanged | StatusKind::Modified | StatusKind::Saving => {
            color(tokens.colors.border)
        }
    };

    container::Style {
        background: Some(color(tokens.colors.surface).into()),
        text_color: Some(text(tokens)),
        border: Border {
            radius: surface_radius(tokens),
            ..border(tokens, border_colour)
        },
        ..container::Style::default()
    }
}

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

/// The style of one control: a page entry, an option, a switch, a step button.
///
/// The four states are drawn so that no two of them look the same, and every
/// colour is a token:
///
/// | | background | text | outline |
/// |---|---|---|---|
/// | plain | none | text | border |
/// | focused | surface variant | accent | accent |
/// | selected | accent | on accent | accent |
/// | selected and focused | accent | on accent | on accent |
///
/// The pair that would otherwise be ambiguous is the last one: an outline in
/// the accent colour against an accent background is invisible, so a focused
/// selection is outlined in the colour the text already uses. That is what keeps
/// the focus visible on the option that is already current.
pub fn control(tokens: &Tokens, emphasis: Emphasis, status: button::Status) -> button::Style {
    let accent = color(tokens.colors.accent);
    let border_colour = color(tokens.colors.border);

    let (background, foreground, outline) = match status {
        button::Status::Disabled => (None, muted_text(tokens), border_colour),
        button::Status::Pressed => (Some(accent.into()), on_accent(tokens), on_accent(tokens)),
        button::Status::Hovered => (
            Some(color(tokens.colors.surface_variant).into()),
            if emphasis.selected {
                on_accent(tokens)
            } else {
                text(tokens)
            },
            if emphasis.focused {
                accent
            } else {
                border_colour
            },
        ),
        button::Status::Active => match (emphasis.selected, emphasis.focused) {
            (true, true) => (Some(accent.into()), on_accent(tokens), on_accent(tokens)),
            (true, false) => (Some(accent.into()), on_accent(tokens), accent),
            (false, true) => (
                Some(color(tokens.colors.surface_variant).into()),
                accent,
                accent,
            ),
            (false, false) => (None, text(tokens), border_colour),
        },
    };

    button::Style {
        background,
        text_color: foreground,
        border: border(tokens, outline),
        ..button::Style::default()
    }
}

/// The style of one action at the foot of the window.
///
/// `primary` is the action the window wants the user to take — Apply — and it is
/// the only one drawn on the accent colour, so the button that writes a file is
/// never the one a user presses by accident. The focus is drawn on top of that,
/// not instead of it.
pub fn action(
    tokens: &Tokens,
    primary: bool,
    focused: bool,
    status: button::Status,
) -> button::Style {
    control(
        tokens,
        Emphasis {
            selected: primary,
            focused,
        },
        status,
    )
}

/// Styles editable fields using the same tokens as the surrounding controls.
pub fn input(
    tokens: &Tokens,
    status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    let focused = matches!(status, iced::widget::text_input::Status::Focused { .. });
    iced::widget::text_input::Style {
        background: color(tokens.colors.surface).into(),
        border: border(
            tokens,
            color(if focused {
                tokens.colors.accent
            } else {
                tokens.colors.border
            }),
        ),
        icon: text(tokens),
        placeholder: muted_text(tokens),
        value: if status == iced::widget::text_input::Status::Disabled {
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
        let apply = action(&tokens, true, false, button::Status::Active);
        let cancel = action(&tokens, false, false, button::Status::Active);

        assert_eq!(apply.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(cancel.background, None);
    }

    #[test]
    fn the_primary_action_still_shows_the_focus() {
        let tokens = Tokens::builtin_dark();
        let focused = action(&tokens, true, true, button::Status::Active);
        let selected = action(&tokens, true, false, button::Status::Active);

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
}
