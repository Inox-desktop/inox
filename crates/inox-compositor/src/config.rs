//! Runtime configuration for the compositor.
//!
//! This module is deliberately free of Smithay types: it turns command line
//! arguments into plain data, which keeps parsing unit-testable and lets the
//! rest of the crate translate that data into Smithay state.

/// Default width of the headless output, in pixels.
pub const DEFAULT_WIDTH: i32 = 1280;

/// Default height of the headless output, in pixels.
pub const DEFAULT_HEIGHT: i32 = 720;

/// Default refresh rate of the headless output, in Hz.
pub const DEFAULT_REFRESH_HZ: i32 = 60;

/// Largest accepted output dimension, in pixels.
pub const MAX_DIMENSION: i32 = 16_384;

/// Largest accepted refresh rate, in Hz.
pub const MAX_REFRESH_HZ: i32 = 1_000;

/// Name advertised for the headless output.
pub const DEFAULT_OUTPUT_NAME: &str = "headless-1";

/// Which compositor backend to run.
///
/// The variants map onto the layers described in ADR 0010:
///
/// - [`BackendKind::Auto`] probes a libseat DRM session first and falls back to
///   the headless path when either the session or the GBM/vblank present path
///   is unavailable.
/// - [`BackendKind::Headless`] keeps the M1/M2 surfaceless EGL path (ADR 0005)
///   as a fixed choice; it is what CI exercises.
/// - [`BackendKind::Drm`] requires the real backend and never falls back, so a
///   session or present failure surfaces to the operator instead of quietly
///   turning into a headless run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendKind {
    /// Try DRM, fall back to headless on any failure.
    #[default]
    Auto,
    /// Always run the surfaceless EGL headless path.
    Headless,
    /// Always require the DRM/KMS backend.
    Drm,
}

impl BackendKind {
    /// Parse the value accepted on the command line.
    fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "auto" => Ok(Self::Auto),
            "headless" => Ok(Self::Headless),
            "drm" => Ok(Self::Drm),
            _ => Err("expected one of: auto, headless, drm"),
        }
    }
}

/// Usage text printed by `--help`.
pub const HELP: &str = "\
inox-compositor - the Inox Wayland compositor

USAGE:
    inox-compositor [OPTIONS]

OPTIONS:
    --backend <KIND>           Backend to run: auto, headless, drm [default: auto]
    --headless                 Alias for --backend=headless
    --output-size <WxH>        Size of the headless output in pixels [default: 1280x720]
    --refresh-rate <HZ>        Refresh rate of the headless output [default: 60]
    --socket <NAME>            Wayland socket name [default: auto-selected]
    --exit-after-frames <N>    Exit with status 0 after rendering N frames
    -h, --help                 Print this help and exit
    -V, --version              Print the version and exit
";

/// Validated compositor configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositorConfig {
    /// Backend selection (ADR 0010).
    pub backend: BackendKind,
    /// Name advertised for the headless output.
    pub output_name: String,
    /// Width of the headless output, in pixels.
    pub width: i32,
    /// Height of the headless output, in pixels.
    pub height: i32,
    /// Refresh rate advertised for the headless output, in Hz.
    pub refresh_hz: i32,
    /// Wayland socket name, or `None` to let the compositor pick the next free one.
    pub socket_name: Option<String>,
    /// Number of frames to render before exiting, or `None` to run until signalled.
    pub exit_after_frames: Option<u64>,
}

impl Default for CompositorConfig {
    fn default() -> Self {
        Self {
            backend: BackendKind::default(),
            output_name: DEFAULT_OUTPUT_NAME.to_owned(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            refresh_hz: DEFAULT_REFRESH_HZ,
            socket_name: None,
            exit_after_frames: None,
        }
    }
}

impl CompositorConfig {
    /// Output size in pixels, as `(width, height)`.
    pub fn output_size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// Refresh rate in millihertz, the unit Smithay uses for output modes.
    pub fn refresh_mhz(&self) -> i32 {
        self.refresh_hz.saturating_mul(1_000)
    }
}

/// What the command line asked the binary to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// Run the compositor with the given configuration.
    Run(CompositorConfig),
    /// Print [`HELP`] and exit successfully.
    Help,
    /// Print the crate version and exit successfully.
    Version,
}

