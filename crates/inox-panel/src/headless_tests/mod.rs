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
//! Nothing here is compiled into the shipped binary: `src/main.rs` declares this
//! module under `#[cfg(test)]`.

pub mod binary;
pub mod catalog;
pub mod projection;
pub mod protocol;
pub mod qml;
pub mod support;
