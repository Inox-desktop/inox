//! Resolving an entry's `Icon=` into a file the window can draw.
//!
//! The launcher reads icons from the icon themes the session already installed —
//! it does not ship artwork and it does not guess. The lookup is the subset of
//! the [icon theme specification] this surface needs:
//!
//! 1. an absolute `Icon=` value is the file itself, nothing to search;
//! 2. otherwise the name is looked up in the installed themes, `hicolor` first
//!    and then the rest in name order, under every icon root:
//!    `$XDG_DATA_HOME/icons`, every `$XDG_DATA_DIRS/icons` and
//!    `~/.local/share/icons`;
//! 3. inside a theme, `{size}x{size}/apps/{name}.png` closest to
//!    [`ICON_TARGET_PX`] wins over `scalable/apps/{name}.svg` — a raster whose
//!    size the compositor does not have to rescale is the sharper choice, and a
//!    theme that ships only the vector still gets an icon;
//! 4. `/usr/share/pixmaps` is the legacy last resort.
//!
//! A miss is a normal answer, not an error: the tile falls back to the letter it
//! always had, and the caller says so by keeping the path optional. Nothing here
//! is Qt-aware — the path is a [`PathBuf`] and [`file_url`] turns it into the
//! `file://` URL QML's `Image` takes, so both halves are unit-testable without a
//! display.
//!
//! [icon theme specification]: https://specifications.freedesktop.org/icon-theme-spec/latest/

use std::path::{Path, PathBuf};

use tracing::warn;

/// The icon size a tile asks for, in logical pixels.
///
/// Themes ship a fixed ladder (48, 64, 96, 128 …); the closest rung to this wins,
/// so a theme without a 96 px rendition still gets the nearest one rather than no
/// icon at all.
pub const ICON_TARGET_PX: u32 = 96;

/// The raster extension looked for first.
const RASTER_EXTENSION: &str = "png";

/// The vector extension, looked for when the theme has no raster at any size.
const VECTOR_EXTENSION: &str = "svg";

/// The theme the specification makes every installation provide, and the one the
/// lookup prefers.
const PREFERRED_THEME: &str = "hicolor";

/// The extension a theme-less pixmap may carry.
const PIXMAP_EXTENSIONS: [&str; 2] = ["png", "svg"];

/// Everywhere an icon may live, in the order it is searched.
///
/// Built once per index scan (or once per test) rather than per lookup: the
/// directories come from the environment, and an entry's `Icon=` is looked up
/// once when the index is read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IconSearch {
    /// The icon theme roots: each holds `<theme>/<size>/apps/<name>.<ext>`.
    roots: Vec<PathBuf>,
    /// The legacy pixmap directories, each holding `<name>.<ext>` directly.
    pixmaps: Vec<PathBuf>,
}

impl IconSearch {
    /// A search over explicit directories, for a caller that resolved them
    /// itself — the tests, and any future caller that caches them.
    pub fn new(roots: Vec<PathBuf>, pixmaps: Vec<PathBuf>) -> Self {
        Self {
            roots: deduplicate(roots),
            pixmaps: deduplicate(pixmaps),
        }
    }

    /// A search that finds nothing, for a caller that has no icon directories.
    pub fn none() -> Self {
        Self::default()
    }

    /// The search the session's environment describes.
    ///
    /// A session whose `$XDG_DATA_HOME` cannot be resolved still gets the system
    /// directories: an unreadable home is a reason to look further, not a reason
    /// to draw no icons at all.
    pub fn from_env() -> Self {
        let mut roots = Vec::new();
        let mut pixmaps = Vec::new();

        match inox_common::paths::data_home() {
            Ok(home) => {
                roots.push(home.join("icons"));
                pixmaps.push(home.join("pixmaps"));
            }
            Err(error) => warn!(%error, "the user icon directory cannot be located"),
        }

        for data in crate::index::data_directories() {
            roots.push(data.join("icons"));
            pixmaps.push(data.join("pixmaps"));
        }

        // `~/.local/share/icons` is `$XDG_DATA_HOME/icons` under its default;
        // spelling it out keeps the lookup working when the variable is unset in
        // the environment but `$HOME` is not.
        if let Ok(home) = std::env::var("HOME") {
            roots.push(Path::new(&home).join(".local/share/icons"));
        }

        Self::new(roots, pixmaps)
    }

