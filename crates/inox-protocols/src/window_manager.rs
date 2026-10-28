//! Bindings for `inox-window-manager-v1`, Inox's private window-management protocol.
//!
//! The protocol definition lives in `protocol/inox-window-manager-v1.xml`; the
//! modules below are generated from it at compile time with `wayland-scanner`,
//! in the same shape as [`crate::ipc`] and [`crate::glass`] so all three private
//! protocols are bound the same way: a private `generated` module silences the
//! lints generated code reliably trips, and only the `client`/`server` halves are
//! re-exported.
//!
//! The protocol exists because the compositor owns windows and the panel draws
//! them, while `AGENTS.md` §2 forbids a Cargo dependency between the two. A
//! client binds the manager once and creates one or more lists; each list is
//! served a snapshot (`session_generation`, `workspace_state`, one `window` per
//! managed window, `snapshot_done`) and then incremental events.
//!
//! Two properties are worth reading the XML for before changing anything here:
//!
//! - **A request is confirmed by a state event, not by an acknowledgement.**
//!   `window` / `window_removed` *is* the confirmation, and `request_failed` is
//!   the only negative outcome. A client therefore never has to guess whether a
//!   request it sent was carried out.
//! - **Handles are only meaningful within their `session_generation`.** The
//!   generation is sent before any handle, which is what keeps a client that
//!   reconnected from matching an old handle against a new window.

/// Generated bindings for the `inox-window-manager-v1` protocol.
pub mod inox_window_manager {
    mod generated {
        #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
        #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
        #![allow(missing_docs, clippy::all)]

        pub mod client {
            use wayland_client;
            use wayland_client::protocol::*;

            pub mod __interfaces {
                use wayland_client::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("protocol/inox-window-manager-v1.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_client_code!("protocol/inox-window-manager-v1.xml");
        }

        pub mod server {
            use wayland_server;
            use wayland_server::protocol::*;

            pub mod __interfaces {
                use wayland_server::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("protocol/inox-window-manager-v1.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_server_code!("protocol/inox-window-manager-v1.xml");
        }
    }

    /// Client-side API of the `inox-window-manager-v1` protocol.
    pub use self::generated::client;
    /// Server-side API of the `inox-window-manager-v1` protocol.
    pub use self::generated::server;
}

#[cfg(test)]
mod tests {
    use super::inox_window_manager::{client, server};

    #[test]
    fn the_manager_is_advertised_as_version_1() {
        assert_eq!(
            client::__interfaces::INOX_WINDOW_MANAGER_V1_INTERFACE.name,
            "inox_window_manager_v1"
        );
        assert_eq!(
            client::__interfaces::INOX_WINDOW_MANAGER_V1_INTERFACE.version,
            1
        );
        assert_eq!(
            server::__interfaces::INOX_WINDOW_MANAGER_V1_INTERFACE.name,
            "inox_window_manager_v1"
        );
        assert_eq!(
            server::__interfaces::INOX_WINDOW_MANAGER_V1_INTERFACE.version,
            1
        );
    }

    #[test]
    fn the_list_is_advertised_as_version_1() {
        assert_eq!(
            client::__interfaces::INOX_WINDOW_LIST_V1_INTERFACE.name,
            "inox_window_list_v1"
        );
        assert_eq!(
            client::__interfaces::INOX_WINDOW_LIST_V1_INTERFACE.version,
            1
        );
    }

    #[test]
    fn the_manager_request_set_is_stable() {
        let names: Vec<&str> = client::__interfaces::INOX_WINDOW_MANAGER_V1_INTERFACE
            .requests
            .iter()
            .map(|request| request.name)
            .collect();

        assert_eq!(names, ["destroy", "get_window_list"]);
    }

    /// The request set is the panel's whole vocabulary; adding one is a protocol
    /// revision that needs an ADR, and this assertion is what makes that visible.
    #[test]
    fn the_list_request_set_is_stable() {
        let names: Vec<&str> = client::__interfaces::INOX_WINDOW_LIST_V1_INTERFACE
            .requests
            .iter()
            .map(|request| request.name)
            .collect();

        assert_eq!(
            names,
            [
                "destroy",
                "activate",
                "minimize",
                "close",
                "set_mode",
                "move_to_workspace",
                "switch_workspace",
            ]
        );
    }

    #[test]
    fn destroy_is_the_only_destructor_request_on_both_interfaces() {
        for interface in [
            &client::__interfaces::INOX_WINDOW_MANAGER_V1_INTERFACE,
            &client::__interfaces::INOX_WINDOW_LIST_V1_INTERFACE,
        ] {
            let destructors: Vec<&str> = interface
                .requests
                .iter()
                .filter(|request| request.is_destructor)
                .map(|request| request.name)
                .collect();

            assert_eq!(destructors, ["destroy"], "in {}", interface.name);
        }
    }

    #[test]
    fn the_list_event_set_is_stable() {
        let names: Vec<&str> = client::__interfaces::INOX_WINDOW_LIST_V1_INTERFACE
            .events
            .iter()
            .map(|event| event.name)
            .collect();

        assert_eq!(
            names,
            [
                "session_generation",
                "workspace_state",
                "window",
                "window_removed",
                "snapshot_done",
                "request_failed",
            ]
        );
    }

    #[test]
    fn the_window_mode_enum_values_are_frozen() {
        use client::inox_window_list_v1::WindowMode;

        // The panel branches on these across a process boundary; a silent
        // renumbering would make it draw a fullscreen window as tiled.
        assert_eq!(u32::from(WindowMode::Tiled), 0);
        assert_eq!(u32::from(WindowMode::Floating), 1);
        assert_eq!(u32::from(WindowMode::Maximized), 2);
        assert_eq!(u32::from(WindowMode::Fullscreen), 3);
    }

    #[test]
    fn the_request_error_enum_values_are_frozen() {
        use client::inox_window_list_v1::RequestError;

        assert_eq!(u32::from(RequestError::UnknownWindow), 0);
        assert_eq!(u32::from(RequestError::NotPermitted), 1);
        assert_eq!(u32::from(RequestError::Unsupported), 2);
    }

    /// The `window` event is the snapshot record *and* the update, so its
    /// arguments are the panel's model. This pins the order the panel decodes
    /// them in: `u` handle, `s` app id, `s` title, then `u` workspace, `s` output,
    /// `u` mode, `u` minimized, `u` visible, `u` focused.
    #[test]
    fn the_window_event_signature_is_stable() {
        use wayland_backend::protocol::ArgumentType;

        let interface = &client::__interfaces::INOX_WINDOW_LIST_V1_INTERFACE;
        let event = interface
            .events
            .iter()
            .find(|event| event.name == "window")
            .expect("the window event exists");

        let signature: String = event
            .signature
            .iter()
            .map(|argument| match argument {
                ArgumentType::Int => 'i',
                ArgumentType::Uint => 'u',
                ArgumentType::Str(_) => 's',
                ArgumentType::Object(_) => 'o',
                ArgumentType::NewId => 'n',
                ArgumentType::Array => 'a',
                ArgumentType::Fd => 'h',
                ArgumentType::Fixed => 'f',
            })
            .collect();

        assert_eq!(signature, "ussusuuuu");
    }
}
