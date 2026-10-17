//! Runtime configuration for the compositor.
//!
//! This module is deliberately free of Smithay types: it turns command line
//! arguments into plain data, which keeps parsing unit-testable and lets the
//! rest of the crate translate that data into Smithay state.

use std::path::PathBuf;

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

/// Largest accepted magnitude of an output position coordinate, in logical pixels.
///
/// The same limit `inox-config` applies to `compositor.toml` (ADR 0019).
pub const MAX_COORDINATE: i32 = 4_194_304;

/// The keys `--output` accepts, for the error message and the help text.
const OUTPUT_KEYS: &str = "expected a comma-separated list of key=value pairs from: name, width, height, refresh, scale, x, y";

/// Name advertised for the default headless output.
pub const DEFAULT_OUTPUT_NAME: &str = "headless-1";

/// Smallest accepted scale factor of a headless output.
///
/// The same range `inox-config` enforces for `compositor.toml` (ADR 0020).
pub const SCALE_MIN: f64 = 0.25;

/// Largest accepted scale factor of a headless output.
pub const SCALE_MAX: f64 = 8.0;

/// One output the headless backend creates.
///
/// The physical backend takes its outputs from the DRM connectors it enumerates
/// and `compositor.toml`; this type describes the synthetic outputs CI drives,
/// which is what lets the multi-output and fractional-scaling behaviour be
/// exercised without a display (ADR 0019).
#[derive(Debug, Clone, PartialEq)]
pub struct HeadlessOutput {
    /// Name advertised on `wl_output`, unique within one compositor run.
    pub name: String,
    /// Width in physical pixels.
    pub width: i32,
    /// Height in physical pixels.
    pub height: i32,
    /// Refresh rate advertised for the output, in Hz.
    pub refresh_hz: i32,
    /// Scale factor between physical and logical pixels.
    pub scale: f64,
    /// Top-left corner in the global logical space, or `None` to let the
    /// compositor place the output automatically.
    pub position: Option<(i32, i32)>,
}

impl Default for HeadlessOutput {
    fn default() -> Self {
        Self {
            name: DEFAULT_OUTPUT_NAME.to_owned(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            refresh_hz: DEFAULT_REFRESH_HZ,
            scale: 1.0,
            position: None,
        }
    }
}

impl HeadlessOutput {
    /// Size in physical pixels, as `(width, height)`.
    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// Refresh rate in millihertz, the unit Smithay uses for output modes.
    pub fn refresh_mhz(&self) -> i32 {
        self.refresh_hz.saturating_mul(1_000)
    }
}

/// Which compositor backend to run.
///
/// The variants map onto the layers described in ADR 0010:
///
/// - [`BackendKind::Auto`] selects an enumerated libseat DRM device and fails if no
///   physical output can be driven. Headless must be explicitly requested (ADR 0012).
/// - [`BackendKind::Headless`] keeps the M1/M2 surfaceless EGL path (ADR 0005)
///   as a fixed choice; it is what CI exercises.
/// - [`BackendKind::Drm`] requires the real backend and never falls back, so a
///   session or present failure surfaces to the operator instead of quietly
///   turning into a headless run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendKind {
    /// Select a physical device; never fall back to headless.
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
    --output-size <WxH>        Size in pixels of the default headless output [default: 1280x720]
    --refresh-rate <HZ>        Refresh rate of the default headless output [default: 60]
    --output <SPEC>            Add a headless output; the first use replaces the default one.
                               SPEC is a comma-separated list of key=value pairs from:
                               name, width, height, refresh, scale, x, y.
                               Example: --output name=DP-2,width=1920,height=1080,scale=1.5,x=1920,y=0
    --socket <NAME>            Wayland socket name [default: auto-selected]
    --input-method <EXEC>      Launch a trusted input method (for example fcitx5)
    --input-method-arg <ARG>   Append one argument to the input method command
    --lock-token <TOKEN>       One-shot token that authorizes the session lock (ADR 0022)
    --exit-after-frames <N>    Exit with status 0 after rendering N frames
    --test-input <PATH>        Serve test commands on a unix socket (test-input builds only)
    -h, --help                 Print this help and exit
    -V, --version              Print the version and exit
";

/// Validated compositor configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositorConfig {
    /// Backend selection (ADR 0010).
    pub backend: BackendKind,
    /// Headless outputs to create, in the order they are advertised.
    ///
    /// Never empty: the headless backend always has at least the default output,
    /// which is what keeps the single-output behaviour of every earlier milestone
    /// unchanged. The physical backend ignores this list and uses the DRM
    /// connectors it enumerates instead.
    pub outputs: Vec<HeadlessOutput>,
    /// Wayland socket name, or `None` to let the compositor pick the next free one.
    pub socket_name: Option<String>,
    /// One-shot token a client must present over `inox-ipc` to be allowed to lock
    /// the session, or `None` when locking is unavailable in this run.
    ///
    /// `inox-session` generates it per session and hands the same value to the
    /// trusted lock client; ADR 0022 records why the authority is a capability
    /// rather than a client-supplied name, and why a compositor started without
    /// one refuses every lock request instead of accepting an unidentified one.
    pub lock_token: Option<String>,
    /// Explicit input method argv; launched on a private privileged connection.
    pub input_method: Vec<String>,
    /// Number of frames to render before exiting, or `None` to run until signalled.
    pub exit_after_frames: Option<u64>,
    /// Path of the test-input control socket, set only by `--test-input`.
    ///
    /// Populated exclusively in builds with the `test-input` feature, where
    /// `crate::test_input` listens on it; the parser rejects the option in every other
    /// build, so this stays `None` there.
    pub test_input_path: Option<PathBuf>,
}

