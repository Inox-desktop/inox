//! The Inox wallpaper: what is drawn below every window, and where.
//!
//! The component is an L3 layer-shell client (PRD §3.1, ADR 0013). Its
//! authority is narrow and deliberate: it reads `[wallpaper]` from `shell.toml`,
//! decodes the image the user selected, works out where that image lands on an
//! output, and puts it there. It holds no state another component owns and
//! consumes no compositor internals — it draws, and it says what it could not
//! draw.
//!
//! # What is implemented, and what is blocked
//!
//! Implemented and tested here (acceptance levels **E0/E1**):
//!
//! - the resolution order from `[wallpaper]` to a per-output image, including
//!   the `*` default ([`model::Selection`]);
//! - decoding PNG and JPEG under a documented bound ([`decode`]);
//! - the geometry of `fill`, `fit`, `stretch`, `center` and `tile`
//!   ([`scaling`]), with the tiling grid bounded as well;
//! - atomic, per-output application ([`apply`]): a frame is complete before it
//!   replaces anything, and one output failing never clears another;
//! - the background-layer placement policy as pure data ([`placement`]);
//! - the ADR 0013 state contract — generation, per-output result, service
//!   state — as [`contract`];
//! - the Iced interface: state and messages ([`app`]), the projection the view
//!   reads ([`view_model`]) and the widget tree ([`view`]).
//!
//! **Blocked (E2/E3)**: no host in CI advertises `wlr-layer-shell-unstable-v1`,
//! and neither `iced_layershell` 0.19.1 nor 0.19.0 compiles against its pinned
//! `iced` generation (ADR 0018). The binary therefore runs as an ordinary
//! `xdg-shell` window and logs one warning saying so; [`placement`] is computed
//! and tested but not applied, and nothing here claims otherwise.
//!
//! # A failure is never a crash and never a black output
//!
//! ADR 0014 draws the line this crate follows: a resource the user explicitly
//! selected and which cannot be used must be *reported*, while the absence of
//! any selection is simply the default. So a missing, unreadable or undecodable
//! file degrades to the theme background — the output stays operable — and
//! carries a reason that names the output, the path and what went wrong
//! ([`model::Unusable`]), which the interface renders from a Fluent catalog
//! (acceptance D05, Q03).

#![forbid(unsafe_code)]

pub mod app;
pub mod apply;
pub mod contract;
pub mod decode;
pub mod i18n;
pub mod model;
pub mod placement;
pub mod scaling;
pub mod source;
pub mod theme;
pub mod view;
pub mod view_model;

pub use crate::app::{Message, Preview, Request, State};
pub use crate::contract::{
    CommandResult, Generation, OutputStatus, Problem, ServiceState, Showing, Snapshot,
};
pub use crate::decode::{DecodeError, Frame};
pub use crate::model::{OutputName, Selection, Size, Unusable};
pub use crate::placement::{Anchors, KeyboardInteractivity, Layer, Margins, SurfacePlacement};
pub use crate::scaling::{Placement, Rect};
pub use crate::source::OutputSource;

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
