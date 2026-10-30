//! The panel's in-binary test harness.
//!
//! The crate is binary only (migration contract §2.1), so the tests that used to
//! live in `tests/*.rs` are modules here instead:
//!
//! | module | what it covers |
//! |---|---|
//! | [`binary`] | runs the panel's own entry point in a child process |
//! | [`catalog`] | `en-US`/`zh-CN` message parity (Q03) |
//! | [`projection`] | what each surface projects — colours, sizes, sentences, clicks |
//! | [`protocol`] | a real `wayland-server` peer drives the built binary (E2) |
//! | [`qml`] | one frame through the offscreen QML host, in a child process |
//! | [`support`] | the harness the projection tests project through |
//!
//! Where the build host's Qt can carry the private ABI (`cfg(native_surface)`)
//! the binary's leg is the **native** one, and two modules change shape:
//!
//! | module | what it covers |
//! |---|---|
//! | `native` | the native shell surface, and the panel loop on it, against a real `inox-compositor` (ADR 0046 P3) |
//!
//! [`protocol`] is compiled only where the binary runs the layer leg
//! (`#[cfg(not(native_surface))]`). It drives `main` as a child process against a
//! hand-written `wlr-layer-shell` compositor, and on a native build `main` creates
//! an `inox_shell_surface_v1` surface instead — driving it against a layer-shell
//! peer would assert that the panel is something it is not (ADR 0046 §9). The
//! wire is still covered on a native build: `native` runs the real compositor,
//! which is one step further, not one step back.
//!
//! Nothing here is compiled into the shipped binary: `src/main.rs` declares this
//! module under `#[cfg(test)]`.

pub mod binary;
pub mod catalog;
#[cfg(native_surface)]
pub mod native;
pub mod projection;
#[cfg(not(native_surface))]
pub mod protocol;
pub mod qml;
pub mod support;
