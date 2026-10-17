//! Session configuration control without restarting consumers.

use anyhow::{Context, Result};
use inox_common::i18n::Arg;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use std::io::{self, Write};
use std::time::Duration;

fn main() -> Result<()> {
    inox_common::logging::init()?;
    let paths = ConfigPaths::discover()?;
    let config = InoxConfig::load(&paths, Appearance::System)?;
    let localizer = inox_settings::i18n::load(&config.locale)?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() != ["reload"] {
        let help = localizer.message("settings-ctl-help");
        writeln!(io::stdout().lock(), "{help}")?;
        anyhow::ensure!(
            args.is_empty() || args == ["--help"] || args == ["-h"],
            "{help}"
        );
        return Ok(());
    }
    let expected = config.snapshot()?;
    let connection = zbus::block_on(
        zbus::connection::Builder::session()
            .method_timeout(Duration::from_secs(6))
            .build(),
    )
    .context("connecting to the session bus")?;
    let mut failed = false;
    let mut reached = 0;
    for destination in [
        "org.inox.Compositor",
        "org.inox.Panel",
        "org.inox.Launcher",
        "org.inox.Wallpaper",
        "org.inox.Notifications",
    ] {
        let result: Result<(u64, String), zbus::Error> = zbus::block_on(async {
            let proxy = zbus::Proxy::new(
                &connection,
                destination,
                "/org/inox/Configuration",
                "org.inox.Configuration1",
            )
            .await?;
            proxy.call("Reload", &(1u64, &expected)).await
        });
        let id = match result {
            Ok((1, snapshot)) if snapshot == expected => {
                reached += 1;
                "settings-ctl-applied"
            }
            Err(zbus::Error::FDO(error))
                if matches!(error.as_ref(), zbus::fdo::Error::ServiceUnknown(_)) =>
            {
                "settings-ctl-absent"
            }
            other => {
                failed = true;
                tracing::warn!(component = destination, result = ?other, "configuration reload was not confirmed");
                "settings-ctl-failed"
            }
        };
        writeln!(
            io::stdout().lock(),
            "{}",
            localizer.message_with(id, &[("component", Arg::from(destination))])
        )?;
    }
    anyhow::ensure!(
        !failed && reached > 0,
        "{}",
        localizer.message("settings-ctl-incomplete")
    );
    Ok(())
}
