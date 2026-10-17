//! The Inox application launcher.
//!
//! The launcher is the L3 component that finds an application and starts it.
//! It owns three things, and the split between them is what makes the crate
//! testable without a compositor:
//!
//! | module | responsibility | contains |
//! |--------|----------------|----------|
//! | [`desktop_entry`], [`exec`], [`index`], [`search`] | the domain | the desktop entry grammar, the XDG directory precedence, the ranking |
//! | [`contract`], [`activation`], [`placement`] | the state machine | request generations, at-most-once activation, the layer-shell policy |
//! | [`app`], [`view`], [`view_model`], [`theme`], [`i18n`] | the Iced shell | `Message` → `update` → `view`, catalogs, tokens |
//!
//! Everything in the first two rows is pure: no Iced, no I/O except where a
//! function says so, and a unit test per rule. The acceptance scenarios the
//! launcher answers are D02 (search, select, launch, Esc, dedup, Exec parsing,
//! retry) plus D05's loading/empty/error/disabled/retry states, Q01 (keyboard
//! navigation), Q02 (the render tests) and Q03 (the catalogs).
//!
//! ## What the launcher does not do yet
//!
//! - **It is a window, not a layer surface.** The Inox compositor does not
//!   advertise `zwlr_layer_shell_v1`, and `iced_layershell` 0.19 does not
//!   compile against this iced generation. [`placement`] computes the policy a
//!   layer-shell host will consume; the binary runs as an ordinary xdg-shell
//!   window and says so at startup ([ADR 0018]).
//! - **It verifies a launch by process liveness.** ADR 0013 asks for readiness
//!   evidence — a window, a D-Bus name — and neither is reachable from here yet;
//!   see [`activation`]'s module documentation.
//! - **It does not load icon themes.** An entry's icon is drawn as a
//!   placeholder; a missing icon is visible as such rather than silent.
//! - **It does not export a semantic tree.** Iced 0.14 has no AT-SPI bridge, so
//!   acceptance Q01's role/name/state/action export is blocked on that
//!   prototype, which ADR 0013 requires before M4.
//!
//! [ADR 0018]: https://github.com/Inox-desktop/inox/blob/main/docs/adr/0018-l3-component-architecture.md

#![forbid(unsafe_code)]

pub mod activation;
pub mod app;
pub mod contract;
pub mod desktop_entry;
pub mod exec;
pub mod i18n;
pub mod index;
pub mod placement;
pub mod search;
pub mod theme;
pub mod view;
pub mod view_model;

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
