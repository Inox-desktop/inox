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
//!
//! The crate is owned by the L2 layer; see the repository `AGENTS.md` and the
//! Inox ADR on protocol strategy for the rationale and the versioning rules.

pub mod ipc;
pub mod standard;

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
