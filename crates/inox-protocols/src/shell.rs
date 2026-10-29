//! Bindings for `inox-shell-v1`, Inox's private shell-surface protocol.
//!
//! The protocol definition lives in `protocol/inox-shell-v1.xml`; the modules
//! below are generated from it at compile time with `wayland-scanner`, in the
//! same shape as [`crate::ipc`], [`crate::glass`] and [`crate::window_manager`]
//! so every private protocol is bound the same way: a private `generated`
//! module silences the lints generated code reliably trips, and only the
//! `client`/`server` halves are re-exported.
//!
//! Why a shell protocol rather than `zwlr-layer-shell` plus `xdg_popup` is
//! answered in `docs/adr/0046-shell-surfaces-and-native-qt.md` and, at the
//! level a client author reads, in the XML's own descriptions. The short
//! version: a panel and the menu it opens are one visual object, and the
//! frames between two of its states can only be drawn by the compositor. No
//! standard protocol can ask for that, and none of them tells a shell what the
//! work area it has to lay out against actually is.
//!
//! Three properties of this protocol are load-bearing and must survive any
//! change to it:
//!
//! - **Placement is one atomic request, and state is committed.** A shell
//!   surface is placed as a unit and takes effect on `wl_surface.commit`, so a
//!   compositor never reserves a strip for half a placement.
//! - **The compositor, not the client, owns dismissal.** A popup declares
//!   whether it is dismissable; the pointer grab, the Escape key and the
//!   parent-unmapping rule are the compositor's, and the client learns what
//!   happened through `dismissed`. There is no serial to hand over, because a
//!   shell menu is routinely opened from the keyboard or from a protocol event.
//! - **Motion is a hint with no completion event.** The compositor may drop a
//!   transition (reduced motion, no renderer, shutdown) and the shell must
//!   still look right. A client that waits for an animation to finish before
//!   committing its next state is a client that hangs on a compositor which
//!   never animated anything.
//!
//! Session locking is deliberately *not* part of this protocol:
//! `ext-session-lock-v1` carries the security rule that no unlocked content may
//! be visible before a locked frame has been presented on every output, and
//! that rule must not be re-implemented by an unstable private protocol. The
//! lock client's migration to a native Qt surface ([`crate`] users: see the ADR)
//! adds a renderer, not a role.

/// Generated bindings for the `inox-shell-v1` protocol.
pub mod inox_shell {
    mod generated {
        #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
        #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
        #![allow(missing_docs, clippy::all)]

        pub mod client {
            use wayland_client;
            use wayland_client::protocol::*;

            pub mod __interfaces {
                use wayland_client::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("protocol/inox-shell-v1.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_client_code!("protocol/inox-shell-v1.xml");
        }

        pub mod server {
            use wayland_server;
            use wayland_server::protocol::*;

            pub mod __interfaces {
                use wayland_server::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("protocol/inox-shell-v1.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_server_code!("protocol/inox-shell-v1.xml");
        }
    }

    /// Client-side API of the `inox-shell-v1` protocol.
    pub use self::generated::client;
    /// Server-side API of the `inox-shell-v1` protocol.
    pub use self::generated::server;
}

#[cfg(test)]
mod tests {
    use super::inox_shell::{client, server};
    use wayland_backend::protocol::ArgumentType;

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

    /// Every event of an interface as `(name, since)`.
    fn events(interface: &wayland_backend::protocol::Interface) -> Vec<(&'static str, u32)> {
        interface
            .events
            .iter()
            .map(|event| (event.name, event.since))
            .collect()
    }

    #[test]
    fn the_factory_is_advertised_as_inox_shell_v1_version_1() {
        assert_eq!(
            client::__interfaces::INOX_SHELL_V1_INTERFACE.name,
            "inox_shell_v1"
        );
        assert_eq!(client::__interfaces::INOX_SHELL_V1_INTERFACE.version, 1);
        assert_eq!(
            server::__interfaces::INOX_SHELL_V1_INTERFACE.name,
            "inox_shell_v1"
        );
        assert_eq!(server::__interfaces::INOX_SHELL_V1_INTERFACE.version, 1);
    }

