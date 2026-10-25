//! The daemon binary: configuration, logging, catalogs, the transports, then the
//! QML window.
//!
//! This file is also the daemon's **host**. Two things about how it runs are
//! worth stating before the code, because they are deliberate and are logged at
//! startup rather than implied:
//!
//! - **The D-Bus service is real and it starts first.** `notify-send hello`
//!   reaches this daemon; a session that already has a notification daemon makes
//!   this one exit non-zero with a readable reason rather than share the name
//!   .
//! - **The surface is an ordinary window.** The popup stack is meant to be a
//!   `wlr-layer-shell` surface; the Inox compositor does not advertise that
//!   protocol, so the placement computed by [`crate::placement`] is data a future
//!   host consumes rather than something applied. The L3 component architecture records the decision
//!   and this binary names it in exactly one warning.
//!
//! # The threading boundary
//!
//! Two kinds of thread exist and they meet in one place:
//!
//! - the **D-Bus transport** ([`crate::bus`], [`crate::lock_state`],
//!   [`crate::reload`]) runs on threads of its own. Those threads never touch
//!   [`crate::app::State`] (it is not `Send`: it holds the `Rc<Localizer>` the
//!   catalogs are read through) and never touch QML.
//! - the **Qt thread** owns `State`, the projection and the QML engine.
//!
//! The only thing that crosses is a message: transports push into
//! [`crate::app::Inbox`], a bounded `std::sync::mpsc` channel, and the Qt thread
//! drains it in `pump()`, which QML drives from a `Timer`. Nothing else touches
//! the interface from another thread — no property assignment, no signal, no
//! engine call leaves the Qt thread (docs/qml-migration-contract.md §3 and §5).

// A binary has no public API, so every accessor only the tests read — the counters
// the bus tests ask the daemon about itself, the settings the window never touches
// — is "never used" as far as `rustc` is concerned. The evidence for those lives in
// `#[cfg(test)] mod bus_tests` / `headless_tests`, which is the only place cxx-qt
// lets a test live in this package (docs/qml-migration-contract.md §2.1). This is
// the accepted cost of a bin-only crate; tagging two dozen accessors one at a time
// would say the same thing less honestly.
#![allow(dead_code)]

mod actions;
mod app;
mod bus;
mod contract;
mod cxxqt_object;
mod i18n;
mod lock_state;
mod markup;
mod placement;
mod policy;
mod protocol;
mod queue;
mod reload;
mod runtime;
mod theme;
mod view;
mod view_model;

#[cfg(test)]
mod bus_tests;
#[cfg(test)]
mod headless_tests;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{debug, error, info, warn};

use crate::app::{Inbox, State};
use crate::cxxqt_object::qml_ready;
use crate::runtime::Runtime;
use crate::view::StackGeometry;

/// Resource URL of the bundled QML module.
///
/// cxx-qt registers `qml/Main.qml` under the module's import path and keeps the
/// file's own relative path as its resource alias, so the module URI
/// `com.inox.notifications.qml` (path segments `com/inox/notifications/qml`) plus
/// the alias `qml/Main.qml` give the double `qml` below. `qml/Main.qml` imports
/// `com.inox.notifications.qml`, the full module URI.
const MAIN_QML_URL: &str = "qrc:/qt/qml/com/inox/notifications/qml/qml/Main.qml";

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
    /// Load the QML surface, wait for it to confirm its tree, and exit.
    ///
    /// This is a diagnostic: it serves no bus, follows no lock state, and runs on
    /// whatever Qt platform the environment selected (the tests use `offscreen`).
    /// It is what the headless evidence for the window is built on.
    self_check: bool,
}

/// Parse the command line.
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
    initialise_logging()?;

    run(std::env::args().skip(1))
}

