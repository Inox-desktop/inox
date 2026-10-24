//! The Inox authentication boundary (ADR 0015, ADR 0029).
//!
//! This crate is the one place in the workspace that may check a password. It has
//! two halves, and they are deliberately separable:
//!
//! - [`wire`] — the `auth-v1` contract a client speaks. It is compiled into the
//!   login screen too, and it holds no PAM, no privilege and no session: a login
//!   *surface* must not contain the code that can start a session on someone's
//!   behalf, so `inox-greet` depends on this crate with `default-features = false`
//!   and only ever sees these types. The normative contract is
//!   `inox-protocols/protocol/auth-v1.md`.
//! - The `server` feature — the privileged half the `inox-auth` binary runs: the
//!   PAM conversation (through the safe `pam` wrapper, so this crate keeps
//!   `#![forbid(unsafe_code)]`), the socket service, the peer-credential check,
//!   the rolling allowance, the logind VT arbitration and the one-shot session
//!   leader that opens the PAM session in its own process and then becomes the
//!   session (ADR 0037 §决策 2 and §决策 5).
//!
//! What this crate is not: it is not a display manager and it draws nothing. It
//! also does not decide *policy* for the desktop — the session program it starts
//! is fixed by its own configuration and never comes off the wire, and nothing a
//! client sends can name a command, an environment variable or a uid.
//!
//! # The trust shape
//!
//! ```text
//!   login screen (greeter uid)                      inox-auth (root)
//!   ─────────────────────────                       ────────────────────────
//!   connect /run/inox/auth.sock
//!   Hello{1} ─────────────────────────────────────► version check
//!                              ◄──────────────────── Welcome{1}
//!   Authenticate{attempt,name,secret} ─────────────► peer uid check
//!                                                    account policy check
//!                                                    rolling allowance
//!                                                    PAM (service = inox-auth)
//!                              ◄──────────────────── Accepted | Rejected | Unavailable
//!   StartSession{attempt} ─────────────────────────► record the active logind session
//!                                                    pick a free VT (logind)
//!                                                    spawn `--session-leader`
//!                              ◄──────────────────── SessionStarted{started}
//!                                                      │ (pipe: "started")
//!                                                      ▼
//!                                            inox-auth --session-leader (root)
//!                                              pam_open_session for the account
//!                                              logind.ActivateSession
//!                                              exec the session program as them
//! ```
//!
//! The session leader is one process per login (ADR 0037 §决策 5): PAM's session
//! stack (`pam_systemd`, `pam_env`, `pam_limits`) registers the logind session for
//! the process that will become the login, so the session and its VT belong to the
//! person rather than to a long-lived helper. The leader never sees a credential —
//! it is told the account name the socket service already verified, and the VT.
//!
//! The socket is root-owned and group-readable, the peer's uid and gid come from
//! `SO_PEERCRED` (never from the message), and a `Secret`'s plaintext leaves the
//! process only as a PAM answer.

#![forbid(unsafe_code)]

pub mod wire;

#[cfg(feature = "server")]
pub mod boundary;
#[cfg(feature = "server")]
pub mod i18n;
#[cfg(feature = "server")]
pub mod leader;
#[cfg(feature = "server")]
pub mod limit;
#[cfg(feature = "server")]
pub mod logind;
#[cfg(feature = "server")]
pub mod pam;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
pub mod session;

#[cfg(feature = "server")]
pub use crate::boundary::{Boundary, BoundaryFactory, Verdict};
#[cfg(feature = "server")]
pub use crate::leader::{LeaderPlan, RunningSession, SessionLauncher};
#[cfg(feature = "server")]
pub use crate::logind::{KnownSession, Logind, SystemLogind, VtRange};
#[cfg(feature = "server")]
pub use crate::server::{Config, Deps, ServerError};
#[cfg(feature = "server")]
pub use crate::session::SessionError;
pub use crate::wire::{Attempt, Reply, Request};

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
