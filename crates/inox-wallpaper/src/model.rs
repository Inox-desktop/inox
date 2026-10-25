//! The wallpaper's domain model: which output draws what.
//!
//! Everything in this module is pure data and pure functions over it. Nothing
//! here touches the filesystem, decodes anything, or knows that Iced exists:
//! the resolution *order* lives here, the filesystem check in
//! [`crate::source`], the geometry in [`crate::scaling`]. That split is what
//! lets the two rules this crate is judged on — a deterministic per-output
//! choice (acceptance O02) and a diagnosable degradation (D05) — be tested
//! without a compositor, a window or a real image.

use std::fmt;
use std::path::{Path, PathBuf};

use inox_config::WallpaperConfig;

/// The key in `[wallpaper.per_output]` that stands for every output without an
/// entry of its own.
///
/// A wildcard rather than "the first entry" or "any output": with it, a user
/// can give a laptop panel and an external screen different images and still
/// have a third, unknown output draw something deliberate.
pub const WILDCARD: &str = "*";

/// The name a compositor reports an output under, for example `HDMI-A-1`.
///
/// It is also the key `[wallpaper.per_output]` is written against, which is why
/// it is a type of its own rather than a bare `String`: a path, a number or an
/// empty string cannot be passed where an output name is expected.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputName(String);

impl OutputName {
    /// Wraps a name as the compositor reports it.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The name as written in the configuration.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OutputName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for OutputName {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for OutputName {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

/// Pixel dimensions of an image or of an output.
///
/// Width and height are `u32` because that is what both the compositor and the
/// image decoders report; the *arithmetic* over them is done in wider types, so
/// that a huge image cannot wrap a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Size {
    /// A size from its parts.
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Whether either side is zero, which no image or output can be.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The number of pixels, computed without wrapping.
    #[must_use]
    pub const fn pixels(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}×{}", self.width, self.height)
    }
}

/// What the configuration asks one output to draw, before the filesystem has
/// been consulted.
///
/// The difference between [`Selection::ThemeBackground`] and
/// [`crate::source::OutputSource::Unavailable`] is the whole of the settings save semantics in this
/// component: "nothing was selected, so the theme background is correct" and
/// "something was selected and is missing, so the user must be told" are not
/// the same state and are not rendered the same way.
///
/// An *absent* `wallpaper.image` is not "nothing selected" any more: a session
/// that names no image shows the wallpaper Inox ships ([`inox_config::DEFAULT_ID`]),
/// which is [`Selection::Bundled`]. The theme background is now reached by the
/// explicit [`inox_config::BACKGROUND_ID`] value, so the two remain distinct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// An image the user named, resolved against the configuration directory.
    Image(PathBuf),
    /// A wallpaper Inox ships, named by its catalogue identifier and resolved to
    /// a path by [`crate::source`] against the wallpaper search directories.
    ///
    /// `explicit` is whether the configuration named this wallpaper, or whether
    /// it is the default the session fell back to because nothing was
    /// configured. The difference is the one the settings save semantics draws: an explicitly
    /// chosen wallpaper that is not installed is *reported*, while the default
    /// the user never asked for merely degrades to the theme background.
    Bundled {
        /// The catalogue entry.
        wallpaper: &'static inox_config::BundledWallpaper,
        /// Whether the configuration named it rather than defaulting to it.
        explicit: bool,
    },
    /// No image at all: the theme background.
    ThemeBackground,
}

impl Selection {
    /// Resolves the configuration for one output.
    ///
    /// The order is exact output → [`WILDCARD`] → [`WallpaperConfig::image`] →
    /// the bundled default wallpaper, and it is total: every output of every
    /// configuration gets an answer, and two runs of the same configuration
    /// give every output the same one.
    ///
    /// A relative path is joined to `config_dir`, which is the directory the
    /// configuration itself was read from; an absolute path is kept as written,
    /// because a wallpaper is a user-visible file that may well live in
    /// `/usr/share/backgrounds`. `..` is not rejected here — unlike the
    /// `[components]` paths the configuration model, rule 11 constrains, a wallpaper path
    /// is not a file the session will *write*, and refusing `../wallpaper.png`
    /// would break configurations that work with every other desktop.
    ///
    /// A configured value that is neither [`inox_config::BACKGROUND_ID`] nor a
    /// bundled identifier is a path, exactly as before, so a configuration
    /// written against the previous versions keeps working.
    #[must_use]
    pub fn from_config(config: &WallpaperConfig, output: &OutputName, config_dir: &Path) -> Self {
        let named = config
            .per_output
            .get(output.as_str())
            .or_else(|| config.per_output.get(WILDCARD))
            .or(config.image.as_ref());

        match named {
            Some(value) => Self::from_value(config_dir, value),
            None => Self::bundled_default(),
        }
    }

