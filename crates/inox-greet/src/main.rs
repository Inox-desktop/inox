//! The login screen, and the two things it refuses to pretend about.
//!
//! `inox-greet` draws the login interface in QML and speaks `auth-v1` to the
//! authentication boundary. It does not authenticate anybody itself, so
//! it has exactly two ways to start:
//!
//! - **No arguments: the real path.** The boundary socket has to be there. If it is
//!   not, the process says so and exits non-zero *before* a window exists — a login
//!   screen that collected a password and only then discovered there was nobody to
//!   check it would waste the one thing a login screen must not waste. The socket is
//!   `/run/inox/auth.sock` unless `INOX_AUTH_SOCKET` names somewhere else, which is
//!   how a development instance and a host that runs the greeter as its own user
//!   both work without a second binary.
//! - **`--preview`: the interface without a boundary.** It opens the same login
//!   screen in the state this build is in without one — unavailable and not
//!   typable — and starts no session. That is not a login path and must never look
//!   like one, which is why the banner in that state says exactly that.
//!
//! What neither path can do is start a session: the boundary does that, it decides
//! the program, and the login screen only ever learns that it happened.
//!
//! Both paths do read the system's accounts ([`accounts`]), because the screen
//! offers the accounts that exist rather than a text box to spell one into. A
//! database that cannot be read is not fatal: the screen says which problem it is
//! and stays up, since a person with a working account should not be refused a login
//! by a greeter that could not list the others.
//!
//! # Why the binary has no library and no `tests/`
//!
//! The package is deliberately **bin-only** (docs/qml-migration-contract.md §2.1).
//! cxx-qt 0.10 embeds the bridge's generated initializers in the target that declares
//! the bridge, so a second target in the same package links the bridge's C++ archive
//! without the Rust symbols it refers to and fails with `undefined reference`
//! (KDAB/cxx-qt#770). Every module therefore hangs off this file, every test is a
//! `#[cfg(test)]` module, and the headless QML check spawns this binary as its child
//! (`--preview --self-check`) so that `QT_QPA_PLATFORM=offscreen` is in the child's
//! environment before Qt starts instead of being mutated in-process, which edition
//! 2024 makes `unsafe`.

mod accounts;
mod app;
mod boundary;
mod cxxqt_object;
mod i18n;
mod runtime;
mod state;
mod theme;
mod view;
mod view_model;

#[cfg(test)]
mod headless_tests;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use inox_auth::wire::DEFAULT_SOCKET;
use inox_common::i18n::Locale;
use inox_common::paths::XdgEnv;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{debug, error, info, warn};

use crate::cxxqt_object::qml_ready;
use crate::runtime::Host;

/// Resource URL of the bundled interface.
///
/// cxx-qt registers `qml/Main.qml` under the module's import path and keeps the
/// file's own relative path as its resource alias, so the module URI
/// `com.inox.greet.qml` (path segments `com/inox/greet/qml`) plus the alias
/// `qml/Main.qml` give the doubled `qml` below. `qml/Main.qml` imports
/// `com.inox.greet.qml`, the full module URI.
const MAIN_QML_URL: &str = "qrc:/qt/qml/com/inox/greet/qml/qml/Main.qml";

/// The environment variable that moves the boundary's socket.
const SOCKET_ENV: &str = "INOX_AUTH_SOCKET";

/// How long `--self-check` waits for QML to instantiate the root object.
const SELF_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `--self-check` sleeps between event pumps.
const SELF_CHECK_POLL: Duration = Duration::from_millis(5);

/// `objectCreated` has not fired yet.
const LOAD_PENDING: u8 = 0;

/// The engine instantiated a root object.
const LOAD_READY: u8 = 1;

/// The engine reported a failure, or handed over a null root object.
const LOAD_FAILED: u8 = 2;

/// What the process was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Open the interface without an authentication boundary.
    pub preview: bool,
    /// Load the interface, confirm it came up, and exit instead of running it.
    pub self_check: bool,
}

/// Parse the command line.
///
/// `--self-check` is a diagnostic for the headless evidence: it loads the QML tree,
/// waits for the engine to instantiate it and for the tree to report that it read
/// the Rust projection, then exits with a status. It is only valid together with
/// `--preview`, because it is a check of the preview interface and not a third way
/// to reach a login.
///
/// `Ok(None)` means help was requested.
fn parse_arguments(arguments: impl Iterator<Item = String>) -> Result<Option<Options>> {
    let mut options = Options {
        preview: false,
        self_check: false,
    };

    for argument in arguments {
        match argument.as_str() {
            "--help" | "-h" => return Ok(None),
            "--preview" => options.preview = true,
            "--self-check" => options.self_check = true,
            _ => bail!("unrecognized argument: {argument}"),
        }
    }

    if options.self_check && !options.preview {
        bail!(
            "--self-check is a diagnostic of the preview interface and requires \
             --preview; it is not a third way to log in"
        );
    }

    Ok(Some(options))
}

