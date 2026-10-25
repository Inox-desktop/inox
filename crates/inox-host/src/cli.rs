//! The command line: a pure parse from arguments to an invocation.
//!
//! the boot login chain
//! configure. Everything it would otherwise hard-code is an option here — where the
//! sibling binaries live, which Wayland socket name the greeter stage uses, which
//! boundary socket to require, and whatever extra arguments a development machine
//! needs to pass to `inox-compositor` (a headless run, for instance).
//!
//! Two properties matter beyond the usual:
//!
//! - **`--help` and `--version` must not touch the system.** They are answered from
//!   the parse alone, before a socket is probed or a directory is resolved, which is
//!   what makes them usable on a machine where the login chain is broken.
//! - **Nothing here reads the process environment.** The two values that come from it
//!   — the executable's own directory and `INOX_AUTH_SOCKET` — are passed in by
//!   [`crate::main`](crate) as [`Defaults`], so a new environment variable cannot
//!   quietly appear in the middle of the parser.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// The Wayland socket name the greeter stage uses.
pub const DEFAULT_WAYLAND_DISPLAY: &str = "wayland-inox-greet";

/// The values the parse starts from, read from the process by the caller.
#[derive(Debug, Clone)]
pub struct Defaults {
    /// Where `inox-compositor` and `inox-greet` are expected, by default the
    /// directory this executable was started from.
    pub bin_dir: PathBuf,
    /// The boundary socket to require, by default `$INOX_AUTH_SOCKET` and then
    /// `/run/inox/auth.sock`.
    pub auth_socket: PathBuf,
}

/// What the host was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Print the usage line and exit successfully.
    Help,
    /// Print the version and exit successfully.
    Version,
    /// Run the greeter stage with these options.
    Run(Options),
}

/// The options one run uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The directory `inox-compositor` and `inox-greet` are started from.
    pub bin_dir: PathBuf,
    /// The Wayland socket name: passed to the compositor and to the login screen.
    pub socket: String,
    /// The `auth-v1` socket that has to be connectable before anything starts.
    pub auth_socket: PathBuf,
    /// Extra arguments appended to the compositor's command line.
    pub compositor_args: Vec<OsString>,
}

/// Why the command line could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// An argument that is not an option this host knows.
    Unknown(OsString),
    /// An option that needs a value and was the last argument.
    Missing(&'static str),
    /// An option that was given an empty value.
    Empty(&'static str),
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Unknown(argument) => {
                write!(
                    formatter,
                    "unrecognized argument: {}",
                    argument.to_string_lossy()
                )
            }
            CliError::Missing(option) => write!(formatter, "{option} requires a value"),
            CliError::Empty(option) => write!(formatter, "{option} requires a nonempty value"),
        }
    }
}

impl std::error::Error for CliError {}

/// Parses `arguments` (without the program name).
///
/// # Errors
///
/// Returns [`CliError::Unknown`] for anything that is not one of the documented
/// options, [`CliError::Missing`] when a value is missing entirely, and
/// [`CliError::Empty`] when it is empty. All three are refusals: a host that
/// silently ignored an argument would start a stage the operator did not ask for.
pub fn parse<I>(arguments: I, defaults: Defaults) -> Result<Invocation, CliError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut options = Options {
        bin_dir: defaults.bin_dir,
        socket: DEFAULT_WAYLAND_DISPLAY.to_owned(),
        auth_socket: defaults.auth_socket,
        compositor_args: Vec::new(),
    };
    let mut arguments = arguments.into_iter();

    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("-h" | "--help") => return Ok(Invocation::Help),
            Some("-V" | "--version") => return Ok(Invocation::Version),
            Some("--bin-dir") => {
                options.bin_dir = PathBuf::from(value(&mut arguments, "--bin-dir")?);
            }
            Some("--socket") => {
                options.socket = text(&mut arguments, "--socket")?;
            }
            Some("--auth-socket") => {
                options.auth_socket = PathBuf::from(value(&mut arguments, "--auth-socket")?);
            }
            // A compositor argument may itself look like an option (`--headless`), so
            // the next argument is taken verbatim rather than matched.
            Some("--compositor-arg") => {
                options
                    .compositor_args
                    .push(value(&mut arguments, "--compositor-arg")?);
            }
            _ => return Err(CliError::Unknown(argument)),
        }
    }

    Ok(Invocation::Run(options))
}

