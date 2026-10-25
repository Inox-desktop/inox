//! Design tokens, turned into the values QML renders.
//!
//! Root AGENTS.md §4 forbids a component from hard-coding a colour, a size, a
//! radius or a duration: every one of them comes from [`inox_config::Theme`].
//! Since the interface is QML, the join between the two is a set
//! of pure functions from tokens to the values a QML property can bind to: a
//! colour becomes the `#AARRGGBB` string Qt spells colours with, and every
//! distance becomes a number derived from the spacing, radii and typography
//! tokens. Nothing here knows what QML is, so the whole mapping is unit-testable
//! without a display.
//!
//! `docs/quality/ui-guidelines.md` is the contract this file implements: §1 token
//! discipline, §2 the four type roles, §3 the spacing rhythm, §4 the control and
//! card geometry, §5 the five states a control is drawn in.
//!
//! The login screen has one job for its colours on top of that: a person must be
//! able to see which control the keyboard is on (acceptance Q01). Every control
//! therefore draws its focus ring in the one token that is legible against its own
//! fill — the accent on a plain control, and the on-accent colour on the filled
//! log-in button — and nothing else on the screen uses the accent.
//!
//! ADR 0018 explains why there is a copy of this file in every component instead
//! of one shared crate: the duplication is the price of not adding a same-layer
//! Cargo dependency that would need its own ADR.

use inox_config::{Rgba, Theme as Tokens};

/// A token colour, as QML spells it: `#AARRGGBB`.
///
/// Qt reads a `#` colour string as `#AARRGGBB` when it carries eight digits (the
/// same ordering `QColor::name(QColor::HexArgb)` writes), so the alpha channel
/// comes first. A component that passed the bytes in the other order would draw
/// the login screen's background as a red tint of its alpha, which is why this
/// conversion has a test of its own.
pub fn qml_color(value: Rgba) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        value.a, value.r, value.g, value.b
    )
}

/// The focus ring, in the one token that is legible on `fill`.
///
/// A ring drawn in the accent on an accent-filled control is a ring nobody can
/// see (guidelines §5), so the filled control gets the on-accent colour instead.
pub fn focus_ring(tokens: &Tokens, fill: Rgba) -> String {
    if fill == tokens.colors.accent {
        qml_color(tokens.colors.on_accent)
    } else {
        qml_color(tokens.colors.accent)
    }
}

// ---------------------------------------------------------------------------
// The type scale (guidelines §2).
//
// Four roles and their multiples are fixed: a window does not get a fifth size,
// and a subsection does not get to be drawn at the size of the page title. What
// they multiply is the base, which is `spacing.lg × typography.scale` — a token
// times the user's scale, so 200 % text is 200 % of the theme's own body size and
// this file names no pixel size of its own (acceptance Q02).
// ---------------------------------------------------------------------------

/// Ratio of the page or window title.
pub const TITLE_RATIO: f32 = 1.5;

/// Ratio of a card or section heading.
pub const HEADING_RATIO: f32 = 1.25;

/// Ratio of body text.
pub const BODY_RATIO: f32 = 1.0;

/// Ratio of captions and secondary lines.
pub const CAPTION_RATIO: f32 = 0.875;

/// Ratio of the head medallion's diameter to the base the type scale is built on.
///
/// A medallion is a shape rather than a type role, so it is not one of the four
/// multiples in guidelines §2; deriving it from the same base is what makes it
/// grow with the person's font scale like everything else (acceptance Q02).
pub const MEDALLION_RATIO: f32 = 3.0;

/// Ratio of the medallion on an account row: smaller, because a column of avatars
/// as tall as the head one is a column of avatars with no room for names.
pub const ROW_MEDALLION_RATIO: f32 = 2.0;

/// The scale used when the token set carries one that cannot be drawn at.
///
/// The user's scale is a ratio, so its neutral value is 1.0; naming it keeps the
/// fallback below from looking like a font size.
pub const DEFAULT_SCALE: f32 = 1.0;

