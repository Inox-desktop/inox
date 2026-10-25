//! The Inox session lock screen: the lock state, its safety invariants, the
//! credential buffer, and the interface that draws them — together with an
//! explicit account of what this crate deliberately does not do.
//!
//! `inox-lock` is the L3 component PRD §3.1 lists as the lock screen, and
//! ADR 0015 is its specification. That ADR decides two things that shape every
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
//!   credentials are all items that require a human to sign off before they
//!   land, and Inox must not roll its own password hashing or verification. A
//!   test stub does not count as authentication.
//!
//! The binary runs in the two modes ADR 0015 and ADR 0022 leave it (ADR 0022
//! decision 6). Without a flag it starts no lock screen at all: it loads the
//! configuration, says through Fluent that this build cannot verify anyone, and
//! exits non-zero — a window that looks like a lock screen over an unlocked
//! session would be worse than none, because a person would believe it. With
//! `--take-lock` it runs the protocol leg: it claims the trusted lock role with
//! the token `inox-session` delivered in `INOX_LOCK_TOKEN`, asks the compositor
//! for the session lock, gives every output a lock surface, and draws the
//! interface onto each of them — the projected screen, rendered by QML
//! offscreen and committed to the surface (ADR 0025, ADR 0038). That mode exists
//! so the security-critical half of the component can be exercised against a
//! real compositor, and it is deliberately incomplete:
//!
//! - it has **no authentication and no unlock path**, so entering it means the
//!   session is released by logging out, not by typing a password;
//! - the state is told the truth about that, so what it draws is the screen's
//!   *unavailable* face: a person reading it is told nothing here can check a
//!   password, rather than invited to type one;
//! - it reads no keyboard (ADR 0022 decision 6): with no boundary there is
//!   nothing on the screen to operate;
//! - it never sends `unlock_and_destroy`, not even when it is signalled.
//!
//! Everything above the protocol leg — the state machine, the limiter, the
//! credential buffer and the suspend ordering — is exercised by tests rather
//! than by a running session, at evidence level E0/E1 (ADR 0018).
//!
//! # This crate is bin-only, on purpose
//!
//! The interface is QML rendered by Qt Quick's software scene graph into an
//! offscreen window, reached through a `#[cxx_qt::bridge]`
//! ([`cxxqt_object`]). cxx-qt 0.10 embeds the bridge's generated initialisers in
//! the target that declares the bridge, so a bridge split between a library and a
//! binary in the same package fails to link with `undefined reference to
//! 'cxx_qt_init_*'` (KDAB/cxx-qt #770). For the same reason every test lives in
//! this binary (`#[cfg(test)]` below) instead of in `tests/`: an integration test
//! is a separate target, and the generated C++ archive it links expects the
//! bridge's Rust symbols. See `docs/qml-migration-contract.md` §2.1.
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
//! | the credential never reaches QML — only the mask and the labels do | [`view::screen`] is given [`view_model::LockViewModel`], which carries the mask's *length*; the `Secret` is not in scope where the JSON is built |
//! | a lock frame is opaque, exactly the configured size, and the projected screen | [`render::Interface`], asserted in the in-binary tests and read back from the wire in the protocol test |
//! | the lock surface, not a Qt window, carries the interface | [`session`] creates it; [`render`] only fills its buffer |
//!
//! # How the crate is organised
//!
//! | module | what lives there |
//! |---|---|
//! | [`state`] | the lock state machine and its invariants — pure, fully tested |
//! | [`attempts`] | the failed-attempt limiter — pure |
//! | [`credential`] | the zeroing credential buffer — pure |
//! | [`suspend`] | the prepare-to-suspend ordering — pure |
//! | [`contract`] | the protocol and authentication boundaries |
//! | [`view_model`] | the projection from state to what the screen shows |
//! | [`app`] | `State`, `Message`, `update` (Elm), `key_binding` |
//! | [`view`] | the projection QML reads: state + tokens → one JSON document |
//! | [`theme`] | design tokens turned into QML colours, sizes and control styles |
//! | [`render`] | the offscreen QML host's frames, into the bytes a lock surface takes |
//! | [`session`] | the `ext-session-lock-v1` client: the requests, and the frames |
//! | [`cxxqt_object`] | the cxx-qt bridge and the C++ offscreen host's Rust face |
//! | [`runtime`] | the crate's minimal `Task` (the iced runtime's replacement) |
//! | [`i18n`] | the `en-US` and `zh-CN` catalogs this component ships |
//!
//! # What is verified, and what is not
//!
//! The crate is delivered at evidence level **E0/E1** for its pure logic: it compiles, lints, documents itself, and the state machine, the
//! limiter, the credential buffer, the suspend ordering and the projection are
//! covered by unit and property tests, with `en-US`/`zh-CN` catalog parity asserted
//! by [`inox_common::i18n::compare_catalogs`]. The interface is at **E1+E2**: the
//! in-binary render tests draw the real QML at the size acceptance Q02 snapshots
//! are taken at and assert on pixels, and the protocol test drives the built
//! binary against a hand-written `ext-session-lock-v1` server and reads back the
//! frame the binary committed to it. The protocol leg is additionally covered at
//! **E3** by the session-level case in `inox-session`, which locks a real headless
//! session and still logs out.
//!
//! Delivered:
//!
//! - **The protocol leg**, under `--take-lock`: this crate
//!   claims the trusted role with the session's token, asks for the lock, gives
//!   every output a lock surface, destroys it when an output goes away, and never
//!   sends `unlock_and_destroy`.
//! - **The interface, drawn onto those surfaces**: [`render`]
//!   projects [`app::State`] into [`view`]'s JSON, the offscreen QML host grabs a
//!   frame of the configured size, and [`session`] commits it — the same frames
//!   the in-binary render tests assert on, and the same [`app::State`] the protocol
//!   events are fed into.
//!
//! Not delivered, not claimed, and each for a stated reason:
//!
//! - **Authentication.** There is no PAM call, no helper, no privileged process
//!   and no password verification of any kind. [`contract::AuthOutcome`] is the
//!   shape an answer will arrive in; who is allowed to produce it is one of the
//!   review items. The binary therefore tells the state the truth —
//!   [`contract::ServiceFailure::NotProvided`] — and the screen draws its
//!   *unavailable* face: it says it can check nothing, rather than asking for a
//!   password nobody will look at.
//! - **An input path.** No lock surface reads the keyboard. With no reviewed
//!   boundary there is nothing on the screen to operate, so the path would be
//!   code that can never run; it lands with the boundary.
//! - **The semantic tree** Q01 asks for: the offscreen platform has no QPA
//!   accessibility bridge, so the role/name/state/action export is recorded as a
//!   gap, not as a solved problem. The QML tree carries `Accessible`
//!   roles and names, and nothing exports them today.
//! - **Taking over a lock whose UI died.** ADR 0025 decision 2: the compositor
//!   refuses the second client, so the session stays locked until it is logged out.

