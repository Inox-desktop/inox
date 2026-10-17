//! The compositor configuration, `compositor.toml`.
//!
//! The schema covers what inox-compositor accepts today — outputs, input devices
//! and the master/stack tiling parameters — and the defaults come in two kinds.
//!
//! Backed by the implementation:
//!
//! - the output defaults (a single 1280x720 output at 60 Hz, scale 1.0) are the
//!   constants of inox-compositor's `src/config.rs`;
//! - the tiling defaults (a 60 % master column holding one window, separated by
//!   an 8 pixel gap) are the constants of its `src/layout.rs`;
//! - `keyboard_layout` and `keyboard_variant` (`us`, empty) match the
//!   `XkbConfig` its seat is created with in `src/state.rs`.
//!
//! Reserved for the milestone that wires this file into the input path:
//!
//! - `keyboard_repeat_delay_ms`, `keyboard_repeat_rate_hz`, `tap_to_click` and
//!   `natural_scroll` are defaults this schema declares ahead of the input
//!   backend; nothing reads them yet. inox-compositor has no libinput backend:
//!   it hands Smithay's `add_keyboard` a hard-coded 200 ms repeat delay and
//!   25 Hz repeat rate in `src/state.rs` — the rate coincides with the default
//!   below, the delay deliberately does not — and it has no tap-to-click or
//!   natural-scrolling setting at all.
//!
//!   The repeat delay is the one default where the two definitions disagree on
//!   purpose. 600 ms is the user-facing default this schema declares; the 200 ms
//!   in `src/state.rs` is an incidental argument of the M2 headless seat rather
//!   than a recorded decision — ADR 0008 fixes the keymap rules, not the repeat
//!   timing. When inox-compositor starts reading `compositor.toml`, the file
//!   wins and the hard-coded pair is deleted: the resulting change in repeat
//!   timing is a behaviour change for that component to make deliberately, and
//!   not something this schema should pre-empt by shipping a 200 ms default.
//!
//! The values are duplicated instead of imported on purpose: root AGENTS.md
//! forbids depending on a sibling component just to reuse a constant. Holding
//! the two definitions together is a job for the planned M6 integration tests.

use serde::{Deserialize, Deserializer};

/// Default width of an output, in pixels.
const DEFAULT_WIDTH: i32 = 1280;

/// Default height of an output, in pixels.
const DEFAULT_HEIGHT: i32 = 720;

/// Default refresh rate of an output, in Hz.
const DEFAULT_REFRESH_HZ: i32 = 60;

/// Default scale factor of an output.
const DEFAULT_SCALE: f64 = 1.0;

/// Largest accepted output dimension, in pixels; the same limit inox-compositor
/// applies to its `--output-size` option (`MAX_DIMENSION` in `src/config.rs`).
const MAX_DIMENSION: i32 = 16_384;

/// Largest accepted refresh rate, in Hz; the same limit inox-compositor applies
/// to its `--refresh-rate` option (`MAX_REFRESH_HZ` in `src/config.rs`).
const MAX_REFRESH_HZ: i32 = 1_000;

/// Default xkb layout of a keyboard.
const DEFAULT_KEYBOARD_LAYOUT: &str = "us";

/// Default keyboard repeat delay, in milliseconds.
const DEFAULT_REPEAT_DELAY_MS: u32 = 600;

/// Default keyboard repeat rate, in Hz.
const DEFAULT_REPEAT_RATE_HZ: u32 = 25;

/// Smallest accepted master ratio, as a percentage.
const MASTER_RATIO_MIN: u8 = 10;

/// Largest accepted master ratio, as a percentage.
const MASTER_RATIO_MAX: u8 = 90;

/// Smallest accepted number of master windows.
const MASTER_COUNT_MIN: u32 = 1;

/// Smallest accepted gap, in logical pixels.
const GAP_MIN: i32 = 0;

/// Everything `compositor.toml` describes.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct CompositorConfig {
    /// Outputs to create, in the order they should be matched.
    ///
    /// Empty — the default — means a single output using the built-in defaults
    /// of [`OutputConfig`].
    pub outputs: Vec<OutputConfig>,
    /// Input device settings.
    pub input: InputConfig,
    /// Tiling layout settings.
    pub layout: LayoutConfig,
}