impl Default for CompositorConfig {
    fn default() -> Self {
        Self {
            backend: BackendKind::default(),
            outputs: vec![HeadlessOutput::default()],
            socket_name: None,
            lock_token: None,
            input_method: Vec::new(),
            exit_after_frames: None,
            test_input_path: None,
        }
    }
}

impl CompositorConfig {
    /// The output the compositor treats as primary.
    ///
    /// The first entry of [`CompositorConfig::outputs`]; the list is never empty.
    pub fn primary_output(&self) -> &HeadlessOutput {
        self.outputs.first().unwrap_or(&EMPTY_OUTPUT)
    }

    /// Primary output size in pixels, as `(width, height)`.
    pub fn output_size(&self) -> (i32, i32) {
        self.primary_output().size()
    }

    /// The first output, materialising a default one if the list is empty.
    fn primary_output_mut(&mut self) -> &mut HeadlessOutput {
        if self.outputs.is_empty() {
            self.outputs.push(HeadlessOutput::default());
        }
        self.outputs
            .first_mut()
            .expect("an output was just pushed when the list was empty")
    }

    /// Name advertised for the primary output.
    pub fn output_name(&self) -> &str {
        &self.primary_output().name
    }

    /// Primary output refresh rate in Hz.
    pub fn refresh_hz(&self) -> i32 {
        self.primary_output().refresh_hz
    }

    /// Primary output refresh rate in millihertz, the unit Smithay uses for modes.
    pub fn refresh_mhz(&self) -> i32 {
        self.primary_output().refresh_mhz()
    }
}

/// Fallback for a configuration whose output list was cleared by hand.
///
/// [`CompositorConfig`] keeps the list non-empty, so this is unreachable through
/// the parser; returning a borrowed default keeps [`CompositorConfig::primary_output`]
/// total instead of panicking on a hand-built value.
static EMPTY_OUTPUT: HeadlessOutput = HeadlessOutput {
    name: String::new(),
    width: 0,
    height: 0,
    refresh_hz: 0,
    scale: 1.0,
    position: None,
};

