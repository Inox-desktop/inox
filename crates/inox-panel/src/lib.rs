//! The Inox panel: the strip along an output's edge that shows what is running.
//!
//! `inox-panel` is an L3 component (PRD §3.1). It draws three things and owns
//! none of the state behind them:
//!
//! - a **task list**, one button per application, built from the window states
//!   the compositor reports ([`tasks`]);
//! - a **system tray** of StatusNotifierItems ([`tray`]);
//! - a **clock** ([`clock`]).
//!
//! Everything it shows comes from an authoritative service, over a contract
//! rather than a shared in-process model: ADR 0013 gives the panel a snapshot
//! plus events, server-assigned window identities, and commands whose sent /
//! accepted / applied / rejected states are distinguishable ([`contract`]).
//! Nothing here guesses a window's identity from a PID, a title or a list index.
//!
//! # How the crate is organised
//!
//! | module | what lives there |
//! |---|---|
//! | [`tasks`] | the window and task-group model — pure, fully tested |
//! | [`tray`] | the StatusNotifierItem and DBusMenu model — pure |
//! | [`clock`] | expanding `panel.clock_format` into a localized label |
//! | [`contract`] | service state, request correlation, deduplication |
//! | [`placement`] | the layer-shell placement the panel asks for, as data |
//! | [`view_model`] | the projection from state to what the view draws |
//! | [`app`] | `State`, `Message`, `update` (Elm), `view`, `subscription` |
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! # What is verified, and what is not
//!
//! The crate is delivered at evidence level **E0/E1** (ADR 0018): it compiles,
//! lints, documents itself, and its pure logic — the task model, the tray
//! lifecycle, the clock, the placement policy, the projection — is covered by
//! unit and property tests, with `en-US`/`zh-CN` catalog parity asserted by
//! [`inox_common::i18n::compare_catalogs`]. `tests/render.rs` draws the real
//! widget tree through the same tiny-skia renderer the binary uses and asserts on
//! pixels, with no compositor involved.
//!
//! Not delivered, and not claimed:
//!
//! - **The layer surface.** The Inox compositor does not advertise
//!   `zwlr_layer_shell_v1`, and `iced_layershell` 0.19 does not compile against
//!   iced 0.14's winit generation, so the panel runs as an ordinary window and
//!   the placement of [`placement`] is data a future host consumes, not something
//!   that is applied. `main.rs` logs exactly that at startup (ADR 0018).
//! - **The transports.** The window service (inox-ipc or Wayland) and the
//!   StatusNotifierItem protocol (D-Bus) are not implemented: [`app`] queues what
//!   it wants in [`app::OutgoingCommand`] and [`app::Message`] is the interface a
//!   transport will feed. No bus is opened, and `main.rs` says so.
//! - **E2/E3 GUI evidence**, which needs a layer-shell host in CI.

#![forbid(unsafe_code)]

pub mod app;
pub mod clock;
pub mod contract;
pub mod i18n;
pub mod placement;
pub mod tasks;
pub mod theme;
pub mod tray;
pub mod view;
pub mod view_model;

pub use crate::clock::{ClockError, ClockLabel};
pub use crate::contract::{
    CommandState, PendingRequests, RejectionReason, RequestId, ServiceFailure, ServiceState,
};
pub use crate::placement::{
    Anchor, Anchors, Autohide, KeyboardInteractivity, Layer, OutputAssignment, PanelPlacement,
};
pub use crate::tasks::{
    Applied, OutputId, SessionGeneration, TaskGroup, TaskList, WindowId, WindowKey, WindowState,
    WorkspaceId,
};
pub use crate::tray::{
    MenuItem, MenuItemId, Tray, TrayError, TrayItem, TrayItemId, TrayItemUpdate, TraySnapshot,
    TrayStatus,
};
pub use crate::view::PanelGeometry;
pub use crate::view_model::{PanelInput, PanelStatus, PanelViewModel, TaskButton, TrayItemView};