/// Errors produced while parsing command line arguments.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// An option name was not recognised.
    #[error("unknown option `{0}`")]
    UnknownOption(String),

    /// A positional argument was given, but the compositor accepts none.
    #[error("unexpected argument `{0}`")]
    UnexpectedArgument(String),

    /// An option that needs a value was given without one.
    #[error("option `{option}` requires a value")]
    MissingValue {
        /// The option that is missing its value.
        option: String,
    },

    /// An option value could not be accepted.
    #[error("invalid value `{value}` for `{option}`: {reason}")]
    InvalidValue {
        /// The option that was given a bad value.
        option: String,
        /// The rejected value.
        value: String,
        /// Why the value was rejected.
        reason: &'static str,
    },
}

/// Parse command line arguments, excluding the program name.
///
/// Both `--option value` and `--option=value` spellings are accepted.
///
/// # Errors
///
/// Returns a [`ConfigError`] for unknown options, missing values and values that
/// fail validation (for example `--output-size 0x720`).
pub fn parse_args<I>(args: I) -> Result<Invocation, ConfigError>
where
    I: IntoIterator<Item = String>,
{
    let mut config = CompositorConfig::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        let (option, inline) = split_inline_value(&arg);

        match option.as_str() {
            "-h" | "--help" => return Ok(Invocation::Help),
            "-V" | "--version" => return Ok(Invocation::Version),
            // `--headless` predates the backend selector and stays as an alias so
            // existing CI invocations keep working (ADR 0010 §6).
            "--headless" => {
                reject_inline_value(&option, inline)?;
                config.backend = BackendKind::Headless;
            }
            "--backend" => {
                let value = take_value(&option, inline, &mut args)?;
                config.backend = BackendKind::parse(&value)
                    .map_err(|reason| invalid_value(&option, &value, reason))?;
            }
            "--output-size" => {
                let value = take_value(&option, inline, &mut args)?;
                let (width, height) = parse_size(&value)?;
                config.width = width;
                config.height = height;
            }
            "--refresh-rate" => {
                let value = take_value(&option, inline, &mut args)?;
                config.refresh_hz = parse_refresh_rate(&value)?;
            }
            "--socket" => {
                let value = take_value(&option, inline, &mut args)?;
                if value.is_empty() {
                    return Err(invalid_value(&option, &value, "must not be empty"));
                }
                config.socket_name = Some(value);
            }
            "--exit-after-frames" => {
                let value = take_value(&option, inline, &mut args)?;
                config.exit_after_frames = Some(parse_frame_budget(&value)?);
            }
            other if other.starts_with('-') => {
                return Err(ConfigError::UnknownOption(other.to_owned()));
            }
            other => return Err(ConfigError::UnexpectedArgument(other.to_owned())),
        }
    }

    Ok(Invocation::Run(config))
}

fn split_inline_value(arg: &str) -> (String, Option<String>) {
    match arg.split_once('=') {
        Some((option, value)) if option.starts_with("--") => {
            (option.to_owned(), Some(value.to_owned()))
        }
        _ => (arg.to_owned(), None),
    }
}

fn take_value<I>(option: &str, inline: Option<String>, rest: &mut I) -> Result<String, ConfigError>
where
    I: Iterator<Item = String>,
{
    match inline {
        Some(value) => Ok(value),
        None => rest.next().ok_or_else(|| ConfigError::MissingValue {
            option: option.to_owned(),
        }),
    }
}

fn reject_inline_value(option: &str, inline: Option<String>) -> Result<(), ConfigError> {
    match inline {
        Some(value) => Err(invalid_value(option, &value, "does not take a value")),
        None => Ok(()),
    }
}

fn invalid_value(option: &str, value: &str, reason: &'static str) -> ConfigError {
    ConfigError::InvalidValue {
        option: option.to_owned(),
        value: value.to_owned(),
        reason,
    }
}

fn parse_size(value: &str) -> Result<(i32, i32), ConfigError> {
    let (width, height) = value
        .split_once('x')
        .or_else(|| value.split_once('X'))
        .ok_or_else(|| invalid_value("--output-size", value, "expected <WIDTH>x<HEIGHT>"))?;

    let width = parse_dimension("--output-size", width)?;
    let height = parse_dimension("--output-size", height)?;

    Ok((width, height))
}

