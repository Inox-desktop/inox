//! The Inox session lock screen: the lock state, its safety invariants, the
//! credential buffer, and the interface that draws them — together with an
//! explicit account of what this crate deliberately does not do.
//!
//! `inox-lock` is the L3 component PRD §3.1 lists as the lock screen, and
//! [ADR 0015] is its specification. That ADR decides two things that shape every
//! module here, and both of them are constraints rather than features:
//!
//! - **A lock is not an overlay.** ADR 0015 rejects a top-level window or a
//!   `wlr-layer-shell` surface as a lock screen, because neither can show that
//!   input and other layers are isolated, or that the session is still covered
//!   when the UI has died. The compositor owns the lock state and `inox-lock` is
//!   the trusted client of `ext-session-lock-v1`. There is therefore no
//!   placement policy in this crate: no anchors, no exclusive zone, no surface
//!   size, because there is no layer surface to place.
//! - **Authentication is a review item, not a milestone artifact.** PAM or any
//!   other authentication mechanism, the helper's privileges and the handling of
//!   credentials are all items ADR 0015 requires a human to sign off before they
//!   land, and Inox must not roll its own password hashing or verification. A
//!   test stub does not count as authentication.
//!
//! So the deliverable is the part that is both in scope and verifiable without
//! either of those: the state machine ([`state`]), its rate limiter
//! ([`attempts`]), the credential buffer ([`credential`]), the prepare-to-suspend
//! ordering ([`suspend`]), the boundaries this screen talks across
//! ([`contract`]), the interface it draws ([`view_model`], [`view`], [`theme`])
//! and the two halves that carry it to a real session ([`session`] and
//! [`render`], ADR 0025).
//!
//! The binary — `src/main.rs` — does not pretend otherwise. With no flag it loads
//! the configuration and its catalogs, says through Fluent why the lock cannot
//! start, and exits non-zero, because a window that looked like a lock screen
//! while the session behind it was unlocked would be worse than no lock screen at
//! all. Under `--take-lock` it takes the real lock and draws the interface onto
//! every output, *truthfully reporting that this build has no authentication
//! boundary* — so what appears is a screen that says it cannot check a password.
//!
//! # The invariants, and where they are enforced
//!
//! | invariant | enforced by |
//! |---|---|
//! | the session is only reported locked once every existing output has a secure frame | [`state::LockMachine::reports_locked`], and the `Locked → Covering → Locked` transition around a new output |
//! | nothing but an authentication success unlocks | [`state::LockMachine::unlock`] is the only method that can enter `Unlocking`, and [`state::LockEvent`] has no success variant |
//! | the UI dying, the boundary failing and a timeout all keep the session locked | `Phase::LockedWithoutUi` is a phase, not an error — see [`state::Phase`] |
//! | a suspend waits for a *confirmed* lock, or is refused | [`suspend::SuspendCoordinator`] |
//! | the credential cannot be printed, and does not survive a hide, a cancel or a suspend | [`credential::Secret`] |
//! | a lock frame is opaque, exactly the configured size, and the projected screen | [`render::Interface`], asserted in `tests/render.rs` and read back from the wire in `tests/protocol.rs` |
//!
//! # How the crate is organised
//!
//! | module | what lives there |
//! |---|---|
//! | [`state`] | the lock state machine and its invariants — pure, fully tested |
//! | [`attempts`] | the failed-attempt limiter — pure |
//! | [`credential`] | the zeroing credential buffer — pure |
//! | [`suspend`] | the prepare-to-suspend ordering — pure |
//! | [`contract`] | the protocol and authentication boundaries (ADR 0013) |
//! | [`view_model`] | the projection from state to what the view shows |
//! | [`app`] | `State`, `Message`, `update` (Elm), `view`, `subscription` |
//! | [`render`] | the widget tree drawn offscreen, into the bytes a lock surface takes |
//! | [`session`] | the `ext-session-lock-v1` client: the requests, and the frames |
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! # What is verified, and what is not
//!
//! The crate is delivered at evidence level **E0/E1** for its pure logic (ADR
//! 0018): it compiles, lints, documents itself, and the state machine, the
//! limiter, the credential buffer, the suspend ordering and the projection are
//! covered by unit and property tests, with `en-US`/`zh-CN` catalog parity asserted
//! by [`inox_common::i18n::compare_catalogs`]. The interface is at **E1+E2**:
//! `tests/render.rs` draws the real widget tree at 1280×720 and asserts on pixels,
//! and `tests/protocol.rs` drives the built binary against a hand-written
//! `ext-session-lock-v1` server and reads back the frame the binary committed to
//! it. The protocol leg is additionally covered at **E3** by the session-level
//! case in `inox-session`, which locks a real headless session and still logs out.
//!
//! Delivered:
//!
//! - **The protocol leg**, under `--take-lock` (ADR 0022 decision 6): this crate
//!   claims the trusted role with the session's token, asks for the lock, gives
//!   every output a lock surface, destroys it when an output goes away, and never
//!   sends `unlock_and_destroy`.
//! - **The interface, drawn onto those surfaces** (ADR 0025): [`render`] lays
//!   [`app::view`] out at the configured size and commits it as the frame — the
//!   same widget tree the render tests assert on, and the same [`app::State`] the
//!   protocol events are fed into.
//!
//! Not delivered, not claimed, and each for a stated reason:
//!
//! - **Authentication.** There is no PAM call, no helper, no privileged process
//!   and no password verification of any kind. [`contract::AuthOutcome`] is the
//!   shape an answer will arrive in; who is allowed to produce it is the review
//!   item ADR 0015 names. The binary therefore tells the state the truth —
//!   [`contract::ServiceFailure::NotProvided`] — and the screen draws its
//!   *unavailable* face: it says it can check nothing, rather than asking for a
//!   password nobody will look at.
//! - **An input path.** No lock surface reads the keyboard (ADR 0022 decision 6,
//!   kept by ADR 0025). With no reviewed boundary there is nothing on the screen
//!   to operate, so the path would be code that can never run; it lands with the
//!   boundary.
//! - **The semantic tree** Q01 asks for: iced 0.14 has no AT-SPI bridge, so the
//!   role/name/state/action export is blocked on that prototype.
//! - **Taking over a lock whose UI died.** ADR 0022 decision 2: the compositor
//!   refuses the second client, so the session stays locked until it is logged out.
//!
//! [ADR 0015]: https://github.com/Inox-desktop/inox/blob/main/docs/adr/0015-session-lock-and-failure-boundaries.md

#![forbid(unsafe_code)]

pub mod app;
pub mod attempts;
pub mod contract;
pub mod credential;
pub mod i18n;
pub mod render;
pub mod session;
pub mod state;
pub mod suspend;
pub mod theme;
pub mod view;
pub mod view_model;

pub use crate::app::{Message, OutgoingCommand, State};
pub use crate::attempts::{AttemptError, AttemptLimiter, AttemptPolicy, Gate};
pub use crate::contract::{
    AuthAttempt, AuthFailure, AuthOutcome, BoundaryState, Generation, LockProtocol,
    ProtocolAbsence, ServiceFailure,
};
pub use crate::credential::Secret;
pub use crate::session::{Outcome as SessionOutcome, SessionError, take as take_session_lock};
pub use crate::state::{
    IgnoreReason, LockEvent, LockMachine, LockReason, OutputId, Phase, Transition,
};
pub use crate::suspend::{
    LockFailure, SuspendCoordinator, SuspendDecision, SuspendEvent, SuspendPhase, SuspendPolicy,
    SuspendRefusal,
};
pub use crate::view_model::{Action, Face, FocusTarget, LockViewModel, Projection};
