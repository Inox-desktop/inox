//! Design tokens, turned into iced's theme and style closures.
//!
//! Root AGENTS.md §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! iced, meanwhile, expects a [`iced::Theme`] and a handful of `Style` structs
//! built from it. This module is the join between the two, and the only place in
//! the crate where a colour becomes a widget's. `docs/quality/ui-guidelines.md`
//! is the contract this file implements: §1 token discipline, §2 the four type
//! roles, §3 the spacing rhythm, §4 the control and card geometry, §5 the five
//! states a control is drawn in.
//!
//! A lock screen has one job for its colours: it must cover everything under it
//! and it must not be confusable with the desktop it is hiding. The card is drawn
//! on the `surface` token over the `background` token, so the whole surface is
//! opaque even where the card is not.
//!
//! # What this surface does *not* draw
//!
//! A lock surface reads no keyboard and no pointer (crate `AGENTS.md`, ADR 0025),
//! so no control on it can be hovered, pressed or focused. The states are defined
//! here — [`ControlState`] has all five, and the tests below walk them — because a
//! theme that could not name them could not be checked, but the view draws only
//! the resting and disabled ones. Drawing a hover or a focus ring onto a surface
//! that cannot receive either would be a picture of an input path that does not
//! exist, which is the same mistake as a password box on a screen with nothing to
//! check a password.
//!
//! Every function takes the tokens as an argument rather than reaching for a
//! global, so the view can hand the styles the same theme the rest of the screen
//! is drawn with — and so a test can render the same widget tree under the light
//! and the dark tokens (acceptance Q02).
//!
//! ADR 0018 explains why there is a copy of this file in every L3 component
//! instead of one shared crate: the duplication is the price of not adding a
//! same-layer Cargo dependency that would need its own ADR.

use iced::widget::{button, container};
use iced::{Border, Color, Padding, Theme};
use inox_config::{Rgba, Theme as Tokens};

/// A token colour, as iced wants it.
pub fn color(value: Rgba) -> Color {
    Color::from_rgba8(value.r, value.g, value.b, f32::from(value.a) / 255.0)
}

/// The iced theme the lock screen runs under.
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

// ---------------------------------------------------------------------------
// The type scale (guidelines §2).
//
// Four roles and their multiples are fixed: a lock screen does not get a fifth
// size, and it does not get to carry a section heading at the size of a page
// title. What they multiply is the base, which is `spacing.lg × typography.scale`
// — a token times the user's scale, so 200 % text is 200 % of the theme's own
// body size and this file names no pixel size of its own (acceptance Q02).
// ---------------------------------------------------------------------------

/// Ratio of the page or window title.
const TITLE_RATIO: f32 = 1.5;

/// Ratio of a card or section heading.
const HEADING_RATIO: f32 = 1.25;

/// Ratio of body text.
const BODY_RATIO: f32 = 1.0;

/// Ratio of captions and secondary lines.
const CAPTION_RATIO: f32 = 0.875;

/// The size every role is a multiple of.
pub fn base_size(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.lg) * tokens.typography.scale
}

/// The size of the surface's main title.
pub fn title_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * TITLE_RATIO
}

/// The size of a card or section heading, which is what the lock screen's state
/// sentence is drawn at.
pub fn heading_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * HEADING_RATIO
}

/// The size of body text: the state lines under the heading, the credential mask
/// and every control label.
pub fn body_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * BODY_RATIO
}

/// The size of a caption: what a line *explains*, never what it asks.
pub fn caption_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * CAPTION_RATIO
}

// ---------------------------------------------------------------------------
// Geometry (guidelines §1, §4).
// ---------------------------------------------------------------------------

/// The width of an ordinary outline: half the smallest spacing step.
pub fn hairline(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xs) / 2.0
}

/// The width of a focus ring, which is one `spacing.xs`.
pub fn focus_ring_width(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.xs)
}

/// The corner radius of a control: a field or a button.
pub fn control_radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.sm).into()
}

/// The corner radius of a card or other boundary of a group.
pub fn card_radius(tokens: &Tokens) -> iced::border::Radius {
    f32::from(tokens.radii.md).into()
}

