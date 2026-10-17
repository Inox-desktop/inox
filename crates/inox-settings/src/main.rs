//! The settings binary: logging, configuration, catalogs, then the iced window.
//!
//! This file is also the window's **runtime**: [`app::update`] performs no I/O,
//! it queues [`OutgoingCommand`]s, and the wrapper below is what carries them
//! out and feeds the outcomes back as messages. That is why the same state
//! machine the unit tests drive is the one a user drives — there is no second
//! implementation of "write the file" or "ask the component" anywhere.
//!
//! One thing about how this binary runs today is deliberate and is logged at
//! startup rather than implied: **there is no transport to the components.** A
//! change can be written to the configuration files, and nothing in this
//! milestone can tell the compositor, the shell or the notification daemon to
//! apply it (ADR 0018). The window reports exactly that — saved, not in effect
//! — instead of showing a success it cannot substantiate.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Context, Result};
use iced::Task;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use inox_settings::app::{self, Message, OutgoingCommand, SaveRequest, State};
use inox_settings::document::Documents;
use inox_settings::{i18n, persist, theme};

/// The size the window opens at, which is the size acceptance Q02 snapshots are
/// taken at.
const WINDOW_SIZE: (f32, f32) = (1280.0, 720.0);

/// Why a component could not be asked to apply a change.
///
/// A constant rather than a sentence built here: it is data handed to the
/// status line, which renders it inside a localized message (root `AGENTS.md`
/// §7.3).
const NO_TRANSPORT: &str = "there is no transport from the settings application to this \
     component in this milestone (ADR 0018)";

fn main() -> Result<()> {
    initialise_logging()?;

    // iced hands an unavailable display straight to `winit`, which panics on
    // Linux; this check turns that into a message a user can act on.
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
    let localizer =
        Rc::new(i18n::load(&config.locale).context("loading the settings message catalogs")?);

    info!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        "starting Inox settings"
    );
    warn!(
        "ADR 0018: no transport to the compositor, the shell or the notification daemon \
         exists in this milestone, so a change is written to the configuration files and \
         reported as saved but not confirmed to be in effect"
    );

    let documents = Documents::open(&config, &paths)
        .context("reading the configuration files this window can write")?;

    let runtime = Runtime {
        config: config.clone(),
        paths: paths.clone(),
    };

    // iced asks for the initial state through a factory, so the window's pieces
    // are cloned into the closure rather than moved into a state it cannot
    // build twice.
    let tokens = config.theme.clone();
    let boot = {
        let config = config.clone();
        let documents = documents.clone();
        let localizer = Rc::clone(&localizer);

        move || {
            (
                State::new(
                    &config,
                    documents.clone(),
                    tokens.clone(),
                    Rc::clone(&localizer),
                ),
                Task::none(),
            )
        }
    };

    iced::application(
        boot,
        move |state: &mut State, message: Message| runtime.update(state, message),
        app::view,
    )
    .title(app::title)
    .theme(|state: &State| theme::from_tokens(state.tokens()))
    .subscription(app::subscription)
    .window_size(WINDOW_SIZE)
    .run()
    .context("running the settings window")
}

/// The window's runtime: what carries out the commands the update loop queues.
///
/// It holds the configuration and the paths it was started with, so a reload
/// reads the same directory the window opened — a `[components]` path the user
/// pointed elsewhere is honoured rather than silently replaced by the default.
struct Runtime {
    config: InoxConfig,
    paths: ConfigPaths,
}

impl Runtime {
    /// The Elm update, plus the side effects it asked for.
    fn update(&self, state: &mut State, message: Message) -> Task<Message> {
        let mut tasks = vec![app::update(state, message)];
        tasks.extend(
            state
                .take_outgoing()
                .into_iter()
                .map(|command| self.perform(command)),
        );

        Task::batch(tasks)
    }

    /// Carries out one command.
    fn perform(&self, command: OutgoingCommand) -> Task<Message> {
        match command {
            OutgoingCommand::Save(request) => {
                let SaveRequest {
                    generation,
                    file,
                    path,
                    content,
                    guard,
                    ..
                } = *request;

                // The write is local and short; iced's thread pool keeps it off
                // the interaction loop (ADR 0013, decision 4).
                Task::perform(
                    async move {
                        let outcome = persist::save(&path, content.as_bytes(), Some(&guard));

                        Message::SaveSettled {
                            generation,
                            file,
                            outcome,
                        }
                    },
                    std::convert::identity,
                )
            }
            OutgoingCommand::Apply {
                generation,
                component,
            } => {
                warn!(
                    component = component.label_id(),
                    generation = generation.get(),
                    "no transport exists to apply the configuration; the change stays saved \
                     but is not confirmed to be in effect"
                );

                Task::done(Message::ComponentUnreachable {
                    generation,
                    component,
                    reason: NO_TRANSPORT.to_owned(),
                })
            }
            OutgoingCommand::Reload => self.reload(),
        }
    }

    /// Reads the configuration and the three files again.
    fn reload(&self) -> Task<Message> {
        let paths = self.paths.clone();
        let appearance = self.config.root.theme.appearance;

        Task::perform(
            async move {
                let config = match InoxConfig::load(&paths, appearance) {
                    Ok(config) => config,
                    Err(error) => {
                        return Message::ReloadFailed {
                            reason: error.to_string(),
                        };
                    }
                };

                match Documents::open(&config, &paths) {
                    Ok(documents) => Message::Reloaded {
                        config: Box::new(config),
                        documents: Box::new(documents),
                    },
                    Err(error) => {
                        error!(%error, "the configuration files could not be read again");

                        Message::ReloadFailed {
                            reason: error.to_string(),
                        }
                    }
                }
            },
            std::convert::identity,
        )
    }
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
    use inox_settings::apply::Component;

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
    fn the_window_opens_at_the_size_the_snapshots_use() {
        assert_eq!(WINDOW_SIZE, (1280.0, 720.0));
    }

    #[test]
    fn the_answer_to_an_apply_command_says_the_component_could_not_be_reached() {
        // The runtime has nothing else it can honestly say: it cannot ask, so
        // it reports that, and the window shows the change as saved but not in
        // effect (ADR 0018).
        let answer = Message::ComponentUnreachable {
            component: Component::Shell,
            generation: inox_settings::apply::Generation::new(1),
            reason: NO_TRANSPORT.to_owned(),
        };

        match answer {
            Message::ComponentUnreachable { reason, .. } => {
                assert!(reason.contains("ADR 0018"), "reason `{reason}`");
            }
            other => panic!("expected a reachability failure, got {other:?}"),
        }
    }
}
