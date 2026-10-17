//! Design tokens and the built-in themes (PRD §9.2).
//!
//! A theme is a flat set of tokens — colours, typography, spacing, radii and
//! motion durations — which is what makes it usable as the single source of
//! truth for the components: root AGENTS.md §4 forbids hard-coding colours or
//! sizes in a component, so every component consumes these tokens instead.
//!
//! Two themes ship with Inox, [`BUILTIN_LIGHT`] and [`BUILTIN_DARK`]; the dark
//! one is also the [`Default`] of [`Theme`], so a theme file only has to describe
//! what it changes. A user theme lives in `themes/<name>/theme.toml` and is
//! selected by [`crate::root::ThemeSelection`], whose [`resolve`] method is the
//! only supported way to turn a selection into a [`Theme`]:
//!
//! [`resolve`]: crate::root::ThemeSelection::resolve

use serde::Deserialize;
use tracing::debug;

use crate::error::{ConfigError, parse, read_text};
use crate::paths::ConfigPaths;
use crate::root::{Appearance, ThemeSelection};

/// Name of the built-in light theme.
pub const BUILTIN_LIGHT: &str = "inox-light";

/// Name of the built-in dark theme. It is also the [`Default`] theme.
pub const BUILTIN_DARK: &str = "inox-dark";

/// An sRGB colour with an alpha channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Rgba {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel; `0xff` is opaque.
    pub a: u8,
}

impl Rgba {
    /// Parse the `#RRGGBB` or `#RRGGBBAA` form used by every colour token.
    ///
    /// Hexadecimal digits may be written in either case, and an absent alpha
    /// channel means opaque (`0xff`).
    ///
    /// # Errors
    ///
    /// Returns [`ColorError::InvalidHex`] for anything else, including a missing
    /// `#`, a length other than six or eight digits, and non-hexadecimal digits.
    pub fn from_hex(value: &str) -> Result<Self, ColorError> {
        let digits = value
            .strip_prefix('#')
            .ok_or_else(|| ColorError::InvalidHex {
                value: value.to_owned(),
            })?;

        let invalid = || ColorError::InvalidHex {
            value: value.to_owned(),
        };

        if !matches!(digits.len(), 6 | 8) {
            return Err(invalid());
        }

        Ok(Self {
            r: hex_pair(digits, 0).ok_or_else(invalid)?,
            g: hex_pair(digits, 2).ok_or_else(invalid)?,
            b: hex_pair(digits, 4).ok_or_else(invalid)?,
            a: match digits.len() {
                8 => hex_pair(digits, 6).ok_or_else(invalid)?,
                _ => 0xff,
            },
        })
    }
}

impl TryFrom<String> for Rgba {
    type Error = ColorError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_hex(&value)
    }
}

/// Failure to read a colour token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ColorError {
    /// The literal is not one of the accepted forms.
    #[error("invalid colour `{value}`: expected `#RRGGBB` or `#RRGGBBAA` in hexadecimal")]
    InvalidHex {
        /// The rejected literal.
        value: String,
    },
}

/// Parse the two hexadecimal digits at byte `index`, or `None` when they are
/// missing or are not both hexadecimal digits.
fn hex_pair(digits: &str, index: usize) -> Option<u8> {
    let pair = digits.get(index..index + 2)?;

    if !pair.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }

    u8::from_str_radix(pair, 16).ok()
}

/// Colour tokens of a theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Colors {
    /// Colour behind the content of a window.
    pub background: Rgba,
    /// Colour of raised surfaces such as panels and cards.
    pub surface: Rgba,
    /// Alternate surface colour, for example a hovered list row.
    pub surface_variant: Rgba,
    /// Separator and outline colour.
    pub border: Rgba,
    /// Default text colour.
    pub text: Rgba,
    /// Text colour of secondary information.
    pub text_muted: Rgba,
    /// Interactive accent colour.
    pub accent: Rgba,
    /// Text or icon colour drawn on top of [`Colors::accent`].
    pub on_accent: Rgba,
    /// Colour signalling a failed or destructive action.
    pub danger: Rgba,
    /// Colour signalling a caution.
    pub warning: Rgba,
    /// Colour signalling a successful or safe action.
    pub success: Rgba,
}

impl Default for Colors {
    fn default() -> Self {
        DARK_COLORS
    }
}

