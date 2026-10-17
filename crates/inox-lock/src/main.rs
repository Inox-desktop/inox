//! The lock screen binary: logging, configuration, catalogs — and then the reason
//! there is nothing to run.
//!
//! This binary does **not** start a lock screen, and it does not pretend to. ADR
//! 0015 puts two things in front of one:
//!
//! 1. `ext-session-lock-v1` is what a session lock *is*; the Inox compositor does
//!    not advertise it, and this crate does not implement the protocol leg. A
//!    window that looks like a lock screen while the session behind it is
//!    unlocked is worse than no lock screen at all, because a user would believe
//!    it.
//! 2. The authentication boundary — PAM or whatever else, the helper's
//!    privileges, and the handling of the credential — is a human security review
//!    item. ADR 0015 is explicit that a test stub does not count as
//!    authentication, and this crate ships no verification of any kind.
//!
//! So the binary loads the session's configuration, builds the catalogs, says
//! exactly that through Fluent, and exits non-zero. Every acceptance criterion
//! this component owns today is at evidence level E0/E1 (ADR 0018): the state
//! machine, the limiter, the credential buffer, the suspend ordering and the
//! interface are exercised by tests, not by a running session.

use std::rc::Rc;

use anyhow::{Context, Result};
use inox_common::i18n::Arg;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::error;
use tracing_subscriber::EnvFilter;

use inox_lock::i18n;

/// The exit status for "this build cannot lock the session".
///
/// One of `sysexits.h`'s meanings close enough to be worth borrowing: the program
/// cannot do what it was asked to do, and retrying it will not help. It is
/// non-zero, so a session script that starts this component fails loudly rather
/// than concluding the session is locked.
const EXIT_SESSION_LOCK_UNAVAILABLE: i32 = 1;

fn main() -> Result<()> {
    initialise_logging()?;

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the Inox configuration")?;

    // The catalogs are loaded before anything is reported: the one thing this
    // binary has to say is for a person, and a person reads their own language
    // (root AGENTS.md §7.3).
    let localizer =
        Rc::new(i18n::load(&config.locale).context("loading the lock screen's message catalogs")?);

    let reason = localizer.message("lock-start-blocked-reason");
    let blocked = localizer.message_with("lock-start-blocked", &[("reason", Arg::from(reason))]);

    error!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        "the Inox session lock cannot start in this build"
    );
    error!("{blocked}");

    // The message has already gone through the log, in the user's language; the
    // exit status is the part of the contract a session script can act on. It is
    // not an `Err`, because `main` returning one would print the same thing twice.
    std::process::exit(EXIT_SESSION_LOCK_UNAVAILABLE);
}

/// Starts logging, honouring `RUST_LOG`.
///
/// The level defaults to `info` (root AGENTS.md §3.5). This binary logs once, and
/// then exits.
fn initialise_logging() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // `try_init` hands back a boxed error, which `anyhow` cannot attach context
    // to directly; its message already says what went wrong.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init()
        .map_err(|error| anyhow::anyhow!("installing the tracing subscriber: {error}"))
}
