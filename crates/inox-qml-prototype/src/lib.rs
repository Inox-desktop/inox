//! Inox QML prototype: a self-contained cxx-qt proof crate for the QML migration.
//!
//! It registers the `com.inox.prototype.qml` QML module and demonstrates a
//! Rust-backed `QObject` exposed to Qt Quick. See the repository `AGENTS.md`
//! for contribution and architecture constraints.
//!
//! # Note on `unsafe`
//!
//! This crate intentionally does **not** set `#![forbid(unsafe_code)]` at the
//! root: the `#[cxx_qt::bridge]` macro in `cxxqt_object` generates the required
//! `unsafe extern "C++"` / `unsafe fn` FFI glue with no `#[allow]` wrapper, and
//! a crate-level `forbid`/`deny` makes that generated code fail to compile. We
//! therefore inherit `unsafe_code = "allow"` from `Cargo.toml` (the cxx-qt binding
//! exception). The `unsafe` lives only in the generated bridge; Inox-authored
//! code elsewhere in this crate stays unsafe-free.

/// The cxx-qt bridge object exposed to QML.
pub mod cxxqt_object;