/// Typography tokens of a theme.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Typography {
    /// Font family for prose.
    pub font_family: String,
    /// Font family for code and monospaced output.
    pub font_mono_family: String,
    /// Multiplier applied to the user's font size.
    pub scale: f32,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            font_family: "Noto Sans".to_owned(),
            font_mono_family: "JetBrains Mono".to_owned(),
            scale: 1.0,
        }
    }
}

/// Spacing tokens, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Spacing {
    /// Extra small step, for the gap inside a control.
    pub xs: u16,
    /// Small step.
    pub sm: u16,
    /// Medium step, the default gap between controls.
    pub md: u16,
    /// Large step, the default padding of a container.
    pub lg: u16,
    /// Extra large step, for the margin around a group.
    pub xl: u16,
}

impl Default for Spacing {
    fn default() -> Self {
        Self {
            xs: 2,
            sm: 4,
            md: 8,
            lg: 16,
            xl: 32,
        }
    }
}

/// Corner-radius tokens, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Radii {
    /// Small radius, for controls.
    pub sm: u16,
    /// Medium radius, the default.
    pub md: u16,
    /// Large radius, for windows and sheets.
    pub lg: u16,
}

impl Default for Radii {
    fn default() -> Self {
        Self {
            sm: 4,
            md: 8,
            lg: 12,
        }
    }
}

/// Motion-duration tokens, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Motion {
    /// Duration of a small state change, such as a hover.
    pub fast_ms: u32,
    /// Default duration, for a transition between two states.
    pub normal_ms: u32,
    /// Duration of an entering or leaving surface.
    pub slow_ms: u32,
}

impl Default for Motion {
    fn default() -> Self {
        Self {
            fast_ms: 100,
            normal_ms: 200,
            slow_ms: 400,
        }
    }
}

/// A complete set of design tokens.
///
/// Deserializing a [`Theme`] directly starts from [`Theme::builtin_dark`], name
/// and appearance included. [`ThemeSelection::resolve`] is the presence-aware
/// loader used by the configuration: it keeps the selection's name and
/// appearance when the theme file does not declare its own.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Theme {
    /// Name of the theme.
    pub name: String,
    /// Appearance the theme was designed for.
    pub appearance: Appearance,
    /// Colour tokens.
    pub colors: Colors,
    /// Typography tokens.
    pub typography: Typography,
    /// Spacing tokens.
    pub spacing: Spacing,
    /// Corner-radius tokens.
    pub radii: Radii,
    /// Motion-duration tokens.
    pub motion: Motion,
}

impl Default for Theme {
    /// The dark theme; see [`Theme::builtin_dark`].
    fn default() -> Self {
        Self::builtin_dark()
    }
}

impl Theme {
    /// The built-in light theme, [`BUILTIN_LIGHT`].
    pub fn builtin_light() -> Self {
        Self {
            name: BUILTIN_LIGHT.to_owned(),
            appearance: Appearance::Light,
            colors: LIGHT_COLORS,
            ..Self::builtin_dark()
        }
    }

    /// The built-in dark theme, [`BUILTIN_DARK`]; also the default theme.
    pub fn builtin_dark() -> Self {
        Self {
            name: BUILTIN_DARK.to_owned(),
            appearance: Appearance::Dark,
            colors: DARK_COLORS,
            typography: Typography::default(),
            spacing: Spacing::default(),
            radii: Radii::default(),
            motion: Motion::default(),
        }
    }

    /// The built-in theme called `name`, or `None` when no built-in carries that
    /// name.
    pub fn builtin(name: &str) -> Option<Self> {
        match name {
            BUILTIN_LIGHT => Some(Self::builtin_light()),
            BUILTIN_DARK => Some(Self::builtin_dark()),
            _ => None,
        }
    }

    /// The built-in theme designed for `appearance`.
    fn builtin_for(appearance: Appearance) -> Self {
        match appearance {
            Appearance::Light => Self::builtin_light(),
            Appearance::Dark | Appearance::System => Self::builtin_dark(),
        }
    }
}

/// The appearance a theme file may declare.
///
/// A theme describes one palette, so it has to pick a side: unlike
/// `[theme] appearance` in `config.toml`, `system` is not a value a theme file
/// can hold. Rejecting it here keeps [`Theme::appearance`] meaningful for the
/// components that read it to decide between light and dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThemeAppearance {
    /// Light colours.
    Light,
    /// Dark colours.
    Dark,
}

