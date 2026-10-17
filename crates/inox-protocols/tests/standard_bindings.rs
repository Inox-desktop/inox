//! Asserts that the curated standard Wayland bindings in
//! [`inox_protocols::standard`] are reachable and correctly named.
//!
//! The socket round-trip lives in `tests/ipc_roundtrip.rs`; this file only
//! checks the compile-time surface of the curation layer, so it needs no
//! compositor or connection.

use inox_protocols::standard::{core, ext, wlr, wp, xdg};

/// The core `wl_*` interfaces are re-exported on both sides.
#[test]
fn core_interfaces_are_reexported() {
    assert_eq!(
        core::client::__interfaces::WL_COMPOSITOR_INTERFACE.name,
        "wl_compositor"
    );
    assert_eq!(
        core::server::__interfaces::WL_COMPOSITOR_INTERFACE.name,
        "wl_compositor"
    );
    assert_eq!(core::client::__interfaces::WL_SHM_INTERFACE.name, "wl_shm");
    assert_eq!(
        core::client::__interfaces::WL_SEAT_INTERFACE.name,
        "wl_seat"
    );
    assert_eq!(
        core::client::__interfaces::WL_OUTPUT_INTERFACE.name,
        "wl_output"
    );
    assert_eq!(
        core::client::__interfaces::WL_DATA_DEVICE_MANAGER_INTERFACE.name,
        "wl_data_device_manager"
    );
}

/// `xdg-shell` and `xdg-output` export the interfaces the compositor binds.
#[test]
fn xdg_interfaces_are_reexported() {
    assert_eq!(
        xdg::shell::client::__interfaces::XDG_WM_BASE_INTERFACE.name,
        "xdg_wm_base"
    );
    assert_eq!(
        xdg::shell::server::__interfaces::XDG_WM_BASE_INTERFACE.name,
        "xdg_wm_base"
    );
    assert_eq!(
        xdg::output::client::__interfaces::ZXDG_OUTPUT_MANAGER_V1_INTERFACE.name,
        "zxdg_output_manager_v1"
    );
}

/// `xdg-decoration` is exposed under `xdg::decoration`.
#[test]
fn xdg_decoration_is_reexported() {
    assert_eq!(
        xdg::decoration::client::__interfaces::ZXDG_DECORATION_MANAGER_V1_INTERFACE.name,
        "zxdg_decoration_manager_v1"
    );
}

/// Each `wp` protocol is exposed under its own module.
#[test]
fn wp_interfaces_are_reexported() {
    assert_eq!(
        wp::viewporter::client::__interfaces::WP_VIEWPORTER_INTERFACE.name,
        "wp_viewporter"
    );
    assert_eq!(
        wp::linux_dmabuf::client::__interfaces::ZWP_LINUX_DMABUF_V1_INTERFACE.name,
        "zwp_linux_dmabuf_v1"
    );
    assert_eq!(
        wp::presentation_time::client::__interfaces::WP_PRESENTATION_INTERFACE.name,
        "wp_presentation"
    );
    assert_eq!(
        wp::idle_inhibit::client::__interfaces::ZWP_IDLE_INHIBIT_MANAGER_V1_INTERFACE.name,
        "zwp_idle_inhibit_manager_v1"
    );
}

/// `ext-idle-notify-v1` is exposed under `ext::idle_notify`.
#[test]
fn ext_idle_notify_is_reexported() {
    assert_eq!(
        ext::idle_notify::client::__interfaces::EXT_IDLE_NOTIFIER_V1_INTERFACE.name,
        "ext_idle_notifier_v1"
    );
    assert_eq!(
        ext::idle_notify::server::__interfaces::EXT_IDLE_NOTIFIER_V1_INTERFACE.name,
        "ext_idle_notifier_v1"
    );
}

/// `wlr-layer-shell-unstable-v1` is exposed under `wlr::layer_shell`.
#[test]
fn wlr_layer_shell_is_reexported() {
    assert_eq!(
        wlr::layer_shell::client::__interfaces::ZWLR_LAYER_SHELL_V1_INTERFACE.name,
        "zwlr_layer_shell_v1"
    );
    assert_eq!(
        wlr::layer_shell::server::__interfaces::ZWLR_LAYER_SHELL_V1_INTERFACE.name,
        "zwlr_layer_shell_v1"
    );
}

/// Every curated interface advertises a non-zero version.
#[test]
fn interfaces_report_a_version() {
    assert!(xdg::shell::client::__interfaces::XDG_WM_BASE_INTERFACE.version >= 1);
    assert!(wlr::layer_shell::client::__interfaces::ZWLR_LAYER_SHELL_V1_INTERFACE.version >= 1);
    assert!(core::client::__interfaces::WL_COMPOSITOR_INTERFACE.version >= 1);
}

/// Both the `client` and `server` halves are reachable for the same protocol.
#[test]
fn client_and_server_halves_are_reachable() {
    use {wayland_client::Proxy as _, wayland_server::Resource as _};

    assert_eq!(
        xdg::shell::client::xdg_wm_base::XdgWmBase::interface().name,
        "xdg_wm_base"
    );
    assert_eq!(
        xdg::shell::server::xdg_wm_base::XdgWmBase::interface().name,
        "xdg_wm_base"
    );

    assert_eq!(
        wlr::layer_shell::client::zwlr_layer_shell_v1::ZwlrLayerShellV1::interface().name,
        "zwlr_layer_shell_v1"
    );
    assert_eq!(
        wlr::layer_shell::server::zwlr_layer_shell_v1::ZwlrLayerShellV1::interface().name,
        "zwlr_layer_shell_v1"
    );
}
