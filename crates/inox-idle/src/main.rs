//! Inhibitor-respecting Wayland idle monitoring.
use anyhow::{Context, Result, bail};
use inox_common::i18n::Locale;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use std::time::Duration;
use tracing::{error, info};
fn main() -> Result<()> {
    inox_common::logging::init()?;
    let early = inox_idle::i18n::load(Locale::from_env())?;
    let mut args = std::env::args().skip(1);
    let mut timeout = Duration::from_secs(300);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                info!("{}", early.message("usage"));
                return Ok(());
            }
            "--timeout-seconds" => {
                let seconds: u64 = args
                    .next()
                    .context("--timeout-seconds requires a value")?
                    .parse()?;
                if !(1..=4_294_967).contains(&seconds) {
                    bail!("timeout must be 1..=4294967 seconds");
                }
                timeout = Duration::from_secs(seconds);
            }
            _ => bail!("unrecognized argument: {arg}"),
        }
    }
    let config = InoxConfig::load(&ConfigPaths::discover()?, Appearance::System)?;
    let localizer = inox_idle::i18n::load(config.locale.clone())?;
    if let Err(failure) = inox_idle::runtime::run(timeout, inox_idle::i18n::load(config.locale)?) {
        error!(
            "{}",
            localizer.message_with("failed", &[("reason", failure.to_string().into())])
        );
        return Err(failure.into());
    }
    Ok(())
}
