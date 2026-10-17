//! The lock screen binary, in the two modes ADR 0015 and ADR 0022 leave it.
//!
//! **Without a flag**, this binary does not start a lock screen, and it does not
//! pretend to. ADR 0015 makes the authentication boundary — PAM or whatever else,
//! the helper's privileges, the handling of the credential — a human security
//! review item, and says in so many words that a test stub does not count as
//! authentication. This build ships no verification of any kind, so the default
//! path loads the configuration, says exactly that through Fluent, and exits
//! non-zero. A window that looks like a lock screen while the session behind it is
//! unlocked would be worse than no lock screen at all, because a user would
//! believe it.
//!
//! **With `--take-lock`**, it runs the protocol leg: it claims the trusted lock
//! role with the token `inox-session` delivered in `INOX_LOCK_TOKEN`, asks the
//! compositor for the session lock, gives every output a lock surface, and draws
//! the lock screen onto each of them — the crate's real widget tree, rendered
//! offscreen and committed to the surface (ADR 0025). That mode exists so the
//! security-critical half of the component can be exercised against a real
//! compositor, and it is deliberately incomplete:
//!
//! - it has **no authentication and no unlock path**, so entering it means the
//!   session is released by logging out, not by typing a password;
//! - the state is told the truth about that, so what it draws is the screen's
//!   *unavailable* face: a person reading it is told nothing here can check a
//!   password, rather than invited to type one;
//! - it reads no keyboard (ADR 0022 decision 6): with no boundary there is nothing
//!   on the screen to operate;
//! - it never sends `unlock_and_destroy`, not even when it is signalled.
//!
//! Everything above the protocol leg — the state machine, the limiter, the
//! credential buffer and the suspend ordering — is still exercised by tests rather
//! than by a running session, at evidence level E0/E1 (ADR 0018). The interface is
//! no longer in that list: the same `view` the render tests assert on is what the
//! lock surface carries.

use std::rc::Rc;

use anyhow::{Context, Result, bail};
use inox_common::i18n::{Arg, Localizer};
use inox_config::{Appearance, ConfigPaths, InoxConfig, Theme as Tokens};
use inox_lock::i18n::{self};
use inox_lock::{SessionError, SessionOutcome, take_session_lock};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

/// The exit status for "this run did not leave the session locked".
///
/// Non-zero, so a session script that starts this component fails loudly rather
/// than concluding the session is locked.
const EXIT_SESSION_NOT_LOCKED: i32 = 1;

/// The flag that runs the protocol leg (ADR 0022 decision 6).
const TAKE_LOCK: &str = "--take-lock";

/// What this binary accepts.
const USAGE: &str = "\
inox-lock [OPTIONS]

OPTIONS:
    --take-lock    Take the real session lock and hold it, drawing the lock screen
                   on every output. Needs INOX_LOCK_TOKEN, which inox-session
                   delivers. This mode has no authentication and no way to release
                   the lock again: what it draws says so.
    -h, --help     Print this help and exit
";

fn main() -> Result<()> {
    initialise_logging()?;

    let mut take_lock = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            TAKE_LOCK => take_lock = true,
            "-h" | "--help" => {
                // A CLI's own usage text is not a translated string: it names
                // options, not user-facing prose (root AGENTS.md §7.3 covers what a
                // person reads in the interface).
                print!("{USAGE}");

                return Ok(());
            }
            other => bail!("unrecognized argument: {other}"),
        }
    }

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the Inox configuration")?;

    // The catalogs are loaded before anything is reported: what this binary has to
    // say is for a person, and a person reads their own language (root AGENTS.md
    // §7.3).
    let localizer =
        Rc::new(i18n::load(&config.locale).context("loading the lock screen's message catalogs")?);

    if take_lock {
        return run_protocol_leg(localizer, &config.theme);
    }

    let reason = localizer.message("lock-start-blocked-reason");
    let blocked = localizer.message_with("lock-start-blocked", &[("reason", Arg::from(reason))]);

    error!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        "the Inox session lock will not authenticate anyone in this build"
    );
    error!("{blocked}");

    // The message has already gone through the log, in the user's language; the
    // exit status is the part of the contract a session script can act on. It is
    // not an `Err`, because `main` returning one would print the same thing twice.
    std::process::exit(EXIT_SESSION_NOT_LOCKED);
}

/// Take the lock, report what happened, and choose an exit status from it.
fn run_protocol_leg(localizer: Rc<Localizer>, tokens: &Tokens) -> Result<()> {
    let token = std::env::var("INOX_LOCK_TOKEN").ok();

    let outcome = match take_session_lock(token.as_deref(), localizer.clone(), tokens.clone()) {
        Ok(outcome) => outcome,
        Err(failure) => {
            let reason = describe(&failure, &localizer);
            error!(
                "{}",
                localizer.message_with("lock-take-failed", &[("reason", Arg::from(reason))])
            );

            std::process::exit(EXIT_SESSION_NOT_LOCKED);
        }
    };

    match outcome {
        SessionOutcome::StoppedLocked => {
            // Normal for a session that is being logged out, and the one outcome
            // where the messaging matters: the session is *still locked*.
            warn!("{}", localizer.message("lock-stopped-locked"));
        }
        SessionOutcome::Released => info!("{}", localizer.message("lock-released")),
        SessionOutcome::Refused => {
            error!("{}", localizer.message("lock-refused"));
            std::process::exit(EXIT_SESSION_NOT_LOCKED);
        }
        SessionOutcome::StoppedBeforeConfirmation => {
            warn!("{}", localizer.message("lock-stopped-unconfirmed"));
            std::process::exit(EXIT_SESSION_NOT_LOCKED);
        }
    }

    Ok(())
}

/// Say which part of the environment was missing, in the user's language.
fn describe(failure: &SessionError, localizer: &Localizer) -> String {
    match failure {
        SessionError::NoToken => localizer.message("lock-token-missing"),
        SessionError::Connect(_) => localizer.message("lock-no-display"),
        SessionError::MissingGlobal(interface) => localizer.message_with(
            "lock-missing-global",
            &[("interface", Arg::from(*interface))],
        ),
        // The remaining failures are I/O and event-loop errors whose message is
        // already specific (a path, an errno); translating them would lose it.
        other => other.to_string(),
    }
}

/// Starts logging, honouring `RUST_LOG`.
///
/// The level defaults to `info` (root AGENTS.md §3.5).
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
