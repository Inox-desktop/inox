//! Shared types, error handling and logging utilities for the Inox desktop.
//!
//! Part of the Inox desktop environment. See the repository `AGENTS.md` for
//! contribution and architecture constraints.

pub mod i18n;
pub mod logging;
pub mod paths;
pub mod secret;

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