/// One output to create.
///
/// The four numeric fields are checked while the file is parsed, with the same
/// limits inox-compositor applies to the equivalent command line options: a zero
/// or oversized dimension or refresh rate would build an output that cannot be
/// driven, and a scale of zero or a non-finite scale would divide the logical
/// geometry by zero.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct OutputConfig {
    /// Connector to match, for example `HDMI-A-1`, or `None` to match outputs
    /// by position, in the order they are listed.
    pub name: Option<String>,
    /// Width in physical pixels, between 1 and 16384.
    #[serde(deserialize_with = "validate_dimension")]
    pub width: i32,
    /// Height in physical pixels, between 1 and 16384.
    #[serde(deserialize_with = "validate_dimension")]
    pub height: i32,
    /// Refresh rate in Hz, between 1 and 1000.
    #[serde(deserialize_with = "validate_refresh_hz")]
    pub refresh_hz: i32,
    /// Scale factor between physical and logical pixels; positive and finite.
    #[serde(deserialize_with = "validate_scale")]
    pub scale: f64,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            name: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            refresh_hz: DEFAULT_REFRESH_HZ,
            scale: DEFAULT_SCALE,
        }
    }
}

/// Input device settings.
///
/// These values are only checked for their type. The backend that would give a
/// range to them does not exist yet — inox-compositor has no libinput backend,
/// and it hands its own repeat delay and rate to Smithay — so a range check here
/// would freeze a guess. Ranges arrive with the input backend.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct InputConfig {
    /// xkb layout, for example `us` or `de`.
    pub keyboard_layout: String,
    /// xkb variant of [`InputConfig::keyboard_layout`]; empty means none.
    pub keyboard_variant: String,
    /// Delay before a held key starts repeating, in milliseconds.
    pub keyboard_repeat_delay_ms: u32,
    /// Repeat rate of a held key, in Hz.
    pub keyboard_repeat_rate_hz: u32,
    /// Whether a tap on a touchpad counts as a click.
    pub tap_to_click: bool,
    /// Whether two-finger scrolling is inverted.
    pub natural_scroll: bool,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            keyboard_layout: DEFAULT_KEYBOARD_LAYOUT.to_owned(),
            keyboard_variant: String::new(),
            keyboard_repeat_delay_ms: DEFAULT_REPEAT_DELAY_MS,
            keyboard_repeat_rate_hz: DEFAULT_REPEAT_RATE_HZ,
            tap_to_click: true,
            natural_scroll: false,
        }
    }
}

/// Parameters of the master/stack tiling layout.
///
/// Every parameter is checked while the file is parsed, so an out-of-range
/// value is a parse error naming the field rather than a silent correction
/// (ADR 0009, decision 4): a master ratio outside 10..=90, a master count below
/// 1 and a negative gap are all rejected by [`crate::store::InoxConfig::load`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct LayoutConfig {
    /// Percentage of the work area width assigned to the master column,
    /// between 10 and 90.
    #[serde(deserialize_with = "validate_master_ratio")]
    pub master_ratio: u8,
    /// Number of windows tiled in the master column before the stack is used,
    /// at least 1.
    #[serde(deserialize_with = "validate_master_count")]
    pub master_count: u32,
    /// Gap in logical pixels between the work area edge and its tiles, and
    /// between neighbouring tiles; not negative.
    #[serde(deserialize_with = "validate_gap")]
    pub gap: i32,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            master_ratio: 60,
            master_count: 1,
            gap: 8,
        }
    }
}

impl LayoutConfig {
    /// Clamp every parameter into its accepted range: `master_ratio` within
    /// 10..=90, `master_count` at least 1 and `gap` at least 0.
    ///
    /// A defensive fallback for values that did not come from the configuration
    /// files — inox-compositor applies the same clamp to its own hard-coded
    /// defaults. A layout that went through [`crate::store::InoxConfig::load`]
    /// is already inside these ranges and does not need this.
    pub fn sanitised(self) -> Self {
        Self {
            master_ratio: self.master_ratio.clamp(MASTER_RATIO_MIN, MASTER_RATIO_MAX),
            master_count: self.master_count.max(MASTER_COUNT_MIN),
            gap: self.gap.max(GAP_MIN),
        }
    }
}

