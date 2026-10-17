//! The Inox shell coordinator.
//!
//! This binary starts the shell's bookkeeping and nothing else: it loads the
//! configuration, builds the catalog for the session's locale, resolves the
//! entry registry and reports what it found. It deliberately creates no
//! surface of its own — the entries are separate processes that draw
//! themselves, and `inox-shell` is not a window (ADR 0013, ADR 0018).
//!
//! The coordinator is also where the two facts of this milestone that are not
//! finished have to be visible rather than implied, so the startup log says
//! them out loud:
//!
//! - the Inox compositor does not advertise `wlr-layer-shell` yet, so the
//!   entries cannot become desktop surfaces and the placement policy in
//!   [`inox_shell`] is data waiting for a host (ADR 0018);
//! - the session's components are started by `inox-session`, which is a later
//!   milestone, so this process reports the registry it resolved instead of
//!   launching anything.
//!
//! `--help` and every message a user is meant to read go through Fluent, so the
//! coordinator speaks the session's language rather than the developer's.

use std::error::Error;
use std::ffi::OsString;

use anyhow::Result;
use inox_common::i18n::{Locale, Localizer};
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use inox_shell::diagnostic::Diagnostic;
use inox_shell::i18n::{self, id};
use inox_shell::registry::EntryRegistry;

/// What the command line asked inox-shell to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Invocation {
    /// Resolve the session and report it.
    Run,
    /// Report the usage text and stop.
    Help,
}

/// Reads the arguments inox-shell was given.
///
/// `Err` carries the first argument the coordinator does not accept, so the
/// message can name it. Anything that is not `--help` is an error rather than
/// being ignored: a mistyped option that silently starts the coordinator is a
/// worse outcome than a message saying it is not recognized.
fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Invocation, OsString> {
    let mut invocation = Invocation::Run;

    for argument in args {
        match argument.to_str() {
            Some("--help" | "-h") => invocation = Invocation::Help,
            _ => return Err(argument),
        }
    }

    Ok(invocation)
}

/// Sends the log to stderr, honouring `RUST_LOG` and defaulting to `info`.
///
/// `tracing_subscriber::fmt` writes to stdout unless it is told otherwise, and
/// this coordinator's log is diagnostics rather than output: `--help` is a log
/// line, so a default subscriber would put the usage text on the stream a
/// caller reads for data and a pipe carries on. Pointing the writer at stderr
/// keeps stdout empty, which is what this crate's `AGENTS.md` asks of
/// `--help` and what `tests/cli.rs` asserts of the real process.
fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
}

/// Reports a startup failure in the session's language and returns it.
///
/// The error is logged with its detail as a structured field and returned so
/// that the process exits non-zero with the error chain intact; the localized
/// line is what a person reading the log gets, the returned error is what a
/// person debugging does.
fn fail(
    localizer: &Localizer,
    message: &'static str,
    error: impl Error + Send + Sync + 'static,
) -> Result<()> {
    let rendered = Diagnostic::new(message)
        .with("reason", error.to_string())
        .render(localizer);
    error!(detail = %error, "{rendered}");

    Err(error.into())
}

fn main() -> Result<()> {
    init_logging();

    let invocation = match parse_args(std::env::args_os().skip(1)) {
        Ok(invocation) => invocation,
        Err(argument) => {
            let localizer = i18n::localizer(Locale::from_env())?;
            let rendered = Diagnostic::new(id::ARGUMENT_UNKNOWN)
                .with("argument", argument.to_string_lossy().into_owned())
                .render(&localizer);
            error!("{rendered}");
            info!("{}", localizer.message(id::USAGE));

            anyhow::bail!("unrecognized argument: {}", argument.to_string_lossy());
        }
    };

    // Before `config.toml` is read the environment decides the language; the
    // configuration may choose a different one, and does further down.
    let early = i18n::localizer(Locale::from_env())?;

    if invocation == Invocation::Help {
        info!("{}", early.message(id::USAGE));
        return Ok(());
    }

    let paths = match ConfigPaths::discover() {
        Ok(paths) => paths,
        Err(error) => return fail(&early, id::CONFIG_ERROR, error),
    };
    let config = match InoxConfig::load(&paths, Appearance::System) {
        Ok(config) => config,
        Err(error) => return fail(&early, id::CONFIG_ERROR, error),
    };

    // The configuration is the one place the session's language is chosen, so
    // from here on the coordinator speaks it.
    let localizer = match i18n::localizer(config.locale.clone()) {
        Ok(localizer) => localizer,
        Err(error) => return fail(&early, id::CATALOG_ERROR, error),
    };

    let registry = match EntryRegistry::standard(&paths, &config.root.components) {
        Ok(registry) => registry,
        Err(error) => {
            // The registry's own message names the entry and the reason, so it
            // is logged as it stands rather than wrapped in a generic one.
            error!(detail = %error, "{}", error.describe().render(&localizer));
            return Err(error.into());
        }
    };

    info!(
        "{}",
        Diagnostic::new(id::STARTING)
            .with("version", inox_shell::VERSION)
            .with("locale", localizer.locale().tag())
            .render(&localizer)
    );
    info!(
        "{}",
        Diagnostic::new(id::REGISTRY_SUMMARY)
            .with("count", registry.len())
            .with(
                "entries",
                registry
                    .iter()
                    .map(|entry| entry.id().as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .render(&localizer)
    );

    for entry in registry.iter() {
        let rendered = Diagnostic::new(id::REGISTRY_ENTRY)
            .with("entry", entry.id().as_str())
            .with("binary", entry.binary().to_string())
            .with("config", entry.config().display().to_string())
            .with("required", if entry.is_required() { "yes" } else { "no" })
            .render(&localizer);
        let state = entry.state().describe().render(&localizer);

        info!(state = %state, "{rendered}");
    }

    // The one thing about this milestone a user of the session would otherwise
    // have to guess at, so it is said rather than implied (ADR 0018).
    warn!(adr = "0018", "{}", localizer.message(id::NO_LAYER_SHELL));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_arguments_starts_the_coordinator() {
        assert_eq!(parse_args(args(&[])), Ok(Invocation::Run));
    }

    #[test]
    fn help_is_recognized_in_both_spellings() {
        for spelling in ["--help", "-h"] {
            assert_eq!(parse_args(args(&[spelling])), Ok(Invocation::Help));
        }
    }

    #[test]
    fn an_unknown_argument_is_rejected_with_its_name() {
        assert_eq!(
            parse_args(args(&["--config", "/tmp/inox"])),
            Err(OsString::from("--config"))
        );
        assert_eq!(
            parse_args(args(&["--help", "extra"])),
            Err(OsString::from("extra"))
        );
    }
}
