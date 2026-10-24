//! The panel binary: configuration, logging, catalogs, then the panel's layer
//! surface.
//!
//! The binary is deliberately thin. Everything that decides anything lives in
//! the library: [`inox_panel::app`] owns the state, [`inox_panel::placement`] the
//! placement the surface asks for, and [`inox_panel::layer`] the protocol leg
//! that applies it. This file loads the configuration, picks the surface,
//! installs logging, and hands control to the leg.
//!
//! Three things are worth stating before the code, because they are deliberate:
//!
//! - **The surface is a real `wlr-layer-shell` surface.** The panel used to run
//!   as an ordinary `xdg-shell` window with its computed placement unapplied, and
//!   logged a warning saying so (ADR 0018). It no longer does: a compositor that
//!   does not advertise `zwlr_layer_shell_v1` is a **readable error and a
//!   non-zero exit**, because a panel that claims to reserve a strip while it is
//!   really a floating window is a lie about the desktop's geometry.
//! - **Which surface a process draws comes from `[panel] surface`, or from
//!   `--surface`.** The flag overrides the file for this process only, which is
//!   how one session runs a menu bar and a dock at once: they are two processes,
//!   because they are two surfaces with different sizes and different placements.
//! - **The window service is not reached yet.** No transport is implemented in
//!   this milestone, so the panel starts in its connecting state and shows that
//!   state honestly instead of inventing a task list.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result};
use inox_config::{Appearance, ConfigPaths, InoxConfig, PanelSurface};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use inox_panel::i18n;
use inox_panel::layer::{self, Outcome};

/// What this binary accepts on the command line.
///
/// A CLI's own usage text is not a translated string: it names options, not
/// user-facing prose (root AGENTS.md §7.3 covers what a person reads in the
/// interface). It is logged like everything else this binary says.
const USAGE: &str = "\
inox-panel [OPTIONS]

OPTIONS:
    --surface <taskbar|menubar|dock>   Draw this surface instead of the one
                                       `[panel] surface` names. A session that
                                       wants a menu bar and a dock runs two
                                       processes, one per surface.
    -h, --help                         Print this help and exit
";

/// The exit status for a run that could not draw a placed panel.
const EXIT_NO_SURFACE: i32 = 1;

/// The exit status for a command line this binary cannot act on.
const EXIT_BAD_ARGUMENTS: i32 = 2;

fn main() -> Result<()> {
    initialise_logging()?;

    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(reason) => {
            error!("{reason}");
            error!("{USAGE}");

            std::process::exit(EXIT_BAD_ARGUMENTS);
        }
    };

    if options.help {
        info!("{USAGE}");

        return Ok(());
    }

    // ADR 0018 asks a component not to hand control to a third-party crate and
    // harvest an internal panic. Without a display there is nothing to connect
    // to, so the failure is reported here, in this process's own words, with a
    // non-zero exit.
    let display = DisplayEnv::from_process();
    if !display.is_available() {
        anyhow::bail!(
            "no display to draw the panel on: set WAYLAND_DISPLAY (and XDG_RUNTIME_DIR), and \
             start the panel inside a session"
        );
    }

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the Inox configuration")?;

    let localizer =
        Rc::new(i18n::load(&config.locale).context("loading the panel's message catalogs")?);

    // The flag is what lets one session run a menu bar and a dock at once; it
    // overrides the file for this process and nothing else.
    let mut shell = config.shell.clone();
    if let Some(surface) = options.surface {
        shell.panel.surface = surface;
    }

    info!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        surface = ?shell.panel.surface,
        "starting the Inox panel"
    );
    warn!(
        "the window service transport (inox-ipc or D-Bus) is not implemented in this \
         milestone: the panel will report that it is waiting for it"
    );

    match layer::run(shell, config.theme.clone(), localizer, paths) {
        Ok(Outcome::Stopped) => {
            info!("the panel stopped");

            Ok(())
        }
        Ok(Outcome::ConnectionLost) => {
            error!(
                "the compositor connection was lost, so the panel's surface is gone; \
                 stopping without a placed surface"
            );

            std::process::exit(EXIT_NO_SURFACE);
        }
        Err(failure) => {
            // The leg's errors are the ones an operator has to act on — no
            // layer-shell on this compositor, no display, a failed event loop —
            // so they are reported in the panel's own log and the status says the
            // run failed.
            error!("{failure}");

            std::process::exit(EXIT_NO_SURFACE);
        }
    }
}

