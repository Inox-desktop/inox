//! The Inox Wayland compositor, built on Smithay.
//!
//! Part of the Inox desktop environment, layer **L2**. See the repository
//! `AGENTS.md` for contribution and architecture constraints.
//!
//! # Current milestone
//!
//! The M2.5 hardware path uses libseat, Smithay DRM/GBM scanout, libinput
//! and explicit frame lifetimes (accepted ADR 0012). Headless remains an explicit
//! software-rendered CI backend. `auto` requires a usable physical output;
//! device acquisition failure never reports a headless session as ready.
//! Hardware support is limited to one active connector on one GPU, scale 1.
//! CI evidence does not replace the O01/O04 human hardware acceptance.
//!
//! # Layout
//!
//! - [`backend`] — DRM/GBM, libinput and shared frame lifecycle (ADR 0012).
//! - [`config`] — command line parsing into plain configuration data.
//! - [`input`] — key presses mapped to compositor actions.
//! - [`layout`] — the pure master/stack tiling geometry.
//! - [`workspace`] — the pure window bookkeeping and layout order.
//! - [`render`] — the EGL/GLES render target and one of the crate's `unsafe` sites.
//! - `test_input` — the test-only control channel behind `--test-input`, present only in
//!   builds with the `test-input` feature (off by default).
//! - [`runtime`] — the calloop event loop: Wayland, device sources, frame scheduling and signals.
//! - [`session_lock`] — the pure half of the session lock: who may lock, and
//!   when every output has been covered (ADR 0015, ADR 0022).
//! - [`state`] — Smithay compositor state and its protocol handlers.

pub mod backend;
pub mod config;
mod cursor;
pub mod input;
mod layers;
pub mod layout;
pub mod render;
pub mod runtime;
pub mod session_lock;
pub mod state;
#[cfg(feature = "test-input")]
pub mod test_input;
pub mod topology;
pub mod workspace;

pub use backend::{BackendError, GpuBackend};
pub use config::{BackendKind, CompositorConfig, ConfigError, HeadlessOutput, Invocation};
pub use input::Action;
pub use layout::{LayoutConfig, MASTER_RATIO_MAX, MASTER_RATIO_MIN};
pub use render::{HeadlessRenderer, RenderError};
pub use runtime::{RunError, run};
pub use state::{ClientState, InoxState, OutputDescriptor, StateError};
pub use topology::{OutputId, Placement};
pub use workspace::{WindowId, Workspace};

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
