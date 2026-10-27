//! The adapter between the material API and the `inox-glass-v1` client objects.
//!
//! This is the only module in the crate that names a wayland type: the
//! [`GlassTarget`] implementation below writes a [`crate::Request`] onto a real
//! `inox_glass_v1` proxy, and [`liquid_flags`] turns the `liquid_capabilities`
//! flags argument into the `u32` bitfield the material API speaks. Everything
//! else — clamps, fallbacks, the version gate — stays plain data and is tested
//! without a socket.
//!
//! A version 1 object silently skips the `since=2` requests here *and* in
//! [`crate::Request::apply`]: the adapter is usable on its own, and a protocol
//! error (sending a `since=2` request to a version 1 object) is not something to
//! leave to a caller's discipline.

use inox_protocols::glass::inox_glass::client::inox_glass_v1::{self, RefractionFlag};
use wayland_client::{Proxy, WEnum};

use crate::REFRACTION_SINCE_VERSION;
use crate::capabilities::Capabilities;
use crate::material::Corners;
use crate::region::Region;
use crate::request::GlassTarget;

/// The flags word of a `liquid_capabilities` event, as the `u32` bitfield
/// [`Capabilities::from_liquid`] takes.
///
/// A bitfield whose bits this client does not know arrives as `Unknown(bits)`;
/// passing those bits on unchanged is the protocol's rule (unknown bits are
/// ignored by whoever does not understand them), so nothing is dropped here.
pub fn liquid_flags(flags: WEnum<RefractionFlag>) -> u32 {
    match flags {
        WEnum::Value(flags) => flags.bits(),
        WEnum::Unknown(bits) => bits,
    }
}

/// The v1 `capabilities` event, normalised.
///
/// A thin alias, kept next to [`liquid_flags`] so both events are normalised in
/// one place instead of at each call site.
pub const fn capabilities(max_radius: u32) -> Capabilities {
    Capabilities::from_legacy(max_radius)
}

impl GlassTarget for inox_glass_v1::InoxGlassV1 {
    fn set_radius(&mut self, radius: u32) {
        inox_glass_v1::InoxGlassV1::set_radius(self, radius);
    }

    fn set_region(&mut self, region: Region) {
        inox_glass_v1::InoxGlassV1::set_region(
            self,
            region.x,
            region.y,
            region.width,
            region.height,
        );
    }

    fn set_corner_radii(&mut self, corners: Corners) {
        if !self.supports_refraction() {
            return;
        }

        inox_glass_v1::InoxGlassV1::set_corner_radii(
            self,
            u32::from(corners.top_left),
            u32::from(corners.top_right),
            u32::from(corners.bottom_right),
            u32::from(corners.bottom_left),
        );
    }

    fn set_refraction(&mut self, height: u32, amount: u32, flags: u32) {
        if !self.supports_refraction() {
            return;
        }

        inox_glass_v1::InoxGlassV1::set_refraction(
            self,
            height,
            amount,
            RefractionFlag::from_bits_truncate(flags),
        );
    }

    fn supports_refraction(&self) -> bool {
        self.version() >= REFRACTION_SINCE_VERSION
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{REFRACTION_FLAG_DEPTH, REFRACTION_FLAG_DISPERSION};

    #[test]
    fn the_wire_flags_convert_to_the_bitfield_word() {
        // The idiom a consumer's event arm uses: `flags` arrives as a
        // `WEnum<RefractionFlag>` and goes straight into the material API.
        let flags: u32 = WEnum::Value(RefractionFlag::Depth | RefractionFlag::Dispersion).into();
        assert_eq!(flags, 3);

        let unknown: u32 = WEnum::<RefractionFlag>::Unknown(0b1000).into();
        assert_eq!(unknown, 0b1000);
        assert_eq!(unknown, liquid_flags(WEnum::Unknown(0b1000)));
    }

    #[test]
    fn a_known_flag_set_keeps_its_bits() {
        assert_eq!(liquid_flags(WEnum::Value(RefractionFlag::Depth)), 1);
        assert_eq!(
            liquid_flags(WEnum::Value(
                RefractionFlag::Depth | RefractionFlag::Dispersion
            )),
            3
        );
        assert_eq!(liquid_flags(WEnum::Value(RefractionFlag::empty())), 0);
    }

    #[test]
    fn an_unknown_flag_set_is_passed_through_rather_than_dropped() {
        assert_eq!(
            liquid_flags(WEnum::Unknown(0b1010)),
            0b1010,
            "a newer compositor's bits are not this client's business"
        );
    }

    #[test]
    fn the_capabilities_normalisers_agree_with_the_material_api() {
        assert_eq!(capabilities(64), Capabilities::from_legacy(64));
        assert!(
            Capabilities::from_liquid(
                64,
                128,
                64,
                liquid_flags(WEnum::Value(
                    RefractionFlag::Depth | RefractionFlag::Dispersion
                ))
            )
            .dispersion,
            "the dispersion bit is what makes the fringe requestable"
        );
        assert_eq!(
            crate::REFRACTION_FLAG_DEPTH | crate::REFRACTION_FLAG_DISPERSION,
            (RefractionFlag::Depth | RefractionFlag::Dispersion).bits(),
            "the crate's flag constants are the protocol's bits"
        );
        assert_ne!(REFRACTION_FLAG_DEPTH, REFRACTION_FLAG_DISPERSION);
    }
}
