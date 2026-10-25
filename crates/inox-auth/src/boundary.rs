//! The seam between the socket service and whatever actually authenticates.
//!
//! The server is written against these two traits and never against PAM, which is
//! what makes every rule the service enforces — one attempt at a time, the rolling
//! allowance, "an answer only for the attempt in hand", "a session only for an
//! authenticated account" — testable without a password, a root account or a
//! `pam.d` file. It is also the *only* thing the tests replace: PAM is a system
//! boundary (root AGENTS.md §6 allows stubbing exactly there: DRM, input devices,
//! D-Bus, and a system authenticator), and replacement happens at the factory, not
//! inside the state machine the tests are about.
//!
//! Since the boot login chain the seam is narrower than it used to be: a boundary only
//! *authenticates*. Opening the PAM session is the session leader's job
//! (`crate::pam::open_session`), because the session has to be registered for the
//! process that becomes the login, and that process is not this one. So a boundary
//! implementation that keeps a PAM handle open between two calls is no longer
//! needed — and the PAM handle of an attempt is closed as soon as the verdict is
//! known.

use inox_common::secret::Secret;

use crate::wire::UnavailableReason;

/// What the authenticator said about one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The credentials are good. The session may be asked for.
    Accepted,
    /// The credentials are not. Nothing else is disclosed.
    Rejected,
    /// The authenticator could not answer at all.
    Unavailable(UnavailableReason),
}

/// An authenticator with a lifetime: created per attempt, dropped when the
/// attempt ends.
///
/// Dropping it must end the PAM conversation — the `pam` crate does that in its own
/// `Drop` — so an implementation that holds a handle needs no extra code for the
/// abandoned-attempt case.
pub trait Boundary {
    /// Checks `credential` for `username`.
    ///
    /// Must not keep `credential` after it returns: the server drops its own copy
    /// the moment the verdict arrives, and a boundary that kept a third copy would
    /// silently extend the credential's life past the attempt.
    fn authenticate(&mut self, username: &str, credential: &Secret) -> Verdict;
}

/// How the server gets a boundary for one attempt.
///
/// A factory rather than a single object because a PAM handle is per-attempt: an
/// abandoned attempt must not leave state behind that the next one inherits, and
/// the type that owns a handle is not `Send` (it holds PAM's conversation
/// pointer), so it has to be built on the thread that uses it.
pub trait BoundaryFactory: Send {
    /// A freshly created boundary, with no attempt in flight.
    fn create(&self) -> Box<dyn Boundary>;
}