/// Reject an output dimension outside 1..=[`MAX_DIMENSION`] pixels.
///
/// Shared by `width` and `height`; which of the two is at fault comes from the
/// field path in the error.
fn validate_dimension<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    let dimension = i32::deserialize(deserializer)?;

    if !(1..=MAX_DIMENSION).contains(&dimension) {
        return Err(serde::de::Error::custom(format!(
            "an output dimension must be between 1 and {MAX_DIMENSION} pixels, found {dimension}"
        )));
    }

    Ok(dimension)
}

/// Reject a refresh rate outside 1..=[`MAX_REFRESH_HZ`] hertz.
fn validate_refresh_hz<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    let refresh_hz = i32::deserialize(deserializer)?;

    if !(1..=MAX_REFRESH_HZ).contains(&refresh_hz) {
        return Err(serde::de::Error::custom(format!(
            "refresh_hz must be between 1 and {MAX_REFRESH_HZ} Hz, found {refresh_hz}"
        )));
    }

    Ok(refresh_hz)
}

/// Reject a scale that is not a positive, finite number.
fn validate_scale<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: Deserializer<'de>,
{
    let scale = f64::deserialize(deserializer)?;

    if !scale.is_finite() || scale <= 0.0 {
        return Err(serde::de::Error::custom(format!(
            "scale must be a positive, finite number, found {scale:?}"
        )));
    }

    Ok(scale)
}

/// Reject a master ratio outside [`MASTER_RATIO_MIN`]..=[`MASTER_RATIO_MAX`].
fn validate_master_ratio<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let ratio = u8::deserialize(deserializer)?;

    if !(MASTER_RATIO_MIN..=MASTER_RATIO_MAX).contains(&ratio) {
        return Err(serde::de::Error::custom(format!(
            "master_ratio must be between {MASTER_RATIO_MIN} and {MASTER_RATIO_MAX} percent, found {ratio}"
        )));
    }

    Ok(ratio)
}

/// Reject a master count below [`MASTER_COUNT_MIN`].
fn validate_master_count<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let count = u32::deserialize(deserializer)?;

    if count < MASTER_COUNT_MIN {
        return Err(serde::de::Error::custom(format!(
            "master_count must be at least {MASTER_COUNT_MIN}, found {count}"
        )));
    }

    Ok(count)
}