    /// The file `icon` names, if this machine has it.
    ///
    /// `None` means "draw the letter instead": a missing file, an `Icon=` that is
    /// a bare name nothing provides, and a value that is neither an absolute path
    /// nor a plain icon name all resolve to the same answer.
    pub fn resolve(&self, icon: Option<&str>) -> Option<PathBuf> {
        let icon = icon?.trim();
        if icon.is_empty() {
            return None;
        }

        let path = Path::new(icon);
        if path.is_absolute() {
            // `Icon=` may be the file itself; there is nothing to search, and a
            // missing file is a miss rather than a name to look up.
            return path.is_file().then(|| path.to_path_buf());
        }
        if icon.contains('/') {
            // A relative path is not an icon name, and resolving it against the
            // process's working directory is not something a quoted `Icon=`
            // means.
            return None;
        }

        let name = strip_extension(icon);

        for theme in self.themes() {
            for root in &self.roots {
                if let Some(found) = icon_in_theme(&root.join(&theme), name) {
                    return Some(found);
                }
            }
        }

        for directory in &self.pixmaps {
            for extension in PIXMAP_EXTENSIONS {
                let candidate = directory.join(format!("{name}.{extension}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }

        None
    }

    /// The theme directories to try, `hicolor` first.
    ///
    /// The set is the union over every root, so a `hicolor` installed in one
    /// root still wins over an `Adwaita` installed in another — theme preference
    /// is a property of the name, not of the directory it was found in. Within a
    /// theme the roots keep their order, which is what makes a user's own theme
    /// override the system's copy.
    fn themes(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for root in &self.roots {
            let Ok(listing) = std::fs::read_dir(root) else {
                continue;
            };
            for item in listing.flatten() {
                if !item.path().is_dir() {
                    continue;
                }
                let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names.sort();
        names.sort_by_key(|name| usize::from(name != PREFERRED_THEME));
        names
    }
}

/// The best icon `name` has inside one theme directory.
///
/// A theme is a set of `<size>x<size>/apps` and `scalable/apps` directories. The
/// largest-of-the-nearest raster wins (a 96 px request prefers 96 over 128 over
/// 48, and a tie prefers the larger file); a theme that only ships the vector
/// gets that.
fn icon_in_theme(theme: &Path, name: &str) -> Option<PathBuf> {
    let Ok(listing) = std::fs::read_dir(theme) else {
        return None;
    };

    let mut rasters: Vec<(u32, PathBuf)> = Vec::new();
    let mut scalable: Option<PathBuf> = None;

    let mut directories: Vec<PathBuf> = listing
        .flatten()
        .map(|item| item.path())
        .filter(|path| path.is_dir())
        .collect();
    directories.sort();

    for directory in directories {
        let Some(directory_name) = directory.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let apps = directory.join("apps");

        if directory_name == "scalable" {
            let candidate = apps.join(format!("{name}.{VECTOR_EXTENSION}"));
            if candidate.is_file() {
                scalable = Some(candidate);
            }
            continue;
        }

        let Some(size) = parse_size(directory_name) else {
            // `symbolic`, `128x128@2` and anything else a theme invents are not
            // candidates: a symbolic icon is a monochrome glyph that reads as a
            // missing icon in a grid of full-colour artwork, and a `@2` directory
            // repeats a size the plain one already provides.
            continue;
        };
        let candidate = apps.join(format!("{name}.{RASTER_EXTENSION}"));
        if candidate.is_file() {
            rasters.push((size, candidate));
        }
    }

    rasters.sort_by_key(|(size, _)| (size.abs_diff(ICON_TARGET_PX), u32::MAX - size));
    rasters
        .into_iter()
        .next()
        .map(|(_, path)| path)
        .or(scalable)
}

/// The pixel size a theme's directory name names, for example `48x48`.
fn parse_size(directory: &str) -> Option<u32> {
    let (width, height) = directory.split_once('x')?;
    let width: u32 = width.parse().ok()?;
    let height: u32 = height.parse().ok()?;
    (width == height && width > 0).then_some(width)
}

/// The icon name without a file extension, if it carries one.
fn strip_extension(name: &str) -> &str {
    for extension in [RASTER_EXTENSION, VECTOR_EXTENSION] {
        if let Some(stem) = name.strip_suffix(&format!(".{extension}")) {
            return stem;
        }
    }
    name
}

/// Drops repeated directories, keeping the first occurrence.
fn deduplicate(directories: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique: Vec<PathBuf> = Vec::with_capacity(directories.len());
    for directory in directories {
        if !unique.contains(&directory) {
            unique.push(directory);
        }
    }
    unique
}

/// `path` as the `file://` URL a QML `Image` source accepts.
///
/// QML hands the string to `QUrl`, which needs the bytes that are not URL-safe
/// percent-encoded — a space in `/usr/share/icons/My Theme/…` is the common case
/// on a real installation. Only the unreserved characters and the path
/// separators are passed through, so a URL this returns is always one QUrl
/// decodes back to the same path.
pub fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                url.push(char::from(*byte));
            }
            other => url.push_str(&format!("%{other:02X}")),
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Writes an empty file, creating its directories.
    fn touch(root: &Path, relative: &str) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        fs::write(&path, b"").expect("a file");
        path
    }

    fn search(root: &Path) -> IconSearch {
        IconSearch::new(vec![root.to_path_buf()], vec![root.join("pixmaps")])
    }

    #[test]
    fn an_absolute_icon_is_the_file_itself() {
        let temporary = tempfile::tempdir().expect("a temporary directory");
        let root = temporary.path();
        let icon = touch(root, "art/custom.png");

        let search = search(root);
        assert_eq!(
            search.resolve(Some(icon.to_str().expect("utf-8"))),
            Some(icon)
        );

        // A file that is not there is a miss, not a name to search for.
        let missing = root.join("art/absent.png");
        assert_eq!(search.resolve(Some(missing.to_str().expect("utf-8"))), None);
    }

    #[test]
    fn an_icon_name_resolves_through_the_theme() {
        let temporary = tempfile::tempdir().expect("a temporary directory");
        let root = temporary.path();
        let icon = touch(root, "hicolor/48x48/apps/example.png");

        assert_eq!(
            search(root).resolve(Some("example")),
            Some(icon.clone()),
            "the icon is found under the theme's size directory",
        );
        assert_eq!(search(root).resolve(Some("nothing-here")), None);
        assert_eq!(search(root).resolve(None), None);
        assert_eq!(search(root).resolve(Some("   ")), None);
    }

    #[test]
    fn hicolor_wins_over_another_theme() {
        let temporary = tempfile::tempdir().expect("a temporary directory");
        let root = temporary.path();
        let preferred = touch(root, "hicolor/48x48/apps/example.png");
        touch(root, "Adwaita/96x96/apps/example.png");

        assert_eq!(search(root).resolve(Some("example")), Some(preferred));
    }

    #[test]
    fn the_size_closest_to_the_target_wins() {
        let temporary = tempfile::tempdir().expect("a temporary directory");
        let root = temporary.path();
        touch(root, "hicolor/48x48/apps/example.png");
        let target = touch(root, "hicolor/96x96/apps/example.png");
        touch(root, "hicolor/256x256/apps/example.png");

        assert_eq!(
            search(root).resolve(Some("example")),
            Some(target),
            "96 is the target, not the largest rendition",
        );

        // 64 and 128 are the same distance from 96; the sharper one wins.
        let other = tempfile::tempdir().expect("a temporary directory");
        let root = other.path();
        touch(root, "hicolor/64x64/apps/example.png");
        let larger = touch(root, "hicolor/128x128/apps/example.png");
        assert_eq!(search(root).resolve(Some("example")), Some(larger));
    }

    #[test]
    fn a_raster_wins_over_the_vector_and_the_vector_still_works() {
        let temporary = tempfile::tempdir().expect("a temporary directory");
        let root = temporary.path();
        let raster = touch(root, "hicolor/64x64/apps/example.png");
        touch(root, "hicolor/scalable/apps/example.svg");
        assert_eq!(search(root).resolve(Some("example")), Some(raster));

        let vector_only = tempfile::tempdir().expect("a temporary directory");
        let root = vector_only.path();
        let vector = touch(root, "hicolor/scalable/apps/example.svg");
        assert_eq!(search(root).resolve(Some("example")), Some(vector));
    }

    #[test]
    fn an_extension_in_the_icon_name_is_not_searched_for_twice() {
        // A desktop file may write `Icon=example.png`; the lookup treats it as
        // the name, so `48x48/apps/example.png` is still found.
        let raster = tempfile::tempdir().expect("a temporary directory");
        let root = raster.path();
        let icon = touch(root, "hicolor/96x96/apps/example.png");
        assert_eq!(search(root).resolve(Some("example.png")), Some(icon));

        // The same for a name that carries the vector extension.
        let vector_only = tempfile::tempdir().expect("a temporary directory");
        let root = vector_only.path();
        let vector = touch(root, "hicolor/scalable/apps/example.svg");
        assert_eq!(search(root).resolve(Some("example.svg")), Some(vector));
    }

    #[test]
    fn pixmaps_are_the_legacy_last_resort() {
        let temporary = tempfile::tempdir().expect("a temporary directory");
        let root = temporary.path();
        let pixmap = touch(root, "pixmaps/example.png");
        let themed = touch(root, "hicolor/96x96/apps/other.png");

        let search = search(root);
        assert_eq!(search.resolve(Some("example")), Some(pixmap));
        assert_eq!(search.resolve(Some("other")), Some(themed));
        assert_eq!(search.resolve(Some("absent")), None);
    }

    #[test]
    fn a_relative_path_is_not_an_icon_name() {
        let temporary = tempfile::tempdir().expect("a temporary directory");
        let root = temporary.path();
        touch(root, "hicolor/96x96/apps/example.png");

        assert_eq!(search(root).resolve(Some("../example")), None);
        assert_eq!(search(root).resolve(Some("sub/example")), None);
    }

    #[test]
    fn a_double_size_directory_is_not_a_candidate() {
        let temporary = tempfile::tempdir().expect("a temporary directory");
        let root = temporary.path();
        touch(root, "hicolor/256x256@2/apps/example.png");

        assert_eq!(
            search(root).resolve(Some("example")),
            None,
            "an @2 directory repeats a logical size the plain one provides",
        );
    }

    #[test]
    fn a_file_url_percent_encodes_what_a_url_cannot_carry() {
        assert_eq!(
            file_url(Path::new("/usr/share/icons/hicolor/96x96/apps/example.png")),
            "file:///usr/share/icons/hicolor/96x96/apps/example.png"
        );
        assert_eq!(
            file_url(Path::new("/usr/share/icons/My Theme/a b.png")),
            "file:///usr/share/icons/My%20Theme/a%20b.png"
        );
        assert_eq!(file_url(Path::new("/tmp/%/x.png")), "file:///tmp/%25/x.png");
        let mut url = file_url(Path::new("/tmp/ünïcode.png"));
        url.push('\n');
        assert!(url.contains('%'), "non-ASCII bytes are encoded: {url}");
    }

    #[test]
    fn directory_names_parse_only_when_they_are_a_square_size() {
        assert_eq!(parse_size("48x48"), Some(48));
        assert_eq!(parse_size("96x96"), Some(96));
        assert_eq!(parse_size("scalable"), None);
        assert_eq!(parse_size("128x128@2"), None);
        assert_eq!(parse_size("48x96"), None);
        assert_eq!(parse_size("0x0"), None);
    }
}
