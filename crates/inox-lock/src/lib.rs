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
//! ([`contract`]) and the interface it draws ([`view_model`], [`view`],
//! [`theme`]). The binary — `src/main.rs` — does not pretend otherwise: it loads
//! the configuration and its catalogs, says through Fluent why the lock cannot
//! start, and exits non-zero.
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
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! # What is verified, and what is not
//!
//! The crate is delivered at evidence level **E0/E1** (ADR 0018): it compiles,
//! lints, documents itself, and its pure logic — the state machine, the limiter,
//! the credential buffer, the suspend ordering, the projection — is covered by
//! unit and property tests, with `en-US`/`zh-CN` catalog parity asserted by
//! [`inox_common::i18n::compare_catalogs`]. `tests/render.rs` draws the real
//! widget tree through the same tiny-skia renderer the binary uses and asserts on
//! pixels, with no compositor involved.
//!
//! Not delivered, not claimed, and each for a stated reason:
//!
//! - **The protocol leg.** Nothing here speaks `ext-session-lock-v1`. The
//!   compositor does not advertise the global yet, and the protocol's lifecycle
//!   rules ("a client that dies does not unlock the session", "an output added
//!   while locked is covered before it is shown") must be implemented and tested
//!   against a locking compositor, not against this crate's model of them.
//! - **Authentication.** There is no PAM call, no helper, no privileged process
//!   and no password verification of any kind. [`contract::AuthOutcome`] is the
//!   shape an answer will arrive in; who is allowed to produce it is the review
//!   item ADR 0015 names.
//! - **A surface.** `main.rs` never creates one, so there is no lock frame on any
//!   output. [`view_model`] and [`view`] are drawn by the test harness only.
//! - **The semantic tree** Q01 asks for: iced 0.14 has no AT-SPI bridge, so the
//!   role/name/state/action export is blocked on that prototype.
//!
//! [ADR 0015]: https://github.com/Inox-desktop/inox/blob/main/docs/adr/0015-session-lock-and-failure-boundaries.md

#![forbid(unsafe_code)]

pub mod app;
pub mod attempts;
pub mod contract;
pub mod credential;
pub mod i18n;
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