/// The size every role is a multiple of.
///
/// A scale the session cannot use — NaN, infinite, zero or negative — falls back
/// to [`DEFAULT_SCALE`] instead of being propagated. A login screen whose type
/// size is zero or negative draws no text at all, and a typo in the configuration
/// must not be able to hide the field somebody types their password into.
pub fn base_size(tokens: &Tokens) -> f32 {
    let scale = tokens.typography.scale;
    let usable = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        DEFAULT_SCALE
    };

    f32::from(tokens.spacing.lg) * usable
}

/// The size of the page title: the one main heading a login surface has.
pub fn title_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * TITLE_RATIO
}

/// The size of a card or section heading.
pub fn heading_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * HEADING_RATIO
}

/// The size of body text: field values, state lines and control labels.
pub fn body_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * BODY_RATIO
}

/// The size of a caption: field labels, the banner, and every explanation.
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
pub fn control_radius(tokens: &Tokens) -> u16 {
    tokens.radii.sm
}

/// The corner radius of a card or other boundary of a group.
pub fn card_radius(tokens: &Tokens) -> u16 {
    tokens.radii.md
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
pub fn control_padding(tokens: &Tokens) -> u16 {
    tokens.spacing.md
}

/// The diameter of the account medallion at the head of the login screen.
pub fn medallion_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * MEDALLION_RATIO
}

/// The diameter of the medallion on one row of the account list.
pub fn row_medallion_size(tokens: &Tokens) -> f32 {
    base_size(tokens) * ROW_MEDALLION_RATIO
}

/// Every text colour the login screen uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    /// Body text and control labels.
    None,
    /// Secondary information: what a line explains.
    Muted,
    /// A refusal or a failure.
    Danger,
    /// Something to notice, such as a spent allowance.
    Warning,
    /// Something that went the right way.
    Success,
}

