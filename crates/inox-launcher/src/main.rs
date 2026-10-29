//! The Inox application launcher.
//!
//! The launcher is the L3 component that finds an application and starts it. It
//! owns three things, and the split between them is what makes the crate
//! testable without a compositor:
//!
//! | module | responsibility | contains |
//! |--------|----------------|----------|
//! | [`desktop_entry`], [`exec`], [`index`], [`search`] | the domain | the desktop entry grammar, the XDG directory precedence, the ranking |
//! | [`contract`], [`activation`], [`placement`] | the state machine | request generations, at-most-once activation, the layer-shell policy |
//! | [`app`], [`view`], [`view_model`], [`theme`], [`i18n`], [`runtime`] | the shell | `Message` → `update` → projection, catalogs, tokens, tasks |
//!
//! Everything in the first two rows is pure: no Qt, no I/O except where a
//! function says so, and a unit test per rule.
//!
//! ## The window is QML + Qt Quick
//!
//! `main` boots Qt (`QGuiApplication` + `QQmlApplicationEngine`) and loads the
//! bundled `qml/Main.qml`; the tree renders the projection from [`view`] and
//! reports everything it is asked to do through one JSON invokable
//! ([`cxxqt_object`]). The state machine stays in safe Rust. This binary is
//! deliberately **bin-only**: cxx-qt 0.10 embeds the bridge's generated
//! initializers in the target that declares the bridge, so a bridge split between
//! a library and a binary in the same package fails to link with `undefined
//! reference to 'cxx_qt_init_*'` (KDAB/cxx-qt #770). Every test therefore lives in
//! this binary (`#[cfg(test)]`) rather than in `tests/`, which is a separate
//! target.
//!
//! ## What the launcher does not do yet
//!
//! - **It is a window, not a layer surface.** The Inox compositor does not
//!   advertise `zwlr_layer_shell_v1`. [`placement`] computes the policy a
//!   layer-shell host will consume; the binary runs as an ordinary Qt window, says
//!   so at startup, and applies nothing about the layer.
//! - **It verifies a launch by process liveness.** asks for readiness
//!   evidence — a window, a D-Bus name — and neither is reachable from here yet;
//!   see [`activation`]'s module documentation.
//! - **It resolves icons, it does not install them.** [`icons`] looks an entry's
//!   `Icon=` up in the icon directories the session already has (`hicolor` first)
//!   and the tile draws the file; an application whose icon no installed theme
//!   provides falls back to its first letter, so a missing icon is visible as one
//!   rather than silent.
//! - **The semantic tree is the Qt one.** QML marks its controls with
//!   `Accessible.role`/`Accessible.name`, so a screen reader reaches them through
//!   Qt's QPA bridge; the headless test only asserts that the items exist, so
//!   acceptance Q01 is not claimed as verified.
//!
//! ## The opener is the same binary
//!
//! [`open`] is the desktop's `xdg-open`: it opens a file or a link with the
//! application the session has registered for it. It lives here because it is
//! the launcher's own job with a different selection rule — the target picks the
//! application instead of the user — and because it needs the same two modules
//! ([`desktop_entry`], [`exec`]) that this crate already owns. It is reached two
//! ways, both handled **before** Qt is initialised so that opening a link never
//! pays for a GUI toolkit: as `inox-open …` (installed as a link to this
//! binary), and as `inox-launcher --open …` for scripts that would rather not
//! depend on that name. See [`open`] for the steps, the exit codes and the two
//! things it deliberately does not do (portals and content sniffing).
//!
//! ## Why `dead_code` is allowed crate-wide
//!
//! This binary is the crate (see above): what used to be the library's public API
//! is now a set of `mod`s, and rustc's dead-code analysis therefore reports every
//! item that only the in-binary tests — or a future host — reach. Those items are
//! the point: the vocabulary ([`contract`]), the readiness states of
//! [`activation`], the unused layer values of [`placement`] and the test
//! constructors (`State::with_index`, `AppIndex::from_entries`) exist to be
//! asserted on and to be consumed by the layer-shell host when there is one.
//! Marking each of them individually would be a second, silent list to maintain,
//! so the allowance is stated once here instead.

// The crate shape is bin-only (KDAB/cxx-qt#770, docs/qml-migration-contract.md
// §2.1); see the module documentation above for why that makes `dead_code` fire
// on API the tests do use.
#![allow(dead_code)]

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{debug, error, info, warn};

use crate::app::{Prepared, State};
use crate::cxxqt_object::qml_ready;
use crate::placement::{LogicalSize, Placement};
use crate::runtime::Task;

mod activation;
mod app;
mod categories;
mod contract;
mod cxxqt_object;
mod desktop_entry;
mod exec;
mod i18n;
mod icons;
mod index;
mod open;
mod placement;
mod reload;
mod runtime;
mod search;
mod theme;
mod view;
mod view_model;

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod headless_tests;