/// The next argument as an uninterpreted value.
fn value<I>(arguments: &mut I, option: &'static str) -> Result<OsString, CliError>
where
    I: Iterator<Item = OsString>,
{
    match arguments.next() {
        Some(value) if value.is_empty() => Err(CliError::Empty(option)),
        Some(value) => Ok(value),
        None => Err(CliError::Missing(option)),
    }
}

/// The next argument as a value that has to be text.
fn text<I>(arguments: &mut I, option: &'static str) -> Result<String, CliError>
where
    I: Iterator<Item = OsString>,
{
    let value = value(arguments, option)?;
    value
        .into_string()
        .map_err(|_| CliError::Empty(option))
        .and_then(|value| {
            if value.is_empty() {
                Err(CliError::Empty(option))
            } else {
                Ok(value)
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Defaults {
        Defaults {
            bin_dir: PathBuf::from("/usr/bin"),
            auth_socket: PathBuf::from("/run/inox/auth.sock"),
        }
    }

    fn parse_args(arguments: &[&str]) -> Result<Invocation, CliError> {
        parse(arguments.iter().map(OsString::from), defaults())
    }

    fn run(arguments: &[&str]) -> Options {
        match parse_args(arguments) {
            Ok(Invocation::Run(options)) => options,
            other => panic!("expected a run invocation, got {other:?}"),
        }
    }

    #[test]
    fn no_arguments_runs_with_the_documented_defaults() {
        let options = run(&[]);

        assert_eq!(options.bin_dir, PathBuf::from("/usr/bin"));
        assert_eq!(options.socket, "wayland-inox-greet");
        assert_eq!(options.auth_socket, PathBuf::from("/run/inox/auth.sock"));
        assert!(options.compositor_args.is_empty());
    }

    #[test]
    fn help_and_version_win_over_everything_after_them() {
        assert_eq!(parse_args(&["--help"]), Ok(Invocation::Help));
        assert_eq!(parse_args(&["-h"]), Ok(Invocation::Help));
        assert_eq!(
            parse_args(&["--bin-dir", "/opt", "--help", "--bogus"]),
            Ok(Invocation::Help)
        );
        assert_eq!(parse_args(&["-V"]), Ok(Invocation::Version));
        assert_eq!(parse_args(&["--version"]), Ok(Invocation::Version));
    }

    #[test]
    fn every_option_is_honoured() {
        let options = run(&[
            "--bin-dir",
            "/opt/inox/bin",
            "--socket",
            "wayland-inox-4",
            "--auth-socket",
            "/run/user/1000/auth.sock",
            "--compositor-arg",
            "--headless",
            "--compositor-arg",
            "--compositor-arg=ignored",
        ]);

        assert_eq!(options.bin_dir, PathBuf::from("/opt/inox/bin"));
        assert_eq!(options.socket, "wayland-inox-4");
        assert_eq!(
            options.auth_socket,
            PathBuf::from("/run/user/1000/auth.sock")
        );
        assert_eq!(
            options.compositor_args,
            [
                OsString::from("--headless"),
                // Everything after `--compositor-arg` is literal, including an
                // argument that happens to look like one of this host's options.
                OsString::from("--compositor-arg=ignored"),
            ]
        );
    }

    #[test]
    fn a_compositor_argument_that_looks_like_an_option_is_passed_through() {
        let options = run(&["--compositor-arg", "--socket", "--compositor-arg", "-V"]);

        assert_eq!(
            options.compositor_args,
            [OsString::from("--socket"), OsString::from("-V")]
        );
    }

    #[test]
    fn an_unknown_argument_is_refused() {
        assert_eq!(
            parse_args(&["--bogus"]),
            Err(CliError::Unknown(OsString::from("--bogus")))
        );
        assert_eq!(
            parse_args(&["extra"]),
            Err(CliError::Unknown(OsString::from("extra")))
        );
        // A missing value is not an unknown argument: the option is known and the
        // refusal has to say which value is missing.
        assert_eq!(
            parse_args(&["--bin-dir"]),
            Err(CliError::Missing("--bin-dir"))
        );
    }

    #[test]
    fn a_value_is_never_taken_from_a_following_option() {
        // `--socket` eats `--bin-dir` as its value: the parse is positional on
        // purpose, because a socket name that looked like an option would otherwise
        // be silently replaced by a default.
        let options = run(&["--socket", "--bin-dir"]);

        assert_eq!(options.socket, "--bin-dir");
        assert_eq!(options.bin_dir, PathBuf::from("/usr/bin"));
    }

    #[test]
    fn every_option_that_needs_a_value_refuses_to_be_last() {
        for option in ["--bin-dir", "--socket", "--auth-socket", "--compositor-arg"] {
            assert!(
                matches!(parse_args(&[option]), Err(CliError::Missing(missing)) if missing == option),
                "{option} must require a value"
            );
        }
    }

    #[test]
    fn an_empty_value_is_refused() {
        for option in ["--bin-dir", "--socket", "--auth-socket"] {
            assert_eq!(
                parse_args(&[option, ""]),
                Err(CliError::Empty(option)),
                "{option} must not be empty"
            );
        }
        assert_eq!(
            parse_args(&["--compositor-arg", ""]),
            Err(CliError::Empty("--compositor-arg"))
        );
    }

    #[test]
    fn refusals_name_the_option_a_person_has_to_change() {
        assert!(
            CliError::Unknown(OsString::from("--bogus"))
                .to_string()
                .contains("--bogus")
        );
        assert!(
            CliError::Missing("--socket")
                .to_string()
                .contains("--socket")
        );
        assert!(
            CliError::Empty("--auth-socket")
                .to_string()
                .contains("--auth-socket")
        );
    }

    #[test]
    fn the_defaults_come_from_the_caller_rather_than_the_environment() {
        let options = match parse(
            std::iter::empty::<OsString>(),
            Defaults {
                bin_dir: PathBuf::from("/tmp/bin"),
                auth_socket: PathBuf::from("/tmp/auth.sock"),
            },
        ) {
            Ok(Invocation::Run(options)) => options,
            other => panic!("expected a run invocation, got {other:?}"),
        };

        assert_eq!(options.bin_dir, PathBuf::from("/tmp/bin"));
        assert_eq!(options.auth_socket, PathBuf::from("/tmp/auth.sock"));
    }
}
