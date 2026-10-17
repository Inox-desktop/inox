//! The optional greeter refuses real login until authentication has been reviewed.
//!
//! `--preview` opens the login interface so it can be exercised without one. What
//! it shows is the shipped interface in the state the shipped build is actually
//! in — the authentication boundary is absent, so the screen says so and collects
//! nothing — and it starts no session.
use anyhow::{Result, bail};
use inox_common::i18n::Locale;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_greet::{app, i18n};
use tracing::{error, info};
fn main() -> Result<()> {
    inox_common::logging::init()?;
    let early = i18n::load(&Locale::from_env())?;
    let mut preview = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--help" | "-h" => {
                info!("{}", early.message("greet-usage"));
                return Ok(());
            }
            "--preview" => preview = true,
            _ => bail!("unrecognized argument: {argument}"),
        }
    }
    if !preview {
        error!("{}", early.message("greet-blocked"));
        bail!("authentication boundary has not passed security review (ADR 0015)");
    }
    let config = InoxConfig::load(&ConfigPaths::discover()?, Appearance::System)?;
    let localizer = std::rc::Rc::new(i18n::load(&config.locale)?);
    let tokens = config.theme.clone();
    iced::application(
        move || {
            let state = app::State::new(localizer.clone(), tokens.clone());
            let focus = state.focus_task();
            (state, focus)
        },
        app::update,
        app::view,
    )
    .title(app::State::title)
    .theme(app::State::theme)
    .subscription(app::subscription)
    .run()?;
    Ok(())
}
