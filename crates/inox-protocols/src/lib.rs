//! Wayland protocol bindings and Inox protocol extensions.
//!
//! `inox-protocols` is the single source of truth for the Wayland protocols
//! Inox implements or consumes. It has two halves:
//!
//! - [`standard`] curates the standard protocol bindings (core `wl_*`, the
//!   `wayland-protocols` extensions and the `wlr` extensions) that Inox targets,
//!   so upstream versions are pinned in one place.
//! - [`ipc`] provides the bindings for `inox-ipc`, Inox's own wayland protocol
//!   for control messages between the compositor and the Inox shell.
//! - [`glass`] provides the bindings for `inox-glass-v1`, Inox's own protocol
//!   that lets a client ask the compositor to frost the backdrop behind a region
//!   of its own surface (version 1) and to refract that backdrop at the region's
//!   rounded edge for a liquid-glass material (version 2).
//! - [`window_manager`] provides the bindings for `inox-window-manager-v1`, the
//!   protocol that lets the shell's panel observe the compositor's managed
//!   windows — handle, application id, title, workspace, output, mode, and
//!   whether each is minimized, visible and focused — and drive them (activate,
//!   minimize, close, set mode, move to a workspace, switch workspace). It
//!   exists because the compositor owns windows and the panel draws them, while
//!   a Cargo dependency between the two is forbidden.
//! - [`shell`] provides the bindings for `inox-shell-v1`, the protocol that gives
//!   Inox's own desktop components their shell role: placement in the work area,
//!   the shell keyboard model, popups the compositor dismisses (submenus are
//!   nested popups), the work-area event a panel has to lay out against, and
//!   enter/exit transitions the compositor draws. It exists because a panel and
//!   the menu it opens are one visual object, and no standard protocol can ask
//!   for the frames between two of its states; see ADR 0046.

//! - [`control`] is `session-control-v1`, the one-line private contract a session's
//!   own desktop uses to ask it to start the trusted lock client. It lives here
//!   rather than beside its server because its client is L3 (`inox-lock`) and its
//!   server is L4 (`inox-session`): the types have to sit at the L2 boundary both
//!   may depend on.
//!
//! The crate is owned by the L2 layer; see the repository `AGENTS.md` and the
//! Inox ADR on protocol strategy for the rationale and the versioning rules.

pub mod control;
pub mod glass;
pub mod ipc;
pub mod shell;
pub mod standard;
pub mod window_manager;

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
