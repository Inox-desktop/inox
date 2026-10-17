//! Physical backend boundaries and the frame lifecycle shared with headless.
//!
//! Smithay owns DRM/GBM, input and session abstractions. The calloop integration
//! lives in `runtime::physical`; no device polling thread is introduced.
//! CI uses the explicit headless backend and exercises the shared logical state.

pub mod drm;
pub(crate) mod frame;
pub(crate) mod input;

pub use drm::{BackendError, GpuBackend};