impl From<ThemeAppearance> for Appearance {
    fn from(appearance: ThemeAppearance) -> Self {
        match appearance {
            ThemeAppearance::Light => Self::Light,
            ThemeAppearance::Dark => Self::Dark,
        }
    }
}

/// A theme file as it is written on disk.
///
/// It differs from [`Theme`] in two ways: the identity is optional, so that a
/// file which does not declare a name or an appearance inherits the ones from
/// the selection instead of the dark defaults, and the appearance is restricted
/// to the two a theme can actually describe (see [`ThemeAppearance`]).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct ThemeFile {
    name: Option<String>,
    appearance: Option<ThemeAppearance>,
    colors: Colors,
    typography: Typography,
    spacing: Spacing,
    radii: Radii,
    motion: Motion,
}

impl Default for ThemeFile {
    fn default() -> Self {
        let dark = Theme::builtin_dark();

        Self {
            name: None,
            appearance: None,
            colors: dark.colors,
            typography: dark.typography,
            spacing: dark.spacing,
            radii: dark.radii,
            motion: dark.motion,
        }
    }
}

impl ThemeFile {
    /// Fill in the identity the file left out from the selection.
    fn materialise(self, name: &str, appearance: Appearance) -> Theme {
        Theme {
            name: self.name.unwrap_or_else(|| name.to_owned()),
            appearance: self.appearance.map_or(appearance, Appearance::from),
            colors: self.colors,
            typography: self.typography,
            spacing: self.spacing,
            radii: self.radii,
            motion: self.motion,
        }
    }
}