/// The command line, as data.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Options {
    /// The surface this process draws, overriding `[panel] surface`.
    surface: Option<PanelSurface>,
    /// Whether the usage text was asked for.
    help: bool,
}

impl Options {
    /// Parses the arguments after the program name.
    ///
    /// # Errors
    ///
    /// Returns a readable reason for an argument this binary does not
    /// understand, an unknown surface name, or a `--surface` with nothing after
    /// it — never a silent default, because the surface decides where the panel
    /// lands.
    fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut arguments = arguments.into_iter();

        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--surface" => {
                    let Some(value) = arguments.next() else {
                        return Err("--surface needs a value: taskbar, menubar or dock".to_owned());
                    };

                    options.surface = Some(parse_surface(&value)?);
                }
                "-h" | "--help" => options.help = true,
                other => return Err(format!("unrecognized argument: {other}")),
            }
        }

        Ok(options)
    }
}

/// The surface one spelling names.
///
/// # Errors
///
/// Returns the unknown spelling itself, with the accepted ones, so the message a
/// user sees says what to type instead.
fn parse_surface(value: &str) -> Result<PanelSurface, String> {
    match value {
        "taskbar" => Ok(PanelSurface::Taskbar),
        "menubar" => Ok(PanelSurface::Menubar),
        "dock" => Ok(PanelSurface::Dock),
        other => Err(format!(
            "unknown surface: {other} (expected taskbar, menubar or dock)"
        )),
    }
}

/// Starts logging, honouring `RUST_LOG`.
///
/// The level defaults to `info` (root AGENTS.md §3.5): the panel logs what it
/// started with, what surface it created, and nothing per frame.
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

#[cfg(test)]
mod tests {
    use super::{DisplayEnv, Options, PanelSurface};

    fn env(wayland: Option<&str>, runtime: Option<&str>, x11: Option<&str>) -> DisplayEnv {
        DisplayEnv {
            wayland_display: wayland.map(str::to_owned),
            wayland_socket: false,
            runtime_dir: runtime.map(str::to_owned),
            x11_display: x11.map(str::to_owned),
        }
    }

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
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
    fn every_surface_spelling_is_accepted() {
        for (text, surface) in [
            ("taskbar", PanelSurface::Taskbar),
            ("menubar", PanelSurface::Menubar),
            ("dock", PanelSurface::Dock),
        ] {
            let options = Options::parse(arguments(&["--surface", text])).expect("a surface");

            assert_eq!(options.surface, Some(surface));
            assert!(!options.help);
        }
    }

    #[test]
    fn no_argument_leaves_the_surface_to_the_configuration() {
        let options = Options::parse(arguments(&[])).expect("nothing to parse");

        assert_eq!(options.surface, None, "the file decides by default");
        assert!(!options.help);
    }

    #[test]
    fn the_last_surface_wins_when_one_is_given_twice() {
        let options = Options::parse(arguments(&["--surface", "dock", "--surface", "menubar"]))
            .expect("a surface");

        assert_eq!(options.surface, Some(PanelSurface::Menubar));
    }

    #[test]
    fn an_unknown_surface_is_refused_with_a_readable_reason() {
        let failure =
            Options::parse(arguments(&["--surface", "task"])).expect_err("task is not a surface");

        assert!(failure.contains("task"), "{failure}");
        assert!(
            failure.contains("menubar"),
            "it names what to type: {failure}"
        );
    }

    #[test]
    fn a_surface_flag_without_a_value_is_refused() {
        let failure = Options::parse(arguments(&["--surface"])).expect_err("no value");

        assert!(failure.contains("--surface"), "{failure}");
    }

    #[test]
    fn help_is_accepted_in_both_spellings() {
        assert!(Options::parse(arguments(&["--help"])).expect("help").help);
        assert!(Options::parse(arguments(&["-h"])).expect("help").help);
    }

    #[test]
    fn an_unknown_argument_is_refused() {
        let failure = Options::parse(arguments(&["--takeover"])).expect_err("unknown");

        assert!(failure.contains("--takeover"), "{failure}");
    }
}