// cxx-qt's `#[cxx_qt::bridge]` macro emits `unsafe extern "C++"` glue with no
// `#[allow]` wrapper, so the generated code cannot compile under the workspace's
// `unsafe_code = "forbid"`. `Cargo.toml` narrows the lint to `allow` for that
// reason; Inox-authored code in this crate still contains no `unsafe`.
//
// The domain modules expose rules the binary does not exercise yet: the reviewed
// authentication boundary, the suspend transport and the unlock path are human
// security review items. Their tests are the specification until then, so the
// API is kept and this allowance keeps `cargo check` from calling the
// specification dead.
#![allow(dead_code)]

mod app;
mod attempts;
mod contract;
mod credential;
mod cxxqt_object;
mod i18n;
mod render;
mod runtime;
mod session;
mod state;
mod suspend;
mod theme;
mod view;
mod view_model;

#[cfg(test)]
mod headless_tests;

use std::rc::Rc;

use anyhow::{Context, Result, bail};
use inox_common::i18n::{Arg, Localizer};
use inox_config::{Appearance, ConfigPaths, InoxConfig, Theme as Tokens};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::i18n as catalogs;
use crate::session::{Outcome as SessionOutcome, SessionError, take as take_session_lock};

/// The exit status for "this run did not leave the session locked".
///
/// Non-zero, so a session script that starts this component fails loudly rather
/// than concluding the session is locked.
const EXIT_SESSION_NOT_LOCKED: i32 = 1;

/// The flag that runs the protocol leg.
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
    run(std::env::args().skip(1))
}

/// Run the lock screen with `arguments` as its command line.
///
/// Kept separate from [`main`] so the tests exercise the real argument handling,
/// the real refusal and the real protocol leg without a second binary target —
/// which this package cannot have (see the crate docs).
fn run(arguments: impl Iterator<Item = String>) -> Result<()> {
    initialise_logging()?;

    let mut take_lock = false;
    for argument in arguments {
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
    let localizer = Rc::new(
        catalogs::load(&config.locale).context("loading the lock screen's message catalogs")?,
    );

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
    // to directly; its message already says what went wrong. A second call — the
    // protocol tests spawn this binary more than once in one process tree — keeps
    // the subscriber that is already installed rather than failing the run.
    if let Err(error) = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init()
    {
        tracing::debug!(%error, "keeping the tracing subscriber that is already installed");
    }

    Ok(())
}
