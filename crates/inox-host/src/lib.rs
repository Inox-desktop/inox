//! The greeter host: the small process that puts the login chain on the greeter's VT.
//!
//! ADR 0037 gives this L4 component one job — **orchestration** — and the modules here
//! are that job and nothing else:
//!
//! | module | what lives there |
//! |---|---|
//! | [`supervisor`] | the orchestration rules as a pure state machine: when to start, stop and restart, and how a failure ends the stage |
//! | [`process`] | the thin adapter that owns the two child processes and turns signals into process-group operations |
//! | [`runtime`] | the calloop loop that observes and executes, plus the one boundary probe from ADR 0037 §决策 3.1 |
//! | [`cli`] | the command line, parsed without touching the system |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs every sentence in the journal comes from |
//!
//! Three boundaries are constraints rather than features, and they shape all of it.
//!
//! - **It does not authenticate anybody.** The host has no `libpam`, no credential and
//!   no session-starting code; the only thing it knows about the authentication
//!   boundary is whether a `connect` to its socket succeeds (ADR 0037 §决策 1). It does
//!   not even depend on `inox-auth`'s types.
//! - **It does not start the user's session.** The user session belongs to `inox-auth`
//!   and its session leader (ADR 0037 §决策 5), on another VT; this host sees nothing
//!   between its own VT being paused and resumed, and needs to see nothing. Its world
//!   ends when the login screen's stage does.
//! - **It does not touch the VT or the devices.** No ioctl, no DRM fd: libseat and
//!   logind hand device access to the compositor, which pauses and resumes itself when
//!   the VT changes (ADR 0037 §决策 1). The host only spawns, waits and signals.
//!
//! The stage it owns is exactly two processes — `inox-compositor --socket
//! wayland-inox-greet` and `inox-greet` — and three ways of ending: systemd's
//! `SIGTERM`, the compositor leaving, or the login screen spending its restart budget.
//! All three are decided in [`supervisor`] and can be tested there without a process.

#![forbid(unsafe_code)]

pub mod cli;
pub mod i18n;
pub mod process;
pub mod runtime;
pub mod supervisor;

pub use crate::cli::{CliError, Invocation, Options};
pub use crate::process::CommandSpec;
pub use crate::runtime::boundary_is_reachable;
pub use crate::supervisor::{Action, Exit, Plan, Supervisor, Target};
