//! The material: what the design tokens say a pane should look like, as data.
//!
//! [`Material`] is the token projection of one pane. It says nothing about what
//! the compositor will agree to; that is [`crate::decide`]'s job. Keeping the
//! two apart is what makes the clamps testable: "the theme asks for a 24 px
//! band" and "the compositor gives 16" are different facts, and the second one
//! must never rewrite the first.

use inox_config::Glass;

use crate::MAX_CORNER_RADIUS;

/// The four corner radii of the rounded rectangle a pane draws and refracts.
///
/// Logical pixels, top-left first, clockwise — the order the protocol's
/// `set_corner_radii` uses. A radius larger than half the region's shorter side
/// is not an error: both the compositor and Qt clamp it when they draw, so a
/// caller may ask for "as round as the pane allows".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Corners {
    /// Top-left corner radius.
    pub top_left: u16,
    /// Top-right corner radius.
    pub top_right: u16,
    /// Bottom-right corner radius.
    pub bottom_right: u16,
    /// Bottom-left corner radius.
    pub bottom_left: u16,
}

impl Corners {
    /// Square corners.
    pub const ZERO: Self = Self {
        top_left: 0,
        top_right: 0,
        bottom_right: 0,
        bottom_left: 0,
    };

    /// The same radius in all four corners, as the radius token gives it.
    pub const fn uniform(radius: u16) -> Self {
        Self {
            top_left: radius,
            top_right: radius,
            bottom_right: radius,
            bottom_left: radius,
        }
    }

    /// Every corner clamped to `limit`.
    ///
    /// Clamping is idempotent and monotonic: clamping twice changes nothing, and
    /// a larger input can never clamp to a smaller corner.
    pub const fn clamped(self, limit: u16) -> Self {
        Self {
            top_left: if self.top_left > limit {
                limit
            } else {
                self.top_left
            },
            top_right: if self.top_right > limit {
                limit
            } else {
                self.top_right
            },
            bottom_right: if self.bottom_right > limit {
                limit
            } else {
                self.bottom_right
            },
            bottom_left: if self.bottom_left > limit {
                limit
            } else {
                self.bottom_left
            },
        }
    }

    /// The largest corner in the set.
    pub const fn max_radius(&self) -> u16 {
        let mut largest = self.top_left;
        if self.top_right > largest {
            largest = self.top_right;
        }
        if self.bottom_right > largest {
            largest = self.bottom_right;
        }
        if self.bottom_left > largest {
            largest = self.bottom_left;
        }
        largest
    }
}

/// The refraction band of a pane: the half of the material the compositor draws.
///
/// `height` is how far the displacement reaches inwards from the outline and
/// `amount` the peak displacement at the outline itself. Either being zero is
/// the explicit "no refraction" value — that is how a theme asks for the plain
/// v1 frosted pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Refraction {
    /// Band thickness in logical pixels; 0 disables refraction.
    pub height: u16,
    /// Peak displacement in logical pixels; 0 disables refraction.
    pub amount: u16,
    /// Add the radial "depth" (towards the centre) component.
    pub depth: bool,
    /// Split the refracted sample into a chromatic separation.
    pub dispersion: bool,
}

impl Refraction {
    /// No refraction: the v1 frosted pane.
    pub const NONE: Self = Self {
        height: 0,
        amount: 0,
        depth: false,
        dispersion: false,
    };

    /// A refraction band, as the `[glass]` tokens carry it.
    pub const fn new(height: u16, amount: u16, depth: bool, dispersion: bool) -> Self {
        Self {
            height,
            amount,
            depth,
            dispersion,
        }
    }

    /// Whether this asks the compositor for any refraction at all.
    pub const fn is_effective(&self) -> bool {
        self.height > 0 && self.amount > 0
    }
}

/// One pane's material, as the design tokens describe it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Material {
    /// Backdrop blur radius in logical pixels; 0 means "no blur".
    pub blur: u16,
    /// The corner radii of the shape the pane draws and refracts.
    pub corners: Corners,
    /// The refraction band the compositor should draw at that shape's edge.
    pub refraction: Refraction,
}

impl Material {
    /// A material from its parts.
    pub const fn new(blur: u16, corners: Corners, refraction: Refraction) -> Self {
        Self {
            blur,
            corners,
            refraction,
        }
    }

    /// A blur-only material with square corners: the v1 frosted pane.
    pub const fn plain(blur: u16) -> Self {
        Self::new(blur, Corners::ZERO, Refraction::NONE)
    }

    /// The same material with different corner radii.
    pub const fn with_corners(self, corners: Corners) -> Self {
        Self { corners, ..self }
    }

    /// The same material with a different refraction band.
    pub const fn with_refraction(self, refraction: Refraction) -> Self {
        Self { refraction, ..self }
    }

    /// Whether the tokens ask for any compositor-drawn effect at all.
    ///
    /// `false` is the "draw plain translucency and create no glass object"
    /// answer; it is a decision, not a failure.
    pub const fn has_effect(&self) -> bool {
        self.blur > 0 || self.refraction.is_effective()
    }

