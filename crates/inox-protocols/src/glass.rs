//! Bindings for `inox-glass-v1`, Inox's private frosted-glass protocol.
//!
//! The protocol definition lives in `protocol/inox-glass-v1.xml`; the modules
//! below are generated from it at compile time with `wayland-scanner`, in the
//! same shape as [`crate::ipc`] so both private protocols are bound the same
//! way: a private `generated` module silences the lints generated code reliably
//! trips, and only the `client`/`server` halves are re-exported.
//!
//! The protocol itself is deliberately narrow. A client asks the compositor to
//! blur the already-composited pixels *behind* a region of its own surface and
//! to let that surface blend over the result, which is the only thing that makes
//! real frosted glass possible in a compositor: a client cannot sample the
//! framebuffer behind its own surface. Nothing is read back to the client, the
//! region is clipped to the surface it was requested for, and the radius is
//! clamped to the compositor's maximum, which the `capabilities` event reports.
//!
//! Version 2 adds the *refraction* half of a liquid-glass material: corner radii
//! (`set_corner_radii`), a refraction band along the inside of that shape
//! (`set_refraction`, with optional depth and dispersion bits) and a second
//! capability event (`liquid_capabilities`) that reports the maxima and which
//! optional components the compositor applies. A manager bound at version 2
//! creates version 2 objects; version 1 objects keep the blur-only behaviour and
//! never receive the new event. The version bump is the opt-in, and because the
//! protocol is unstable until Inox 1.0 the addition is how it evolves.

/// Generated bindings for the `inox-glass-v1` protocol.
pub mod inox_glass {
    mod generated {
        #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
        #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
        #![allow(missing_docs, clippy::all)]

        pub mod client {
            use wayland_client;
            use wayland_client::protocol::*;

            pub mod __interfaces {
                use wayland_client::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("protocol/inox-glass-v1.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_client_code!("protocol/inox-glass-v1.xml");
        }

        pub mod server {
            use wayland_server;
            use wayland_server::protocol::*;

            pub mod __interfaces {
                use wayland_server::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("protocol/inox-glass-v1.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_server_code!("protocol/inox-glass-v1.xml");
        }
    }

    /// Client-side API of the `inox-glass-v1` protocol.
    pub use self::generated::client;
    /// Server-side API of the `inox-glass-v1` protocol.
    pub use self::generated::server;
}

#[cfg(test)]
mod tests {
    use super::inox_glass::{client, server};

    /// Every request of an interface as `(name, since, is_destructor)`.
    fn requests(
        interface: &wayland_backend::protocol::Interface,
    ) -> Vec<(&'static str, u32, bool)> {
        interface
            .requests
            .iter()
            .map(|request| (request.name, request.since, request.is_destructor))
            .collect()
    }

    #[test]
    fn the_manager_is_advertised_as_inox_glass_manager_v1_version_2() {
        assert_eq!(
            client::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE.name,
            "inox_glass_manager_v1"
        );
        // Version 2 exists so a client can create version 2 glass objects: a
        // child object is created at its parent's version, and the new
        // refraction requests are only legal at version 2.
        assert_eq!(
            client::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE.version,
            2
        );
        assert_eq!(
            server::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE.name,
            "inox_glass_manager_v1"
        );
        assert_eq!(
            server::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE.version,
            2
        );
    }

    #[test]
    fn the_surface_object_is_advertised_as_inox_glass_v1_version_2() {
        assert_eq!(
            client::__interfaces::INOX_GLASS_V1_INTERFACE.name,
            "inox_glass_v1"
        );
        assert_eq!(client::__interfaces::INOX_GLASS_V1_INTERFACE.version, 2);
    }

    #[test]
    fn the_manager_request_set_is_stable() {
        assert_eq!(
            requests(&client::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE),
            [("destroy", 1, true), ("get_glass", 1, false)]
        );
    }

    #[test]
    fn the_glass_request_set_is_stable_and_the_lens_requests_are_version_2() {
        assert_eq!(
            requests(&client::__interfaces::INOX_GLASS_V1_INTERFACE),
            [
                ("set_radius", 1, false),
                ("set_region", 1, false),
                ("set_corner_radii", 2, false),
                ("set_refraction", 2, false),
                ("destroy", 1, true),
            ]
        );
    }