/// The padding inside a control.
///
/// Guidelines §3 names `spacing.sm` above and below a control and `md` beside it,
/// and §4 floors an interactive target at `spacing.xl`. With the default 16 px
/// body text a `sm` above and below yields a 27 px target, under the floor, and
/// the tokens carry no second spacing step between `sm` and `md`. The floor is the
/// accessibility rule and the rhythm row is a nominal value, so the vertical
/// padding is `md` here: the target is 32 px at the default scale and grows with
/// `typography.scale` and with the body size, because nothing pins a height.
pub fn control_padding(tokens: &Tokens) -> Padding {
    let step = f32::from(tokens.spacing.md);

    Padding {
        top: step,
        bottom: step,
        left: step,
        right: step,
    }
}

/// Every text colour the lock screen uses.
///
/// There is no "plain" variant: text that carries no emphasis is drawn in
/// [`text`], and a variant nothing constructs would be a colour nobody can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    /// Body text and control labels.
    None,
    /// Secondary information: what a line explains.
    Muted,
    /// A refusal, a failure, or a state that cannot run.
    Danger,
    /// Something to notice, such as a spent allowance.
    Warning,
}

impl Emphasis {
    /// The colour this emphasis is drawn in.
    pub fn colour(self, tokens: &Tokens) -> Color {
        match self {
            Self::None => text(tokens),
            Self::Muted => muted_text(tokens),
            Self::Danger => danger_text(tokens),
            Self::Warning => warning_text(tokens),
        }
    }
}

/// Default text colour.
pub fn text(tokens: &Tokens) -> Color {
    color(tokens.colors.text)
}

/// Colour of secondary information.
pub fn muted_text(tokens: &Tokens) -> Color {
    color(tokens.colors.text_muted)
}

/// Colour of a refusal.
pub fn danger_text(tokens: &Tokens) -> Color {
    color(tokens.colors.danger)
}

/// Colour of something the user should notice but which is not their fault.
pub fn warning_text(tokens: &Tokens) -> Color {
    color(tokens.colors.warning)
}

/// A border of `colour` at `width`, rounded like a control.
fn control_border(tokens: &Tokens, colour: Rgba, width: f32) -> Border {
    Border {
        color: color(colour),
        width,
        radius: control_radius(tokens),
    }
}

/// The focus ring, in the one token that is legible on `fill`.
///
/// The lock surface never draws one — it has no focus to show — but the ring is
/// part of the vocabulary a control theme owes (guidelines §10), and the fill it
/// would be drawn over decides whether the accent or the on-accent colour is
/// legible on it.
pub fn focus_ring(tokens: &Tokens, fill: Rgba) -> Border {
    let ring = if fill == tokens.colors.accent {
        tokens.colors.on_accent
    } else {
        tokens.colors.accent
    };

    control_border(tokens, ring, focus_ring_width(tokens))
}

// ---------------------------------------------------------------------------
// The five states (guidelines §5).
// ---------------------------------------------------------------------------

/// How a control is being used, in the one vocabulary the guidelines define.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    /// Nothing is happening to the control.
    Resting,
    /// The pointer is over it.
    Hovered,
    /// It is being pressed.
    Pressed,
    /// The keyboard is on it.
    Focused,
    /// It cannot be used, and the screen says why beside it.
    Disabled,
}

impl ControlState {
    /// The state iced's own interaction status describes.
    pub fn from_status(status: button::Status) -> Self {
        match status {
            button::Status::Active => Self::Resting,
            button::Status::Hovered => Self::Hovered,
            button::Status::Pressed => Self::Pressed,
            button::Status::Disabled => Self::Disabled,
        }
    }
}

