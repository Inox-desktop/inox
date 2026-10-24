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
//! | [`bus`] | the `org.freedesktop.Notifications` service on the session bus |
//! | [`lock_state`] | the compositor's session lock state, over `inox-ipc` |
//! | [`placement`] | the layer-shell placement the stack asks for, as data |
//! | [`view_model`] | the projection from state to what the view draws |
//! | [`app`] | `State`, `Message`, `update` (Elm), `view`, `subscription` |
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! # What is verified, and what is not
//!
//! The crate is delivered at evidence level **E1 with an E2 leg** (ADR 0022's
//! acceptance row for D06): its pure logic — the protocol data, the sanitiser,
//! action identity, the queue with its bound and its lifetimes, the policy, call
//! deduplication, the projection — is covered by unit and property tests, with
//! `en-US`/`zh-CN` catalog parity asserted by
//! [`inox_common::i18n::compare_catalogs`]. `tests/render.rs` draws the real
//! widget tree through the same tiny-skia renderer the binary uses and asserts on
//! pixels, with no compositor involved.
//!
//! What is **real** as of M5: the D-Bus service. [`bus::Service`] serves
//! `org.freedesktop.Notifications` on the session bus — the four methods of the
//! specification, the two signals, and the standard reflection interfaces a
//! client's proxy asks about first — and `tests/bus.rs` drives it over a real
//! D-Bus connection with the real state machine behind it: identifiers,
//! replacements, closes and their reasons, expiry, at-most-once actions, do not
//! disturb, the bounded queue under a hundred calls a second for ten seconds,
//! the locked redaction, and the three errors this daemon answers with when it
//! refuses a call. [`lock_state`] follows the compositor's `session_lock_state`
//! over `inox-ipc` (ADR 0022 §4) and is what tells the daemon to withhold.
//!
//! Not delivered, and still not claimed:
//!
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
//! - **The bus name on a real bus.** The service claims
//!   [`protocol::BUS_NAME`] and refuses to start when another daemon owns it, and
//!   the refusal is a unit test of the mapping from `zbus::Error::NameTaken`;
//!   the live case needs a session bus, which the crate's CI shard does not have.
//!   The same shard is why the transport's tests connect peer-to-peer: the wire
//!   protocol, the replies and the signals are the real ones, but a peer-to-peer
//!   connection cannot carry a well-known name.
//! - **A compositor that can be asked for the lock state in CI.** [`lock_state`]
//!   is wired, and what it does with the events is tested, but no test in this
//!   crate binds a real `inox_ipc` global: the fake compositor that would make
//!   that possible does not exist in this workspace, and the compositor's own
//!   tests are where the other end is exercised.
//! - **E3 evidence.** Nothing here runs in a session: the popup stack is still an
//!   ordinary window (above), and `tests/bus.rs` is a transport test, not a
//!   desktop.

#![forbid(unsafe_code)]

pub mod actions;
pub mod app;
pub mod bus;
pub mod contract;
pub mod i18n;
pub mod lock_state;
pub mod markup;
pub mod placement;
pub mod policy;
pub mod protocol;
pub mod queue;
pub mod theme;
pub mod view;
pub mod view_model;

pub use crate::actions::{Action, ActionError, ActionId, Actions, Invoked};
pub use crate::app::{Inbox, Undelivered};
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
    ActionButton, ActionEmphasis, OverflowNotice, Popup, PopupStack, StackInput, StackStatus,
};
