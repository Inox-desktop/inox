//! The panel binary: configuration, logging, catalogs, then the iced application.
//!
//! Two things about how this binary runs today are worth stating before the
//! code, because they are deliberate and are logged at startup rather than
//! implied:
//!
//! - **The surface is an ordinary window.** The panel is meant to be a
//!   `wlr-layer-shell` surface; the compositor does not advertise that protocol
//!   and `iced_layershell` has no release for iced 0.14, so the placement
//!   computed by [`inox_panel::placement`] is not applied. ADR 0018 records the
//!   decision and this binary names it in a warning.
//! - **The window service is not reached yet.** No transport is implemented in
//!   this milestone, so the panel starts in its connecting state and shows that
//!   state honestly instead of inventing a task list.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result};
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use inox_panel::app::{self, State};
use inox_panel::placement::PanelPlacement;
use inox_panel::{i18n, theme};

/// The width the panel's window is opened at while it is an ordinary window.
///
/// A layer surface is sized by the compositor along the edge it is anchored to;
/// a window is not, so the fallback starts at the width the acceptance snapshots
/// use (Q02). It says nothing about the panel's behaviour.
const REFERENCE_WIDTH: f32 = 1280.0;

mod reload;

#[derive(Debug, Clone)]
enum RuntimeMessage {
    App(app::Message),
    Reload(Box<reload::Request>),
}

fn main() -> Result<()> {
    initialise_logging()?;

    // ADR 0018 asks a component not to hand control to a third-party crate and
    // harvest an internal panic. Without a display, iced's window creation walks
    // straight into `winit`, which panics on Linux; this check turns that into a
    // message a user can act on and a non-zero exit.
    let display = DisplayEnv::from_process();
    if !display.is_available() {
        anyhow::bail!(
            "no display to draw the panel on: set WAYLAND_DISPLAY (and XDG_RUNTIME_DIR) or \
             DISPLAY, and start the panel inside a session"
        );
    }

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the Inox configuration")?;
    let reload_host = reload::Host::start(paths.clone())?;

    let localizer =
        Rc::new(i18n::load(&config.locale).context("loading the panel's message catalogs")?);
    info!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        "starting the Inox panel"
    );

    let panel = config.shell.panel.clone();
    let height = panel.height;
    let placement = PanelPlacement::derive(&panel);
    warn!(
        reserved = placement.reserved_strip(),
        autohide = ?placement.autohide,
        "ADR 0018: no wlr-layer-shell host is available, so the panel runs as an \
         ordinary window and its computed placement is not applied"
    );
    warn!(
        "the window service transport (inox-ipc or D-Bus) is not implemented in \
         this milestone: the panel will report that it is waiting for it"
    );

    // iced asks for the initial state through a factory, so the panel's pieces
    // are cloned into the closure rather than moved into a state it cannot
    // build twice.
    let tokens = config.theme.clone();
    let boot = move || {
        (
            State::new(panel.clone(), tokens.clone(), Rc::clone(&localizer)),
            reload_host.task(),
        )
    };

    iced::application(boot, update_runtime, view_runtime)
        .title(app::title)
        .theme(|state: &State| theme::from_tokens(state.tokens()))
        .subscription(|state: &State| app::subscription(state).map(RuntimeMessage::App))
        .window_size((REFERENCE_WIDTH, f32::from(height)))
        .run()
        .context("running the panel")
}

/// Starts logging, honouring `RUST_LOG`.
///
/// The level defaults to `info` (root AGENTS.md §3.5): the panel logs what it
/// started with, why its surface is what it is, and nothing per frame.
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
    /// Reads the panel's environment.
    fn from_process() -> Self {
        let value = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

        Self {
            wayland_display: value("WAYLAND_DISPLAY"),
            wayland_socket: value("WAYLAND_SOCKET").is_some(),
            runtime_dir: value("XDG_RUNTIME_DIR"),
            x11_display: value("DISPLAY"),
        }
    }

    /// Whether a display the panel could draw on is reachable.
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

fn update_runtime(state: &mut State, message: RuntimeMessage) -> iced::Task<RuntimeMessage> {
    match message {
        RuntimeMessage::App(message) => app::update(state, message).map(RuntimeMessage::App),
        RuntimeMessage::Reload(request) => {
            let result = (|| {
                if !request.is_current() {
                    return Err("configuration reload expired".to_owned());
                }
                let old = state.config();
                let next = &request.config.shell.panel;
                if old.position != next.position
                    || old.exclusive != next.exclusive
                    || old.autohide != next.autohide
                    || old.margin != next.margin
                {
                    return Err("panel.position/exclusive/autohide/margin: live placement requires a layer-shell host".into());
                }
                let localizer =
                    Rc::new(i18n::load(&request.config.locale).map_err(|e| e.to_string())?);
                state.apply_configuration(
                    request.config.shell.panel.clone(),
                    request.config.theme.clone(),
                    localizer,
                );
                Ok(())
            })();
            let applied = result.is_ok();
            request.finish(result);
            if applied {
                let size = iced::Size::new(REFERENCE_WIDTH, f32::from(state.config().height));
                iced::window::oldest().then(move |id| match id {
                    Some(id) => iced::window::resize(id, size),
                    None => iced::Task::none(),
                })
            } else {
                iced::Task::none()
            }
        }
    }
}

fn view_runtime(state: &State) -> iced::Element<'_, RuntimeMessage> {
    app::view(state).map(RuntimeMessage::App)
}

#[cfg(test)]
mod tests {
    use super::DisplayEnv;

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
}
