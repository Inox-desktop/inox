//! Compositor backend selection (ADR 0010).
//!
//! The crate ships two backends:
//!
//! - The M1/M2 headless path built on surfaceless EGL plus an offscreen
//!   `GlesRenderbuffer`, driven by a calloop timer (see ADR 0005). Its code
//!   lives in [`crate::render`] and [`crate::state`]. It is the only backend
//!   CI can exercise end-to-end because GitHub runners have no DRM master
//!   and no input-device permissions.
//! - [`drm`] — the M2.5 real backend that opens a libseat session and a
//!   DRM/KMS device. Its GBM swapchain, page flip and vblank-driven frame
//!   loop are still being wired up (see ADR 0010 §M3a); this module exposes
//!   enough of the shape that CI can compile-check the DRM feature surface
//!   and reviewers can see the intended boundary.
//!
//! The selector type [`crate::config::BackendKind`] lives in [`crate::config`]
//! so command-line parsing stays independent of Smithay types; runtime
//! dispatch happens in [`crate::runtime::run`].

pub mod drm;

pub use drm::{BackendError, DrmBackend};
