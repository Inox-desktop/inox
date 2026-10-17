//! The Inox application launcher.

use std::rc::Rc;

use anyhow::{Context, Result};
use iced::Task;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_launcher::app::{self, Message, Prepared, State};
use inox_launcher::placement::{LogicalSize, Placement};
use inox_launcher::view;
use tracing::{info, warn};

/// The output size the placement policy is computed against.
///
/// The launcher cannot ask the compositor for its output's size: that comes
/// from the layer surface the compositor does not offer yet (see the crate
/// documentation). Until it does, the policy is computed against a nominal
/// output and logged, and the window is opened at the size it produces. This is
/// not a design token and not a size the view draws with; when a layer-shell
/// host exists, `main` will pass it the real output and nothing else changes.
const NOMINAL_OUTPUT_WIDTH: u32 = 1920;

/// See [`NOMINAL_OUTPUT_WIDTH`].
const NOMINAL_OUTPUT_HEIGHT: u32 = 1080;

fn main() -> Result<()> {
    inox_common::logging::init().context("installing the tracing subscriber")?;

    let paths = ConfigPaths::discover().context("locating the configuration directory")?;
    let config =
        InoxConfig::load(&paths, Appearance::System).context("loading the configuration")?;

    let localizer =
        Rc::new(inox_launcher::i18n::localizer(&config.locale).context("loading the catalogs")?);

    let placement = Placement::overlay(
        &config.shell.launcher,
        &config.theme,
        LogicalSize::new(NOMINAL_OUTPUT_WIDTH, NOMINAL_OUTPUT_HEIGHT),
        None,
    );

    warn!(
        "the Inox compositor does not advertise wlr-layer-shell yet, so this is an ordinary \
         xdg-shell window and not a layer surface; the placement policy below is computed and \
         reported but not applied by any host (ADR 0018)"
    );
    info!(placement = %placement.describe(), "launcher placement policy");
    info!(
        locale = %config.locale.tag(),
        theme = %config.theme.name,
        index = %localizer.message("launcher-window-title"),
        "inox-launcher starting"
    );

    let prepared = Rc::new(Prepared {
        tokens: config.theme,
        localizer,
        config: config.shell.launcher,
        placement: placement.clone(),
    });

    let size = iced::Size::new(placement.size.width as f32, placement.size.height as f32);

    iced::application(move || boot(&prepared), app::update, view::view)
        .title(app::title)
        .theme(app::theme)
        .subscription(app::subscription)
        .window_size(size)
        .resizable(false)
        .run()
        .context("running the launcher window")
}

/// The boot function: the initial state, and the task that puts the caret in the
/// search field.
///
/// The launcher is a thing you type into, so the field is focused before the
/// first frame: without this the window would open with nothing focused and the
/// first keystroke would go nowhere.
fn boot(prepared: &Prepared) -> (State, Task<Message>) {
    (State::new(prepared), app::focus_search())
}