/// What the command line asked the binary to do.
///
/// Not `Eq`: the output list carries a scale factor, and `f64` is only `PartialEq`.
#[derive(Debug, Clone, PartialEq)]
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

    /// `--test-input` was given to a binary built without the `test-input` feature.
    #[error(
        "--test-input {path} is unavailable: this binary was built without the `test-input` feature"
    )]
    TestInputUnsupported {
        /// The socket path the rejected option carried.
        path: String,
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
    // The default output stands in for "no --output was given"; the first
    // explicit one replaces it rather than adding to it, so a user who lists
    // outputs gets exactly the outputs they listed.
    let mut replaced_outputs = false;
    // Counted separately from `config.outputs.len()`, which still holds the
    // default output until the first explicit one replaces it.
    let mut explicit_outputs = 0usize;
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
                let output = config.primary_output_mut();
                output.width = width;
                output.height = height;
            }
            "--refresh-rate" => {
                let value = take_value(&option, inline, &mut args)?;
                config.primary_output_mut().refresh_hz =
                    parse_refresh_rate("--refresh-rate", &value)?;
            }
            "--output" => {
                let value = take_value(&option, inline, &mut args)?;
                let output = parse_output(&value, explicit_outputs)?;
                if !replaced_outputs {
                    config.outputs.clear();
                    replaced_outputs = true;
                }
                explicit_outputs += 1;
                config.outputs.push(output);
            }
            "--socket" => {
                let value = take_value(&option, inline, &mut args)?;
                if value.is_empty() {
                    return Err(invalid_value(&option, &value, "must not be empty"));
                }
                config.socket_name = Some(value);
            }
            "--input-method" => {
                let value = take_value(&option, inline, &mut args)?;
                if value.is_empty() || !config.input_method.is_empty() {
                    return Err(invalid_value(
                        &option,
                        &value,
                        "requires one nonempty executable, specified once",
                    ));
                }
                config.input_method.push(value);
            }
            "--input-method-arg" => {
                let value = take_value(&option, inline, &mut args)?;
                if config.input_method.is_empty() {
                    return Err(invalid_value(&option, &value, "must follow --input-method"));
                }
                config.input_method.push(value);
            }
            "--lock-token" => {
                let value = take_value(&option, inline, &mut args)?;
                if value.is_empty() {
                    return Err(invalid_value(&option, &value, "must not be empty"));
                }
                config.lock_token = Some(value);
            }
            "--exit-after-frames" => {
                let value = take_value(&option, inline, &mut args)?;
                config.exit_after_frames = Some(parse_frame_budget(&value)?);
            }
            // The test-input channel only exists in builds that opted into the
            // feature (see `crate::test_input`); a default build rejects the option
            // instead of accepting and silently ignoring it. Both arms consume the
            // value first so a missing one is still reported as such.
            #[cfg(feature = "test-input")]
            "--test-input" => {
                let value = take_value(&option, inline, &mut args)?;
                if value.is_empty() {
                    return Err(invalid_value(&option, &value, "must not be empty"));
                }
                config.test_input_path = Some(PathBuf::from(value));
            }
            #[cfg(not(feature = "test-input"))]
            "--test-input" => {
                let value = take_value(&option, inline, &mut args)?;

                return Err(ConfigError::TestInputUnsupported { path: value });
            }
            other if other.starts_with('-') => {
                return Err(ConfigError::UnknownOption(other.to_owned()));
            }
            other => return Err(ConfigError::UnexpectedArgument(other.to_owned())),
        }
    }

    reject_duplicate_output_names(&config)?;

    Ok(Invocation::Run(config))
}

/// Reject two outputs that would advertise the same `wl_output` name.
///
/// The compositor keys the output topology on the name, so a duplicate would make
/// one output silently unreachable (ADR 0019).
fn reject_duplicate_output_names(config: &CompositorConfig) -> Result<(), ConfigError> {
    let mut seen = Vec::with_capacity(config.outputs.len());
    for output in &config.outputs {
        if seen.contains(&output.name) {
            return Err(invalid_value(
                "--output",
                &output.name,
                "each output needs a distinct name",
            ));
        }
        seen.push(output.name.clone());
    }
    Ok(())
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

fn parse_refresh_rate(option: &str, value: &str) -> Result<i32, ConfigError> {
    let refresh: i32 = value
        .parse()
        .map_err(|_| invalid_value(option, value, "expected a whole number of hertz"))?;

    if !(1..=MAX_REFRESH_HZ).contains(&refresh) {
        return Err(invalid_value(
            option,
            value,
            "must be between 1 and 1000 Hz",
        ));
    }

    Ok(refresh)
}

/// Parse one `--output` specification.
///
/// `index` is the position the output will take in the configured list; it names
/// the output when the specification does not, so a listed output is always
/// addressable on `wl_output`.
fn parse_output(spec: &str, index: usize) -> Result<HeadlessOutput, ConfigError> {
    if spec.is_empty() {
        return Err(invalid_value("--output", spec, "must not be empty"));
    }

    let mut output = HeadlessOutput {
        name: format!("headless-{}", index.saturating_add(1)),
        ..HeadlessOutput::default()
    };
    let mut keys: Vec<&str> = Vec::new();
    let mut x: Option<i32> = None;
    let mut y: Option<i32> = None;

    for pair in spec.split(',') {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| invalid_value("--output", pair, OUTPUT_KEYS))?;
        if keys.contains(&key) {
            return Err(invalid_value("--output", key, "each key may appear once"));
        }
        keys.push(key);

        match key {
            "name" => {
                if value.is_empty() {
                    return Err(invalid_value(
                        "--output",
                        pair,
                        "an output name must not be empty",
                    ));
                }
                output.name = value.to_owned();
            }
            "width" => output.width = parse_dimension("--output", value)?,
            "height" => output.height = parse_dimension("--output", value)?,
            "refresh" => output.refresh_hz = parse_refresh_rate("--output", value)?,
            "scale" => output.scale = parse_scale(value)?,
            "x" => x = Some(parse_coordinate(value)?),
            "y" => y = Some(parse_coordinate(value)?),
            _ => return Err(invalid_value("--output", key, OUTPUT_KEYS)),
        }
    }

    if x.is_some() || y.is_some() {
        output.position = Some((x.unwrap_or(0), y.unwrap_or(0)));
    }

    Ok(output)
}

