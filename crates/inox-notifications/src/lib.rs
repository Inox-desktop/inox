//! The Inox notification daemon.
//!
//! `inox-notifications` is an L3 component (PRD §3.1): the process behind
//! `org.freedesktop.Notifications`, the one applications send notifications to.
//! It owns the notification stack — the identifiers, the lifetimes, the bound,
//! the policy — and draws the popups.
//!
//! # How the crate is organised
//!
//! | module | what lives there |
//! |---|---|
//! | [`protocol`] | the freedesktop contract as data: bus name, methods, signatures, capabilities, close reasons, hints |
//! | [`markup`] | the body sanitiser: the accepted markup subset, as spans |
//! | [`actions`] | action identity, and the at-most-once rule |
//! | [`queue`] | identifiers, lifetimes, the bound, and the terminal-state counters |
//! | [`policy`] | do not disturb, and the locked-session redaction |
//! | [`contract`] | calls, replies, signals and call deduplication (ADR 0013) |
//! | [`placement`] | the layer-shell placement the stack asks for, as data |
//! | [`view_model`] | the projection from state to what the view draws |
//! | [`app`] | `State`, `Message`, `update` (Elm), `view`, `subscription` |
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! # What is verified, and what is not
//!
//! The crate is delivered at evidence level **E0/E1** (ADR 0018): it compiles,
//! lints, documents itself, and its pure logic — the protocol data, the
//! sanitiser, action identity, the queue with its bound and its lifetimes, the
//! policy, call deduplication, the projection — is covered by unit and property
//! tests, with `en-US`/`zh-CN` catalog parity asserted by
//! [`inox_common::i18n::compare_catalogs`]. `tests/render.rs` draws the real
//! widget tree through the same tiny-skia renderer the binary uses and asserts on
//! pixels, with no compositor involved.
//!
//! Not delivered, and not claimed:
//!
//! - **The D-Bus service.** PRD §13 puts it in M5, and acceptance D06 with it.
//!   What this milestone delivers is the protocol contract, the queue, the policy
//!   and the UI model: [`contract::Request`] is what the M5 bus will deliver and
//!   [`contract::Outgoing`] is what it will drain, and `main.rs` says so. No bus
//!   is opened.
//! - **The layer surface.** The Inox compositor does not advertise
//!   `zwlr_layer_shell_v1`, and `iced_layershell` does not compile against iced
//!   0.14's winit generation (ADR 0018), so the stack runs as an ordinary window
//!   and the placement of [`placement`] is data a future host consumes rather
//!   than something applied. `main.rs` logs exactly that at startup.
//! - **Icons.** `icon-static` and `body-images` are deliberately absent from
//!   [`protocol::CAPABILITIES`]: resolving and decoding an icon or a body image
//!   needs a file or theme lookup off the UI thread. An `<img>` in a body is
//!   parsed and its alt text is shown, and a popup with an icon reference draws
//!   the space the icon will occupy.
//! - **Persistence.** `persistence` is deliberately absent from
//!   [`protocol::CAPABILITIES`]: it would promise that the `resident` hint keeps
//!   a notification after one of its actions is invoked, and this daemon closes
//!   it instead (see [`protocol::Hints::resident`]).
//! - **Configuration hot reload.** [`app::Message::ConfigReloaded`] applies a new
//!   configuration and is tested, but nothing watches `notifications.toml` for
//!   changes yet.
//! - **E2/E3 GUI evidence**, which needs a layer-shell host in CI and a session
//!   bus.

#![forbid(unsafe_code)]

pub mod actions;
pub mod app;
pub mod contract;
pub mod i18n;
pub mod markup;
pub mod placement;
pub mod policy;
pub mod protocol;
pub mod queue;
pub mod theme;
pub mod view;
pub mod view_model;

pub use crate::actions::{Action, ActionError, ActionId, Actions, Invoked};
pub use crate::contract::{
    CallId, Calls, NotifyRequest, Outgoing, Reply, ReplyBody, Request, RequestBody, Signal,
};
pub use crate::markup::{Emphasis, Span};
pub use crate::placement::{
    Anchor, Anchors, KeyboardInteractivity, Layer, NotificationPlacement, OutputAssignment,
    OutputId,
};
pub use crate::policy::{Policy, PopupContent, Presentation, SessionState};
pub use crate::protocol::{CloseReason, Expiry, HintValue, Hints, ServerInformation, Urgency};
pub use crate::queue::{
    Admission, Closed, Counters, Draft, Eviction, Lifetime, Notification, NotificationId, Queue,
};
pub use crate::view::StackGeometry;
pub use crate::view_model::{
    ActionButton, OverflowNotice, Popup, PopupStack, StackInput, StackStatus,
};