fn parse_dimension(option: &str, raw: &str) -> Result<i32, ConfigError> {
    let dimension: i32 = raw
        .parse()
        .map_err(|_| invalid_value(option, raw, "expected a whole number of pixels"))?;

    if !(1..=MAX_DIMENSION).contains(&dimension) {
        return Err(invalid_value(
            option,
            raw,
            "must be between 1 and 16384 pixels",
        ));
    }

    Ok(dimension)
}

fn parse_refresh_rate(value: &str) -> Result<i32, ConfigError> {
    let refresh: i32 = value
        .parse()
        .map_err(|_| invalid_value("--refresh-rate", value, "expected a whole number of hertz"))?;

    if !(1..=MAX_REFRESH_HZ).contains(&refresh) {
        return Err(invalid_value(
            "--refresh-rate",
            value,
            "must be between 1 and 1000 Hz",
        ));
    }

    Ok(refresh)
}

fn parse_frame_budget(value: &str) -> Result<u64, ConfigError> {
    let frames: u64 = value
        .parse()
        .map_err(|_| invalid_value("--exit-after-frames", value, "expected a whole number"))?;

    if frames == 0 {
        return Err(invalid_value(
            "--exit-after-frames",
            value,
            "must be at least 1",
        ));
    }

    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(raw: &[&str]) -> Vec<String> {
        raw.iter().map(|arg| (*arg).to_owned()).collect()
    }

    fn run_config(raw: &[&str]) -> CompositorConfig {
        match parse_args(args(raw)) {
            Ok(Invocation::Run(config)) => config,
            other => panic!("expected a run invocation, got {other:?}"),
        }
    }

    #[test]
    fn no_arguments_uses_defaults() {
        assert_eq!(run_config(&[]), CompositorConfig::default());
    }

    #[test]
    fn defaults_match_the_documented_values() {
        let config = CompositorConfig::default();

        assert_eq!(config.backend, BackendKind::Auto);
        assert_eq!(config.output_size(), (1280, 720));
        assert_eq!(config.refresh_mhz(), 60_000);
        assert_eq!(config.socket_name, None);
        assert_eq!(config.exit_after_frames, None);
    }

    #[test]
    fn headless_alias_selects_the_headless_backend() {
        let config = run_config(&["--headless"]);

        assert_eq!(config.backend, BackendKind::Headless);
        // Every other field keeps its default value, matching the M1/M2 path.
        assert_eq!(
            config,
            CompositorConfig {
                backend: BackendKind::Headless,
                ..CompositorConfig::default()
            }
        );
    }

    #[test]
    fn backend_option_accepts_every_kind() {
        for (value, expected) in [
            ("auto", BackendKind::Auto),
            ("headless", BackendKind::Headless),
            ("drm", BackendKind::Drm),
        ] {
            assert_eq!(
                run_config(&["--backend", value]).backend,
                expected,
                "expected `--backend={value}` to parse"
            );
            assert_eq!(
                run_config(&[&format!("--backend={value}")]).backend,
                expected,
                "expected the inline `--backend={value}` to parse"
            );
        }
    }

    #[test]
    fn backend_last_wins_and_headless_alias_still_works_after_explicit_backend() {
        // Explicit `--backend=drm` after the alias must be honoured so users can
        // override a config file or shell alias that already added `--headless`.
        assert_eq!(
            run_config(&["--headless", "--backend", "drm"]).backend,
            BackendKind::Drm
        );
        // The reverse order still lands on headless.
        assert_eq!(
            run_config(&["--backend", "drm", "--headless"]).backend,
            BackendKind::Headless
        );
    }

    #[test]
    fn unknown_backend_kinds_are_rejected() {
        for value in ["", "wayland", "X11", "auto "] {
            let outcome = parse_args(args(&["--backend", value]));

            assert!(
                matches!(outcome, Err(ConfigError::InvalidValue { .. })),
                "expected `--backend={value:?}` to be rejected, got {outcome:?}"
            );
        }
    }

    #[test]
    fn every_option_is_parsed_in_the_separated_form() {
        let config = run_config(&[
            "--headless",
            "--output-size",
            "1920x1080",
            "--refresh-rate",
            "75",
            "--socket",
            "inox-test",
            "--exit-after-frames",
            "3",
        ]);

        assert_eq!(config.output_size(), (1920, 1080));
        assert_eq!(config.refresh_hz, 75);
        assert_eq!(config.refresh_mhz(), 75_000);
        assert_eq!(config.socket_name.as_deref(), Some("inox-test"));
        assert_eq!(config.exit_after_frames, Some(3));
    }

    #[test]
    fn every_option_is_parsed_in_the_inline_form() {
        let config = run_config(&[
            "--output-size=800x600",
            "--refresh-rate=30",
            "--socket=inline",
            "--exit-after-frames=1",
        ]);

        assert_eq!(config.output_size(), (800, 600));
        assert_eq!(config.refresh_hz, 30);
        assert_eq!(config.socket_name.as_deref(), Some("inline"));
        assert_eq!(config.exit_after_frames, Some(1));
    }

    #[test]
    fn uppercase_separator_is_accepted() {
        assert_eq!(
            run_config(&["--output-size", "640X480"]).output_size(),
            (640, 480)
        );
    }

    #[test]
    fn help_and_version_short_circuit() {
        assert_eq!(parse_args(args(&["--help"])), Ok(Invocation::Help));
        assert_eq!(parse_args(args(&["-h"])), Ok(Invocation::Help));
        assert_eq!(parse_args(args(&["--version"])), Ok(Invocation::Version));
        assert_eq!(parse_args(args(&["-V"])), Ok(Invocation::Version));

        // A flag that follows is not consulted once help was requested.
        assert_eq!(
            parse_args(args(&["--help", "--nonsense"])),
            Ok(Invocation::Help)
        );
    }

    #[test]
    fn unknown_options_and_positional_arguments_are_rejected() {
        assert_eq!(
            parse_args(args(&["--nonsense"])),
            Err(ConfigError::UnknownOption("--nonsense".to_owned()))
        );
        assert_eq!(
            parse_args(args(&["start"])),
            Err(ConfigError::UnexpectedArgument("start".to_owned()))
        );
    }

    #[test]
    fn options_without_a_value_are_rejected() {
        for option in [
            "--backend",
            "--output-size",
            "--refresh-rate",
            "--socket",
            "--exit-after-frames",
        ] {
            assert_eq!(
                parse_args(args(&[option])),
                Err(ConfigError::MissingValue {
                    option: option.to_owned()
                }),
                "expected `{option}` to require a value"
            );
        }
    }

    #[test]
    fn flags_that_take_no_value_reject_inline_values() {
        assert_eq!(
            parse_args(args(&["--headless=yes"])),
            Err(ConfigError::InvalidValue {
                option: "--headless".to_owned(),
                value: "yes".to_owned(),
                reason: "does not take a value",
            })
        );
    }

    #[test]
    fn malformed_sizes_are_rejected() {
        for value in ["1280", "1280x", "x720", "a x b", "1280x720x1"] {
            let outcome = parse_args(args(&["--output-size", value]));

            assert!(
                matches!(outcome, Err(ConfigError::InvalidValue { .. })),
                "expected `{value}` to be rejected, got {outcome:?}"
            );
        }
    }

    #[test]
    fn out_of_range_sizes_are_rejected() {
        for value in ["0x720", "1280x0", "-1x720", "16385x720"] {
            let outcome = parse_args(args(&["--output-size", value]));

            assert!(
                matches!(outcome, Err(ConfigError::InvalidValue { .. })),
                "expected `{value}` to be rejected, got {outcome:?}"
            );
        }

        assert_eq!(
            run_config(&["--output-size", "16384x16384"]).output_size(),
            (16384, 16384)
        );
    }

    #[test]
    fn out_of_range_refresh_rates_are_rejected() {
        for value in ["0", "-1", "1001", "fast"] {
            let outcome = parse_args(args(&["--refresh-rate", value]));

            assert!(
                matches!(outcome, Err(ConfigError::InvalidValue { .. })),
                "expected `{value}` to be rejected, got {outcome:?}"
            );
        }
    }

    #[test]
    fn empty_socket_names_and_zero_frame_budgets_are_rejected() {
        assert!(matches!(
            parse_args(args(&["--socket", ""])),
            Err(ConfigError::InvalidValue { .. })
        ));
        assert!(matches!(
            parse_args(args(&["--exit-after-frames", "0"])),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[test]
    fn help_text_mentions_every_option() {
        for option in [
            "--backend",
            "--headless",
            "--output-size",
            "--refresh-rate",
            "--socket",
            "--exit-after-frames",
            "--help",
            "--version",
        ] {
            assert!(HELP.contains(option), "help text omits `{option}`");
        }
    }
}
