//! The settings window: logging, configuration, the Qt Quick host.
//!
//! This file is the crate root, and the window's **entry point** — the crate is
//! bin-only on purpose. cxx-qt 0.10 embeds the bridge's generated initializers in
//! the target that declares the bridge, so a bridge split between a library and a
//! binary in the same package fails to link with `undefined reference to
//! 'cxx_qt_init_*'` (KDAB/cxx-qt #770, `docs/qml-migration-contract.md` §2.1).
//! Everything that used to be a library item lives in this binary's modules, and
//! every test lives in `headless_tests` (`#[cfg(test)]` below) for the same
//! reason: an integration test in `tests/` is a separate target that links the
//! same generated archive but not the bridge's Rust symbols.
//!
//! The runtime itself is [`runtime::UiRuntime`]: it owns the state machine, turns
//! each [`OutgoingCommand`](app::OutgoingCommand) into work on another thread, and
//! answers the QML tree with a projection. This file only boots: logging, the
//! configuration, the catalogs, then Qt.
//!
//! The window is an ordinary Qt window: `QGuiApplication` +
//! `QQmlApplicationEngine`, with a QML `Window` root. There is no Qt Wayland
//! Compositor anywhere in the dependency graph, and no GPL-only Qt module.

// The crate is bin-only (contract §2.1), so every module here is internal to the
// binary and the evidence that drives these items lives in the binary's own
// `#[cfg(test)]` modules: the state machine's accessors, the message variants the
// workflow tests send, and the catalog checklists read as "never used" from the
// binary target, because a `#[cfg(test)]` read does not count as a use of the
// non-test build. `inox-launcher`, `inox-lock` and `inox-notifications` carry the
// same allowance for the same reason after their own migrations.
#![allow(dead_code)]

mod app;
mod apply;
mod collections;
mod cxxqt_object;
mod document;
mod draft;
mod focus;
mod i18n;
mod live;
mod pages;
mod persist;
mod runtime;
mod theme;
mod ui_host;
mod validate;
mod view;
mod view_model;

#[cfg(test)]
mod headless_tests;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use crate::document::Documents;
use crate::runtime::UiRuntime;

/// Resource URL of the bundled window.
///
/// cxx-qt registers `qml/Main.qml` under the module's import path and keeps the
/// file's own relative path as its resource alias, so the module URI
/// `com.inox.settings.qml` (path segments `com/inox/settings/qml`) plus the alias
/// `qml/Main.qml` give the double `qml` below. `qml/Main.qml` imports
/// `com.inox.settings.qml`, the full module URI.
const MAIN_QML_URL: &str = "qrc:/qt/qml/com/inox/settings/qml/qml/Main.qml";

fn main() -> Result<()> {
    initialise_logging()?;
    run()
}

/// Boots the window: configuration, catalogs, runtime, then Qt.
fn run() -> Result<()> {
    // Qt hands an unavailable display straight to its platform plugin, which
    // aborts the process on Linux; this check turns that into a message a user
    // can act on.
    let display = DisplayEnv::from_process();
    if !display.is_available() {
        anyhow::bail!(
            "no display to draw the settings window on: set WAYLAND_DISPLAY (and \
             XDG_RUNTIME_DIR) or DISPLAY, and start Inox from a session"
        );
    }

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the Inox configuration")?;

    boot(&paths, &config, false)
}

/// Builds the runtime, installs it, and runs the Qt host.
///
/// `self_check` loads the tree, waits for it to report ready and returns instead
/// of entering the event loop: it is how the headless test proves that the module
/// loads and that the tree read the projection.
fn boot(paths: &ConfigPaths, config: &InoxConfig, self_check: bool) -> Result<()> {
    let localizer = i18n::load(&config.locale).context("loading the settings message catalogs")?;

    info!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        "starting Inox settings"
    );
    let documents = Documents::open(config, paths)
        .context("reading the configuration files this window can write")?;

    let runtime = UiRuntime::new(config, documents, paths.clone(), localizer);
    if !ui_host::install(Box::new(runtime)) {
        anyhow::bail!("a settings window is already installed on this thread");
    }

    run_window(self_check)
}

/// Boots Qt, loads the QML module and either enters the event loop or checks it.
fn run_window(self_check: bool) -> Result<()> {
    let mut app = QGuiApplication::new();
    if app.as_mut().is_none() {
        anyhow::bail!(
            "Qt could not create a GUI application; is a display or the offscreen platform \
             available?"
        );
    }

    let mut engine = QQmlApplicationEngine::new();
    let Some(engine) = engine.as_mut() else {
        anyhow::bail!("Qt could not create a QML engine");
    };

    engine.load(&QUrl::from(MAIN_QML_URL));

    if self_check {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);

        loop {
            if let Some(app) = app.as_mut() {
                app.process_events();
            }

            if cxxqt_object::qml_ready() {
                return Ok(());
            }

            if std::time::Instant::now() >= deadline {
                anyhow::bail!(
                    "the settings window did not come up: the QML module failed to load, or \
                     the tree it built does not match the projection"
                );
            }

            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    let status = match app.as_mut() {
        Some(app) => app.exec(),
        None => 1,
    };

    if ui_host::with(|host| host.readiness()).is_none() {
        warn!("the window closed after its runtime had already been dropped");
    }

    if status != 0 {
        anyhow::bail!("the settings window exited with status {status}");
    }

    Ok(())
}

/// Starts logging, honouring `RUST_LOG`.
///
/// The level defaults to `info` (root `AGENTS.md` §3.5): the window logs what it
/// started with, what it wrote and what it could not reach, and nothing per
/// frame.
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

/// The environment variables that decide whether there is a display at all.
///
/// Split out from the check so the decision itself is a pure function of the
/// values: edition 2024 makes writing the process environment `unsafe`, which
/// this crate forbids, so the process's own environment cannot be rearranged in
/// a test.
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
    /// Reads the window's environment.
    fn from_process() -> Self {
        let value = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

        Self {
            wayland_display: value("WAYLAND_DISPLAY"),
            wayland_socket: value("WAYLAND_SOCKET").is_some(),
            runtime_dir: value("XDG_RUNTIME_DIR"),
            x11_display: value("DISPLAY"),
        }
    }

    /// Whether a display the window could be drawn on is reachable.
    ///
    /// A Wayland socket is looked for on disk rather than assumed from the
    /// variable, because a stale `WAYLAND_DISPLAY` from a session that has ended
    /// is the common way to get here and "the socket is not there" is the
    /// actionable part of the message. X11 is taken at its word: its socket
    /// lives outside the runtime directory and may be abstract, so there is
    /// nothing portable to look for.
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
    fn the_window_module_is_bundled_under_the_uri_its_qml_imports() {
        assert_eq!(
            MAIN_QML_URL,
            "qrc:/qt/qml/com/inox/settings/qml/qml/Main.qml"
        );
    }
}
