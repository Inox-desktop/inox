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
//! Those three are presented on one of three surfaces, chosen by
//! `[panel] surface`:
//!
//! - the **taskbar** ([`view::panel`]), a strip along one edge;
//! - the **menu bar** ([`menubar`]), a full-width glass bar along the top edge;
//! - the **dock** ([`dock`]), a floating glass pane above the bottom edge.
//!
//! The surfaces are presentations of the *same* data, never new data: the
//! projection in [`view_model`] is shared, and the menu bar and the dock add
//! only their own geometry and arrangement. The menu bar shows the focused
//! application's name and nothing that pretends to be an application menu:
//! Inox has no application-menu protocol, and the bar does not invent one.
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
//! | [`menubar`] | the menu bar's geometry and projection — pure |
//! | [`dock`] | the dock's geometry and projection — pure |
//! | [`app`] | `State`, `Message`, `update` (Elm), `view`, `subscription` |
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`frost`] | the frosted-glass decisions — pure, fully tested |
//! | [`layer`] | the protocol leg: a real `wlr-layer-shell` surface |
//! | [`reload`] | `SIGHUP` and `org.inox.Panel` reload requests |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! # What is verified, and what is not
//!
//! The crate is delivered at evidence level **E0/E1** (ADR 0018): it compiles,
//! lints, documents itself, and its pure logic — the task model, the tray
//! lifecycle, the clock, the placement policy, the projection, the menu bar's and
//! the dock's geometry, the frosted-glass decisions — is covered by unit and
//! property tests, with `en-US`/`zh-CN` catalog parity asserted by
//! [`inox_common::i18n::compare_catalogs`]. `tests/render.rs` draws the real
//! widget tree of every surface through the same tiny-skia renderer the binary
//! uses and asserts on pixels, with no compositor involved.
//!
//! `tests/protocol.rs` goes one step further than E1: it plays the compositor
//! side of `wlr-layer-shell`, `wl_shm`, `wl_seat` and `inox-glass-v1` on a real
//! socket pair and drives the built binary, so the placement, the shared-memory
//! frame and the glass request are asserted **on the wire** rather than in a
//! comment. What that needs to run is a Unix socket pair, which CI has.
//!
//! Not delivered, and not claimed:
//!
//! - **The transports.** The window service (inox-ipc or Wayland) and the
//!   StatusNotifierItem protocol (D-Bus) are not implemented: [`app`] queues what
//!   it wants in [`app::OutgoingCommand`] and [`app::Message`] is the interface a
//!   transport will feed. `layer` drops those commands with one log line
//!   explaining that nothing can carry them yet, and no bus is opened by [`app`].
//! - **One surface per process, on one output.** `layer` asks the compositor to
//!   place the surface on the output it considers active (the `wlr-layer-shell`
//!   null-output request); it does not enumerate `wl_output`s and create one
//!   surface per output. A session that wants a panel on every output runs one
//!   process per output, which is also how it runs a menu bar and a dock at once.
//! - **E3 GUI evidence** on real hardware, which CI cannot provide.

#![forbid(unsafe_code)]

pub mod app;
pub mod clock;
pub mod contract;
pub mod dock;
pub mod frost;
pub mod i18n;
pub mod layer;
pub mod menubar;
pub mod placement;
pub mod reload;
pub mod tasks;
pub mod theme;
pub mod tray;
pub mod view;
pub mod view_model;

pub use crate::clock::{ClockError, ClockLabel};
pub use crate::contract::{
    CommandState, PendingRequests, RejectionReason, RequestId, ServiceFailure, ServiceState,
};
pub use crate::dock::{DockContent, DockGeometry, DockTile, DockView};
pub use crate::layer::{Interface, LegError, Outcome, Pointer, Produced};
pub use crate::menubar::{MenubarGeometry, MenubarView};
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