/// Run the greeter with `arguments` as its command line.
///
/// Kept separate from [`main`] so the tests exercise the real argument handling and
/// the real refusals without spawning a process for the paths that need no Qt.
pub fn run(arguments: impl Iterator<Item = String>) -> Result<()> {
    let early = i18n::load(&Locale::from_env())?;

    let Some(options) = parse_arguments(arguments)? else {
        info!("{}", early.message("greet-usage"));
        return Ok(());
    };

    let config = InoxConfig::load(&ConfigPaths::discover()?, Appearance::System)?;
    let localizer = Rc::new(i18n::load(&config.locale)?);
    let tokens = config.theme.clone();

    let socket = if options.preview {
        None
    } else {
        let socket = socket_path();
        if !boundary::is_available(&socket) {
            error!("{}", localizer.message("greet-blocked"));
            bail!(
                "no authentication boundary is listening on {} (start inox-auth, or pass \
                 --preview to open the interface without one)",
                socket.display()
            );
        }
        info!(socket = %socket.display(), "the login screen found its authentication boundary");
        Some(socket)
    };

    // The accounts the screen offers, and the record of which one a session was
    // started for last. The record lives in this component's state directory, so a
    // greeter that has never started a session offers the first account the system
    // lists instead.
    let roster = accounts::Accounts::from_system(accounts::record_file(&XdgEnv::from_process()));
    info!(
        accounts = roster.accounts.len(),
        "the system's accounts were read"
    );

    let state = match socket {
        Some(socket) => app::State::connected(localizer, tokens, socket, roster),
        None => app::State::new(localizer, tokens, roster),
    };

    run_interface(options, Host::new(state))
}

fn main() -> Result<()> {
    inox_common::logging::init()?;

    run(std::env::args().skip(1))
}

/// The boundary socket this invocation talks to.
fn socket_path() -> PathBuf {
    std::env::var_os(SOCKET_ENV)
        .map(PathBuf::from)
        .filter(|path: &PathBuf| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOCKET))
}

/// Boot Qt, load the QML module and either run the window or self-check it.
///
/// The state is installed before the module is loaded: the engine creates the
/// Rust-backed element itself, and it takes the login screen from
/// [`crate::runtime`] as it is constructed.
fn run_interface(options: Options, host: Host) -> Result<()> {
    let mut application = QGuiApplication::new();

    if application.as_mut().is_none() {
        bail!(
            "Qt could not create a GUI application; is a display or the offscreen \
             platform available?"
        );
    }

    let mut engine = QQmlApplicationEngine::new();

    if engine.as_mut().is_none() {
        bail!("Qt could not create a QML engine");
    }

    if !runtime::install(host) {
        warn!("a login screen was already installed; keeping the first one");
    }

    // `load` is asynchronous, so the outcome is observed through `objectCreated`: a
    // null object means the module failed to load. Without this the process would
    // look healthy even when QML never came up.
    let load_state = Arc::new(AtomicU8::new(LOAD_PENDING));
    let observed = Arc::clone(&load_state);
    let guard = engine
        .as_mut()
        .map(|engine| {
            engine.on_object_created(move |_engine, object, url| {
                if object.is_null() {
                    error!(%url, "the QML root object could not be created");
                    observed.store(LOAD_FAILED, Ordering::SeqCst);
                } else {
                    debug!(%url, "the QML root object was created");
                    observed.store(LOAD_READY, Ordering::SeqCst);
                }
            })
        })
        .ok_or_else(|| anyhow::anyhow!("Qt could not create a QML engine"))?;

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(MAIN_QML_URL));
    }

    if options.self_check {
        let ready = wait_for_qml(application.as_mut(), &load_state);
        drop(guard);
        drop(engine);
        drop(application);

        if !ready {
            bail!(
                "the login screen did not come up: the QML module failed to load, or \
                 the tree it built does not match the Rust projection (see the \
                 warnings above)"
            );
        }

        info!("the login screen's QML loaded and reported ready");
        return Ok(());
    }

    if let Some(application) = application.as_mut() {
        let status = application.exec();
        drop(guard);
        drop(engine);

        if status != 0 {
            bail!("the QML login screen exited with status {status}");
        }
    }

    Ok(())
}

/// Pump the event loop until QML has instantiated its root object and the tree
/// has confirmed it read the Rust projection.
///
/// This is a bounded startup wait for the `--self-check` diagnostic, not an event
/// loop of its own: Qt's loop stays in charge, and the wait ends when the tree
/// reports through `reportReady`, when the engine reports a failure, or when the
/// deadline passes. A created root object is not the answer — the engine can hand
/// one over before the tree has bound itself — so the loop keeps pumping until
/// `qml_ready()` agrees.
fn wait_for_qml(
    application: Option<std::pin::Pin<&mut QGuiApplication>>,
    load_state: &Arc<AtomicU8>,
) -> bool {
    let Some(application) = application else {
        return false;
    };

    let deadline = Instant::now() + SELF_CHECK_TIMEOUT;

    loop {
        application.process_events();

        if qml_ready() {
            return true;
        }

        match load_state.load(Ordering::SeqCst) {
            LOAD_FAILED => return false,
            _ if Instant::now() >= deadline => return false,
            _ => std::thread::sleep(SELF_CHECK_POLL),
        }
    }
}
