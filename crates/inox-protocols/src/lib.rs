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
pub mod standard;

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
