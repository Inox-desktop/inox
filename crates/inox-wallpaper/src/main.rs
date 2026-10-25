//! The `inox-wallpaper` binary.
//!
//! Startup is short and deliberately refuses to fail on a bad configuration: the
//! configuration is read, the language negotiated, the first frame is queued, and
//! the window opens. Whatever could not be read becomes a reason the interface
//! shows rather than a process that never appears.
//!
//! # The surface
//!
//! A wallpaper belongs on the background layer of a compositor that speaks
//! `wlr-layer-shell-unstable-v1`. That prerequisite does not exist yet: the
//! compositor does not advertise the protocol, so no layer surface can be
//! created at all. The binary therefore runs as an ordinary window — the preview
//! a human can look at — logs exactly one warning saying so, and claims nothing
//! about placement. The placement policy itself ([`placement`]) is computed and
//! tested regardless.
//!
//! The window is a Qt Quick window: `QGuiApplication` +
//! `QQmlApplicationEngine`, with `qml/Main.qml` as the root. The state machine,
//! the tokens and the Fluent catalogs stay in Rust; QML renders one JSON
//! document ([`view`]) and reports actions back as JSON.
//!
//! # Why the crate is binary-only
//!
//! cxx-qt 0.10 embeds the bridge's generated initializers in the target that
//! declares the bridge, so a bridge split between a library and a binary in the
//! same package fails to link with `undefined reference to 'cxx_qt_init_*'`
//! (KDAB/cxx-qt #770). Every module — and every test — therefore lives in this
//! binary, and `#[cfg(test)]` modules stand in for the `tests/` directory.

// The domain modules below were a library with a binary on top, and they are
// still the tested contract the blocked layer surface and the future
// `inox-ipc` snapshot are written against. A binary target compiles without
// `cfg(test)`, so rustc reports every accessor only the tests exercise as dead
// code. The tests are why those items exist, so the lint is silenced for the
// non-test build and kept for the test build, where a genuine leftover shows.
#![cfg_attr(not(test), allow(dead_code))]

mod app;
mod apply;
mod contract;
mod cxxqt_object;
mod decode;
mod i18n;
mod model;
mod placement;
mod reload;
mod runtime;
mod scaling;
mod source;
mod theme;
mod view;
mod view_model;

#[cfg(test)]
mod headless_tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use inox_config::{
    Appearance, CompositorConfig, ConfigPaths, InoxConfig, LocaleConfig, NotificationsConfig,
    RootConfig, ShellConfig, Theme,
};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::app::{PREVIEW_SIZE, Preview, State};
use crate::model::Unusable;

/// Resource URL of the bundled preview.
///
/// cxx-qt registers `qml/Main.qml` under the module's import path and keeps the
/// file's own relative path as its resource alias, so the module URI
/// `com.inox.wallpaper.qml` (path segments `com/inox/wallpaper/qml`) plus the
/// alias `qml/Main.qml` give the double `qml` below. `qml/Main.qml` imports
/// `com.inox.wallpaper.qml`, the full module URI.
const MAIN_QML_URL: &str = "qrc:/qt/qml/com/inox/wallpaper/qml/qml/Main.qml";

/// How long `--self-check` waits for the tree to report a matching projection.
const SELF_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `--self-check` sleeps between event pumps.
const SELF_CHECK_POLL: Duration = Duration::from_millis(5);

/// `objectCreated` has not fired yet.
const LOAD_PENDING: u8 = 0;
/// The engine instantiated a root object.
const LOAD_READY: u8 = 1;
/// The engine reported a failure, or handed over a null root object.
const LOAD_FAILED: u8 = 2;

/// A reload prepared by [`reload`]: the configuration and the frame it resolves.
type ReloadResource = (Preview, apply::Preparation);

/// What the process was asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Options {
    self_check: bool,
}

/// Parse the command line.
///
/// The component takes no operands. `--self-check` is the diagnostic the
/// headless evidence uses: it loads the QML tree, waits until the tree reports a
/// state that matches the Rust projection, and exits with a status instead of
/// entering the event loop.
///
/// `Ok(None)` means help was requested.
fn parse_arguments(arguments: impl Iterator<Item = String>) -> Result<Option<Options>> {
    let mut self_check = false;
    for argument in arguments {
        match argument.as_str() {
            "--help" | "-h" => return Ok(None),
            "--self-check" => self_check = true,
            _ => bail!("unrecognized argument: {argument}"),
        }
    }

    Ok(Some(Options { self_check }))
}

fn main() -> Result<()> {
    init_logging();

    run(std::env::args().skip(1))
}