/// Run the daemon with `arguments` as its command line.
///
/// Kept separate from [`main`] so the tests exercise the real argument handling
/// and the real startup refusals without starting a second process for the paths
/// that do not need Qt.
fn run(arguments: impl Iterator<Item = String>) -> Result<()> {
    let Some(options) = parse_arguments(arguments)? else {
        info!(
            "usage: inox-notifications [--self-check]\n\
             \x20 --self-check load the QML surface, confirm its tree, and exit"
        );

        return Ok(());
    };

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the Inox configuration")?;
    let localizer =
        Rc::new(i18n::load(&config.locale).context("loading the notification message catalogs")?);

    // the L3 component architecture asks a component not to hand control to a third-party crate and
    // harvest an internal panic. Without a display this check turns the failure
    // into a message a user can act on and a non-zero exit; the self-check is the
    // one mode that runs without one, because it is how the surface is verified
    // on a machine that has no session at all.
    if !options.self_check {
        let display = DisplayEnv::from_process();

        if !display.is_available() {
            bail!(
                "no display to draw notifications on: set WAYLAND_DISPLAY (and \
                 XDG_RUNTIME_DIR) or DISPLAY, and start the daemon inside a session"
            );
        }
    }

    let notifications = config.notifications.clone();
    let geometry = StackGeometry::new(&config.theme, &notifications.display);
    info!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        do_not_disturb = notifications.policy.do_not_disturb,
        "starting the Inox notification daemon"
    );
    warn_about_placement(&notifications.display, &geometry);

    let inbox = Inbox::new();
    let state = State::new(notifications, config.theme.clone(), Rc::clone(&localizer));
    let mut runtime = Runtime::new(state, &inbox);

    // The transports come before the window. A session that already has a
    // notification daemon, or has no session bus at all, is a reason not to
    // start: a daemon that opened a window and then never answered a call would
    // look healthy while being useless. The self-check deliberately skips this:
    // it verifies the surface, not the service, and it has to run on a machine
    // with no bus.
    if options.self_check {
        info!(
            "self-check: serving no bus and following no lock state; this process only \
             verifies that the QML surface loads and agrees with the projection"
        );
    } else {
        let service = bus::Service::serve(inbox.clone())
            .context("starting the org.freedesktop.Notifications service")?;
        runtime.attach(service);

        // The lock state is the opposite: the compositor is the authority, and a
        // session where this daemon cannot read it is a session the daemon still
        // serves. `watch` says what happened, in a warning and in its status.
        let lock = lock_state::watch(inbox.clone());
        info!(
            tracking = ?lock.tracking(),
            "following the session lock state the compositor publishes"
        );

        let reload = reload::Host::start(paths.clone())?;
        runtime.attach_reload(reload);
    }

    // The daemon has to be installed before the engine loads: QML constructs the
    // bridge element while loading, and its first property value is the
    // projection the daemon already computed.
    if !cxxqt_object::install(runtime) {
        bail!("a notification daemon was already installed in this process");
    }

    run_window(options)
}

/// Logs the one warning the surface's placement is worth.
///
/// : the popups are an ordinary window, and the placement this component
/// computes is data a future layer-shell host consumes. Saying it once, at
/// startup, is what keeps the code from claiming a placement it does not have.
fn warn_about_placement(display: &inox_config::DisplayConfig, geometry: &StackGeometry) {
    let placement = placement::NotificationPlacement::derive(display);
    let (width, height) = geometry.window_size(display);

    warn!(
        corner = ?placement.corner,
        margin = placement.margin,
        width = placement.width,
        keyboard = ?placement.keyboard_interactivity,
        window_width = width,
        window_height = height,
        "the L3 component architecture: no wlr-layer-shell host is available, so the popup stack runs as \
         an ordinary window and its computed placement is not applied"
    );
}

/// Boot Qt, load the QML module, and either enter the event loop or self-check it.
fn run_window(options: Options) -> Result<()> {
    let mut app = QGuiApplication::new();

    if app.as_mut().is_none() {
        bail!(
            "Qt could not create a GUI application; is a display or the offscreen platform \
             available?"
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
            bail!(
                "the notification surface did not come up: the QML module failed to load, or \
                 the tree it built does not match the Rust projection (see the warnings above)"
            );
        }

        info!("notification surface self-check passed");

        return Ok(());
    }

    if let Some(app) = app.as_mut() {
        let status = app.exec();
        drop(guard);
        drop(engine);

        if status != 0 {
            bail!("the notification surface exited with status {status}");
        }
    }

    Ok(())
}

/// Pump the event loop until QML has instantiated its root object and confirmed
/// the tree it built.
///
/// This is a bounded startup wait for the `--self-check` diagnostic, not an event
/// loop of its own: Qt's loop stays in charge, and the wait ends as soon as the
/// engine reports the outcome or the deadline passes. `true` means the root
/// object exists *and* the QML tree confirmed it read the Rust projection.
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

/// Starts logging, honouring `RUST_LOG`.
///
/// The level defaults to `info` (root AGENTS.md §3.5): the daemon logs what it
/// started with, why its surface is what it is, and nothing per notification
/// beyond the debug lines that name what it refused.
fn initialise_logging() -> Result<()> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // `try_init` hands back a boxed error, which `anyhow` cannot attach context
    // to directly; its message already says what went wrong.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init()
        .map_err(|error| anyhow::anyhow!("installing the tracing subscriber: {error}"))
}

/// The environment variables that decide whether there is a display at all.
///
/// Split out from the check so the decision itself is a pure function of the
/// values: edition 2024 makes writing the process environment `unsafe`, which
/// this crate's code does not do, so the process's own environment cannot be
/// rearranged in a test.
struct DisplayEnv {
    /// `WAYLAND_DISPLAY`, which is a socket name or an absolute path.
    wayland_display: Option<String>,
    /// Whether a socket was handed over through `WAYLAND_SOCKET`.
    wayland_socket: bool,
    /// `XDG_RUNTIME_DIR`, the directory a Wayland socket name is relative to.
    runtime_dir: Option<String>,
    /// `DISPLAY`, for the X11 fallback.
    x11_display: Option<String>,
}

