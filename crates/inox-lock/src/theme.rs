//! Design tokens, turned into the values QML binds to.
//!
//! Root AGENTS.md §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! QML, meanwhile, wants strings it can assign to `color` and numbers it can
//! assign to `font.pixelSize` / `width` / `radius`. This module is the join
//! between the two: it is the only place in the crate where a token becomes a
//! QML value, and the projection in [`crate::view`] is its only caller.
//! `docs/quality/ui-guidelines.md` is the contract this file implements: §1 token
//! discipline, §2 the four type roles, §3 the spacing rhythm, §4 the control and
//! card geometry, §5 the five states a control is drawn in.
//! the L3 component architecture explains why there is a copy of this file in every L3 component
//! A lock screen has one job for its colours: it must cover everything under it
//! and it must not be confusable with the desktop it is hiding. The card is drawn
//! on the `surface` token over the `background` token, so the whole surface is
//! opaque even where the card is not.
//!
//! # What this surface does *not* draw
//!
//! A lock surface reads no keyboard and no pointer,
//! so no control on it can be hovered, pressed or focused. The states are defined
//! here — [`ControlState`] has all five, and the tests below walk them — because a
//! theme that could not name them could not be checked, but the projection hands
//! QML only the resting and disabled ones. Drawing a hover or a focus ring onto a
//! surface that cannot receive either would be a picture of an input path that
//! does not exist, which is the same mistake as a password box on a screen with
//! nothing to check a password.
//!
//! Every function takes the tokens as an argument rather than reaching for a
//! global, so the projection can hand QML the same theme the rest of the screen
//! is drawn with — and so a test can render the same state under the light and
//! the dark tokens (acceptance Q02).
//!
//! the L3 component architecture explains why there is a copy of this file in every L3 component
//! instead of one shared crate: the duplication is the price of not adding a
//! same-layer Cargo dependency that would need its own ADR.

use inox_config::{Rgba, Theme as Tokens};

/// A token colour as QML spells a colour with an alpha channel: `#AARRGGBB`.
/// what is under it, so a scrim that let the desktop show through would
/// Qt's `QColor::name(QColor::HexArgb)` format, and the one Qt Quick parses
/// without a `Qt.rgba(...)` call. The alpha comes first, which is the opposite of
/// the `#RRGGBBAA` order the token files use — a swapped pair would draw the lock
/// screen with somebody else's colours and nothing would fail, so the conversion
/// is a test of its own.
pub fn qml_color(value: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        value.a, value.r, value.g, value.b
    )
}