impl ThemeSelection {
    /// Resolve the selection into a complete theme.
    ///
    /// A name that matches a built-in theme wins, so `inox-light` and `inox-dark`
    /// never touch the disk. Any other name is read from
    /// `themes/<name>/theme.toml`: a file that declares its own `name` or
    /// `appearance` overrides the selection, and everything it leaves out falls
    /// back to the dark tokens. A theme file may only declare `light` or `dark`;
    /// `system` belongs to this selection and is a parse error there. Without a
    /// name, the built-in theme for the effective appearance is used.
    ///
    /// `system` is the appearance the session reports. It is used when
    /// `appearance` is [`Appearance::System`]; a session that reports `system`
    /// as well leaves nothing to follow, so that case falls back to
    /// [`Appearance::Dark`].
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidThemeName`] for a name that is not a single
    /// plain directory name, [`ConfigError::ThemeNotFound`] when the selected
    /// theme has no file, and [`ConfigError::Read`] or [`ConfigError::Parse`]
    /// when its file exists but cannot be used.
    pub fn resolve(&self, paths: &ConfigPaths, system: Appearance) -> Result<Theme, ConfigError> {
        let appearance = effective_appearance(self.appearance, system);

        let selected = match self.name.as_deref() {
            Some(name) => name,
            None => return Ok(Theme::builtin_for(appearance)),
        };

        if let Some(builtin) = Theme::builtin(selected) {
            return Ok(builtin);
        }

        let path = paths.theme_file(selected)?;

        match read_text(&path)? {
            Some(text) => {
                let file: ThemeFile = parse(&path, &text)?;

                Ok(file.materialise(selected, appearance))
            }
            None => Err(ConfigError::ThemeNotFound {
                name: selected.to_owned(),
                path,
            }),
        }
    }
}

/// The appearance to use, given what the component asked for and what the
/// session reports.
fn effective_appearance(selected: Appearance, system: Appearance) -> Appearance {
    match (selected, system) {
        (Appearance::System, Appearance::System) => {
            debug!("the session reports `system` as its appearance; using dark");

            Appearance::Dark
        }
        (Appearance::System, reported) => reported,
        (selected, _) => selected,
    }
}

/// Build an opaque colour.
const fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    Rgba { r, g, b, a: 0xff }
}

/// The default (dark) palette.
const DARK_COLORS: Colors = Colors {
    background: rgb(0x1b, 0x1d, 0x23),
    surface: rgb(0x23, 0x26, 0x2e),
    surface_variant: rgb(0x2c, 0x31, 0x3b),
    border: rgb(0x3a, 0x41, 0x50),
    text: rgb(0xe6, 0xe9, 0xef),
    text_muted: rgb(0xa5, 0xad, 0xba),
    accent: rgb(0x4c, 0x8b, 0xf5),
    on_accent: rgb(0xff, 0xff, 0xff),
    danger: rgb(0xe5, 0x53, 0x4b),
    warning: rgb(0xe0, 0xa3, 0x3e),
    success: rgb(0x4c, 0xaf, 0x7d),
};

/// The light palette.
const LIGHT_COLORS: Colors = Colors {
    background: rgb(0xf5, 0xf6, 0xf8),
    surface: rgb(0xff, 0xff, 0xff),
    surface_variant: rgb(0xec, 0xee, 0xf2),
    border: rgb(0xd6, 0xda, 0xe1),
    text: rgb(0x1b, 0x1d, 0x23),
    text_muted: rgb(0x5c, 0x64, 0x72),
    accent: rgb(0x2f, 0x6f, 0xe0),
    on_accent: rgb(0xff, 0xff, 0xff),
    danger: rgb(0xc9, 0x40, 0x3a),
    warning: rgb(0xb7, 0x79, 0x1f),
    success: rgb(0x2f, 0x85, 0x5a),
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    /// Directory that exists but holds no configuration at all.
    fn empty_paths() -> ConfigPaths {
        ConfigPaths::new("/nonexistent/inox")
    }

    fn selection(name: Option<&str>, appearance: Appearance) -> ThemeSelection {
        ThemeSelection {
            name: name.map(str::to_owned),
            appearance,
        }
    }

    fn parse_theme_file(text: &str) -> Result<ThemeFile, ConfigError> {
        parse(Path::new("/cfg/inox/themes/mine/theme.toml"), text)
    }

    fn write_theme(dir: &TempDir, name: &str, text: &str) {
        let directory = dir.path().join("themes").join(name);
        std::fs::create_dir_all(&directory).expect("create theme directory");
        std::fs::write(directory.join("theme.toml"), text).expect("write theme file");
    }

    /// Parse a colour literal that the test expects to be valid.
    fn hex(value: &str) -> Rgba {
        Rgba::from_hex(value).expect("valid hexadecimal colour")
    }

    #[test]
    fn the_dark_theme_is_the_default_and_matches_the_documented_tokens() {
        let theme = Theme::default();

        assert_eq!(theme.name, BUILTIN_DARK);
        assert_eq!(theme.appearance, Appearance::Dark);

        let expected = Colors {
            background: hex("#1b1d23"),
            surface: hex("#23262e"),
            surface_variant: hex("#2c313b"),
            border: hex("#3a4150"),
            text: hex("#e6e9ef"),
            text_muted: hex("#a5adba"),
            accent: hex("#4c8bf5"),
            on_accent: hex("#ffffff"),
            danger: hex("#e5534b"),
            warning: hex("#e0a33e"),
            success: hex("#4caf7d"),
        };

        assert_eq!(theme.colors, expected);
        assert_eq!(Colors::default(), expected);
    }

    #[test]
    fn the_light_theme_matches_the_documented_tokens() {
        let theme = Theme::builtin_light();

        assert_eq!(theme.name, BUILTIN_LIGHT);
        assert_eq!(theme.appearance, Appearance::Light);

        let expected = Colors {
            background: hex("#f5f6f8"),
            surface: hex("#ffffff"),
            surface_variant: hex("#eceef2"),
            border: hex("#d6dae1"),
            text: hex("#1b1d23"),
            text_muted: hex("#5c6472"),
            accent: hex("#2f6fe0"),
            on_accent: hex("#ffffff"),
            danger: hex("#c9403a"),
            warning: hex("#b7791f"),
            success: hex("#2f855a"),
        };

        assert_eq!(theme.colors, expected);
    }

    #[test]
    fn both_built_in_themes_share_their_non_colour_tokens() {
        let light = Theme::builtin_light();
        let dark = Theme::builtin_dark();

        for theme in [&light, &dark] {
            assert_eq!(
                theme.typography,
                Typography {
                    font_family: "Noto Sans".to_owned(),
                    font_mono_family: "JetBrains Mono".to_owned(),
                    scale: 1.0,
                }
            );
            assert_eq!(
                theme.spacing,
                Spacing {
                    xs: 2,
                    sm: 4,
                    md: 8,
                    lg: 16,
                    xl: 32,
                }
            );
            assert_eq!(
                theme.radii,
                Radii {
                    sm: 4,
                    md: 8,
                    lg: 12,
                }
            );
            assert_eq!(
                theme.motion,
                Motion {
                    fast_ms: 100,
                    normal_ms: 200,
                    slow_ms: 400,
                }
            );
        }
    }

    #[test]
    fn built_in_themes_are_looked_up_by_name() {
        assert_eq!(Theme::builtin(BUILTIN_LIGHT), Some(Theme::builtin_light()));
        assert_eq!(Theme::builtin(BUILTIN_DARK), Some(Theme::builtin_dark()));
        assert_eq!(Theme::builtin("mine"), None);
        assert_eq!(Theme::builtin(""), None);
    }

    #[test]
    fn both_hexadecimal_forms_are_accepted() {
        assert_eq!(
            hex("#1b2c3d"),
            Rgba {
                r: 0x1b,
                g: 0x2c,
                b: 0x3d,
                a: 0xff,
            }
        );
        assert_eq!(
            hex("#1b2c3d80"),
            Rgba {
                r: 0x1b,
                g: 0x2c,
                b: 0x3d,
                a: 0x80,
            }
        );
        assert_eq!(hex("#00000000").a, 0x00);
        assert_eq!(hex("#1B2C3D").r, 0x1b);
    }

    #[test]
    fn malformed_hexadecimal_literals_are_rejected() {
        for value in [
            "",
            "#",
            "1b2c3d",
            "#1b2c3",
            "#1b2c3d4",
            "#1b2c3d4e5",
            "#gg2c3dxx",
            "#1b2c3dzz",
            "#1b2c3dé",
            " #1b2c3d",
            "#1b2c3d\n",
        ] {
            match Rgba::from_hex(value) {
                Err(ColorError::InvalidHex { value: rejected }) => {
                    assert_eq!(
                        rejected, value,
                        "the error must name the literal it rejected"
                    );
                }
                other => panic!("expected `{value}` to be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_rejected_literal_names_the_colour_and_the_accepted_forms() {
        let error = Rgba::from_hex("#zzz").expect_err("not hexadecimal");

        assert_eq!(
            error,
            ColorError::InvalidHex {
                value: "#zzz".to_owned()
            }
        );

        let rendered = error.to_string();

        assert!(rendered.contains("`#zzz`"), "rendered `{rendered}`");
        assert!(rendered.contains("#RRGGBBAA"), "rendered `{rendered}`");
    }

    #[test]
    fn a_colour_token_is_deserialized_through_its_hexadecimal_form() {
        #[derive(Debug, Deserialize)]
        struct Token {
            accent: Rgba,
        }

        let token: Token = parse(Path::new("token.toml"), "accent = \"#4c8bf5\"\n").expect("token");

        assert_eq!(token.accent, hex("#4c8bf5"));

        let error = parse::<Token>(Path::new("token.toml"), "accent = \"#zzz\"\n")
            .expect_err("not hexadecimal");

        assert!(error.to_string().contains("#zzz"), "{error}");
    }

    #[test]
    fn a_partial_theme_file_inherits_the_dark_tokens() {
        let file =
            parse_theme_file("colors = { background = \"#101010\" }\n").expect("partial file");
        let theme = file.materialise("mine", Appearance::Light);

        assert_eq!(theme.colors.background, hex("#101010"));
        assert_eq!(theme.colors.text, hex("#e6e9ef"));
        assert_eq!(theme.colors.text_muted, hex("#a5adba"));
        assert_eq!(theme.colors.accent, hex("#4c8bf5"));
        assert_eq!(
            theme.typography,
            Typography {
                font_family: "Noto Sans".to_owned(),
                font_mono_family: "JetBrains Mono".to_owned(),
                scale: 1.0,
            }
        );
        assert_eq!(
            theme.spacing,
            Spacing {
                xs: 2,
                sm: 4,
                md: 8,
                lg: 16,
                xl: 32,
            }
        );
        assert_eq!(
            theme.radii,
            Radii {
                sm: 4,
                md: 8,
                lg: 12,
            }
        );
        assert_eq!(
            theme.motion,
            Motion {
                fast_ms: 100,
                normal_ms: 200,
                slow_ms: 400,
            }
        );

        // The file declares no identity, so the selection's is kept.
        assert_eq!(theme.name, "mine");
        assert_eq!(theme.appearance, Appearance::Light);
    }

    #[test]
    fn a_theme_file_may_declare_its_own_identity() {
        let file = parse_theme_file("name = \"declared\"\nappearance = \"light\"\n")
            .expect("identity only");
        let theme = file.materialise("selected", Appearance::Dark);

        assert_eq!(theme.name, "declared");
        assert_eq!(theme.appearance, Appearance::Light);
        assert_eq!(theme.colors.background, hex("#1b1d23"));
        assert_eq!(theme.colors.accent, hex("#4c8bf5"));
    }

    #[test]
    fn a_theme_file_cannot_ask_for_the_system_appearance() {
        let error =
            parse_theme_file("appearance = \"system\"\n").expect_err("`system` is not a theme");

        match error {
            ConfigError::Parse {
                field,
                line,
                message,
                ..
            } => {
                assert_eq!(field.as_deref(), Some("appearance"));
                assert_eq!(line, Some(1));
                assert!(message.contains("system"), "message `{message}`");
                assert!(message.contains("light"), "message `{message}`");
                assert!(message.contains("dark"), "message `{message}`");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_theme_file_appearance_overrides_the_selection() {
        for (declared, expected) in [("light", Appearance::Light), ("dark", Appearance::Dark)] {
            let file = parse_theme_file(&format!("appearance = \"{declared}\"\n"))
                .expect("a declared appearance");

            // The selection asked for the other one; the file wins.
            let other = if expected == Appearance::Light {
                Appearance::Dark
            } else {
                Appearance::Light
            };

            assert_eq!(file.materialise("mine", other).appearance, expected);
        }
    }

    #[test]
    fn an_empty_theme_file_is_the_dark_theme() {
        let theme: Theme = parse(Path::new("theme.toml"), "").expect("empty theme");

        assert_eq!(theme.name, BUILTIN_DARK);
        assert_eq!(theme.appearance, Appearance::Dark);
        assert_eq!(theme.colors.background, hex("#1b1d23"));
        assert_eq!(theme.colors.accent, hex("#4c8bf5"));
        assert_eq!(theme.spacing.md, 8);
        assert_eq!(theme.motion.normal_ms, 200);
    }

    #[test]
    fn an_unknown_token_is_rejected_with_its_field_path() {
        let error = parse_theme_file("colours = { accent = \"#ffffff\" }\n").expect_err("typo");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("colours"));
                assert_eq!(line, Some(1));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_named_built_in_is_resolved_without_touching_the_disk() {
        // The configuration directory does not exist, which proves the built-in
        // was used instead of a file.
        let theme = selection(Some(BUILTIN_LIGHT), Appearance::System)
            .resolve(&empty_paths(), Appearance::Dark)
            .expect("built-in theme");

        assert_eq!(theme, Theme::builtin_light());
    }

    #[test]
    fn without_a_name_the_built_in_of_the_effective_appearance_is_used() {
        for (requested, reported, expected) in [
            // An explicit appearance wins over what the session reports.
            (Appearance::Light, Appearance::Dark, Appearance::Light),
            (Appearance::Dark, Appearance::Light, Appearance::Dark),
            // `system` follows the session.
            (Appearance::System, Appearance::Light, Appearance::Light),
            (Appearance::System, Appearance::Dark, Appearance::Dark),
            // Nothing left to follow.
            (Appearance::System, Appearance::System, Appearance::Dark),
        ] {
            let theme = selection(None, requested)
                .resolve(&empty_paths(), reported)
                .expect("built-in theme");

            assert_eq!(theme.appearance, expected);
            assert_eq!(
                theme.name,
                if expected == Appearance::Light {
                    BUILTIN_LIGHT
                } else {
                    BUILTIN_DARK
                }
            );
        }
    }

    #[test]
    fn a_named_theme_is_read_from_the_themes_directory() {
        let dir = tempfile::tempdir().expect("temp directory");
        write_theme(&dir, "mine", "colors = { accent = \"#ff0000\" }\n");

        let theme = selection(Some("mine"), Appearance::Dark)
            .resolve(&ConfigPaths::new(dir.path()), Appearance::System)
            .expect("custom theme");

        assert_eq!(theme.name, "mine");
        assert_eq!(theme.appearance, Appearance::Dark);
        assert_eq!(theme.colors.accent, hex("#ff0000"));
        assert_eq!(theme.colors.background, hex("#1b1d23"));
    }

    #[test]
    fn a_named_theme_may_declare_its_own_identity() {
        let dir = tempfile::tempdir().expect("temp directory");
        write_theme(&dir, "mine", "name = \"other\"\nappearance = \"light\"\n");

        let theme = selection(Some("mine"), Appearance::Dark)
            .resolve(&ConfigPaths::new(dir.path()), Appearance::System)
            .expect("custom theme");

        assert_eq!(theme.name, "other");
        assert_eq!(theme.appearance, Appearance::Light);
    }

    #[test]
    fn a_theme_without_a_name_follows_the_session() {
        let dir = tempfile::tempdir().expect("temp directory");
        write_theme(&dir, "mine", "colors = { accent = \"#00ff00\" }\n");

        let theme = selection(Some("mine"), Appearance::System)
            .resolve(&ConfigPaths::new(dir.path()), Appearance::Light)
            .expect("custom theme");

        assert_eq!(theme.name, "mine");
        assert_eq!(theme.appearance, Appearance::Light);
    }

    #[test]
    fn a_missing_theme_names_the_file_it_looked_for() {
        let dir = tempfile::tempdir().expect("temp directory");
        let paths = ConfigPaths::new(dir.path());
        let error = selection(Some("absent"), Appearance::System)
            .resolve(&paths, Appearance::System)
            .expect_err("missing theme");

        assert!(
            matches!(
                &error,
                ConfigError::ThemeNotFound { name, path }
                    if name == "absent" && path.ends_with("themes/absent/theme.toml")
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn a_malformed_theme_is_reported_with_file_line_and_field() {
        let dir = tempfile::tempdir().expect("temp directory");
        write_theme(&dir, "mine", "colors = { accent = \"red\" }\n");

        let error = selection(Some("mine"), Appearance::System)
            .resolve(&ConfigPaths::new(dir.path()), Appearance::System)
            .expect_err("malformed theme");

        match error {
            ConfigError::Parse {
                path,
                field,
                line,
                message,
                ..
            } => {
                assert!(path.ends_with("themes/mine/theme.toml"), "got {path:?}");
                assert_eq!(field.as_deref(), Some("colors.accent"));
                assert_eq!(line, Some(1));
                assert!(message.contains("invalid colour"), "got `{message}`");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_parse_error_column_counts_characters_rather_than_bytes() {
        // The font family is four three-byte characters long, so a byte-based
        // column would point more than eight columns further right.
        let document = "typography = { font_family = \"中文字体\", scale = \"big\" }\n";
        let value = document
            .find("\"big\"")
            .expect("the document contains the value");
        let expected_column =
            u32::try_from(document[..value].chars().count()).expect("column fits in u32") + 1;
        assert!(
            document[..value].len() > document[..value].chars().count(),
            "the case is only meaningful with multi-byte characters before the value"
        );

        let error = parse_theme_file(document).expect_err("scale is not a number");

        match error {
            ConfigError::Parse {
                field,
                line,
                column,
                ..
            } => {
                assert_eq!(field.as_deref(), Some("typography.scale"));
                assert_eq!(line, Some(1));
                assert_eq!(column, Some(expected_column));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_theme_name_that_escapes_the_themes_directory_is_rejected() {
        let error = selection(Some("../escape"), Appearance::System)
            .resolve(&empty_paths(), Appearance::System)
            .expect_err("traversal");

        match error {
            ConfigError::InvalidThemeName { name } => assert_eq!(name, "../escape"),
            other => panic!("expected an invalid theme name, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_theme_name_is_rejected_rather_than_ignored() {
        let error = selection(Some(""), Appearance::System)
            .resolve(&empty_paths(), Appearance::System)
            .expect_err("empty name");

        match error {
            ConfigError::InvalidThemeName { name } => assert_eq!(name, ""),
            other => panic!("expected an invalid theme name, got {other:?}"),
        }
    }
}
