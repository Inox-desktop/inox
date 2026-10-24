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

    #[test]
    fn the_manager_is_advertised_as_inox_glass_manager_v1() {
        assert_eq!(
            client::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE.name,
            "inox_glass_manager_v1"
        );
        assert_eq!(
            client::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE.version,
            1
        );
        assert_eq!(
            server::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE.name,
            "inox_glass_manager_v1"
        );
    }

    #[test]
    fn the_surface_object_is_advertised_as_inox_glass_v1() {
        assert_eq!(
            client::__interfaces::INOX_GLASS_V1_INTERFACE.name,
            "inox_glass_v1"
        );
        assert_eq!(client::__interfaces::INOX_GLASS_V1_INTERFACE.version, 1);
    }

    #[test]
    fn the_manager_request_set_is_stable() {
        let names: Vec<&str> = client::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE
            .requests
            .iter()
            .map(|request| request.name)
            .collect();

        assert_eq!(names, ["destroy", "get_glass"]);
        assert_eq!(
            client::__interfaces::INOX_GLASS_MANAGER_V1_INTERFACE
                .requests
                .iter()
                .filter(|request| request.is_destructor)
                .map(|request| request.name)
                .collect::<Vec<_>>(),
            ["destroy"]
        );
    }

    #[test]
    fn the_glass_request_set_is_stable() {
        let names: Vec<&str> = client::__interfaces::INOX_GLASS_V1_INTERFACE
            .requests
            .iter()
            .map(|request| request.name)
            .collect();

        assert_eq!(names, ["set_radius", "set_region", "destroy"]);
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
    }

    /// `capabilities` is the only event: the compositor tells the client what it
    /// will do, and never sends pixels or colours back.
    #[test]
    fn capabilities_is_the_only_event_and_carries_no_pixels() {
        let events: Vec<&str> = client::__interfaces::INOX_GLASS_V1_INTERFACE
            .events
            .iter()
            .map(|event| event.name)
            .collect();

        assert_eq!(events, ["capabilities"]);

        let event = client::__interfaces::INOX_GLASS_V1_INTERFACE
            .events
            .iter()
            .find(|event| event.name == "capabilities")
            .expect("the capabilities event exists");
        let signature = format!("{:?}", event.signature);
        assert!(
            signature.contains("Uint"),
            "the event must report a radius, not an image: {signature}"
        );
        assert!(
            !signature.contains("Fd") && !signature.contains("Array"),
            "no buffer may travel back to the client: {signature}"
        );
    }

    #[test]
    fn the_error_enum_values_are_frozen() {
        use client::inox_glass_manager_v1::Error;

        assert_eq!(u32::from(Error::AlreadyCreated), 0);
    }
}
