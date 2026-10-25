//! The Inox shell: the coordinator, and the contracts the L3 entries share.
//!
//! `inox-shell` is the L3 entry that draws nothing. PRD §3.1 gives it the
//! shell's main framework and its plugin system, and the interaction contract sharpens that
//! into the two things that can actually be shared between processes: **the
//! desktop's interaction rules** — which entry takes the keyboard, what `Esc`
//! closes, where focus goes back to — and **the state contract** every entry
//! is written against when it talks to a service. The entries themselves (the
//! panel, the launcher, the notification daemon and the settings application)
//! are separate binaries that own their own views; none of them links this
//! crate, because a Cargo dependency between two L3 components is what ADR
//! 0002 rule 1 forbids.
//!
//! That is why this crate has no Iced dependency, no view and no screenshot
//! test: there is nothing here to draw. What it has instead is the part of the
//! shell that *is* testable without a compositor, and the acceptance criteria
//! that name it say exactly that — D01 (navigation, focus and `Esc`), D05
//! (loading, empty, error and retry states) and Q03 (both shipped locales):
//!
//! - [`navigation`] — the entry stack, its return targets and the `Esc` rule,
//!   as a pure state machine including the cases a live session rarely shows:
//!   a window destroyed under an open layer, and a session that restarts.
//! - [`contract`] — snapshots and their generations, the difference between a
//!   command that was sent and one that was applied, request generations and
//!   bounded retries.
//! - [`registry`] — the entries a session knows about, validated on
//!   construction. Third-party plugins are out of scope pending their own ADR.
//! - [`diagnostic`] — why something happened, as a Fluent message id rather
//!   than as a sentence: no component may hard-code user-visible English
//!   (root `AGENTS.md` §7.3).
//! - [`i18n`] — the two catalogs, and the ids this crate uses.
//!
//! ```
//! use inox_shell::navigation::{
//!     Entry, EntryFocus, EntryId, NavigationOutcome, NavigationStack, ReturnTarget,
//!     SessionGeneration, WindowId,
//! };
//!
//! let generation = SessionGeneration::new(1);
//! let window = WindowId::new(42);
//! let mut stack = NavigationStack::new(generation);
//!
//! let launcher = Entry::new(EntryId::Launcher, EntryFocus::Keyboard);
//! stack.open(launcher, Some(ReturnTarget::new(window, generation)));
//!
//! // `Esc` closes one layer and says where focus goes back to.
//! assert_eq!(
//!     stack.close_top(),
//!     NavigationOutcome::Closed {
//!         returned_to: Some(ReturnTarget::new(window, generation)),
//!     },
//! );
//! assert_eq!(stack.close_top(), NavigationOutcome::NothingToClose);
//! ```

#![forbid(unsafe_code)]

pub mod contract;
pub mod diagnostic;
pub mod i18n;
pub mod navigation;
pub mod registry;

pub use crate::contract::{
    CommandResult, RequestGeneration, RetryPolicy, RetryPolicyError, RetrySchedule, ServiceState,
    Snapshot, SnapshotGeneration, SnapshotView,
};
pub use crate::diagnostic::Diagnostic;
pub use crate::navigation::{
    Entry, EntryFocus, EntryId, FallbackReason, NavigationOutcome, NavigationStack, ReturnTarget,
    SessionGeneration, WindowId,
};
pub use crate::registry::{EntryRegistry, EntrySpec, RegistryError};

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
