//! The daemon binary: configuration, logging, catalogs, the transport, then the
//! iced application.
//!
//! This file is also the daemon's **runtime**. [`app::update`] performs no I/O —
//! it queues [`inox_notifications::Outgoing`] items — and [`Runtime::update`]
//! below is what carries them out, exactly as `inox-settings` does. That is why
//! the state machine a user drives is the same one the unit tests drive: there
//! is no second implementation of "answer this call" anywhere.
//!
//! Three things about how this binary runs today are worth stating before the
//! code, because they are deliberate and are logged at startup rather than
//! implied:
//!
//! - **The D-Bus service is real and it starts first.** `notify-send hello`
//!   reaches this daemon; a session that already has a notification daemon makes
//!   this one exit non-zero with a readable reason rather than share the name
//!   (ADR 0022 §5).
//! - **The surface is still an ordinary window.** The popup stack is meant to be
//!   a `wlr-layer-shell` surface; the compositor does not advertise that protocol
//!   and `iced_layershell` has no release for iced 0.14, so the placement
//!   computed by [`inox_notifications::placement`] is not applied. ADR 0018
//!   records the decision and this binary names it in a warning.
//! - **The session lock state is watched, not owned.** The compositor says
//!   whether the session is locked over `inox-ipc`; a session where that cannot
//!   be read is logged as such and the daemon keeps running, because nothing
//!   here is the compositor's supervisor.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result};
use iced::Subscription;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

use inox_notifications::app::{self, Inbox, State};
use inox_notifications::view::StackGeometry;
use inox_notifications::{bus, i18n, lock_state, theme};

fn main() -> Result<()> {
    initialise_logging()?;

    // ADR 0018 asks a component not to hand control to a third-party crate and
    // harvest an internal panic. Without a display, iced's window creation walks
    // straight into `winit`, which panics on Linux; this check turns that into a
    // message a user can act on and a non-zero exit.
    let display = DisplayEnv::from_process();
    if !display.is_available() {
        anyhow::bail!(
            "no display to draw notifications on: set WAYLAND_DISPLAY (and XDG_RUNTIME_DIR) or \
             DISPLAY, and start the daemon inside a session"
        );
    }

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the Inox configuration")?;

    let localizer =
        Rc::new(i18n::load(&config.locale).context("loading the notification message catalogs")?);
    info!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        do_not_disturb = config.notifications.policy.do_not_disturb,
        "starting the Inox notification daemon"
    );

    let notifications = config.notifications.clone();
    let geometry = StackGeometry::new(&config.theme, &notifications.display);
    let (width, height) = geometry.window_size(&notifications.display);
    let placement =
        inox_notifications::placement::NotificationPlacement::derive(&notifications.display);
    warn!(
        corner = ?placement.corner,
        margin = placement.margin,
        width = placement.width,
        keyboard = ?placement.keyboard_interactivity,
        "ADR 0018: no wlr-layer-shell host is available, so the popup stack runs as \
         an ordinary window and its computed placement is not applied"
    );
    // The transport comes before the window. A session that already has a
    // notification daemon, or has no session bus at all, is a reason not to
    // start: a daemon that opened a window and then never answered a call would
    // look healthy while being useless.
    let inbox = Inbox::new();
    let service = bus::Service::serve(inbox.clone())
        .context("starting the org.freedesktop.Notifications service")?;

    // The lock state is the opposite: the compositor is the authority, and a
    // session where this daemon cannot read it is a session the daemon still
    // serves. `watch` says what happened, in a warning and in its status.
    let lock = lock_state::watch(inbox.clone());
    info!(
        tracking = ?lock.tracking(),
        "following the session lock state the compositor publishes"
    );

    let runtime = Runtime { service };

    // iced asks for the initial state through a factory, so the daemon's pieces
    // are cloned into the closure rather than moved into a state it cannot build
    // twice.
    let tokens = config.theme.clone();
    let localizer = Rc::clone(&localizer);
    let boot = move || {
        (
            State::new(notifications.clone(), tokens.clone(), Rc::clone(&localizer)),
            iced::Task::none(),
        )
    };

    iced::application(
        boot,
        move |state: &mut State, message: app::Message| runtime.update(state, message),
        app::view,
    )
    .title(app::title)
    .theme(|state: &State| theme::iced_theme(state.tokens()))
    .subscription(move |state: &State| {
        Subscription::batch([app::subscription(state), inbox.subscription()])
    })
    .window_size((width, height))
    .run()
    .context("running the notification daemon")
}

/// The daemon's runtime: what carries out what the update loop decided.
///
/// The Elm update is [`app::update`], unchanged and unaware of any of this. What
/// the wrapper adds is the one side effect the daemon has — draining
/// `take_outgoing` after every update and handing each item to the transport —
/// which is what keeps "the daemon decided something" and "the wire knows about
/// it" from being two different code paths.
struct Runtime {
    /// The running D-Bus service.
    service: bus::Service,
}

impl Runtime {
    /// The Elm update, plus the side effects it asked for.
    fn update(&self, state: &mut State, message: app::Message) -> iced::Task<app::Message> {
        let task = app::update(state, message);

        for outgoing in state.take_outgoing() {
            self.service.deliver(outgoing);
        }

        task
    }
}

/// Starts logging, honouring `RUST_LOG`.
///
/// The level defaults to `info` (root AGENTS.md §3.5): the daemon logs what it
/// started with, why its surface is what it is, and nothing per notification
/// beyond the debug lines that name what it refused.
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
/// this crate forbids, so the process's own environment cannot be rearranged in a
/// test.
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