    /// The selection one configured value names.
    fn from_value(config_dir: &Path, value: &Path) -> Self {
        // An empty value names nothing, which is the same request as an absent
        // key: the default wallpaper. It is deliberately *not* the theme
        // background — that has its own explicit spelling
        // ([`inox_config::BACKGROUND_ID`]), and the settings window's "default"
        // option stores exactly this (no key at all).
        if value.as_os_str().is_empty() {
            return Self::bundled_default();
        }

        if let Some(text) = value.to_str() {
            if text == inox_config::BACKGROUND_ID {
                return Self::ThemeBackground;
            }
            if let Some(wallpaper) = inox_config::wallpapers::bundled(text) {
                return Self::Bundled {
                    wallpaper,
                    explicit: true,
                };
            }
        }

        Self::Image(absolute(config_dir, value))
    }

    /// The wallpaper a session shows when the configuration names none.
    fn bundled_default() -> Self {
        match inox_config::wallpapers::default_bundled() {
            Some(wallpaper) => Self::Bundled {
                wallpaper,
                explicit: false,
            },
            // Only if the catalogue is empty, which it is not; degrading to the
            // theme background is still better than failing to draw anything.
            None => Self::ThemeBackground,
        }
    }

    /// The image this selection names, if it names a file.
    ///
    /// A bundled wallpaper has no path until [`crate::source`] resolves it
    /// against the search directories, so this returns `None` for it; ask
    /// [`Selection::wants_image`] whether anything is to be drawn.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Image(path) => Some(path),
            Self::Bundled { .. } | Self::ThemeBackground => None,
        }
    }

    /// The bundled wallpaper this selection names, if it names one.
    #[must_use]
    pub fn bundled(&self) -> Option<&'static inox_config::BundledWallpaper> {
        match self {
            Self::Bundled { wallpaper, .. } => Some(wallpaper),
            Self::Image(_) | Self::ThemeBackground => None,
        }
    }

    /// Whether the configuration named this selection rather than the session
    /// defaulting to it.
    ///
    /// Always true for a path and for the explicit background: those only ever
    /// come from a value in the configuration. It is false only for the bundled
    /// default.
    #[must_use]
    pub const fn is_explicit(&self) -> bool {
        match self {
            Self::Bundled { explicit, .. } => *explicit,
            Self::Image(_) | Self::ThemeBackground => true,
        }
    }

    /// Whether drawing this selection needs an image to be loaded.
    ///
    /// The theme background is the only selection that draws without reading
    /// anything; a bundled wallpaper is as much an image to load as a path is.
    #[must_use]
    pub const fn wants_image(&self) -> bool {
        !matches!(self, Self::ThemeBackground)
    }
}

/// `path` as an absolute path, joined to `config_dir` when it is relative.
///
/// A function rather than a method so it reads the same in a test as it does in
/// [`Selection::from_config`].
#[must_use]
pub fn absolute(config_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        config_dir.join(path)
    }
}

/// What a path is, when it is not a regular file.
///
/// A closed set rather than a word: the word a user reads is copy, and copy
/// lives in the catalogs (root `AGENTS.md` §7.3), so the interface picks a
/// message per variant instead of being handed English to substitute into one.
/// A symbolic link is followed before this is decided ([`crate::source`]), so a
/// link is reported as whatever it resolves to, never as itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// A directory.
    Directory,
    /// Anything else that is not a regular file: a device, a socket, a FIFO.
    Other,
}