/// The style of a control that is *not* the screen's one primary action.
///
/// Guidelines §5, in order: resting is text on a hairline outline, hovered takes
/// the alternate surface, pressed fills with the accent, the focus ring is the
/// accent at one `spacing.xs`, and disabled keeps only the muted text.
pub fn action_style(tokens: &Tokens, state: ControlState) -> button::Style {
    match state {
        ControlState::Resting => button::Style {
            background: None,
            text_color: text(tokens),
            border: control_border(tokens, tokens.colors.border, hairline(tokens)),
            ..button::Style::default()
        },
        ControlState::Hovered => button::Style {
            background: Some(color(tokens.colors.surface_variant).into()),
            text_color: text(tokens),
            border: control_border(tokens, tokens.colors.border, hairline(tokens)),
            ..button::Style::default()
        },
        ControlState::Pressed => button::Style {
            background: Some(color(tokens.colors.accent).into()),
            text_color: color(tokens.colors.on_accent),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: control_radius(tokens),
            },
            ..button::Style::default()
        },
        ControlState::Focused => button::Style {
            background: None,
            text_color: text(tokens),
            border: focus_ring(tokens, tokens.colors.surface),
            ..button::Style::default()
        },
        ControlState::Disabled => button::Style {
            background: None,
            text_color: muted_text(tokens),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: control_radius(tokens),
            },
            ..button::Style::default()
        },
    }
}

/// The style of the screen's one primary action.
///
/// Filled with the accent while it can be used, and reduced to muted text with no
/// background when it cannot (guidelines §5; acceptance D05 draws the reason
/// beside it). The fill does not follow the pointer: the tokens carry no hover
/// variant of the accent, and the one token that means "hover" would make the
/// primary action look disabled. A press is answered, though, and by an exchange
/// of the two colours that are defined to contrast with each other — the press is
/// visible the instant it happens, without the theme inventing a shade.
pub fn submit_style(tokens: &Tokens, state: ControlState) -> button::Style {
    if state == ControlState::Disabled {
        return button::Style {
            background: None,
            text_color: muted_text(tokens),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: control_radius(tokens),
            },
            ..button::Style::default()
        };
    }

    if state == ControlState::Pressed {
        return button::Style {
            background: Some(color(tokens.colors.on_accent).into()),
            text_color: color(tokens.colors.accent),
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: control_radius(tokens),
            },
            ..button::Style::default()
        };
    }

    button::Style {
        background: Some(color(tokens.colors.accent).into()),
        text_color: color(tokens.colors.on_accent),
        border: if state == ControlState::Focused {
            focus_ring(tokens, tokens.colors.accent)
        } else {
            Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: control_radius(tokens),
            }
        },
        ..button::Style::default()
    }
}

// ---------------------------------------------------------------------------
// The surface, the card, and the field.
// ---------------------------------------------------------------------------

/// How much of the covered surface a scrim lets through.
///
/// None of it, and that is not a stylistic value: the session lock exists to hide
/// what is under it (ADR 0015), so a scrim that let the desktop show through would
/// be showing exactly what it is there to cover. The ratio is named here because a
/// cover surface is the one place where "how opaque" is a decision rather than an
/// accident.
const SCRIM_ALPHA: u8 = u8::MAX;

/// The whole surface of the lock screen.
///
/// The `background` token at `SCRIM_ALPHA`, full-bleed.
pub fn scrim(tokens: &Tokens) -> impl Fn(&Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(
            color(Rgba {
                a: SCRIM_ALPHA,
                ..tokens.colors.background
            })
            .into(),
        ),
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The card the prompt sits on: `surface`, a hairline outline and `md` corners.
pub fn card(tokens: &Tokens) -> impl Fn(&Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.surface).into()),
        border: Border {
            color: color(tokens.colors.border),
            width: hairline(tokens),
            radius: card_radius(tokens),
        },
        text_color: Some(text(tokens)),
        ..container::Style::default()
    }
}

/// The credential field.
///
/// Empty, it carries the placeholder in the muted colour; typed into, it carries
/// the mask in the text colour. The frame is the `background` token on the card's
/// `surface`, so the field reads as a place something goes rather than as a second
/// card; the typed border is the muted colour rather than the accent, so the
/// accent belongs to exactly one thing on the screen: the unlock button.
pub fn field(tokens: &Tokens, empty: bool) -> impl Fn(&Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(color(tokens.colors.background).into()),
        border: control_border(
            tokens,
            if empty {
                tokens.colors.border
            } else {
                tokens.colors.text_muted
            },
            hairline(tokens),
        ),
        text_color: Some(if empty {
            muted_text(tokens)
        } else {
            text(tokens)
        }),
        ..container::Style::default()
    }
}