impl DisplayEnv {
    /// Reads the daemon's environment.
    fn from_process() -> Self {
        let value = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

        Self {
            wayland_display: value("WAYLAND_DISPLAY"),
            wayland_socket: value("WAYLAND_SOCKET").is_some(),
            runtime_dir: value("XDG_RUNTIME_DIR"),
            x11_display: value("DISPLAY"),
        }
    }

    /// Whether a display the daemon could draw on is reachable.
    ///
    /// A Wayland socket is looked for on disk rather than assumed from the
    /// variable, because a stale `WAYLAND_DISPLAY` from a session that has ended
    /// is the common way to get here and "the socket is not there" is the
    /// actionable part of the message. X11 is taken at its word: its socket lives
    /// outside the runtime directory and may be abstract, so there is nothing
    /// portable to look for.
    fn is_available(&self) -> bool {
        self.wayland_socket || self.wayland_path().is_some() || self.x11_display.is_some()
    }

    /// The Wayland socket, when one is there.
    fn wayland_path(&self) -> Option<PathBuf> {
        let display = self
            .wayland_display
            .as_deref()
            .filter(|name| !name.is_empty())?;
        let path = Path::new(display);

        let candidate = if path.is_absolute() {
            path.to_owned()
        } else {
            Path::new(self.runtime_dir.as_deref()?).join(display)
        };

        candidate.exists().then_some(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;

    fn env(wayland: Option<&str>, runtime: Option<&str>, x11: Option<&str>) -> DisplayEnv {
        DisplayEnv {
            wayland_display: wayland.map(str::to_owned),
            wayland_socket: false,
            runtime_dir: runtime.map(str::to_owned),
            x11_display: x11.map(str::to_owned),
        }
    }

    #[test]
    fn a_session_with_neither_wayland_nor_x11_has_no_display() {
        assert!(!env(None, None, None).is_available());
        assert!(!env(None, Some("/run/user/1000"), None).is_available());
    }

    #[test]
    fn an_x11_display_is_enough() {
        assert!(env(None, None, Some(":0")).is_available());
    }

    #[test]
    fn a_wayland_socket_name_is_looked_for_in_the_runtime_directory() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-9");
        std::fs::write(&socket, b"").expect("a placeholder for the socket");

        let runtime = directory.path().to_str().expect("a UTF-8 path");
        assert!(env(Some("wayland-9"), Some(runtime), None).is_available());
        assert!(
            !env(Some("wayland-1"), Some(runtime), None).is_available(),
            "a stale variable naming a socket that is not there is not a display"
        );
        assert!(
            !env(Some("wayland-9"), None, None).is_available(),
            "a socket name means nothing without a runtime directory"
        );
        assert!(
            !env(Some(""), Some(runtime), None).is_available(),
            "an empty variable names nothing"
        );
    }

    #[test]
    fn an_absolute_wayland_socket_path_is_used_as_it_is() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let socket = directory.path().join("wayland-1");
        std::fs::write(&socket, b"").expect("a placeholder for the socket");

        let path = socket.to_str().expect("a UTF-8 path");
        assert!(env(Some(path), None, None).is_available());
        assert!(!env(Some("/nonexistent/wayland-1"), None, None).is_available());
    }

    #[test]
    fn a_handed_over_socket_is_trusted() {
        let mut environment = env(None, None, None);
        environment.wayland_socket = true;

        assert!(environment.is_available());
    }

    #[test]
    fn the_argument_line_is_read_or_refused() {
        assert_eq!(
            parse_arguments(std::iter::empty()).expect("no arguments"),
            Some(Options { self_check: false })
        );
        assert_eq!(
            parse_arguments(["--self-check".to_owned()].into_iter()).expect("the diagnostic"),
            Some(Options { self_check: true })
        );
        assert_eq!(
            parse_arguments(["--help".to_owned()].into_iter()).expect("help"),
            None
        );
        assert!(
            parse_arguments(["--bogus".to_owned()].into_iter()).is_err(),
            "an unknown argument is an error, not a silent fallback"
        );
    }

    #[test]
    fn the_qml_url_names_the_bundled_module() {
        assert!(MAIN_QML_URL.starts_with("qrc:/qt/qml/com/inox/notifications/qml/"));
        assert!(MAIN_QML_URL.ends_with("Main.qml"));
    }

    #[test]
    fn a_locale_the_catalogs_do_not_ship_still_starts_the_daemon() {
        // The startup path loads the catalogs for the configured locale; a
        // language Inox has not translated yet must fall back rather than refuse
        // to start (Q03).
        let french = Locale::parse("fr-FR").expect("a well-formed tag");
        let localizer = i18n::load(&french).expect("the catalogs parse");

        assert_eq!(localizer.locale().tag(), "en-US");
    }
}