/// Why a configured image cannot be shown.
///
/// Every variant is a *degradation*, never a crash and never a black output
/// (acceptance D05): the output falls back to the theme background and the
/// reason is named on screen. The variant is the diagnosis; naming the output
/// and the path around it is the caller's job, because only the caller knows
/// which output was being drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unusable {
    /// No file exists at the path.
    Missing,
    /// Something exists there, but it is not a regular file.
    NotAFile {
        /// What the path actually is.
        kind: PathKind,
    },
    /// The file exists but could not be opened.
    Unreadable {
        /// The operating system's own explanation.
        detail: String,
    },
    /// The bytes are not an image any decoder in this build understands.
    Undecodable {
        /// The decoder's own explanation.
        detail: String,
    },
    /// The image has more pixels than one decode may commit.
    TooLarge {
        /// The size the decoder reported.
        size: Size,
        /// How many pixels [`Size`] is.
        pixels: u64,
        /// The bound that was exceeded ([`crate::decode::MAX_PIXELS`]).
        limit: u64,
    },
    /// One side of the image is longer than any output this build can drive.
    TooLongSide {
        /// The size the decoder reported.
        size: Size,
        /// The bound that was exceeded ([`crate::decode::MAX_DIMENSION`]).
        limit: u32,
    },
    /// The image or the output has a zero side, so no geometry exists for it.
    Degenerate {
        /// The image as decoded.
        image: Size,
        /// The output it was to be drawn on.
        output: Size,
    },
    /// Tiling this image on this output would need more copies than are drawn.
    TooManyTiles {
        /// The tile's native size.
        tile: Size,
        /// How many copies the output would need.
        tiles: u64,
        /// The bound that was exceeded ([`crate::scaling::MAX_TILES`]).
        limit: u32,
    },
}