/// The output size the placement policy is computed against.
///
/// The launcher cannot ask the compositor for its output's size: that comes from
/// the layer surface the compositor does not offer yet (see the crate
/// documentation). Until it does, the policy is computed against a nominal output
/// and logged, and the window is opened at the size it produces. This is not a
/// design token and not a size the view draws with; when a layer-shell host
/// exists, `main` will pass it the real output and nothing else changes.
pub(crate) const NOMINAL_OUTPUT_WIDTH: u32 = 1920;

/// See [`NOMINAL_OUTPUT_WIDTH`].
pub(crate) const NOMINAL_OUTPUT_HEIGHT: u32 = 1080;

/// Resource URL of the bundled QML module.
///
/// cxx-qt registers `qml/Main.qml` under the module's import path and keeps the
/// file's own relative path as its resource alias, so the module URI
/// `com.inox.launcher.qml` (path segments `com/inox/launcher/qml`) plus the alias
/// `qml/Main.qml` give the double `qml` below.
const MAIN_QML_URL: &str = "qrc:/qt/qml/com/inox/launcher/qml/qml/Main.qml";

/// How long the self-check waits for QML to instantiate the root object.
const SELF_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the self-check sleeps between event pumps.
const SELF_CHECK_POLL: Duration = Duration::from_millis(5);

/// `objectCreated` has not fired yet.
const LOAD_PENDING: u8 = 0;
/// The engine instantiated a root object.
const LOAD_READY: u8 = 1;
/// The engine reported a failure, or handed over a null root object.
const LOAD_FAILED: u8 = 2;

/// A message the Qt host acts on.
#[derive(Debug)]
pub(crate) enum RuntimeMessage {
    /// Something the launcher's own state machine handles.
    App(app::Message),
    /// A configuration reload that was prepared off the UI thread.
    Reload(Box<reload::Request>),
}

/// The index the configuration's locale and hidden-entry rule produce.
pub(crate) type ReloadResource = index::AppIndex;

/// Reads the index a reloaded configuration implies.
///
/// # Errors
///
/// Returns the readable reason the XDG directories could not be resolved, which
/// the caller reports as a rejected reload.
pub(crate) fn prepare_reload(
    config: &InoxConfig,
    _paths: &ConfigPaths,
) -> Result<ReloadResource, String> {
    let options =
        index::IndexOptions::from_env(&config.locale, config.shell.launcher.include_hidden)
            .map_err(|e| e.to_string())?;
    Ok(index::scan(&options, 0))
}

/// Everything `main` resolved before the window exists, handed to the QML bridge.
///
/// The bridge object is instantiated *by the QML engine*, so the two sides have to
/// meet somewhere. They meet on this thread-local: booting and loading QML both
/// happen on the thread that runs the Qt event loop, so a thread-local needs no
/// `Send` bound (the state holds an `Rc<Localizer>`) and cannot be read by a
/// worker by mistake.
pub(crate) struct Boot {
    /// The state machine, already booted and focused.
    pub(crate) state: State,
    /// Messages from the worker threads.
    pub(crate) queue: runtime::Queue<RuntimeMessage>,
    /// The SIGHUP / D-Bus reload source.
    pub(crate) host: Option<reload::Host>,
}

thread_local! {
    static BOOT: std::cell::RefCell<Option<Boot>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Hands `boot` to the bridge object the QML engine is about to construct.
///
/// Returns `false` when a boot was already installed: the binary installs exactly
/// one, and a second call is a programming error rather than something to
/// overwrite silently.
pub(crate) fn install_boot(boot: Boot) -> bool {
    BOOT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return false;
        }
        *slot = Some(boot);
        true
    })
}

/// Takes the installed boot, if the binary installed one.
pub(crate) fn take_boot() -> Option<Boot> {
    BOOT.with(|slot| slot.borrow_mut().take())
}

fn main() -> Result<()> {
    let arguments: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let program = arguments.first().cloned();
    let rest = arguments.get(1..).unwrap_or_default();

    if rest
        .first()
        .is_some_and(|arg| arg == std::ffi::OsStr::new("--exec-application"))
    {
        let signals = calloop::signals::Signals::new(&[calloop::signals::Signal::SIGHUP])?;
        drop(signals);
        let mut tail = rest.iter().skip(1);
        let program = tail
            .next()
            .context("application launcher requires a program")?;
        use std::os::unix::process::CommandExt;
        return Err(std::process::Command::new(program).args(tail).exec().into());
    }

    inox_common::logging::init().context("installing the tracing subscriber")?;

    // The opener reports an exit code and exits; it must not build a QGuiApplication
    // first, because a link click has no display to wait for and no window to show.
    if let Some(opener_arguments) = open::opener_arguments(program.as_deref(), rest) {
        std::process::exit(i32::from(open::run_cli(&opener_arguments)));
    }

    let boot = boot()?;
    if !install_boot(boot) {
        warn!("a launcher boot was already installed; keeping the first one");
    }
    run_window(false)
}