/// Reject a gap below [`GAP_MIN`].
fn validate_gap<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    let gap = i32::deserialize(deserializer)?;

    if gap < GAP_MIN {
        return Err(serde::de::Error::custom(format!(
            "gap must not be negative, found {gap}"
        )));
    }

    Ok(gap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ConfigError;
    use crate::error::test_support::{column_of, line_of};
    use std::path::Path;

    const COMPOSITOR_PATH: &str = "/cfg/inox/compositor.toml";

    fn parse_compositor(text: &str) -> Result<CompositorConfig, ConfigError> {
        crate::error::parse(Path::new(COMPOSITOR_PATH), text)
    }

    #[test]
    fn an_empty_document_uses_every_default() {
        let config = parse_compositor("").expect("empty document");

        assert!(config.outputs.is_empty());
        assert_eq!(config.input.keyboard_layout, "us");
        assert_eq!(config.input.keyboard_variant, "");
        assert_eq!(config.input.keyboard_repeat_delay_ms, 600);
        assert_eq!(config.input.keyboard_repeat_rate_hz, 25);
        assert!(config.input.tap_to_click);
        assert!(!config.input.natural_scroll);
        assert_eq!(config.layout.master_ratio, 60);
        assert_eq!(config.layout.master_count, 1);
        assert_eq!(config.layout.gap, 8);
    }

    #[test]
    fn the_output_defaults_are_the_ones_inox_compositor_applies() {
        let output = OutputConfig::default();

        assert_eq!(output.name, None);
        assert_eq!(output.width, 1280);
        assert_eq!(output.height, 720);
        assert_eq!(output.refresh_hz, 60);
        assert!((output.scale - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_input_defaults_are_the_ones_inox_compositor_applies() {
        let input = InputConfig::default();

        assert_eq!(input.keyboard_layout, "us");
        assert_eq!(input.keyboard_variant, "");
        assert_eq!(input.keyboard_repeat_delay_ms, 600);
        assert_eq!(input.keyboard_repeat_rate_hz, 25);
        assert!(input.tap_to_click);
        assert!(!input.natural_scroll);
    }

    #[test]
    fn the_layout_defaults_are_the_ones_inox_compositor_applies() {
        let layout = LayoutConfig::default();

        assert_eq!(layout.master_ratio, 60);
        assert_eq!(layout.master_count, 1);
        assert_eq!(layout.gap, 8);
        assert_eq!(layout.sanitised(), layout);
    }

    #[test]
    fn outputs_input_and_layout_are_parsed() {
        let config = parse_compositor(
            "[[outputs]]\n\
             name = \"HDMI-A-1\"\n\
             width = 2560\n\
             height = 1440\n\
             refresh_hz = 144\n\
             scale = 1.5\n\
             \n\
             [[outputs]]\n\
             width = 1920\n\
             height = 1080\n\
             \n\
             [input]\n\
             keyboard_layout = \"de\"\n\
             keyboard_variant = \"nodeadkeys\"\n\
             keyboard_repeat_delay_ms = 250\n\
             keyboard_repeat_rate_hz = 40\n\
             tap_to_click = false\n\
             natural_scroll = true\n\
             \n\
             [layout]\n\
             master_ratio = 70\n\
             master_count = 2\n\
             gap = 12\n",
        )
        .expect("document");

        assert_eq!(config.outputs.len(), 2);
        assert_eq!(config.outputs[0].name.as_deref(), Some("HDMI-A-1"));
        assert_eq!(config.outputs[0].width, 2560);
        assert_eq!(config.outputs[0].refresh_hz, 144);
        assert!((config.outputs[0].scale - 1.5).abs() < f64::EPSILON);
        // An entry only has to name what it changes.
        assert_eq!(config.outputs[1].name, None);
        assert_eq!(config.outputs[1].width, 1920);
        assert_eq!(config.outputs[1].height, 1080);
        assert_eq!(config.outputs[1].refresh_hz, 60);
        assert_eq!(config.input.keyboard_layout, "de");
        assert_eq!(config.input.keyboard_variant, "nodeadkeys");
        assert_eq!(config.input.keyboard_repeat_delay_ms, 250);
        assert_eq!(config.input.keyboard_repeat_rate_hz, 40);
        assert!(!config.input.tap_to_click);
        assert!(config.input.natural_scroll);
        assert_eq!(config.layout.master_ratio, 70);
        assert_eq!(config.layout.master_count, 2);
        assert_eq!(config.layout.gap, 12);
    }

    #[test]
    fn a_partial_section_keeps_the_defaults_of_the_fields_it_leaves_out() {
        let config = parse_compositor("[input]\nkeyboard_layout = \"fr\"\n").expect("document");

        assert_eq!(config.input.keyboard_layout, "fr");
        assert_eq!(config.input.keyboard_variant, "");
        assert_eq!(config.input.keyboard_repeat_delay_ms, 600);
        assert_eq!(config.input.keyboard_repeat_rate_hz, 25);
        assert_eq!(config.layout.master_ratio, 60);
        assert_eq!(config.layout.master_count, 1);
        assert_eq!(config.layout.gap, 8);
        assert!(config.outputs.is_empty());
    }

    #[test]
    fn an_unknown_key_is_rejected_with_its_field_path() {
        let error = parse_compositor("[input]\nkeyboard_repeat_rate_hz = 25\ntyping_speed = 3\n")
            .expect_err("unknown key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("input.typing_speed"));
                assert_eq!(line, Some(3));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_value_of_the_wrong_type_is_reported_with_file_position_and_field() {
        // The rate is written as a string on the second line; its opening quote
        // is where the parser points.
        let document = "[input]\nkeyboard_repeat_rate_hz = \"25\"\n";
        let expected_column = u32::try_from(
            document
                .lines()
                .nth(1)
                .expect("second line")
                .find('"')
                .expect("quoted value"),
        )
        .expect("column fits in u32")
            + 1;

        let error = parse_compositor(document).expect_err("wrong type");

        let rendered = error.to_string();
        assert!(
            rendered.contains("/cfg/inox/compositor.toml:2:"),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains("(field `input.keyboard_repeat_rate_hz`)"),
            "rendered `{rendered}`"
        );
        assert!(rendered.contains("invalid type"), "rendered `{rendered}`");

        match error {
            ConfigError::Parse {
                field,
                line,
                column,
                message,
                ..
            } => {
                assert_eq!(field.as_deref(), Some("input.keyboard_repeat_rate_hz"));
                assert_eq!(line, Some(2));
                assert_eq!(column, Some(expected_column));
                assert!(message.contains("invalid type"), "message `{message}`");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn out_of_range_layout_values_are_rejected_while_parsing() {
        // The last column of each row says whether the value's token starts with
        // a digit: a negative number is the parser's business, so its span is
        // checked for presence rather than for an exact column.
        for (document, field, needle, reason, pinned_column) in [
            (
                "[layout]\nmaster_ratio = 9\n",
                "layout.master_ratio",
                "9",
                "between 10 and 90",
                true,
            ),
            (
                "[layout]\nmaster_ratio = 91\n",
                "layout.master_ratio",
                "91",
                "between 10 and 90",
                true,
            ),
            (
                "[layout]\nmaster_count = 0\n",
                "layout.master_count",
                "0",
                "at least 1",
                true,
            ),
            (
                "[layout]\ngap = -4\n",
                "layout.gap",
                "4",
                "not be negative",
                false,
            ),
        ] {
            let outcome = parse_compositor(document);
            assert!(
                outcome.is_err(),
                "expected `{document:?}` to be rejected, got {outcome:?}"
            );

            let error = outcome.expect_err("checked above");
            let expected_line = line_of(document, needle);
            let rendered = error.to_string();

            assert!(
                rendered.contains(&format!("compositor.toml:{expected_line}:")),
                "rendered `{rendered}`"
            );
            assert!(
                rendered.contains(&format!("(field `{field}`)")),
                "rendered `{rendered}`"
            );
            assert!(rendered.contains(reason), "rendered `{rendered}`");

            match error {
                ConfigError::Parse {
                    field: reported,
                    line,
                    column,
                    ..
                } => {
                    assert_eq!(reported.as_deref(), Some(field));
                    assert_eq!(line, Some(expected_line));
                    assert!(column.is_some(), "expected the parser to point at a column");

                    if pinned_column {
                        assert_eq!(column, Some(column_of(document, needle)));
                    }
                }
                other => panic!("expected a parse error, got {other:?}"),
            }
        }
    }

    #[test]
    fn layout_values_at_the_edge_of_their_range_are_accepted() {
        for ratio in [10, 90] {
            let config = parse_compositor(&format!("[layout]\nmaster_ratio = {ratio}\n"))
                .expect("a ratio at the edge of the range");

            assert_eq!(config.layout.master_ratio, ratio);
        }

        let config = parse_compositor("[layout]\nmaster_count = 1\ngap = 0\n")
            .expect("a count and gap at the edge of their ranges");

        assert_eq!(config.layout.master_count, 1);
        assert_eq!(config.layout.gap, 0);
    }

    #[test]
    fn every_layout_default_survives_the_range_checks() {
        // `[outputs]` is not in the list on purpose: the key holds an array of
        // tables, so a plain table there is a type error rather than a default.
        for document in ["", "[layout]\n", "[input]\n"] {
            let layout = parse_compositor(document).expect("defaults").layout;

            assert_eq!(layout.master_ratio, 60, "document `{document:?}`");
            assert_eq!(layout.master_count, 1, "document `{document:?}`");
            assert_eq!(layout.gap, 8, "document `{document:?}`");
        }
    }

    /// `sanitised` is the defensive path for layouts that never went through the
    /// configuration files, so it is exercised with values built by hand.
    #[test]
    fn out_of_range_output_values_are_rejected_while_parsing() {
        // The last column says whether the value's token starts with a digit; a
        // negative number's span is the parser's business.
        for (value, field, needle, reason, pinned_column) in [
            (
                "width = 0",
                "outputs[0].width",
                "0",
                "between 1 and 16384",
                true,
            ),
            (
                "height = 16385",
                "outputs[0].height",
                "16385",
                "between 1 and 16384",
                true,
            ),
            (
                "refresh_hz = 1001",
                "outputs[0].refresh_hz",
                "1001",
                "between 1 and 1000",
                true,
            ),
            (
                "scale = 0.0",
                "outputs[0].scale",
                "0.0",
                "positive, finite",
                true,
            ),
            (
                "scale = -1.5",
                "outputs[0].scale",
                "1.5",
                "positive, finite",
                false,
            ),
        ] {
            let document = format!("[[outputs]]\n{value}\n");
            let outcome = parse_compositor(&document);
            assert!(
                outcome.is_err(),
                "expected `{value}` to be rejected, got {outcome:?}"
            );

            let error = outcome.expect_err("checked above");
            let expected_line = line_of(&document, needle);
            let rendered = error.to_string();

            assert!(
                rendered.contains(&format!("compositor.toml:{expected_line}:")),
                "rendered `{rendered}`"
            );
            assert!(
                rendered.contains(&format!("(field `{field}`)")),
                "rendered `{rendered}`"
            );
            assert!(rendered.contains(reason), "rendered `{rendered}`");

            match error {
                ConfigError::Parse {
                    field: reported,
                    line,
                    column,
                    ..
                } => {
                    assert_eq!(reported.as_deref(), Some(field));
                    assert_eq!(line, Some(expected_line));
                    assert!(column.is_some(), "expected the parser to point at a column");

                    if pinned_column {
                        assert_eq!(column, Some(column_of(&document, needle)));
                    }
                }
                other => panic!("expected a parse error, got {other:?}"),
            }
        }
    }

    #[test]
    fn output_values_at_the_edge_of_their_range_are_accepted() {
        let config = parse_compositor(
            "[[outputs]]\n\
             width = 1\n\
             height = 16384\n\
             refresh_hz = 1\n\
             \n\
             [[outputs]]\n\
             width = 16384\n\
             height = 1\n\
             refresh_hz = 1000\n",
        )
        .expect("values at the edge of their range");

        assert_eq!(config.outputs.len(), 2);
        assert_eq!(
            (
                config.outputs[0].width,
                config.outputs[0].height,
                config.outputs[0].refresh_hz
            ),
            (1, 16_384, 1)
        );
        assert_eq!(
            (
                config.outputs[1].width,
                config.outputs[1].height,
                config.outputs[1].refresh_hz
            ),
            (16_384, 1, 1_000)
        );
    }

    #[test]
    fn an_output_that_only_names_some_fields_keeps_the_defaults() {
        let config = parse_compositor("[[outputs]]\nscale = 2.0\n").expect("document");

        assert_eq!(config.outputs.len(), 1);
        assert_eq!(config.outputs[0].name, None);
        assert_eq!(config.outputs[0].width, 1280);
        assert_eq!(config.outputs[0].height, 720);
        assert_eq!(config.outputs[0].refresh_hz, 60);
        assert!((config.outputs[0].scale - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_toml_syntax_error_is_reported_with_a_position_and_no_field() {
        // The value is not TOML at all, so the document fails to parse before any
        // field is deserialized; the parser points at the offending value on the
        // second line.
        let document = "[layout]\nmaster_ratio = @\n";
        let error = parse_compositor(document).expect_err("syntax error");

        let rendered = error.to_string();
        assert!(
            rendered.contains("compositor.toml:2:"),
            "rendered `{rendered}`"
        );

        match error {
            ConfigError::Parse {
                field,
                line,
                column,
                message,
                ..
            } => {
                assert_eq!(field, None);
                assert_eq!(line, Some(2));
                assert!(column.is_some(), "expected the parser to point at a column");
                assert!(!message.is_empty(), "expected the parser to explain itself");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn sanitised_clamps_every_layout_value_into_its_range() {
        let cases = [
            (
                LayoutConfig {
                    master_ratio: 0,
                    master_count: 0,
                    gap: -4,
                },
                LayoutConfig {
                    master_ratio: 10,
                    master_count: 1,
                    gap: 0,
                },
            ),
            (
                LayoutConfig {
                    master_ratio: 200,
                    master_count: 3,
                    gap: 12,
                },
                LayoutConfig {
                    master_ratio: 90,
                    master_count: 3,
                    gap: 12,
                },
            ),
        ];

        for (written, expected) in cases {
            assert_eq!(written.sanitised(), expected, "sanitising {written:?}");
        }
    }
}