/// A fully transparent colour, for a control that has no fill.
///
/// The structural sentinel the crate already accepted for a disabled control
/// (`Color::TRANSPARENT` before the migration): it is not a design colour and it
/// is not available from the tokens, because "no background" is the absence of a
/// token rather than one of them.
pub const TRANSPARENT: Rgba = Rgba {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

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
pub fn control_radius(tokens: &Tokens) -> f32 {
    f32::from(tokens.radii.sm)
}

/// The corner radius of a card or other boundary of a group.
pub fn card_radius(tokens: &Tokens) -> f32 {
    f32::from(tokens.radii.md)
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
pub fn control_padding(tokens: &Tokens) -> f32 {
    f32::from(tokens.spacing.md)
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
    pub fn colour(self, tokens: &Tokens) -> Rgba {
        match self {
            Self::None => text(tokens),
            Self::Muted => muted_text(tokens),
            Self::Danger => danger_text(tokens),
            Self::Warning => warning_text(tokens),
        }
    }

    /// The colour this emphasis is drawn in, as QML spells it.
    pub fn qml_colour(self, tokens: &Tokens) -> String {
        qml_color(self.colour(tokens))
    }
}

/// Default text colour.
pub fn text(tokens: &Tokens) -> Rgba {
    tokens.colors.text
}

/// Colour of secondary information.
pub fn muted_text(tokens: &Tokens) -> Rgba {
    tokens.colors.text_muted
}

/// Colour of a refusal.
pub fn danger_text(tokens: &Tokens) -> Rgba {
    tokens.colors.danger
}

/// Colour of something the user should notice but which is not their fault.
pub fn warning_text(tokens: &Tokens) -> Rgba {
    tokens.colors.warning
}

/// The focus ring, in the one token that is legible on `fill`.
///
/// The lock surface never draws one — it has no focus to show — but the ring is
/// part of the vocabulary a control theme owes (guidelines §10), and the fill it
/// would be drawn over decides whether the accent or the on-accent colour is
/// legible on it.
pub fn focus_ring(tokens: &Tokens, fill: Rgba) -> Rgba {
    if fill == tokens.colors.accent {
        tokens.colors.on_accent
    } else {
        tokens.colors.accent
    }
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

/// What a control is drawn with: the four values QML needs for one rectangle.
///
/// A plain value rather than a style closure: the projection reads it and writes
/// it into the frame's JSON, so the widget-free half of the crate can be tested
/// without a renderer.
/// `Eq` is deliberately absent: `border_width` is a token-derived `f32`, and a
/// hairline is compared approximately, never for bit equality.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlColours {
    /// The fill, or `None` for a control with no background.
    pub background: Option<Rgba>,
    /// The label colour.
    pub text: Rgba,
    /// The outline colour.
    pub border: Rgba,
    /// The outline width.
    pub border_width: f32,
}

impl ControlColours {
    /// The fill as QML spells it, `#00000000` where the control has none.
    pub fn background_qml(&self) -> String {
        qml_color(self.background.unwrap_or(TRANSPARENT))
    }

    /// The outline colour as QML spells it.
    pub fn border_qml(&self) -> String {
        qml_color(self.border)
    }

    /// The label colour as QML spells it.
    pub fn text_qml(&self) -> String {
        qml_color(self.text)
    }
}

/// A control with no outline: one token radius, no border.
fn unfilled(text: Rgba) -> ControlColours {
    ControlColours {
        background: None,
        text,
        border: TRANSPARENT,
        border_width: 0.0,
    }
}

/// A control at rest: text on a hairline outline.
fn outlined(tokens: &Tokens, text: Rgba) -> ControlColours {
    ControlColours {
        background: None,
        text,
        border: tokens.colors.border,
        border_width: hairline(tokens),
    }
}

/// The style of a control that is *not* the screen's one primary action.
///
/// Guidelines §5, in order: resting is text on a hairline outline, hovered takes
/// the alternate surface, pressed fills with the accent, the focus ring is the
/// accent at one `spacing.xs`, and disabled keeps only the muted text.
pub fn action_style(tokens: &Tokens, state: ControlState) -> ControlColours {
    match state {
        ControlState::Resting => outlined(tokens, text(tokens)),
        ControlState::Hovered => ControlColours {
            background: Some(tokens.colors.surface_variant),
            ..outlined(tokens, text(tokens))
        },
        ControlState::Pressed => ControlColours {
            background: Some(tokens.colors.accent),
            text: tokens.colors.on_accent,
            border: TRANSPARENT,
            border_width: 0.0,
        },
        ControlState::Focused => ControlColours {
            background: None,
            text: text(tokens),
            border: focus_ring(tokens, tokens.colors.surface),
            border_width: focus_ring_width(tokens),
        },
        ControlState::Disabled => unfilled(muted_text(tokens)),
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
pub fn submit_style(tokens: &Tokens, state: ControlState) -> ControlColours {
    match state {
        ControlState::Disabled => unfilled(muted_text(tokens)),
        ControlState::Pressed => ControlColours {
            background: Some(tokens.colors.on_accent),
            text: tokens.colors.accent,
            border: TRANSPARENT,
            border_width: 0.0,
        },
        ControlState::Focused => ControlColours {
            background: Some(tokens.colors.accent),
            text: tokens.colors.on_accent,
            border: focus_ring(tokens, tokens.colors.accent),
            border_width: focus_ring_width(tokens),
        },
        ControlState::Resting | ControlState::Hovered => ControlColours {
            background: Some(tokens.colors.accent),
            text: tokens.colors.on_accent,
            border: TRANSPARENT,
            border_width: 0.0,
        },
    }
}

// ---------------------------------------------------------------------------
// The surface, the card, and the field.
// ---------------------------------------------------------------------------

/// How much of the covered surface a scrim lets through.
///
/// None of it, and that is not a stylistic value: the session lock exists to hide
/// what is under it, so a scrim that let the desktop show through would
/// be showing exactly what it is there to cover. The ratio is named here because a
/// cover surface is the one place where "how opaque" is a decision rather than an
/// accident.
const SCRIM_ALPHA: u8 = u8::MAX;

/// The whole surface of the lock screen: the `background` token, made opaque.
///
/// The alpha is forced rather than taken from the token: a theme may author a
/// translucent `background` for windows, and a lock surface that carried it
/// through would show the session it is hiding.
pub fn scrim(tokens: &Tokens) -> Rgba {
    Rgba {
        a: SCRIM_ALPHA,
        ..tokens.colors.background
    }
}

/// The card the prompt sits on: `surface`, a hairline outline and `md` corners.
pub fn card(tokens: &Tokens) -> (Rgba, Rgba, f32) {
    (
        tokens.colors.surface,
        tokens.colors.border,
        hairline(tokens),
    )
}

/// The credential field: its background, outline and text colour.
///
/// Empty, it carries the placeholder in the muted colour; typed into, it carries
/// the mask in the text colour. The frame is the `background` token on the card's
/// `surface`, so the field reads as a place something goes rather than as a second
/// card; the typed border is the muted colour rather than the accent, so the
/// accent belongs to exactly one thing on the screen: the unlock button.
pub fn field(tokens: &Tokens, empty: bool) -> (Rgba, Rgba, Rgba) {
    (
        tokens.colors.background,
        if empty {
            tokens.colors.border
        } else {
            tokens.colors.text_muted
        },
        if empty {
            muted_text(tokens)
        } else {
            text(tokens)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::{BUILTIN_DARK, BUILTIN_LIGHT};

    fn tokens() -> Tokens {
        Tokens::builtin_dark()
    }

    #[test]
    fn a_token_colour_is_written_in_the_alpha_first_order_qml_parses() {
        // A swapped pair would draw the lock screen in somebody else's colours and
        // nothing else would notice, so the order is pinned by value.
        let rgba = Rgba {
            r: 0x23,
            g: 0x26,
            b: 0x2e,
            a: 0x80,
        };

        assert_eq!(qml_color(rgba), "#8023262E");
        assert_eq!(
            qml_color(Rgba {
                r: 0x23,
                g: 0x26,
                b: 0x2e,
                a: 0xff
            }),
            "#FF23262E",
            "an opaque colour still carries its alpha digit"
        );
    }

    #[test]
    fn the_scrim_is_fully_opaque() {
        let tokens = tokens();
        let covered = scrim(&tokens);

        assert_eq!(
            covered.a,
            u8::MAX,
            "a lock surface that is not opaque shows what it is hiding"
        );
        assert_eq!(SCRIM_ALPHA, u8::MAX, "the scrim lets nothing through");
        assert_eq!(
            qml_color(covered).len(),
            9,
            "a QML colour with an alpha channel is nine characters"
        );
    }

    #[test]
    fn the_scrim_is_the_background_token_with_the_alpha_forced() {
        let mut tokens = tokens();
        tokens.colors.background.a = 0x20;

        let covered = scrim(&tokens);

        assert_eq!(covered.r, tokens.colors.background.r);
        assert_eq!(covered.g, tokens.colors.background.g);
        assert_eq!(covered.b, tokens.colors.background.b);
        assert_eq!(covered.a, 0xff);
    }

    #[test]
    fn the_card_sits_on_the_surface_token() {
        let tokens = tokens();
        let (background, border, width) = card(&tokens);

        assert_eq!(background, tokens.colors.surface);
        assert_eq!(border, tokens.colors.border);
        assert_eq!(width, hairline(&tokens));
    }

    #[test]
    fn an_empty_field_and_a_typed_one_look_different() {
        let tokens = tokens();

        let (empty_background, empty_border, empty_text) = field(&tokens, true);
        let (typed_background, typed_border, typed_text) = field(&tokens, false);

        assert_eq!(empty_background, typed_background);
        assert_ne!(empty_border, typed_border);
        assert_ne!(empty_text, typed_text);
        assert_eq!(typed_text, text(&tokens));
        assert_ne!(
            typed_border, tokens.colors.accent,
            "the accent belongs to the unlock button alone"
        );
    }

    #[test]
    fn a_disabled_submit_is_muted_rather_than_emphasized() {
        let tokens = tokens();

        let enabled = submit_style(&tokens, ControlState::Resting);
        let disabled = submit_style(&tokens, ControlState::Disabled);

        assert_eq!(enabled.background, Some(tokens.colors.accent));
        assert_eq!(enabled.text, tokens.colors.on_accent);
        assert_eq!(disabled.background, None);
        assert_eq!(disabled.text, muted_text(&tokens));
        assert_ne!(enabled.text, disabled.text);
        assert_eq!(
            disabled.background_qml(),
            qml_color(TRANSPARENT),
            "a control with no fill has to say so in the colour QML is given"
        );
    }

    #[test]
    fn a_control_keeps_the_token_radius() {
        let tokens = tokens();

        assert!(
            hairline(&tokens) < focus_ring_width(&tokens),
            "a hairline and a ring are two widths, not one"
        );
        assert!(
            card_radius(&tokens) > control_radius(&tokens),
            "guidelines §4 gives a card `md` corners and its controls `sm`"
        );
        assert_eq!(control_radius(&tokens), f32::from(tokens.radii.sm));
        assert_eq!(card_radius(&tokens), f32::from(tokens.radii.md));
    }

    #[test]
    fn every_distance_is_derived_from_a_token() {
        let mut tokens = tokens();
        tokens.spacing.xs = 4;
        tokens.spacing.md = 10;
        tokens.radii.sm = 1;

        assert_eq!(hairline(&tokens), 2.0, "half of the smallest spacing step");
        assert_eq!(focus_ring_width(&tokens), 4.0);
        assert_eq!(control_padding(&tokens), 10.0);
        assert_eq!(control_radius(&tokens), 1.0);
    }

    #[test]
    fn the_control_target_clears_the_guidelines_floor() {
        // Guidelines §4 floors an interactive target at `spacing.xl`, and the
        // tokens carry no height, so the floor has to fall out of the padding and
        // the text together.
        for tokens in [Tokens::builtin_light(), Tokens::builtin_dark()] {
            let target = body_size(&tokens) + control_padding(&tokens) * 2.0;

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

            assert_eq!(ring, tokens.colors.accent);
            assert_ne!(ring, fill);
        }

        let on_accent = focus_ring(&tokens, tokens.colors.accent);

        assert_eq!(on_accent, tokens.colors.on_accent);
        assert_ne!(
            on_accent, tokens.colors.accent,
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
            Some(tokens.colors.surface_variant)
        );
        assert_eq!(
            action_style(&tokens, ControlState::Pressed).background,
            Some(tokens.colors.accent)
        );
        assert_eq!(
            action_style(&tokens, ControlState::Focused).border,
            tokens.colors.accent
        );
        assert_eq!(
            action_style(&tokens, ControlState::Disabled).text,
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
        assert_eq!(pressed.background, Some(tokens.colors.on_accent));
        assert_eq!(pressed.text, tokens.colors.accent);

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

        let idle = submit_style(&tokens, ControlState::Resting);
        let hovered = submit_style(&tokens, ControlState::Hovered);

        assert_eq!(idle.background, hovered.background);
        assert_eq!(idle.text, hovered.text);
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
    fn the_two_builtin_themes_produce_different_screens() {
        let light = Tokens::builtin_light();
        let dark = Tokens::builtin_dark();

        assert_ne!(scrim(&light), scrim(&dark));
        assert_ne!(text(&light), text(&dark));
        assert_eq!(Tokens::builtin_light().name, BUILTIN_LIGHT);
        assert_eq!(Tokens::builtin_dark().name, BUILTIN_DARK);
    }
}
