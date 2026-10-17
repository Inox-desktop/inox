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
    /// `text-input-v3`: focused applications' text editing context.
    ///
    /// Only the keyboard-focused client receives enter and editing events.
    /// State is double buffered until commit, and reset on focus loss.
    pub mod text_input {
        /// Client-side text input bindings.
        pub use wayland_protocols::wp::text_input::zv3::client;
        /// Server-side text input bindings.
        pub use wayland_protocols::wp::text_input::zv3::server;
    }

    /// `input-method-v2`: privileged input method service, one per seat.
    ///
    /// Only a compositor-launched connection may bind this global. Grabs,
    /// candidate surfaces and editing access are suspended during session lock.
    /// Input method commits must match the most recent done serial.
    pub mod input_method {
        /// Client-side input method bindings.
        pub use wayland_protocols_misc::zwp_input_method_v2::client;
        /// Server-side input method bindings.
        pub use wayland_protocols_misc::zwp_input_method_v2::server;
    }

    /// `virtual-keyboard-v1`: privileged passthrough of unconsumed IME keys.
    ///
    /// Shares the input method connection's authority. Existing objects cannot
    /// inject input while the session is locked or the seat has no keyboard.
    pub mod virtual_keyboard {
        /// Client-side virtual keyboard bindings.
        pub use wayland_protocols_misc::zwp_virtual_keyboard_v1::client;
        /// Server-side virtual keyboard bindings.
        pub use wayland_protocols_misc::zwp_virtual_keyboard_v1::server;
    }

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

    /// `fractional-scale-v1`: the exact (non-integer) scale a surface should render at.
    ///
    /// PRD §2.3 support level: `计划` until ADR 0020's pixel and geometry evidence
    /// exists; the compositor advertises the global from that ADR's implementation.
    /// This protocol is a hint: a client may ignore it, so a compositor must composite
    /// a surface correctly whatever buffer size it is handed.
    pub mod fractional_scale {
        /// Client-side `fractional-scale-v1` bindings.
        pub use wayland_protocols::wp::fractional_scale::v1::client;
        /// Server-side `fractional-scale-v1` bindings.
        pub use wayland_protocols::wp::fractional_scale::v1::server;
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

    /// `ext-session-lock-v1`: the secure session lock (PRD §2.3, ADR 0015).
    ///
    /// PRD §2.3 support level: `必须` from M5.
    ///
    /// This is a security boundary, not a placement hint. The compositor owns
    /// the lock state; the client of this protocol is the trusted lock screen.
    /// The rules the compositor must uphold are in the protocol description
    /// itself and are quoted here so an implementer does not have to go looking:
    ///
    /// - The `locked` event must not be sent until a new "locked" frame has been
    ///   presented on **all** outputs, so that no unlocked content is possibly
    ///   visible when the client is told it owns the screen.
    /// - A client that dies while the session is locked does **not** unlock it.
    ///   The session may stay locked permanently; a compositor may instead let a
    ///   new client take over the lock, which is compositor policy.
    /// - While locked, the compositor stops rendering and providing input to
    ///   normal clients, and hides their content behind an opaque frame.
    /// - `destroy` is a protocol error once `locked` has been sent;
    ///   `unlock_and_destroy` is a protocol error before it.
    ///
    /// The protocol is in upstream *staging*, so its version is pinned by this
    /// crate rather than by a downstream consumer: `ext_session_lock_manager_v1`
    /// and its two children are all at version 1, and a version bump upstream
    /// lands here first as a Cargo update plus a review of the rules above.
    pub mod session_lock {
        /// Client-side `ext-session-lock-v1` bindings.
        pub use wayland_protocols::ext::session_lock::v1::client;
        /// Server-side `ext-session-lock-v1` bindings.
        pub use wayland_protocols::ext::session_lock::v1::server;
    }
}

/// wlroots extensions (`wlr` namespace).
pub mod wlr {
    /// `wlr-layer-shell-unstable-v1`: layers for panels, wallpapers and locks.
    ///
    /// PRD §2.3 support level: `必须`.
    ///
    /// This is ordinary desktop placement, not a security or authentication
    /// boundary. Exclusive keyboard focus follows the mapped top/overlay surface;
    /// clients with `None` never receive keyboard focus. A null output selects the
    /// compositor's output. Clients must commit without a buffer, acknowledge the
    /// configure, then attach a buffer; null-buffer commits unmap the surface and
    /// release its exclusive zone and focus. Destroying either surface does likewise.
    pub mod layer_shell {
        /// Client-side `wlr-layer-shell` bindings.
        pub use wayland_protocols_wlr::layer_shell::v1::client;
        /// Server-side `wlr-layer-shell` bindings.
        pub use wayland_protocols_wlr::layer_shell::v1::server;
    }
}
