//! The Inox Wayland compositor, built on Smithay.
//!
//! Part of the Inox desktop environment, layer **L2**. See the repository
//! `AGENTS.md` for contribution and architecture constraints.
//!
//! # Current milestone
//!
//! The crate is on **M2.5** (PRD §13, ADR 0010): the M1/M2 headless path
//! (surfaceless EGL, timer-driven frames, injected input) is complete and
//! remains the CI target. The real DRM/KMS backend is being landed through a
//! typed scaffold: `--backend` selects the surfaceless path (`headless`), the
//! real path (`drm`) or an automatic probe that falls back to headless
//! (`auto`, the default). Until M3a fills in the GBM swapchain and vblank
//! present path, a successful DRM probe reports
//! [`state::StateError`]-free `BackendError::Pending` and the compositor
//! continues on headless, which is what ADR 0010 §9 defines as the CI
//! contract. libinput arrives in M3b.
//!
//! # Layout
//!
//! - [`backend`] — DRM/KMS real backend scaffold (ADR 0010).
//! - [`config`] — command line parsing into plain configuration data.
//! - [`input`] — key presses mapped to compositor actions.
//! - [`layout`] — the pure master/stack tiling geometry.
//! - [`workspace`] — the pure window bookkeeping and layout order.
//! - [`render`] — the EGL/GLES render target and one of the crate's `unsafe` sites.
//! - `test_input` — the test-only control channel behind `--test-input`, present only in
//!   builds with the `test-input` feature (off by default).
//! - [`runtime`] — the calloop event loop: backend probe, Wayland socket, frame timer, signals.
//! - [`state`] — Smithay compositor state and its protocol handlers.

pub mod backend;
pub mod config;
pub mod input;
pub mod layout;
pub mod render;
pub mod runtime;
pub mod state;
#[cfg(feature = "test-input")]
pub mod test_input;
pub mod workspace;

pub use backend::{BackendError, DrmBackend};
pub use config::{BackendKind, CompositorConfig, ConfigError, Invocation};
pub use input::Action;
pub use layout::{LayoutConfig, MASTER_RATIO_MAX, MASTER_RATIO_MIN};
pub use render::{HeadlessRenderer, RenderError};
pub use runtime::{RunError, run};
pub use state::{ClientState, InoxState, StateError};
pub use workspace::{WindowId, Workspace};

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