impl fmt::Display for Unusable {
    /// The reason in one English line, for logs.
    ///
    /// Not user-visible copy: what a user reads comes from the Fluent catalog
    /// through [`crate::view_model`] (root `AGENTS.md` §7.3).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("the file does not exist"),
            Self::NotAFile { kind } => match kind {
                PathKind::Directory => f.write_str("the path is a directory, not a file"),
                PathKind::Other => f.write_str("the path is not a regular file"),
            },
            Self::Unreadable { detail } => write!(f, "the file cannot be opened: {detail}"),
            Self::Undecodable { detail } => write!(f, "the image cannot be decoded: {detail}"),
            Self::TooLarge {
                size,
                pixels,
                limit,
            } => write!(
                f,
                "the image is {size} ({pixels} pixels), above the {limit} pixel bound"
            ),
            Self::TooLongSide { size, limit } => {
                write!(f, "the image is {size}, above the {limit}-pixel side bound")
            }
            Self::Degenerate { image, output } => write!(
                f,
                "the geometry is degenerate: the image is {image} and the output is {output}"
            ),
            Self::TooManyTiles { tile, tiles, limit } => write!(
                f,
                "a {tile} tile needs {tiles} copies, above the {limit} copy bound"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::WallpaperMode;
    use std::collections::BTreeMap;

    fn config(image: Option<&str>, per_output: &[(&str, &str)]) -> WallpaperConfig {
        WallpaperConfig {
            mode: WallpaperMode::default(),
            image: image.map(PathBuf::from),
            per_output: per_output
                .iter()
                .map(|(key, value)| ((*key).to_owned(), PathBuf::from(*value)))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    const DIR: &str = "/home/user/.config/inox";

    fn selection(config: &WallpaperConfig, output: &str) -> Selection {
        Selection::from_config(config, &OutputName::from(output), Path::new(DIR))
    }

    #[test]
    fn an_output_with_no_entry_anywhere_draws_the_bundled_default() {
        let default = inox_config::wallpapers::default_bundled().expect("a default is shipped");
        let expected = Selection::Bundled {
            wallpaper: default,
            explicit: false,
        };

        assert_eq!(selection(&config(None, &[]), "HDMI-A-1"), expected);
        assert_eq!(selection(&config(None, &[]), "HDMI-A-1").path(), None);
        assert_eq!(
            selection(&config(None, &[]), "HDMI-A-1").bundled(),
            Some(default)
        );
        assert!(!selection(&config(None, &[]), "HDMI-A-1").is_explicit());
        assert!(selection(&config(None, &[]), "HDMI-A-1").wants_image());
    }

    #[test]
    fn the_background_id_asks_for_the_theme_background_explicitly() {
        let background = inox_config::BACKGROUND_ID;

        assert_eq!(
            selection(&config(Some(background), &[]), "HDMI-A-1"),
            Selection::ThemeBackground
        );
        assert!(!selection(&config(Some(background), &[]), "HDMI-A-1").wants_image());
        assert!(selection(&config(Some(background), &[]), "HDMI-A-1").is_explicit());

        // The explicit background wins at every level of the lookup.
        let per_output = config(Some("wall.png"), &[("*", background)]);
        assert_eq!(selection(&per_output, "DP-2"), Selection::ThemeBackground);
    }

    #[test]
    fn an_empty_value_names_nothing_and_so_draws_the_bundled_default() {
        // The settings window's "default" option stores no key at all, but a
        // hand-written `image = ""` has to mean the same thing rather than the
        // theme background: "not the background" has one spelling, and it is
        // the background id.
        let default = inox_config::wallpapers::default_bundled().expect("a default is shipped");

        let empty = selection(&config(Some(""), &[]), "HDMI-A-1");

        assert_eq!(
            empty,
            Selection::Bundled {
                wallpaper: default,
                explicit: false,
            }
        );
        assert!(empty.wants_image());
    }

    #[test]
    fn a_bundled_identifier_names_the_shipped_wallpaper_explicitly() {
        let nebula = inox_config::wallpapers::bundled("nebula").expect("shipped");
        let expected = Selection::Bundled {
            wallpaper: nebula,
            explicit: true,
        };

        assert_eq!(
            selection(&config(Some("nebula"), &[]), "HDMI-A-1"),
            expected
        );
        assert_eq!(
            selection(&config(Some("nebula"), &[]), "HDMI-A-1").bundled(),
            Some(nebula)
        );
        assert!(selection(&config(Some("nebula"), &[]), "HDMI-A-1").is_explicit());
    }

    #[test]
    fn a_value_that_is_not_an_identifier_is_still_a_path() {
        // The one shape that changes meaning: a relative file that happens to
        // spell an identifier can no longer be chosen by bare name. Every other
        // relative and absolute path keeps working.
        assert_eq!(
            selection(&config(Some("my-own.jpg"), &[]), "HDMI-A-1"),
            Selection::Image(PathBuf::from(DIR).join("my-own.jpg"))
        );
        assert_eq!(
            selection(&config(Some("/walls/nebula"), &[]), "HDMI-A-1"),
            Selection::Image(PathBuf::from("/walls/nebula"))
        );
    }

    #[test]
    fn the_configured_image_applies_to_every_output_without_an_entry() {
        let config = config(Some("wall.png"), &[]);

        assert_eq!(
            selection(&config, "HDMI-A-1"),
            Selection::Image(PathBuf::from(DIR).join("wall.png"))
        );
        assert_eq!(
            selection(&config, "eDP-1"),
            Selection::Image(PathBuf::from(DIR).join("wall.png"))
        );
    }

    #[test]
    fn a_named_output_wins_over_the_configured_image() {
        let config = config(Some("wall.png"), &[("HDMI-A-1", "wide.png")]);

        assert_eq!(
            selection(&config, "HDMI-A-1"),
            Selection::Image(PathBuf::from(DIR).join("wide.png"))
        );
        assert_eq!(
            selection(&config, "eDP-1"),
            Selection::Image(PathBuf::from(DIR).join("wall.png"))
        );
    }

    #[test]
    fn the_wildcard_wins_over_the_configured_image_and_loses_to_a_named_output() {
        let config = config(
            Some("wall.png"),
            &[("*", "default.png"), ("HDMI-A-1", "wide.png")],
        );

        assert_eq!(
            selection(&config, "HDMI-A-1"),
            Selection::Image(PathBuf::from(DIR).join("wide.png")),
            "a named entry outranks the wildcard"
        );
        assert_eq!(
            selection(&config, "DP-2"),
            Selection::Image(PathBuf::from(DIR).join("default.png")),
            "the wildcard outranks the shared image"
        );
    }

    #[test]
    fn a_wildcard_alone_replaces_nothing_but_shares_one_image() {
        let config = config(None, &[("*", "default.png")]);

        assert_eq!(
            selection(&config, "HDMI-A-1"),
            Selection::Image(PathBuf::from(DIR).join("default.png"))
        );
    }

    #[test]
    fn the_lookup_is_case_sensitive_like_the_output_names_it_mirrors() {
        // `hdmi-a-1` is not `HDMI-A-1`: a compositor's spelling is the key, and
        // quietly matching a different case would make two entries collide.
        let config = config(Some("wall.png"), &[("HDMI-A-1", "wide.png")]);

        assert_eq!(
            selection(&config, "hdmi-a-1"),
            Selection::Image(PathBuf::from(DIR).join("wall.png"))
        );
    }

    #[test]
    fn a_relative_path_is_joined_to_the_configuration_directory() {
        let config = config(Some("backgrounds/wall.png"), &[]);

        assert_eq!(
            selection(&config, "HDMI-A-1"),
            Selection::Image(PathBuf::from(DIR).join("backgrounds/wall.png"))
        );
    }

    #[test]
    fn an_absolute_path_is_kept_as_written() {
        let config = config(Some("/usr/share/backgrounds/inox.png"), &[]);

        assert_eq!(
            selection(&config, "HDMI-A-1"),
            Selection::Image(PathBuf::from("/usr/share/backgrounds/inox.png"))
        );
    }

    #[test]
    fn a_relative_path_that_climbs_out_of_the_configuration_directory_is_kept() {
        // A deliberate difference from the `[components]` paths: this is a file
        // the user points at and the session only ever reads.
        let config = config(Some("../wall.png"), &[]);

        assert_eq!(
            selection(&config, "HDMI-A-1"),
            Selection::Image(PathBuf::from(DIR).join("../wall.png"))
        );
    }

    #[test]
    fn the_same_configuration_gives_the_same_answer_every_time() {
        let config = config(Some("wall.png"), &[("HDMI-A-1", "wide.png")]);

        for _ in 0..100 {
            assert_eq!(
                selection(&config, "HDMI-A-1"),
                Selection::Image(PathBuf::from(DIR).join("wide.png"))
            );
        }
    }

    #[test]
    fn a_size_reports_whether_it_is_empty_and_how_many_pixels_it_has() {
        assert!(Size::new(0, 720).is_empty());
        assert!(Size::new(1280, 0).is_empty());
        assert!(!Size::new(1280, 720).is_empty());
        assert_eq!(Size::new(7680, 4320).pixels(), 33_177_600);
        assert_eq!(Size::new(1280, 720).to_string(), "1280×720");
    }

    #[test]
    fn an_output_name_round_trips_through_display_and_ordering() {
        assert_eq!(OutputName::from("HDMI-A-1").as_str(), "HDMI-A-1");
        assert_eq!(
            OutputName::from("HDMI-A-1".to_owned()).to_string(),
            "HDMI-A-1"
        );
        assert!(OutputName::from("DP-1") < OutputName::from("eDP-1"));
    }

    #[test]
    fn every_reason_renders_a_log_line_that_names_the_problem() {
        let reasons = [
            Unusable::Missing,
            Unusable::NotAFile {
                kind: PathKind::Directory,
            },
            Unusable::Unreadable {
                detail: "permission denied".to_owned(),
            },
            Unusable::Undecodable {
                detail: "unexpected end of file".to_owned(),
            },
            Unusable::TooLarge {
                size: Size::new(20000, 20000),
                pixels: 400_000_000,
                limit: 33_177_600,
            },
            Unusable::TooLongSide {
                size: Size::new(40_000, 8),
                limit: 32_768,
            },
            Unusable::Degenerate {
                image: Size::new(0, 10),
                output: Size::new(1280, 720),
            },
            Unusable::TooManyTiles {
                tile: Size::new(1, 1),
                tiles: 921_600,
                limit: 1024,
            },
        ];

        for reason in reasons {
            let rendered = reason.to_string();
            assert!(!rendered.is_empty());
            assert!(
                rendered.len() > 10,
                "`{rendered}` should explain itself to whoever reads the log"
            );
        }
    }

    #[test]
    fn the_too_large_reason_names_both_the_image_and_the_bound() {
        let rendered = Unusable::TooLarge {
            size: Size::new(20000, 20000),
            pixels: 400_000_000,
            limit: 33_177_600,
        }
        .to_string();

        assert!(rendered.contains("20000×20000"), "rendered `{rendered}`");
        assert!(rendered.contains("400000000"), "rendered `{rendered}`");
        assert!(rendered.contains("33177600"), "rendered `{rendered}`");
    }
}
