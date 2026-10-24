//! The `inox-wallpaper` binary.
//!
//! Startup is short and deliberately refuses to fail on a bad configuration: the
//! configuration is read, the language negotiated, the first frame is queued, and
//! the window opens. Whatever could not be read becomes a reason the interface
//! shows rather than a process that never appears (ADR 0014, acceptance D05).
//!
//! # Why this is a window and not a background surface
//!
//! A wallpaper belongs on the background layer of a compositor that speaks
//! `wlr-layer-shell-unstable-v1`. Neither prerequisite exists yet: the compositor
//! does not advertise the protocol, and neither `iced_layershell` 0.19.1 nor
//! 0.19.0 compiles against the `iced` generation ADR 0018 fixes, so a layer
//! surface cannot be created at all. The binary therefore runs as an ordinary
//! `xdg-shell` window — the preview a human can look at — logs exactly one
//! warning saying so, and claims nothing about placement. The placement policy
//! itself ([`inox_wallpaper::placement`]) is computed and tested regardless.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context, Result};
use inox_config::{
    Appearance, CompositorConfig, ConfigPaths, InoxConfig, LocaleConfig, NotificationsConfig,
    RootConfig, ShellConfig, Theme,
};
use inox_wallpaper::app::{PREVIEW_SIZE, Preview, State};
use inox_wallpaper::model::Unusable;
use inox_wallpaper::{app, i18n, theme, view};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod reload;

#[derive(Debug, Clone)]
enum RuntimeMessage {
    App(app::Message),
    Reload(Box<reload::Request>),
}
type ReloadResource = (Preview, inox_wallpaper::apply::Preparation);
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
    let preparation = inox_wallpaper::apply::prepare(
        preview.output.clone(),
        &preview.selection,
        preview.mode,
        preview.size,
        wallpaper_dirs,
    );
    if let inox_wallpaper::apply::Preparation::Degraded { reason, .. } = &preparation {
        return Err(format!("wallpaper.image: {reason:?}"));
    }
    Ok((preview, preparation))
}

fn main() -> Result<()> {
    init_logging();

    let paths = ConfigPaths::discover().context("locating the Inox configuration directory")?;
    let (config, problem) = read_configuration(&paths);
    let wallpaper_dirs: Vec<PathBuf> = inox_common::paths::inox_wallpaper_dirs();
    let reload_host = reload::Host::start(paths.clone(), wallpaper_dirs.clone())?;

    let localizer = Rc::new(
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
         wlr-layer-shell-unstable-v1 and iced_layershell does not build against this iced \
         generation, so the background-layer placement is computed and tested but not applied \
         (ADR 0018)"
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

    iced::application(
        move || {
            let mut state = state.clone();
            let task = app::boot(&mut state);

            (
                state,
                iced::Task::batch([task.map(RuntimeMessage::App), reload_host.task()]),
            )
        },
        update_runtime,
        view_runtime,
    )
    .title(view::title)
    .theme(|state: &State| theme::iced_theme(state.theme()))
    .window_size((PREVIEW_SIZE.width as f32, PREVIEW_SIZE.height as f32))
    .run()
    .context("running the wallpaper preview")
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

fn update_runtime(state: &mut State, message: RuntimeMessage) -> iced::Task<RuntimeMessage> {
    match message {
        RuntimeMessage::App(message) => app::update(state, message).map(RuntimeMessage::App),
        RuntimeMessage::Reload(request) => {
            let result = (|| {
                if !request.is_current() {
                    return Err("configuration reload expired".to_owned());
                }
                let localizer = Rc::new(
                    i18n::localizer(request.config.locale.clone()).map_err(|e| e.to_string())?,
                );
                state
                    .apply_configuration(
                        Arc::new(request.config.theme.clone()),
                        localizer,
                        request.resource.0.clone(),
                        request.resource.1.clone(),
                    )
                    .map_err(|e| format!("{e:?}"))
            })();
            request.finish(result);
            iced::Task::none()
        }
    }
}

fn view_runtime(state: &State) -> iced::Element<'_, RuntimeMessage> {
    view::view(state).map(RuntimeMessage::App)
}
