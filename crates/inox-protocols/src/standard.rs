//! Curated standard Wayland protocol bindings.
//!
//! This module is a thin curation layer over the upstream binding crates. It
//! re-exports exactly the standard protocols Inox targets so that downstream
//! crates depend on `inox_protocols::standard` instead of on
//! `wayland-protocols*` directly and protocol versions are pinned in one place.
//! It only re-exports; it never wraps or re-implements upstream bindings.
//!
//! The tree mirrors the upstream layout:
//!
//! - [`core`]: the core `wl_*` interfaces (`wayland-client` / `wayland-server`).
//! - [`xdg`]: window-management protocols from `wayland-protocols`.
//! - [`wp`]: general-purpose protocols from `wayland-protocols`.
//! - [`ext`]: `wayland-protocols` protocols that fit no other category.
//! - [`wlr`]: wlroots extensions from `wayland-protocols-wlr`.
//!
//! Every protocol module exposes a `client` and a `server` submodule. The
//! support level from the PRD §2.3 protocol matrix is noted on each protocol
//! module; `必须` means required for the M1 target, `计划` means planned.

/// Core `wl_*` interfaces.
///
/// PRD §2.3 support level: `必须` (compositor/subcompositor, shm,
/// seat/pointer/keyboard, output, data device).
pub mod core {
    /// Core interfaces, client side (from `wayland-client`).
    pub use wayland_client::protocol as client;
    /// Core interfaces, server side (from `wayland-server`).
    pub use wayland_server::protocol as server;
}

/// Window-management protocols (`xdg` namespace).
pub mod xdg {
    /// `xdg-shell`: standard window management.
    ///
    /// PRD §2.3 support level: `必须`.
    pub mod shell {
        /// Client-side `xdg-shell` bindings.
        pub use wayland_protocols::xdg::shell::client;
        /// Server-side `xdg-shell` bindings.
        pub use wayland_protocols::xdg::shell::server;
    }

    /// `xdg-decoration`: server/client decoration negotiation.
    ///
    /// PRD §2.3 support level: `必须`.
    pub mod decoration {
        /// Client-side `xdg-decoration` bindings.
        pub use wayland_protocols::xdg::decoration::zv1::client;
        /// Server-side `xdg-decoration` bindings.
        pub use wayland_protocols::xdg::decoration::zv1::server;
    }

    /// `xdg-output`: desktop-oriented output description.
    ///
    /// Not a standalone PRD §2.3 row, but `必须` in practice: the compositor's
    /// `OutputManagerState` needs it alongside `xdg-shell`.
    pub mod output {
        /// Client-side `xdg-output` bindings.
        pub use wayland_protocols::xdg::xdg_output::zv1::client;
        /// Server-side `xdg-output` bindings.
        pub use wayland_protocols::xdg::xdg_output::zv1::server;
    }
}

/// General-purpose protocols (`wp` namespace).
pub mod wp {
    /// `viewporter`: surface scaling and cropping.
    ///
    /// PRD §2.3 support level: `必须`.
    pub mod viewporter {
        /// Client-side `viewporter` bindings.
        pub use wayland_protocols::wp::viewporter::client;
        /// Server-side `viewporter` bindings.
        pub use wayland_protocols::wp::viewporter::server;
    }

    /// `linux-dmabuf`: GPU buffer zero-copy import/export.
    ///
    /// PRD §2.3 support level: `必须`.
    pub mod linux_dmabuf {
        /// Client-side `linux-dmabuf` bindings.
        pub use wayland_protocols::wp::linux_dmabuf::zv1::client;
        /// Server-side `linux-dmabuf` bindings.
        pub use wayland_protocols::wp::linux_dmabuf::zv1::server;
    }

    /// `presentation-time`: frame presentation feedback.
    ///
    /// PRD §2.3 support level: `必须`.
    pub mod presentation_time {
        /// Client-side `presentation-time` bindings.
        pub use wayland_protocols::wp::presentation_time::client;
        /// Server-side `presentation-time` bindings.
        pub use wayland_protocols::wp::presentation_time::server;
    }

    /// `idle-inhibit`: client request to inhibit the idle timeout.
    ///
    /// PRD §2.3 support level: `必须`.
    pub mod idle_inhibit {
        /// Client-side `idle-inhibit` bindings.
        pub use wayland_protocols::wp::idle_inhibit::zv1::client;
        /// Server-side `idle-inhibit` bindings.
        pub use wayland_protocols::wp::idle_inhibit::zv1::server;
    }
}

/// Miscellaneous protocols (`ext` namespace).
pub mod ext {
    /// `ext-idle-notify-v1`: compositor idle-status notifications.
    ///
    /// PRD §2.3 support level: `必须`.
    pub mod idle_notify {
        /// Client-side `ext-idle-notify-v1` bindings.
        pub use wayland_protocols::ext::idle_notify::v1::client;
        /// Server-side `ext-idle-notify-v1` bindings.
        pub use wayland_protocols::ext::idle_notify::v1::server;
    }
}

/// wlroots extensions (`wlr` namespace).
pub mod wlr {
    /// `wlr-layer-shell-unstable-v1`: layers for panels, wallpapers and locks.
    ///
    /// PRD §2.3 support level: `必须`.
    pub mod layer_shell {
        /// Client-side `wlr-layer-shell` bindings.
        pub use wayland_protocols_wlr::layer_shell::v1::client;
        /// Server-side `wlr-layer-shell` bindings.
        pub use wayland_protocols_wlr::layer_shell::v1::server;
    }
}
