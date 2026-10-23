//! The optional greeter refuses real login until authentication has been reviewed.
//!
//! The preview is a QML surface (ADR 0033/0035/0036) whose state machine, design
//! tokens and Fluent catalogs live in safe Rust. The binary is deliberately
//! **bin-only**: cxx-qt 0.10 embeds the bridge's generated initializers in the
//! target that declares the bridge, so a bridge split between a library and a
//! binary in the same package fails to link with `undefined reference to
//! 'cxx_qt_init_*'` (KDAB/cxx-qt #770). For the same reason every test lives in
//! this binary (`#[cfg(test)]` below) instead of in `tests/`: an integration
//! test is a separate target, and the generated C++ archive it links expects the
//! bridge's Rust symbols, which only exist in the binary.

mod app;
mod cxxqt_object;
mod i18n;
// The reply/command half of the state machine is the contract a reviewed
// transport will honour (ADR 0015); the preview cannot produce those events
// yet, so only the unit tests reference them. Keeping them is the point —
// removing them would silently drop the security-review surface this component
// exists to hold.
#[allow(dead_code)]
mod state;
mod view;

#[cfg(test)]
mod headless_tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use inox_common::i18n::Locale;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{debug, error, info, warn};

use crate::cxxqt_object::qml_ready;
use crate::view::View;

/// Resource URL of the bundled preview.
///
/// cxx-qt registers `qml/main.qml` under the module's import path and keeps the
/// file's own relative path as its resource alias, so the module URI
/// `com.inox.greet.qml` (path segments `com/inox/greet/qml`) plus the alias
/// `qml/main.qml` give the double `qml` below. `qml/main.qml` imports
/// `com.inox.greet.qml`, the full module URI.
const MAIN_QML_URL: &str = "qrc:/qt/qml/com/inox/greet/qml/qml/main.qml";

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
struct Options {
    preview: bool,
    self_check: bool,
}

/// Parse the command line.
///
/// `--self-check` is a diagnostic for the headless preview evidence: it loads
/// the QML tree, waits for the engine to instantiate it, and exits with a
/// status instead of entering the event loop. It is only valid together with
/// `--preview` and changes nothing about the default refusal to log in.
///
/// `Ok(None)` means help was requested.
fn parse_arguments(arguments: impl Iterator<Item = String>) -> Result<Option<Options>> {
    let mut preview = false;
    let mut self_check = false;
    for argument in arguments {
        match argument.as_str() {
            "--help" | "-h" => return Ok(None),
            "--preview" => preview = true,
            "--self-check" => self_check = true,
            _ => bail!("unrecognized argument: {argument}"),
        }
    }
    if self_check && !preview {
        bail!("--self-check requires --preview; the greeter does not log in (ADR 0015)");
    }
    Ok(Some(Options {
        preview,
        self_check,
    }))
}

/// Run the greeter with `arguments` as its command line.
///
/// Kept separate from [`main`] so the tests exercise the real argument handling
/// and the real refusal without starting a second process for the non-Qt paths.
fn run(arguments: impl Iterator<Item = String>) -> Result<()> {
    let early = i18n::load(Locale::from_env())?;

    let Some(options) = parse_arguments(arguments)? else {
        info!("{}", early.message("usage"));
        info!("{}", early.message("self-check"));
        return Ok(());
    };

    // The authentication boundary has not passed security review (ADR 0015):
    // without `--preview` the greeter must fail with a readable error.
    if !options.preview {
        error!("{}", early.message("blocked"));
        bail!("authentication boundary has not passed security review (ADR 0015)");
    }

    let config = InoxConfig::load(&ConfigPaths::discover()?, Appearance::System)?;
    let localizer = i18n::load(config.locale.clone())?;
    let view = View::new(config.theme.clone(), &localizer);
    if !View::install(view) {
        warn!("a preview projection was already installed; keeping the first one");
    }

    run_preview(options)
}

fn main() -> Result<()> {
    inox_common::logging::init()?;
    run(std::env::args().skip(1))
}

/// Boot Qt, load the preview and either enter the event loop or self-check it.
fn run_preview(options: Options) -> Result<()> {
    let mut app = QGuiApplication::new();
    if app.as_mut().is_none() {
        bail!(
            "Qt could not create a GUI application; is a display or the offscreen platform available?"
        );
    }

    let mut engine = QQmlApplicationEngine::new();
    if engine.as_mut().is_none() {
        bail!("Qt could not create a QML engine");
    }

    // `load` is asynchronous, so the outcome is observed through
    // `objectCreated`: a null object means the module failed to load. Without
    // this the process would look healthy even when QML never came up.
    let load_state = Arc::new(AtomicU8::new(LOAD_PENDING));
    let observed = Arc::clone(&load_state);
    let guard = engine
        .as_mut()
        .map(|engine| {
            engine.on_object_created(move |_engine, object, url| {
                if object.is_null() {
                    error!(%url, "QML root object could not be created");
                    observed.store(LOAD_FAILED, Ordering::SeqCst);
                } else {
                    debug!(%url, "QML root object created");
                    observed.store(LOAD_READY, Ordering::SeqCst);
                }
            })
        })
        .ok_or_else(|| anyhow::anyhow!("Qt could not create a QML engine"))?;

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(MAIN_QML_URL));
    }

    if options.self_check {
        let ready = wait_for_qml(app.as_mut(), &load_state);
        drop(guard);
        drop(engine);
        drop(app);
        if !ready {
            bail!("the preview QML module did not load (see the Qt warnings above)");
        }
        info!("preview self-check passed");
        return Ok(());
    }

    if let Some(app) = app.as_mut() {
        let status = app.exec();
        drop(guard);
        drop(engine);
        if status != 0 {
            bail!("the QML preview exited with status {status}");
        }
    }
    Ok(())
}

/// Pump the event loop until QML has instantiated its root object.
///
/// This is a bounded startup wait for the `--self-check` diagnostic, not an
/// event loop of its own: Qt's loop stays in charge, and the wait ends as soon
/// as the engine reports the outcome or the deadline passes. `true` means the
/// root object exists *and* the QML tree confirmed it read the Rust projection.
fn wait_for_qml(
    app: Option<std::pin::Pin<&mut QGuiApplication>>,
    load_state: &Arc<AtomicU8>,
) -> bool {
    let Some(app) = app else {
        return false;
    };
    let deadline = Instant::now() + SELF_CHECK_TIMEOUT;
    loop {
        app.process_events();
        match load_state.load(Ordering::SeqCst) {
            LOAD_READY => return qml_ready(),
            LOAD_FAILED => return false,
            _ if Instant::now() >= deadline => return false,
            _ => std::thread::sleep(SELF_CHECK_POLL),
        }
    }
}
