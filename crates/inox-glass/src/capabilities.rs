//! What the compositor says it can do with one surface's backdrop.
//!
//! The protocol reports this twice: `capabilities` (v1) carries the largest blur
//! radius, and `liquid_capabilities` (v2) adds the refraction maxima and the
//! optional refraction components the compositor applies. Both are normalised
//! into one value here so the material decision never has to care which
//! version answered.
//!
//! A zero maximum is not an error: it is the compositor saying it will not draw
//! that effect for this surface — no renderer support, a locked session, or a
//! surface with no layer role. The client's job is to fall back to plain
//! translucency, not to ask again.

use crate::REFRACTION_FLAG_DISPERSION;

/// The effects the compositor will actually draw for one surface.
///
/// The refraction maximums are reported per dimension, and both must be non-zero
/// for refraction to have any visible effect: a band that is never displaced and
/// a displacement with no band are both "nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Largest accepted blur radius in logical pixels; 0 means "no blur".
    pub max_radius: u32,
    /// Largest accepted refraction band thickness in logical pixels.
    pub max_refraction_height: u32,
    /// Largest accepted refraction displacement in logical pixels.
    pub max_refraction_amount: u32,
    /// Whether the compositor applies the chromatic (`dispersion`) component.
    ///
    /// Depth is not tracked: the protocol lets a client set every bit it
    /// understands and the compositor ignores the ones it does not apply, so a
    /// request may always carry the depth bit. Dispersion is different — it is
    /// the one component whose absence a client can see (no coloured fringe), so
    /// the material asks for it only when the compositor advertises it.
    pub dispersion: bool,
}

impl Capabilities {
    /// Nothing at all: plain translucency, the same value a session with no
    /// `inox-glass-v1` global effectively has.
    pub const NONE: Self = Self {
        max_radius: 0,
        max_refraction_height: 0,
        max_refraction_amount: 0,
        dispersion: false,
    };

    /// The v1 `capabilities` event: a blur maximum and no refraction vocabulary.
    ///
    /// This is exactly what a version 1 compositor advertises, and it is also
    /// what a version 2 object knows between its `capabilities` and
    /// `liquid_capabilities` events — falling back to blur only is the honest
    /// answer in both cases.
    pub const fn from_legacy(max_radius: u32) -> Self {
        Self {
            max_radius,
            max_refraction_height: 0,
            max_refraction_amount: 0,
            dispersion: false,
        }
    }

    /// The v2 `liquid_capabilities` event, whose last argument is the raw
    /// `refraction_flag` bitfield.
    ///
    /// Unknown bits are ignored rather than rejected: the protocol says a
    /// compositor ignores flags it does not know, and the reverse holds too — a
    /// client keeps working when a newer compositor sets a bit it has never
    /// heard of.
    pub const fn from_liquid(
        max_radius: u32,
        max_refraction_height: u32,
        max_refraction_amount: u32,
        flags: u32,
    ) -> Self {
        Self {
            max_radius,
            max_refraction_height,
            max_refraction_amount,
            dispersion: flags & REFRACTION_FLAG_DISPERSION != 0,
        }
    }

    /// Whether the backdrop can be blurred at all.
    pub const fn can_blur(&self) -> bool {
        self.max_radius > 0
    }

    /// Whether the backdrop can be refracted at all.
    pub const fn can_refract(&self) -> bool {
        self.max_refraction_height > 0 && self.max_refraction_amount > 0
    }

    /// Whether the compositor will draw anything for this surface.
    pub const fn is_none(&self) -> bool {
        !self.can_blur() && !self.can_refract()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::REFRACTION_FLAG_DEPTH;

    #[test]
    fn the_default_is_nothing_at_all() {
        assert_eq!(Capabilities::default(), Capabilities::NONE);
        assert!(Capabilities::NONE.is_none());
        assert!(!Capabilities::NONE.can_blur());
        assert!(!Capabilities::NONE.can_refract());
    }

    #[test]
    fn a_legacy_capability_has_blur_and_no_refraction() {
        let caps = Capabilities::from_legacy(64);

        assert!(caps.can_blur());
        assert!(!caps.can_refract(), "v1 has no refraction vocabulary");
        assert!(!caps.dispersion);
        assert!(!caps.is_none());
    }

    #[test]
    fn a_liquid_capability_reads_the_dispersion_bit_and_ignores_unknown_ones() {
        let both = Capabilities::from_liquid(64, 128, 64, 0b11);
        assert!(both.can_blur());
        assert!(both.can_refract());
        assert!(both.dispersion);

        let depth_only = Capabilities::from_liquid(64, 128, 64, REFRACTION_FLAG_DEPTH);
        assert!(
            !depth_only.dispersion,
            "depth is not reported through Capabilities; only dispersion gates a request"
        );
        assert!(depth_only.can_refract());

        let newer = Capabilities::from_liquid(64, 128, 64, 0b1_0000);
        assert!(
            !newer.dispersion,
            "an unknown bit is not the dispersion bit"
        );
        assert!(newer.can_refract());
    }

    #[test]
    fn refraction_needs_both_maxima() {
        assert!(!Capabilities::from_liquid(64, 0, 64, 0).can_refract());
        assert!(!Capabilities::from_liquid(64, 128, 0, 0).can_refract());
        assert!(!Capabilities::from_liquid(0, 0, 0, 0b11).can_blur());
        assert!(Capabilities::from_liquid(0, 1, 1, 0b11).can_refract());
    }
}