/// The unlock button, in iced's own style shape.
///
/// A control that cannot be pressed is a button with no `on_press`, which is how
/// iced reports it as disabled — so "the control is disabled" and "the control
/// sends nothing" are the same fact rather than two that could disagree.
pub fn submit(
    tokens: &Tokens,
    enabled: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        if enabled {
            submit_style(tokens, ControlState::from_status(status))
        } else {
            submit_style(tokens, ControlState::Disabled)
        }
    }
}

/// A secondary action, such as the retry in the unavailable state.
pub fn action(tokens: &Tokens) -> impl Fn(&Theme, button::Status) -> button::Style + '_ {
    move |_, status| action_style(tokens, ControlState::from_status(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::theme::Base;
    use inox_config::{BUILTIN_DARK, BUILTIN_LIGHT};

    fn tokens() -> Tokens {
        Tokens::builtin_dark()
    }

    #[test]
    fn a_token_colour_keeps_its_channels_and_its_alpha() {
        let rgba = Rgba {
            r: 0x23,
            g: 0x26,
            b: 0x2e,
            a: 0x80,
        };
        let colour = color(rgba);

        assert!((colour.r - 0x23 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.g - 0x26 as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.b - 0x2e as f32 / 255.0).abs() < f32::EPSILON);
        assert!((colour.a - 128.0 / 255.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_iced_theme_is_named_after_the_tokens_it_came_from() {
        assert_eq!(from_tokens(&Tokens::builtin_light()).name(), BUILTIN_LIGHT);
        assert_eq!(from_tokens(&Tokens::builtin_dark()).name(), BUILTIN_DARK);
    }

    #[test]
    fn the_scrim_is_fully_opaque() {
        let tokens = tokens();
        let covered = Rgba {
            a: SCRIM_ALPHA,
            ..tokens.colors.background
        };
        let style = scrim(&tokens)(&from_tokens(&tokens));

        assert_eq!(style.background, Some(color(covered).into()));
        assert_eq!(
            color(covered).a,
            1.0,
            "a lock surface that is not opaque shows what it is hiding"
        );
        assert_eq!(SCRIM_ALPHA, u8::MAX, "the scrim lets nothing through");
    }

    #[test]
    fn the_card_sits_on_the_surface_token() {
        let tokens = tokens();
        let style = card(&tokens)(&from_tokens(&tokens));

        assert_eq!(style.background, Some(color(tokens.colors.surface).into()));
        assert_eq!(style.text_color, Some(text(&tokens)));
        assert_eq!(style.border.radius, card_radius(&tokens));
        assert_eq!(style.border.width, hairline(&tokens));
    }

    #[test]
    fn an_empty_field_and_a_typed_one_look_different() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let empty = field(&tokens, true)(&theme);
        let typed = field(&tokens, false)(&theme);

        assert_ne!(empty.text_color, typed.text_color);
        assert_ne!(empty.border.color, typed.border.color);
        assert_eq!(typed.text_color, Some(color(tokens.colors.text)));
        assert_ne!(
            typed.border.color,
            color(tokens.colors.accent),
            "the accent belongs to the unlock button alone"
        );
    }

    #[test]
    fn a_disabled_submit_is_muted_rather_than_emphasized() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let enabled = submit(&tokens, true)(&theme, button::Status::Active);
        let disabled = submit(&tokens, false)(&theme, button::Status::Active);

        assert_eq!(enabled.background, Some(color(tokens.colors.accent).into()));
        assert_eq!(enabled.text_color, color(tokens.colors.on_accent));
        assert_eq!(disabled.background, None);
        assert_eq!(disabled.text_color, muted_text(&tokens));
        assert_ne!(enabled.text_color, disabled.text_color);
    }

    #[test]
    fn a_button_taken_away_by_iced_is_drawn_disabled_too() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let style = submit(&tokens, true)(&theme, button::Status::Disabled);

        assert_eq!(style.background, None);
        assert_eq!(style.text_color, muted_text(&tokens));
    }

    #[test]
    fn a_control_keeps_the_token_radius() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        assert_eq!(
            submit(&tokens, true)(&theme, button::Status::Active)
                .border
                .radius,
            control_radius(&tokens)
        );
        assert_eq!(
            action(&tokens)(&theme, button::Status::Active)
                .border
                .radius,
            control_radius(&tokens)
        );
        assert!(
            hairline(&tokens) < focus_ring_width(&tokens),
            "a hairline and a ring are two widths, not one"
        );
    }

    #[test]
    fn a_card_is_rounder_than_the_controls_on_it() {
        let tokens = tokens();

        assert_ne!(
            card_radius(&tokens),
            control_radius(&tokens),
            "guidelines §4 gives a card `md` corners and its controls `sm`, so the \
             two cannot be the same radius"
        );
        assert_eq!(
            control_radius(&tokens),
            iced::border::Radius::from(f32::from(tokens.radii.sm))
        );
        assert_eq!(
            card_radius(&tokens),
            iced::border::Radius::from(f32::from(tokens.radii.md))
        );
    }

    #[test]
    fn every_distance_is_derived_from_a_token() {
        let mut tokens = tokens();
        tokens.spacing.xs = 4;
        tokens.spacing.md = 10;
        tokens.radii.sm = 1;

        assert_eq!(hairline(&tokens), 2.0, "half of the smallest spacing step");
        assert_eq!(focus_ring_width(&tokens), 4.0);

        let padding = control_padding(&tokens);

        assert_eq!(padding.top, 10.0);
        assert_eq!(padding.bottom, 10.0);
        assert_eq!(padding.left, 10.0);
        assert_eq!(padding.right, 10.0);
        assert_eq!(control_radius(&tokens).top_left, 1.0);
    }

    #[test]
    fn the_control_target_clears_the_guidelines_floor() {
        // Guidelines §4 floors an interactive target at `spacing.xl`, and the
        // tokens carry no height, so the floor has to fall out of the padding and
        // the text together.
        for tokens in [Tokens::builtin_light(), Tokens::builtin_dark()] {
            let target = body_size(&tokens) + control_padding(&tokens).top * 2.0;

            assert!(
                target >= f32::from(tokens.spacing.xl),
                "a control is {target} px tall, under the {} px floor",
                tokens.spacing.xl
            );
        }
    }

    #[test]
    fn the_focus_ring_is_legible_on_the_fills_it_is_drawn_over() {
        let tokens = tokens();

        for fill in [tokens.colors.background, tokens.colors.surface] {
            let ring = focus_ring(&tokens, fill);

            assert_eq!(ring.width, focus_ring_width(&tokens));
            assert_eq!(ring.color, color(tokens.colors.accent));
            assert_ne!(ring.color, color(fill));
        }

        let on_accent = focus_ring(&tokens, tokens.colors.accent);

        assert_eq!(on_accent.color, color(tokens.colors.on_accent));
        assert_ne!(
            on_accent.color,
            color(tokens.colors.accent),
            "a ring in the fill's own colour is an invisible focus"
        );
    }

    #[test]
    fn the_five_control_states_are_pairwise_distinguishable() {
        let tokens = tokens();
        let states = [
            ControlState::Resting,
            ControlState::Hovered,
            ControlState::Pressed,
            ControlState::Focused,
            ControlState::Disabled,
        ];

        let shapes: Vec<_> = states
            .iter()
            .map(|state| (*state, action_style(&tokens, *state)))
            .collect();

        for (index, (one, first)) in shapes.iter().enumerate() {
            for (other, second) in &shapes[index + 1..] {
                assert_ne!(
                    first, second,
                    "{one:?} and {other:?} are drawn as the same control"
                );
            }
        }

        assert_eq!(
            action_style(&tokens, ControlState::Resting).background,
            None,
            "guidelines §5: a control at rest carries no background"
        );
        assert_eq!(
            action_style(&tokens, ControlState::Hovered).background,
            Some(color(tokens.colors.surface_variant).into())
        );
        assert_eq!(
            action_style(&tokens, ControlState::Pressed).background,
            Some(color(tokens.colors.accent).into())
        );
        assert_eq!(
            action_style(&tokens, ControlState::Focused).border.color,
            color(tokens.colors.accent)
        );
        assert_eq!(
            action_style(&tokens, ControlState::Disabled).text_color,
            muted_text(&tokens)
        );
    }

    #[test]
    fn a_press_is_answered_before_the_pointer_leaves_the_control() {
        let tokens = tokens();

        let resting = submit_style(&tokens, ControlState::Resting);
        let pressed = submit_style(&tokens, ControlState::Pressed);

        assert_ne!(
            resting, pressed,
            "guidelines §5: the feedback happens on the press, not on the release"
        );
        assert_eq!(
            pressed.background,
            Some(color(tokens.colors.on_accent).into())
        );
        assert_eq!(pressed.text_color, color(tokens.colors.accent));

        assert_ne!(
            submit_style(&tokens, ControlState::Disabled),
            resting,
            "a control that cannot be used is not drawn as one that can"
        );
        assert_ne!(
            submit_style(&tokens, ControlState::Focused),
            resting,
            "the focus ring has to be visible on the filled control too"
        );
    }

    #[test]
    fn the_submit_button_does_not_react_to_the_pointer() {
        let tokens = tokens();
        let theme = from_tokens(&tokens);

        let idle = submit(&tokens, true)(&theme, button::Status::Active);
        let hovered = submit(&tokens, true)(&theme, button::Status::Hovered);

        assert_eq!(idle.background, hovered.background);
        assert_eq!(idle.text_color, hovered.text_color);
    }

    #[test]
    fn text_sizes_follow_the_token_scale() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert!(body_size(&tokens) > 0.0, "a zero text size draws nothing");
        assert_eq!(body_size(&tokens), f32::from(tokens.spacing.lg));
        assert_eq!(body_size(&doubled), body_size(&tokens) * 2.0);
        assert_eq!(title_size(&doubled), title_size(&tokens) * 2.0);
        assert_eq!(heading_size(&doubled), heading_size(&tokens) * 2.0);
        assert_eq!(caption_size(&doubled), caption_size(&tokens) * 2.0);
    }

    #[test]
    fn the_four_roles_are_ordered_and_none_of_them_is_too_small() {
        let tokens = tokens();

        assert!(title_size(&tokens) > heading_size(&tokens));
        assert!(heading_size(&tokens) > body_size(&tokens));
        assert!(body_size(&tokens) > caption_size(&tokens));
        assert!(
            caption_size(&tokens) >= body_size(&tokens) * 0.75,
            "guidelines §2: a caption is never smaller than 0.75 of the body"
        );
        assert_eq!(title_size(&tokens), body_size(&tokens) * 1.5);
        assert_eq!(heading_size(&tokens), body_size(&tokens) * 1.25);
        assert_eq!(caption_size(&tokens), body_size(&tokens) * 0.875);
    }

    #[test]
    fn every_emphasis_is_a_different_colour() {
        let tokens = tokens();
        let colours = [
            Emphasis::None.colour(&tokens),
            Emphasis::Muted.colour(&tokens),
            Emphasis::Danger.colour(&tokens),
            Emphasis::Warning.colour(&tokens),
        ];

        for (index, one) in colours.iter().enumerate() {
            for other in &colours[index + 1..] {
                assert_ne!(one, other, "two emphases share a colour");
            }
        }
    }

    #[test]
    fn the_two_builtin_themes_produce_different_palettes() {
        let light = from_tokens(&Tokens::builtin_light());
        let dark = from_tokens(&Tokens::builtin_dark());

        assert_ne!(light.palette().background, dark.palette().background);
        assert_ne!(light.palette().text, dark.palette().text);
    }
}
