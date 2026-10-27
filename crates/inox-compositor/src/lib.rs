//! The Inox Wayland compositor, built on Smithay.
//!
//! Part of the Inox desktop environment, layer **L2**. See the repository
//! `AGENTS.md` for contribution and architecture constraints.
//!
//! # Current milestone
//!
//! The hardware path uses libseat, Smithay DRM/GBM scanout, libinput and
//! explicit frame lifetimes. Headless remains an explicit software-rendered CI
//! backend. `auto` requires a usable physical output; device acquisition failure
//! never reports a headless session as ready.
//!
//! Every GPU on the seat can drive its own connectors, and each output carries
//! its own position and scale (fractional included). There is no cross-GPU buffer
//! copy path yet, which is why `linux-dmabuf` is not advertised. CI evidence does
//! not replace the O01–O04 human hardware acceptance.
//!
//! Window management is tiling-only: a toplevel becomes a window on the first
//! commit that carries a buffer, the focused window is told it is `activated`,
//! and `Super+Shift+Q` asks it to close. See [`state`] for those paths.
//!
//! # Layout
//!
//! - [`backend`] — DRM/GBM, libinput and shared frame lifecycle.
//! - [`config`] — command line parsing into plain configuration data.
//! - [`glass`] — the pure half of the private `inox-glass-v1` glass protocol:
//!   the blur and refraction parameters, the two capability decisions and the
//!   pane list the renderer draws.
//! - [`input`] — key presses mapped to compositor actions.
//! - [`layout`] — the pure master/stack tiling geometry.
//! - [`workspace`] — the pure window bookkeeping and layout order.
//! - [`render`] — the EGL/GLES render target and one of the crate's `unsafe` sites.
//! - `test_input` — the test-only control channel behind `--test-input`, present only in
//!   builds with the `test-input` feature (off by default).
//! - [`runtime`] — the calloop event loop: Wayland, device sources, frame scheduling and signals.
//! - [`session_lock`] — the pure half of the session lock: who may lock, and
//!   when every output has been covered.
//! - [`state`] — Smithay compositor state and its protocol handlers.
//! - [`trial`] — the pure display trial state machine: generation, deadline and
//!   rollback decisions.

pub mod backend;
pub mod config;
mod cursor;
pub mod glass;
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
pub mod trial;
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