    /// The material of a pane from the theme's `[glass]` tokens and a corner
    /// radius.
    ///
    /// Every value comes from the token group (`docs/quality/ui-guidelines.md`
    /// §12), so a theme can make the glass warm, cool, dark, light, sharper or
    /// more liquid without any component changing. `radius` is the token the
    /// caller chose for the pane's shape (`radii.md`, `radii.lg`, …).
    pub fn from_tokens(tokens: &Glass, radius: u16) -> Self {
        Self::new(
            tokens.blur,
            Corners::uniform(radius),
            Refraction::new(
                tokens.refraction,
                tokens.refraction_amount,
                tokens.depth,
                tokens.dispersion,
            ),
        )
    }
}

/// The corner radius a pane can actually use: the protocol maximum, or half the
/// region's shorter side, whichever is smaller.
///
/// Half the shorter side is where two rounded corners meet; beyond it a corner
/// radius has no shape left to describe. Both the compositor and Qt clamp there
/// as well, so this is the value the request carries instead of asking for
/// something nobody will draw.
pub(crate) const fn usable_corner_radius(region: crate::Region) -> u16 {
    let shorter = if region.width < region.height {
        region.width
    } else {
        region.height
    };

    let half = if shorter > 0 {
        // The protocol extent is `i32`; half of it fits a `u16` only after the
        // clamp below, so compare before narrowing.
        (shorter / 2) as u32
    } else {
        0
    };

    if half < MAX_CORNER_RADIUS as u32 {
        half as u16
    } else {
        MAX_CORNER_RADIUS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::Theme;

    #[test]
    fn uniform_puts_the_same_radius_in_every_corner() {
        let corners = Corners::uniform(12);

        assert_eq!(corners.top_left, 12);
        assert_eq!(corners.top_right, 12);
        assert_eq!(corners.bottom_right, 12);
        assert_eq!(corners.bottom_left, 12);
        assert_eq!(corners.max_radius(), 12);
        assert_eq!(Corners::ZERO.max_radius(), 0);
    }

    #[test]
    fn clamping_is_idempotent_and_monotonic() {
        let lopsided = Corners {
            top_left: 4,
            top_right: 40_000,
            bottom_right: 16_384,
            bottom_left: 0,
        };

        let once = lopsided.clamped(16);
        assert_eq!(
            once,
            Corners {
                top_left: 4,
                top_right: 16,
                bottom_right: 16,
                bottom_left: 0,
            }
        );
        assert_eq!(once.clamped(16), once, "clamping twice changes nothing");
        assert_eq!(
            lopsided.clamped(8).max_radius(),
            8,
            "a smaller limit can only clamp further"
        );
        assert_eq!(
            Corners::uniform(3).clamped(u16::MAX),
            Corners::uniform(3),
            "a limit above every corner changes nothing"
        );
    }

    #[test]
    fn refraction_is_disabled_by_either_dimension_being_zero() {
        assert!(!Refraction::NONE.is_effective());
        assert!(!Refraction::new(24, 0, false, true).is_effective());
        assert!(!Refraction::new(0, 12, false, true).is_effective());
        assert!(Refraction::new(1, 1, false, false).is_effective());
        assert!(Refraction::new(24, 12, true, true).is_effective());
    }

    #[test]
    fn a_material_has_an_effect_if_either_half_is_asked_for() {
        assert!(!Material::plain(0).has_effect());
        assert!(Material::plain(1).has_effect());
        assert!(
            Material::new(0, Corners::ZERO, Refraction::new(24, 12, false, true)).has_effect(),
            "blur 0 with a refraction band is a liquid pane, not a plain one"
        );
        assert!(
            !Material::new(0, Corners::ZERO, Refraction::new(0, 12, false, false)).has_effect()
        );
    }

    #[test]
    fn from_tokens_takes_every_glass_value_from_the_theme() {
        let tokens = Theme::builtin_dark();
        let material = Material::from_tokens(&tokens.glass, tokens.radii.md);

        assert_eq!(material.blur, tokens.glass.blur);
        assert_eq!(material.corners, Corners::uniform(tokens.radii.md));
        assert_eq!(material.refraction.height, tokens.glass.refraction);
        assert_eq!(material.refraction.amount, tokens.glass.refraction_amount);
        assert_eq!(material.refraction.depth, tokens.glass.depth);
        assert_eq!(material.refraction.dispersion, tokens.glass.dispersion);
    }

    #[test]
    fn from_tokens_follows_a_theme_that_turns_refraction_off() {
        let mut tokens = Theme::builtin_dark();

        tokens.glass.refraction = 0;
        tokens.glass.dispersion = false;

        let material = Material::from_tokens(&tokens.glass, 8);
        assert!(
            !material.refraction.is_effective(),
            "refraction = 0 is the explicit `no refraction` value"
        );
        assert!(material.has_effect(), "the blur is still asked for");
    }

    #[test]
    fn a_corner_radius_never_exceeds_the_protocol_or_half_the_region() {
        use crate::Region;

        assert_eq!(usable_corner_radius(Region::pane(640, 68)), 34);
        assert_eq!(usable_corner_radius(Region::pane(68, 640)), 34);
        assert_eq!(usable_corner_radius(Region::pane(0, 68)), 0);
        assert_eq!(usable_corner_radius(Region::pane(4, 4)), 2);
        assert_eq!(
            usable_corner_radius(Region {
                x: 0,
                y: 0,
                width: i32::MAX,
                height: i32::MAX,
            }),
            MAX_CORNER_RADIUS,
            "a pane larger than the protocol maximum is clamped to it, not wrapped"
        );
    }
}