/// Parse an output scale factor, bounded to the range `inox-config` enforces.
fn parse_scale(value: &str) -> Result<f64, ConfigError> {
    let scale: f64 = value
        .parse()
        .map_err(|_| invalid_value("--output", value, "expected a number"))?;

    if !scale.is_finite() || !(SCALE_MIN..=SCALE_MAX).contains(&scale) {
        return Err(invalid_value(
            "--output",
            value,
            "scale must be between 0.25 and 8",
        ));
    }

    Ok(scale)
}

/// Parse an output position coordinate, bounded to the range `inox-config` enforces.
fn parse_coordinate(value: &str) -> Result<i32, ConfigError> {
    let coordinate: i32 = value
        .parse()
        .map_err(|_| invalid_value("--output", value, "expected a whole number of pixels"))?;

    if coordinate.abs() > MAX_COORDINATE {
        return Err(invalid_value(
            "--output",
            value,
            "a position coordinate must be between -4194304 and 4194304",
        ));
    }

    Ok(coordinate)
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
    fn input_method_argv_is_explicit_and_preserves_literal_arguments() {
        assert!(run_config(&[]).input_method.is_empty());
        let config = run_config(&[
            "--input-method",
            "/usr/bin/fcitx5",
            "--input-method-arg=--verbose=debug",
            "--input-method-arg",
            "a b;$(touch /tmp/not-executed)",
        ]);
        assert_eq!(
            config.input_method,
            [
                "/usr/bin/fcitx5",
                "--verbose=debug",
                "a b;$(touch /tmp/not-executed)"
            ]
        );
        for invalid in [
            vec!["--input-method"],
            vec!["--input-method="],
            vec!["--input-method-arg=x"],
            vec!["--input-method=a", "--input-method=b"],
        ] {
            assert!(parse_args(args(&invalid)).is_err());
        }
    }

    #[test]
    fn defaults_match_the_documented_values() {
        let config = CompositorConfig::default();

        assert_eq!(config.backend, BackendKind::Auto);
        assert_eq!(config.output_size(), (1280, 720));
        assert_eq!(config.refresh_hz(), 60);
        assert_eq!(config.refresh_mhz(), 60_000);
        assert_eq!(config.socket_name, None);
        assert_eq!(config.exit_after_frames, None);
        // One synthetic output, placed automatically and unscaled: exactly the
        // single-output behaviour M1 through M3 had.
        assert_eq!(config.outputs, vec![HeadlessOutput::default()]);
        assert_eq!(config.output_name(), "headless-1");
        assert_eq!(config.primary_output().position, None);
    }

    #[test]
    fn a_single_output_replacement_keeps_the_legacy_options_working() {
        let config = run_config(&["--output-size", "1920x1080", "--refresh-rate", "75"]);

        assert_eq!(config.outputs.len(), 1);
        assert_eq!(
            config.outputs[0],
            HeadlessOutput {
                width: 1920,
                height: 1080,
                refresh_hz: 75,
                ..HeadlessOutput::default()
            }
        );
    }

    #[test]
    fn several_outputs_are_configured_in_the_order_they_are_given() {
        let config = run_config(&[
            "--output",
            "name=DP-1,width=2560,height=1440,refresh=144,scale=1.5,x=0,y=0",
            "--output",
            "name=DP-2,width=1920,height=1080,scale=2,x=2560,y=-120",
        ]);

        assert_eq!(
            config.outputs,
            vec![
                HeadlessOutput {
                    name: "DP-1".to_owned(),
                    width: 2560,
                    height: 1440,
                    refresh_hz: 144,
                    scale: 1.5,
                    position: Some((0, 0)),
                },
                HeadlessOutput {
                    name: "DP-2".to_owned(),
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60,
                    scale: 2.0,
                    position: Some((2560, -120)),
                },
            ]
        );
        // The primary output is the first one listed.
        assert_eq!(config.output_name(), "DP-1");
        assert_eq!(config.output_size(), (2560, 1440));
        assert_eq!(config.refresh_mhz(), 144_000);
    }

    #[test]
    fn an_output_specification_only_has_to_name_what_it_changes() {
        let config = run_config(&["--output", "scale=1.25"]);

        assert_eq!(
            config.outputs,
            vec![HeadlessOutput {
                scale: 1.25,
                ..HeadlessOutput::default()
            }]
        );
    }

    #[test]
    fn a_position_coordinate_defaults_to_zero_when_only_one_is_given() {
        let config = run_config(&["--output", "y=1080"]);

        assert_eq!(config.outputs[0].position, Some((0, 1080)));
    }

    #[test]
    fn malformed_output_specifications_are_rejected() {
        for spec in [
            "",
            "name",
            "width=1920,",
            "name=DP-1,,scale=2",
            "name=DP-1,name=DP-2",
            "name=",
            "depth=32",
            "width=0",
            "width=16385",
            "height=-1",
            "refresh=0",
            "refresh=1001",
            "scale=0",
            "scale=0.1",
            "scale=9",
            "scale=narrow",
            "x=4194305",
            "y=-4194305",
            "x=1.5",
        ] {
            let outcome = parse_args(args(&["--output", spec]));

            assert!(
                matches!(outcome, Err(ConfigError::InvalidValue { .. })),
                "expected `--output {spec}` to be rejected, got {outcome:?}"
            );
        }
    }

    #[test]
    fn scales_and_coordinates_at_the_edge_of_their_range_are_accepted() {
        let config = run_config(&[
            "--output",
            "scale=0.25,x=4194304,y=-4194304",
            "--output",
            "scale=8",
        ]);

        assert!((config.outputs[0].scale - 0.25).abs() < f64::EPSILON);
        assert_eq!(config.outputs[0].position, Some((4_194_304, -4_194_304)));
        assert!((config.outputs[1].scale - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn duplicate_output_names_are_rejected() {
        let outcome = parse_args(args(&["--output", "name=DP-1", "--output", "name=DP-1"]));

        assert!(
            matches!(outcome, Err(ConfigError::InvalidValue { .. })),
            "expected the duplicate name to be rejected, got {outcome:?}"
        );
    }

    #[test]
    fn outputs_left_unnamed_are_numbered_in_order() {
        let config = run_config(&["--output", "scale=2", "--output", "scale=3"]);

        assert_eq!(config.outputs[0].name, "headless-1");
        assert_eq!(config.outputs[1].name, "headless-2");
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
        assert_eq!(config.refresh_hz(), 75);
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
        assert_eq!(config.refresh_hz(), 30);
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
            "--output",
            "--socket",
            "--exit-after-frames",
            "--test-input",
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

    #[cfg(feature = "test-input")]
    #[test]
    fn test_input_is_accepted_by_test_input_builds() {
        let config = run_config(&["--test-input", "/run/inox/test-input.sock"]);

        assert_eq!(
            config.test_input_path.as_deref(),
            Some(std::path::Path::new("/run/inox/test-input.sock"))
        );
        // Nothing else changes, so a test-input run stays the default headless run.
        assert_eq!(config.backend, BackendKind::Auto);

        // The inline spelling parses to the same value.
        assert_eq!(
            run_config(&["--test-input=/tmp/inox.sock"]).test_input_path,
            Some(std::path::PathBuf::from("/tmp/inox.sock"))
        );

        assert!(matches!(
            parse_args(args(&["--test-input", ""])),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[cfg(not(feature = "test-input"))]
    #[test]
    fn test_input_is_rejected_without_the_feature() {
        for invocation in [
            args(&["--test-input", "/tmp/inox.sock"]),
            args(&["--test-input=/tmp/inox.sock"]),
        ] {
            assert_eq!(
                parse_args(invocation),
                Err(ConfigError::TestInputUnsupported {
                    path: "/tmp/inox.sock".to_owned()
                })
            );
        }
    }

    #[test]
    fn help_text_mentions_every_option() {
        for option in [
            "--backend",
            "--headless",
            "--output-size",
            "--refresh-rate",
            "--output",
            "--socket",
            "--exit-after-frames",
            "--test-input",
            "--help",
            "--version",
        ] {
            assert!(HELP.contains(option), "help text omits `{option}`");
        }
    }
}