    /// The two version 2 requests carry integers only: four corner radii, and a
    /// band height plus a displacement plus a flags word. Nothing in the new
    /// surface area can carry an image or a file descriptor.
    #[test]
    fn the_refraction_requests_carry_four_three_and_no_pixels() {
        let request = |name: &str| {
            client::__interfaces::INOX_GLASS_V1_INTERFACE
                .requests
                .iter()
                .find(|request| request.name == name)
                .unwrap_or_else(|| panic!("{name} exists"))
        };

        let corners = format!("{:?}", request("set_corner_radii").signature);
        assert_eq!(
            corners.matches("Uint").count(),
            4,
            "four corner radii: {corners}"
        );

        let refraction = format!("{:?}", request("set_refraction").signature);
        assert_eq!(
            refraction.matches("Uint").count(),
            3,
            "a band height, a displacement and flags: {refraction}"
        );
        assert!(
            !corners.contains("Fd") && !refraction.contains("Fd"),
            "no file descriptor may be part of a lens request"
        );
    }

    /// The `get_glass` request is the only way to obtain the object, and it must
    /// take the surface it frosts: a glass object that is not bound to a surface
    /// would describe an effect on nothing, and the compositor's clipping promise
    /// ("a region outside the surface is clipped") would have no subject.
    #[test]
    fn a_glass_object_is_bound_to_exactly_one_surface() {
        let request = client::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE
            .requests
            .iter()
            .find(|request| request.name == "get_glass")
            .expect("the creation request exists");

        let signature = format!("{:?}", request.signature);
        assert!(
            signature.contains("NewId") && signature.contains("Object"),
            "expected a new id plus one object argument: {signature}"
        );
        assert_eq!(
            request.child_interface.map(|interface| interface.name),
            Some("inox_glass_v1"),
            "get_glass must create a glass object, not an unrelated interface"
        );
    }

    /// `capabilities` (v1) and `liquid_capabilities` (v2) are the only events.
    /// Both report maxima; neither carries pixels, colours or buffers.
    #[test]
    fn the_capabilities_events_carry_numbers_and_no_pixels() {
        assert_eq!(
            client::__interfaces::INOX_GLASS_V1_INTERFACE
                .events
                .iter()
                .map(|event| (event.name, event.since))
                .collect::<Vec<_>>(),
            [("capabilities", 1), ("liquid_capabilities", 2)]
        );

        for event in client::__interfaces::INOX_GLASS_V1_INTERFACE.events {
            let signature = format!("{:?}", event.signature);
            assert!(
                signature.contains("Uint"),
                "{} must report maxima: {signature}",
                event.name
            );
            assert!(
                !signature.contains("Fd") && !signature.contains("Array"),
                "{} must not let a buffer travel back to the client: {signature}",
                event.name
            );
        }

        let liquid = client::__interfaces::INOX_GLASS_V1_INTERFACE
            .events
            .iter()
            .find(|event| event.name == "liquid_capabilities")
            .expect("the v2 event exists");
        assert_eq!(
            format!("{:?}", liquid.signature).matches("Uint").count(),
            4,
            "a radius, a band height, a displacement and flags"
        );
    }

    #[test]
    fn the_error_enum_values_are_frozen() {
        use client::inox_glass_manager_v1::Error;

        assert_eq!(u32::from(Error::AlreadyCreated), 0);
    }

    /// The refraction flag bits are wire values: a client that sets the wrong
    /// bit asks for a different effect, so they are pinned here rather than
    /// only described in the XML.
    #[test]
    fn the_refraction_flags_are_frozen_bits() {
        use client::inox_glass_v1::RefractionFlag;

        assert_eq!(RefractionFlag::Depth.bits(), 1);
        assert_eq!(RefractionFlag::Dispersion.bits(), 2);
        assert_eq!(
            (RefractionFlag::Depth | RefractionFlag::Dispersion).bits(),
            3
        );
        assert_eq!(u32::from(RefractionFlag::Depth), 1);
        assert_eq!(u32::from(RefractionFlag::Dispersion), 2);
        assert!(
            RefractionFlag::from_bits(4).is_none(),
            "the flag set is closed: an unknown bit is not a refraction flag"
        );
    }
}