/// Run the component with `arguments` as its command line.
///
/// Kept separate from [`main`] so the tests exercise the real argument handling
/// — and the real exit status — without starting a Qt window.
fn run(arguments: impl Iterator<Item = String>) -> Result<()> {
    let Some(options) = parse_arguments(arguments)? else {
        info!("usage: inox-wallpaper [--self-check]");
        info!(
            "--self-check loads the QML preview offscreen, waits until it reports a state that \
             matches the Rust projection, and exits with a status"
        );
        return Ok(());
    };

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let (config, problem) = read_configuration(&paths);
    let wallpaper_dirs: Vec<PathBuf> = inox_common::paths::inox_wallpaper_dirs();
    // Started before Qt so the SIGHUP mask is installed before any thread that
    // could handle the signal exists.
    let reload_host = reload::Host::start(paths.clone(), wallpaper_dirs.clone())?;

    let localizer = std::rc::Rc::new(
        i18n::localizer(config.locale.clone())
            .context("loading the wallpaper's message catalogs")?,
    );
    let preview = Preview::from_config(
        &config.shell.wallpaper,
        paths.root(),
        PREVIEW_SIZE,
        Arc::from(wallpaper_dirs),
    );

    warn!(
        "running as an ordinary window: this compositor does not advertise \
         wlr-layer-shell-unstable-v1, so the background-layer placement is computed and tested \
         but not applied"
    );
    info!(
        output = %preview.output,
        size = %preview.size,
        mode = ?preview.mode,
        wallpaper = ?preview.selection.bundled().map(|wallpaper| wallpaper.id),
        image = ?preview.selection.path(),
        "previewing the wallpaper"
    );

    let state = State::new(Arc::new(config.theme), localizer, preview, problem);
    let Some(reloads) = reload_host.receiver() else {
        bail!("the configuration reload queue was already taken");
    };
    if !runtime::install(runtime::Runtime::start(state, reloads)) {
        warn!("a wallpaper runtime was already installed; keeping the first one");
    }

    run_preview(options)
}

/// Sends the component's logs to the session.
fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

/// Reads the configuration, or says why it could not be read.
///
/// The fallback is exactly the configuration an empty directory produces: the
/// theme and locale the session would have resolved, and every component's
/// defaults. A wallpaper that refused to start over a typo in `shell.toml` would
/// leave the user with no desktop and one log line, which is the worse of the two
/// outcomes; this way the failure is on screen *and* in the log.
fn read_configuration(paths: &ConfigPaths) -> (InoxConfig, Option<Unusable>) {
    match InoxConfig::load(paths, Appearance::System) {
        Ok(config) => (config, None),
        Err(error) => {
            let detail = error.to_string();
            warn!(
                error = %detail,
                "the configuration could not be read; drawing the theme background instead"
            );

            (
                fallback_configuration(),
                Some(Unusable::Unreadable { detail }),
            )
        }
    }
}

/// The configuration an empty configuration directory produces.
///
/// Spelled out rather than borrowed from a store, because this is the one place
/// that has to answer "what does the wallpaper do when it knows nothing", and the
/// answer has to be readable here: nothing is configured, so every output draws
/// the bundled default wallpaper — and, if even that is not installed, the theme
/// background.
fn fallback_configuration() -> InoxConfig {
    InoxConfig {
        root: RootConfig::default(),
        // The built-in dark theme is what `appearance = "system"` resolves to
        // when the session reports no preference of its own.
        theme: Theme::default(),
        compositor: CompositorConfig::default(),
        shell: ShellConfig::default(),
        notifications: NotificationsConfig::default(),
        locale: LocaleConfig::default().resolve(),
    }
}

/// Prepares a reload: the configuration's first output and its frame.
///
/// Called on the reload thread, before the window thread is asked to apply
/// anything, so a reload that cannot decode keeps the previous frame.
fn prepare_reload(
    config: &InoxConfig,
    paths: &ConfigPaths,
    wallpaper_dirs: &[PathBuf],
) -> Result<ReloadResource, String> {
    let preview = Preview::from_config(
        &config.shell.wallpaper,
        paths.root(),
        PREVIEW_SIZE,
        Arc::from(wallpaper_dirs.to_vec()),
    );
    let preparation = apply::prepare(
        preview.output.clone(),
        &preview.selection,
        preview.mode,
        preview.size,
        wallpaper_dirs,
    );
    if let apply::Preparation::Degraded { reason, .. } = &preparation {
        return Err(format!("wallpaper.image: {reason:?}"));
    }
    Ok((preview, preparation))
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
                    info!(%url, "QML root object created");
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
            bail!(
                "the preview did not come up: the QML module failed to load, or the tree it \
                 built does not match the Rust projection (see the warnings above)"
            );
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

/// Pump the event loop until the QML tree has reported a matching projection.
///
/// This is a bounded startup wait for the `--self-check` diagnostic, not an
/// event loop of its own: Qt's loop stays in charge, and the wait ends as soon as
/// the tree reports (through `reportReady`) that it built itself against the
/// current projection, or when the deadline passes. In between, QML's own timers
/// drive [`runtime::Runtime::pump`], which is what lets a decode started at boot
/// finish while this loop waits.
///
/// A created root object is *not* the answer: `objectCreated` fires as soon as the
/// engine has an object, before the tree has read its bindings, decoded the
/// wallpaper or sent its report. So the loop keeps pumping until
/// [`cxxqt_object::qml_ready`] says the report arrived — the wait is what the
/// deadline is for.
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
        if cxxqt_object::qml_ready() {
            return true;
        }
        match load_state.load(Ordering::SeqCst) {
            LOAD_FAILED => return false,
            _ if Instant::now() >= deadline => return false,
            _ => std::thread::sleep(SELF_CHECK_POLL),
        }
    }
}