    #[test]
    fn the_three_interfaces_are_what_a_shell_client_binds() {
        assert_eq!(
            client::__interfaces::INOX_SHELL_SURFACE_V1_INTERFACE.name,
            "inox_shell_surface_v1"
        );
        assert_eq!(
            client::__interfaces::INOX_SHELL_POPUP_V1_INTERFACE.name,
            "inox_shell_popup_v1"
        );
    }

    /// The request sets are the contract a compositor implements. A change here
    /// is a protocol change: it must come with a version bump and an ADR update,
    /// never with a "small" edit to the generated code.
    #[test]
    fn the_factory_request_set_is_stable() {
        assert_eq!(
            requests(&client::__interfaces::INOX_SHELL_V1_INTERFACE),
            [("destroy", 1, true), ("get_surface", 1, false)]
        );
    }

    #[test]
    fn the_surface_request_set_is_stable() {
        assert_eq!(
            requests(&client::__interfaces::INOX_SHELL_SURFACE_V1_INTERFACE),
            [
                ("set_placement", 1, false),
                ("set_keyboard", 1, false),
                ("set_transition", 1, false),
                ("get_popup", 1, false),
                ("destroy", 1, true),
            ]
        );
    }

    #[test]
    fn the_popup_request_set_is_stable() {
        assert_eq!(
            requests(&client::__interfaces::INOX_SHELL_POPUP_V1_INTERFACE),
            [
                ("set_position", 1, false),
                ("set_size", 1, false),
                ("set_keyboard", 1, false),
                ("set_dismissable", 1, false),
                ("dismiss", 1, false),
                ("set_transition", 1, false),
                ("get_popup", 1, false),
                ("destroy", 1, true),
            ]
        );
    }

    /// A popup carries no `ack_configure`: the ack is a `wl_surface` commit with
    /// the serial, and a protocol that grew a second ack would let a client
    /// acknowledge a configure it never applied.
    #[test]
    fn acking_a_configure_is_a_surface_commit_and_never_a_request() {
        for interface in [
            &client::__interfaces::INOX_SHELL_SURFACE_V1_INTERFACE,
            &client::__interfaces::INOX_SHELL_POPUP_V1_INTERFACE,
        ] {
            assert!(
                !interface
                    .requests
                    .iter()
                    .any(|request| request.name.contains("ack")),
                "{} must not have an ack request",
                interface.name
            );
            assert!(
                interface
                    .events
                    .iter()
                    .any(|event| event.name == "configure"),
                "{} reports its size through configure",
                interface.name
            );
        }
    }

    /// `closed` is how the compositor refuses a role. If it were missing, a shell
    /// on a compositor that will not show it would keep rendering into a surface
    /// nobody displays.
    #[test]
    fn the_surface_reports_its_work_area_keyboard_state_and_closure() {
        assert_eq!(
            events(&client::__interfaces::INOX_SHELL_SURFACE_V1_INTERFACE),
            [
                ("configure", 1),
                ("workarea", 1),
                ("keyboard_state", 1),
                ("closed", 1),
            ]
        );
    }

    #[test]
    fn a_popup_reports_configure_dismissal_and_keyboard_state() {
        assert_eq!(
            events(&client::__interfaces::INOX_SHELL_POPUP_V1_INTERFACE),
            [("configure", 1), ("dismissed", 1), ("keyboard_state", 1),]
        );
    }

    /// Placement is one request with nine arguments and no setters beside it.
    /// Splitting it is what lets a compositor observe half a placement — see the
    /// XML and the ADR; this test is the tripwire for a well-meaning refactor.
    #[test]
    fn placement_is_one_atomic_request_and_has_no_setter_siblings() {
        let interface = &client::__interfaces::INOX_SHELL_SURFACE_V1_INTERFACE;

        let placement = interface
            .requests
            .iter()
            .find(|request| request.name == "set_placement")
            .expect("set_placement exists");

        assert_eq!(
            placement.signature.len(),
            9,
            "layer, anchor, exclusive zone, four margins, width, height"
        );

        for setter in [
            "set_anchor",
            "set_margin",
            "set_exclusive_zone",
            "set_size",
            "set_layer",
        ] {
            assert!(
                !interface
                    .requests
                    .iter()
                    .any(|request| request.name == setter),
                "{setter} must not exist: placement is set as a unit"
            );
        }
    }

