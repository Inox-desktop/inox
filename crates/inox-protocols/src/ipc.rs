//! Bindings for `inox-ipc`, Inox's private control protocol.
//!
//! The protocol definition lives in `protocol/inox-ipc.xml`; the modules below
//! are generated from it at compile time with `wayland-scanner`. The layout
//! mirrors the `wayland_protocol!` macro used by `wayland-protocols`: a private
//! `generated` module silences the lints that generated code reliably trips,
//! and only the `client`/`server` halves are re-exported.

/// Generated bindings for the `inox-ipc` protocol.
pub mod inox_ipc {
    mod generated {
        #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
        #![allow(non_upper_case_globals, non_snake_case, unused_imports)]
        #![allow(missing_docs, clippy::all)]

        pub mod client {
            use wayland_client;
            use wayland_client::protocol::*;

            pub mod __interfaces {
                use wayland_client::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("protocol/inox-ipc.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_client_code!("protocol/inox-ipc.xml");
        }

        pub mod server {
            use wayland_server;
            use wayland_server::protocol::*;

            pub mod __interfaces {
                use wayland_server::protocol::__interfaces::*;
                wayland_scanner::generate_interfaces!("protocol/inox-ipc.xml");
            }
            use self::__interfaces::*;

            wayland_scanner::generate_server_code!("protocol/inox-ipc.xml");
        }
    }

    /// Client-side API of the `inox-ipc` protocol.
    pub use self::generated::client;
    /// Server-side API of the `inox-ipc` protocol.
    pub use self::generated::server;
}

#[cfg(test)]
mod tests {
    use super::inox_ipc::{client, server};

    #[test]
    fn the_interface_is_advertised_as_inox_ipc_v1() {
        assert_eq!(client::__interfaces::INOX_IPC_INTERFACE.name, "inox_ipc");
        assert_eq!(client::__interfaces::INOX_IPC_INTERFACE.version, 1);
        assert_eq!(server::__interfaces::INOX_IPC_INTERFACE.name, "inox_ipc");
        assert_eq!(server::__interfaces::INOX_IPC_INTERFACE.version, 1);
    }

    #[test]
    fn the_request_set_is_stable() {
        let names: Vec<&str> = client::__interfaces::INOX_IPC_INTERFACE
            .requests
            .iter()
            .map(|request| request.name)
            .collect();

        assert_eq!(names, ["destroy", "get_version", "reload_config"]);
    }

    #[test]
    fn destroy_is_the_only_destructor_request() {
        let destructors: Vec<&str> = client::__interfaces::INOX_IPC_INTERFACE
            .requests
            .iter()
            .filter(|request| request.is_destructor)
            .map(|request| request.name)
            .collect();

        assert_eq!(destructors, ["destroy"]);
    }

    #[test]
    fn the_event_set_is_stable() {
        let names: Vec<&str> = client::__interfaces::INOX_IPC_INTERFACE
            .events
            .iter()
            .map(|event| event.name)
            .collect();

        assert_eq!(names, ["version", "config_reloaded"]);
    }

    #[test]
    fn the_reload_result_enum_values_are_frozen() {
        use client::inox_ipc::ReloadResult;

        assert_eq!(u32::from(ReloadResult::Success), 0);
        assert_eq!(u32::from(ReloadResult::Rejected), 1);
        assert_eq!(u32::from(ReloadResult::IoError), 2);
        assert_eq!(u32::from(ReloadResult::Unsupported), 3);
    }
}
