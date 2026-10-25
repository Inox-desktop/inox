//! Session configuration control without restarting consumers.
//!
//! `inoxctl reload` asks every consumer that can receive a configuration to read
//! it again, and reports what each one answered. A refusal or a missing
//! confirmation is a non-zero exit, because a script that reloads a desktop must
//! be able to tell that it did not happen.
//!
//! # The second binary of the package
//!
//! cxx-qt links the generated initializer library into **every target** of a
//! package (with `+whole-archive`), and the initializers reference Rust symbols
//! that only the target declaring the bridge defines. A second binary that did not
//! contain the bridge would therefore fail to link with
//! `undefined reference to 'cxx_qt_init_*'` (KDAB/cxx-qt #770).
//!
//! This binary resolves that by **compiling the bridge** — the same file the
//! window compiles — together with the two modules the bridge and this tool need.
//! It never installs a runtime and never creates a `QGuiApplication`, so it holds
//! no window and opens no display connection; it only has to contain the symbols
//! the generated archive expects. That is the price of one package holding both
//! binaries, and it is cheaper than splitting the package (see the crate
//! `AGENTS.md`).

// The bridge, compiled for its symbols rather than for its behaviour: the control
// tool never instantiates the QObject. It is the same file the window uses, so the
// two binaries cannot drift.
#[allow(dead_code)]
#[path = "../cxxqt_object.rs"]
mod cxxqt_object;

// The Qt-free contract the bridge is written against, and the catalogs this tool
// prints its own messages from.
#[path = "../i18n.rs"]
mod i18n;
#[allow(dead_code)]
#[path = "../ui_host.rs"]
mod ui_host;

use std::io::{self, Write};
use std::time::Duration;

use anyhow::{Context, Result};
use inox_common::i18n::{Arg, Localizer};
use inox_config::{Appearance, ConfigPaths, InoxConfig};

/// What the command line asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
    /// Print the usage line.
    Help,
    /// Ask the consumers to read the configuration again.
    Reload,
}

/// Reads a command line.
///
/// `reload` is the only thing this tool does, and the usage line is what a user
/// gets for anything else — including the help flags, which are the same thing.
fn request(arguments: &[String]) -> Request {
    match arguments {
        [command] if command == "reload" => Request::Reload,
        _ => Request::Help,
    }
}

/// Prints the usage line.
fn usage(localizer: &Localizer) -> Result<()> {
    writeln!(
        io::stdout().lock(),
        "{}",
        localizer.message("settings-ctl-help")
    )?;

    Ok(())
}

fn main() -> Result<()> {
    inox_common::logging::init()?;
    let paths = ConfigPaths::discover()?;
    let config = InoxConfig::load(&paths, Appearance::System)?;
    let localizer = i18n::load(&config.locale)?;
    let arguments: Vec<String> = std::env::args().skip(1).collect();

    match request(&arguments) {
        Request::Reload => reload(&config, &localizer),
        Request::Help => {
            usage(&localizer)?;
            // Anything that is not `reload` is a usage error once the line has
            // been printed: silently doing nothing would look like success to a
            // script.
            anyhow::ensure!(
                arguments.is_empty() || arguments == ["--help"] || arguments == ["-h"],
                "{}",
                localizer.message("settings-ctl-help")
            );
            Ok(())
        }
    }
}

/// Asks every consumer to read the configuration again, and reports each one.
///
/// A consumer that is not running is reported as absent rather than as a failure;
/// one that refuses, or answers with a different snapshot, fails the command.
fn reload(config: &InoxConfig, localizer: &Localizer) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn reload_is_the_one_command() {
        assert_eq!(request(&arguments(&["reload"])), Request::Reload);
    }

    #[test]
    fn no_arguments_and_the_help_flags_ask_for_the_usage_line() {
        assert_eq!(request(&arguments(&[])), Request::Help);
        assert_eq!(request(&arguments(&["--help"])), Request::Help);
        assert_eq!(request(&arguments(&["-h"])), Request::Help);
    }

    #[test]
    fn anything_else_is_answered_with_the_usage_line() {
        // The caller prints the line and then fails, so a script sees a non-zero
        // status rather than a silent success.
        assert_eq!(request(&arguments(&["restart"])), Request::Help);
        assert_eq!(request(&arguments(&["reload", "now"])), Request::Help);
    }
}
