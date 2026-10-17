//! The optional greeter refuses real login until authentication has been reviewed.
use anyhow::{Result, bail};
use inox_common::i18n::Locale;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_greet::{app, i18n};
use tracing::{error, info};
fn main() -> Result<()> {
    inox_common::logging::init()?;
    let early = i18n::load(Locale::from_env())?;
    let mut preview = false;
    for argument in std::env::args().skip(1) {
        match argument.as_str() {
            "--help" | "-h" => {
                info!("{}", early.message("usage"));
                return Ok(());
            }
            "--preview" => preview = true,
            _ => bail!("unrecognized argument: {argument}"),
        }
    }
    if !preview {
        error!("{}", early.message("blocked"));
        bail!("authentication boundary has not passed security review (ADR 0015)");
    }
    let config = InoxConfig::load(&ConfigPaths::discover()?, Appearance::System)?;
    let localizer = std::rc::Rc::new(i18n::load(config.locale)?);
    iced::application(
        move || {
            (
                app::State::new(localizer.clone(), config.theme.clone()),
                iced::widget::operation::focus("username"),
            )
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
