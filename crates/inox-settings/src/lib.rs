//! The Inox settings application: what a user edits, and what happens to it.
//!
//! `inox-settings` is an L3 component (PRD §3.1) and the one component that
//! *writes* the Inox configuration. Its whole job is the distinction ADR 0014
//! draws: what has been **saved** and what is **in effect** are different
//! things, and a window that shows them as one thing lies to its user.
//!
//! That distinction runs through every module:
//!
//! | module | what lives there |
//! |---|---|
//! | [`draft`] | the values a user is editing, over the ones the files hold — pure, fully tested |
//! | [`mod@validate`] | what a draft has to satisfy before it may be written, as Fluent ids |
//! | [`document`] | the four configuration files as they were read, and the keys a save rewrites |
//! | [`persist`] | the atomic write: temporary file, flush, rename, directory sync, conflict detection |
//! | [`apply`] | the difference between saved and in effect, per component, with generations |
//! | [`pages`] | the settings pages, each a pure description plus its view builder |
//! | [`view_model`] | the projection from state to what the window draws |
//! | [`app`] | `State`, `Message`, `update` (Elm) — no I/O, only [`app::OutgoingCommand`] |
//! | [`view`] | iced widgets, built from the view model and the tokens |
//! | [`theme`] | design tokens turned into iced's theme and styles |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! # The four rules a save obeys
//!
//! 1. **Only what the user changed is written.** [`draft::Draft::changed`] is
//!    the list a save acts on, and [`document::Document::patch`] rewrites the
//!    lines of those keys and copies every other byte through. A preference the
//!    user never opened cannot be lost by saving an unrelated one.
//! 2. **A failed save leaves the old file.** [`persist::save`] writes a
//!    temporary file in the target directory, flushes it, renames it over the
//!    target and syncs the directory; the file in use is never truncated.
//! 3. **An external edit is never overwritten.** The write carries the digest of
//!    the bytes the window read, and a file that no longer has it is reported as
//!    [`persist::SaveOutcome::Conflicted`] instead of being replaced.
//! 4. **Success is never claimed by the window.** [`apply::ApplyState`] reaches
//!    [`apply::ApplyState::Applied`] only when every consumer's readback carries
//!    the generation of the request, and a stale answer changes nothing.
//!
//! # What is verified, and what is not
//!
//! The crate is delivered at evidence level **E0/E1** (ADR 0018): it compiles,
//! lints, documents itself, and its pure logic — the draft, the validator, the
//! document patcher, the atomic write, the apply state machine, the projection
//! — is covered by unit, integration and property tests, with `en-US`/`zh-CN`
//! catalog parity asserted by [`inox_common::i18n::compare_catalogs`].
//! `tests/render.rs` draws the real widget tree through the same tiny-skia
//! renderer the binary uses and asserts on pixels, with no compositor involved.
//!
//! Not delivered, and not claimed:
//!
//! - **The component transport.** There is no way for this component to ask the
//!   compositor, the shell or the notification daemon to apply a new
//!   configuration (ADR 0018: no transport exists in this milestone). The window
//!   therefore reports the change as saved and *not* in effect, through
//!   [`apply::Rejection::Unreachable`], rather than claiming it took.
//! - **Custom keybindings and live display trials.** The output preference editor
//!   saves the existing schema for the next session. Custom keybindings have no
//!   schema or consumer yet; live display trials require server-owned rollback.
//! - **E2/E3 GUI evidence**, which needs a host running an Inox session.

#![forbid(unsafe_code)]

pub mod app;
pub mod apply;
pub mod document;
pub mod draft;
pub mod i18n;
pub mod pages;
pub mod persist;
pub mod theme;
pub mod validate;
pub mod view;
pub mod view_model;

pub use crate::app::{Message, OutgoingCommand, SaveRequest, State};
pub use crate::apply::{Application, ApplyState, Component, ComponentState, Generation, Rejection};
pub use crate::document::{Document, DocumentError, Documents, Edit, SettingFile, Toml};
pub use crate::draft::{
    Control, Draft, DraftError, Field, LocaleChoice, Origin, Range, Settings, Value, ValueKind,
};
pub use crate::pages::PageId;
pub use crate::persist::{Digest, Guard, SaveOutcome};
pub use crate::validate::{FieldError, ValidationMessage, is_valid, validate};
pub use crate::view_model::{
    Action, ControlView, FieldView, FocusTarget, SettingsViewModel, StatusKind, StatusView,
};

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Structured configuration editors and their authoritative schema checks.
pub mod collections;

mod focus;