impl Emphasis {
    /// The name this emphasis has in the projection's `colors` object.
    ///
    /// QML resolves a line's colour by looking this key up, so a line that names
    /// a colour the projection does not carry would be a binding error rather
    /// than a silently defaulted colour — `view::tests` asserts every key used is
    /// a key defined.
    pub const fn key(self) -> &'static str {
        match self {
            Self::None => "text",
            Self::Muted => "textMuted",
            Self::Danger => "danger",
            Self::Warning => "warning",
            Self::Success => "success",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::Appearance;

    fn tokens() -> Tokens {
        Tokens::builtin_dark()
    }

    #[test]
    fn a_token_colour_becomes_the_argb_string_qt_parses() {
        let colour = qml_color(Rgba {
            r: 0x23,
            g: 0x26,
            b: 0x2e,
            a: 0x80,
        });

        assert_eq!(colour, "#8023262E", "alpha comes first, as Qt reads it");
        assert_eq!(colour.len(), 9);
        assert!(colour.starts_with('#'));
    }

    #[test]
    fn an_opaque_background_is_written_as_opaque() {
        let tokens = tokens();
        let background = qml_color(tokens.colors.background);

        assert_eq!(&background[1..3], "FF", "the login screen is opaque");
    }

    #[test]
    fn the_focus_ring_is_visible_on_every_control_it_is_drawn_on() {
        let tokens = tokens();

        for (control, fill) in [
            ("field", tokens.colors.background),
            ("clear", tokens.colors.surface),
            ("log in", tokens.colors.accent),
        ] {
            let ring = focus_ring(&tokens, fill);

            assert_ne!(ring, qml_color(fill), "{control}'s ring is its own fill");
            assert!(ring.starts_with('#'));
            assert_ne!(ring, "#00000000", "{control}'s focus ring is invisible");
        }

        // The two fills really do resolve to two different rings, so "the ring is
        // legible against the fill" is a rule and not a coincidence.
        assert_ne!(
            focus_ring(&tokens, tokens.colors.background),
            focus_ring(&tokens, tokens.colors.accent)
        );
        assert_eq!(
            focus_ring(&tokens, tokens.colors.accent),
            qml_color(tokens.colors.on_accent)
        );
    }

    #[test]
    fn every_distance_is_derived_from_a_token() {
        let mut tokens = tokens();
        tokens.spacing.xs = 4;
        tokens.spacing.sm = 6;
        tokens.spacing.md = 10;
        tokens.spacing.lg = 20;
        tokens.spacing.xl = 30;
        tokens.radii.sm = 1;
        tokens.radii.md = 5;

        assert_eq!(hairline(&tokens), 2.0, "half of the smallest spacing step");
        assert_eq!(focus_ring_width(&tokens), 4.0);
        assert_eq!(control_padding(&tokens), 10);
        assert_eq!(control_radius(&tokens), 1);
        assert_eq!(card_radius(&tokens), 5);
    }

    #[test]
    fn a_card_is_rounder_than_the_controls_on_it() {
        let tokens = tokens();

        assert_ne!(card_radius(&tokens), control_radius(&tokens));
        assert_eq!(control_radius(&tokens), tokens.radii.sm);
        assert_eq!(card_radius(&tokens), tokens.radii.md);
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
    fn a_nonsensical_font_scale_still_yields_a_usable_size() {
        let mut tokens = tokens();
        for scale in [f32::NAN, f32::INFINITY, -4.0, 0.0] {
            tokens.typography.scale = scale;
            // The projection saturates rather than panicking; what it must never
            // do is produce a size that hides every string on the screen.
            let size = body_size(&tokens);
            assert!(
                !size.is_sign_negative(),
                "scale {scale} produced a negative size"
            );
        }
    }

    #[test]
    fn the_control_target_clears_the_guidelines_floor() {
        // Guidelines §4 floors an interactive target at `spacing.xl`, and the tokens
        // carry no height, so the floor has to fall out of the padding and the text
        // together.
        for tokens in [Tokens::builtin_light(), Tokens::builtin_dark()] {
            let target = body_size(&tokens) + f32::from(control_padding(&tokens)) * 2.0;

            assert!(
                target >= f32::from(tokens.spacing.xl),
                "a control is {target} px tall, under the {} px floor",
                tokens.spacing.xl
            );
        }
    }

    #[test]
    fn a_medallion_is_a_circle_that_grows_with_the_text() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert_eq!(
            medallion_size(&tokens),
            body_size(&tokens) * MEDALLION_RATIO
        );
        assert_eq!(
            row_medallion_size(&tokens),
            body_size(&tokens) * ROW_MEDALLION_RATIO
        );
        assert!(
            row_medallion_size(&tokens) < medallion_size(&tokens),
            "a row leaves room for the name beside it"
        );

        for (one, other) in [
            (medallion_size(&tokens), medallion_size(&doubled)),
            (row_medallion_size(&tokens), row_medallion_size(&doubled)),
        ] {
            assert_eq!(
                other,
                one * 2.0,
                "a medallion that did not grow with the text would be a picture \
                 of a size nobody asked for"
            );
        }
    }

    #[test]
    fn every_emphasis_has_its_own_key_and_a_colour_behind_it() {
        let tokens = tokens();
        let emphases = [
            Emphasis::None,
            Emphasis::Muted,
            Emphasis::Danger,
            Emphasis::Warning,
            Emphasis::Success,
        ];

        for (index, one) in emphases.iter().enumerate() {
            for other in &emphases[index + 1..] {
                assert_ne!(one.key(), other.key(), "two emphases share a key");
            }
        }

        // The colours the keys resolve to, through the same conversion the
        // projection uses: two emphases on the same colour would be two names for
        // one thing.
        let colours: Vec<String> = {
            let mut colours = vec![
                qml_color(tokens.colors.text),
                qml_color(tokens.colors.text_muted),
                qml_color(tokens.colors.danger),
                qml_color(tokens.colors.warning),
                qml_color(tokens.colors.success),
            ];
            colours.sort();
            colours.dedup();
            colours
        };

        assert_eq!(colours.len(), emphases.len(), "two emphases share a colour");
    }

    #[test]
    fn the_two_builtin_themes_produce_different_colours() {
        let light = Tokens::builtin_light();
        let dark = Tokens::builtin_dark();

        assert_eq!(light.appearance, Appearance::Light);
        assert_ne!(qml_color(light.colors.text), qml_color(dark.colors.text));
        assert_ne!(
            qml_color(light.colors.background),
            qml_color(dark.colors.background)
        );
    }

    #[test]
    fn the_font_families_come_from_the_tokens() {
        let tokens = tokens();

        assert_eq!(tokens.typography.font_family, "Noto Sans");
        assert!(!tokens.typography.font_mono_family.is_empty());
    }
}