    /// The only interface that grants a role takes the surface it applies to, so
    /// a shell surface object can never describe a surface the client does not
    /// own.
    #[test]
    fn a_shell_surface_is_bound_to_exactly_one_surface() {
        let request = client::__interfaces::INOX_SHELL_V1_INTERFACE
            .requests
            .iter()
            .find(|request| request.name == "get_surface")
            .expect("the creation request exists");

        assert_eq!(
            request.child_interface.map(|interface| interface.name),
            Some("inox_shell_surface_v1")
        );
        assert_eq!(
            request
                .signature
                .iter()
                .filter(|argument| matches!(argument, ArgumentType::Object(_)))
                .count(),
            1
        );
        assert_eq!(request.arg_interfaces.len(), 1);
        assert_eq!(request.arg_interfaces[0].name, "wl_surface");
    }

    /// Nested popups are what submenus are made of, so the popup interface must
    /// be able to create popups of its own type. Without this the deepest menu a
    /// shell can draw is one level.
    #[test]
    fn a_popup_can_be_the_parent_of_another_popup() {
        let request = client::__interfaces::INOX_SHELL_POPUP_V1_INTERFACE
            .requests
            .iter()
            .find(|request| request.name == "get_popup")
            .expect("a popup can nest");

        assert_eq!(
            request.child_interface.map(|interface| interface.name),
            Some("inox_shell_popup_v1")
        );
    }

    /// Nothing in this protocol carries a file descriptor or a readback: a shell
    /// surface is a role, not a capability. The compositor never hands pixels
    /// back, which is what keeps this out of S04's privileged interface list.
    /// The only object arguments are the `wl_surface` a role is assigned to and
    /// the `wl_output` nothing here takes — never a compositor-owned object.
    #[test]
    fn no_message_carries_a_file_descriptor_and_objects_are_only_inputs() {
        for interface in [
            &client::__interfaces::INOX_SHELL_V1_INTERFACE,
            &client::__interfaces::INOX_SHELL_SURFACE_V1_INTERFACE,
            &client::__interfaces::INOX_SHELL_POPUP_V1_INTERFACE,
        ] {
            for message in interface.requests.iter().chain(interface.events.iter()) {
                assert!(
                    !message
                        .signature
                        .iter()
                        .any(|argument| matches!(argument, ArgumentType::Fd)),
                    "{}::{} must not carry a file descriptor",
                    interface.name,
                    message.name
                );
                // Events run the other way: an event that handed out an object
                // would let the compositor give a client a resource the client
                // did not ask for, and a shell needs none.
                if interface
                    .events
                    .iter()
                    .any(|event| event.name == message.name)
                {
                    assert!(
                        !message
                            .signature
                            .iter()
                            .any(|argument| matches!(argument, ArgumentType::Object(_))),
                        "{}::{} must not hand out objects",
                        interface.name,
                        message.name
                    );
                }
            }
        }
    }

    /// The capability bits are a bitfield: a compositor sets the ones it has and
    /// a client checks the ones it needs. Encoding them as separate events would
    /// make every future addition a new event, and a client that knows only the
    /// old ones would have to answer a question it was never asked.
    #[test]
    fn capabilities_are_a_bitfield_plus_a_duration() {
        let event = client::__interfaces::INOX_SHELL_V1_INTERFACE
            .events
            .iter()
            .find(|event| event.name == "capabilities")
            .expect("the capability event exists");

        assert_eq!(
            event.signature,
            &[ArgumentType::Uint, ArgumentType::Uint],
            "a capability bitfield and the longest transition it will animate"
        );
    }
}