/// Resolves the configuration, the catalogs and the index, then the initial state.
///
/// The first index scan happens here rather than in a task: it runs before the
/// window exists, so it blocks nothing, and the launcher's first frame is
/// therefore never an empty list that fills in a moment later.
fn boot() -> Result<Boot> {
    let paths = ConfigPaths::discover().context("locating the configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the configuration")?;
    let host = reload::Host::start(paths.clone())?;

    let localizer = Rc::new(i18n::localizer(&config.locale).context("loading the catalogs")?);
    let placement = Placement::overlay(
        &config.shell.launcher,
        &config.theme,
        LogicalSize::new(NOMINAL_OUTPUT_WIDTH, NOMINAL_OUTPUT_HEIGHT),
        None,
    );

    warn!(
        "the Inox compositor does not advertise wlr-layer-shell yet, so this is an ordinary \
         xdg-shell window and not a layer surface; the placement policy below is computed and \
         reported but not applied by any host"
    );
    info!(placement = %placement.describe(), "launcher placement policy");
    info!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        window = %localizer.message("launcher-window-title"),
        "inox-launcher starting"
    );

    let prepared = Rc::new(Prepared {
        tokens: config.theme,
        localizer,
        config: config.shell.launcher,
        placement,
    });

    let helper = std::env::current_exe().context("locating the application exec helper")?;
    let mut state = State::new(&prepared);
    state.launch_helper = Some(helper);
    // The launcher is a thing you type into, so the caret is asked for before the
    // first frame; QML moves it when the serial the bridge publishes changes.
    app::focus_search(&mut state);

    Ok(Boot {
        state,
        queue: runtime::Queue::new(),
        host: Some(host),
    })
}

/// Applies a prepared reload on the UI thread.
///
/// A reload that is no longer current, or whose catalogs do not load, is rejected
/// and reported as such: the previous configuration stays in force, which is what
/// asks for. The window follows the new placement because the bridge
/// republishes `windowWidth`/`windowHeight` from the state after every message.
pub(crate) fn reload_state(state: &mut State, request: reload::Request) -> Task<RuntimeMessage> {
    let result = (|| {
        if !request.is_current() {
            return Err("configuration reload expired".to_owned());
        }
        let localizer =
            Rc::new(i18n::localizer(&request.config.locale).map_err(|e| e.to_string())?);
        let placement = Placement::overlay(
            &request.config.shell.launcher,
            &request.config.theme,
            LogicalSize::new(NOMINAL_OUTPUT_WIDTH, NOMINAL_OUTPUT_HEIGHT),
            None,
        );
        state.apply_configuration(
            Prepared {
                tokens: request.config.theme.clone(),
                localizer,
                config: request.config.shell.launcher.clone(),
                placement,
            },
            Some(request.resource.clone()),
        );
        Ok(())
    })();

    let applied = result.is_ok();
    request.finish(result);
    if applied {
        info!(
            placement = %state.placement.describe(),
            "launcher placement policy after a configuration reload"
        );
    }
    Task::none()
}

/// Boots Qt, loads the QML module and either runs the window or self-checks it.
///
/// `self_check` is the headless diagnostic the tests spawn this binary for: it
/// loads the tree, waits for the engine to instantiate it and for QML to report
/// that the semantic tree exists, then exits with a status instead of entering the
/// event loop. It changes nothing about what the launcher does.
fn run_window(self_check: bool) -> Result<()> {
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

    // `load` is asynchronous, so the outcome is observed through `objectCreated`:
    // a null object means the module failed to load. Without this the process
    // would look healthy even when QML never came up.
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
        .ok_or_else(|| anyhow!("Qt could not create a QML engine"))?;

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(MAIN_QML_URL));
    }

    if self_check {
        let ready = wait_for_qml(app.as_mut(), &load_state);
        drop(guard);
        drop(engine);
        drop(app);
        if !ready {
            bail!(
                "the launcher window did not come up: the QML module failed to load, or the \
                 tree it built does not match the Rust projection (see the warnings above)"
            );
        }
        info!("launcher QML self-check passed");
        return Ok(());
    }

    let Some(app) = app.as_mut() else {
        bail!("Qt could not create a GUI application");
    };
    let status = app.exec();
    drop(guard);
    drop(engine);
    if status != 0 {
        bail!("the launcher window exited with status {status}");
    }
    Ok(())
}

/// Pump the event loop until QML has instantiated its root object.
///
/// This is a bounded startup wait for the self-check diagnostic, not an event
/// loop of its own: Qt's loop stays in charge, and the wait ends as soon as the
/// engine reports the outcome or the deadline passes. `true` means the root object
/// exists *and* the QML tree confirmed it read the Rust projection.
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
