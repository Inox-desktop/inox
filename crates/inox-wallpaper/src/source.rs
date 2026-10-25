//! Turning a configured selection into something an output can actually draw.
//!
//! [`Selection`] answers "what did the user ask for";
//! this module answers "is it there". The two are separate because the answers
//! are reported differently: nothing to draw is the theme background and
//! deserves no complaint, while something configured and missing is a defect the
//! user has to be told about. The case in between is
//! the shipped default a user never chose: it degrades with a warning rather
//! than an error, because nobody can act on a wallpaper they did not pick
//!.
//!
//! The check is deliberately shallow — exists, is a file, opens — so that the
//! diagnosis arrives before a decode commits any memory. Whatever happens
//! *after* the check (a truncated file, a decoder that gives up) is
//! [`crate::decode`]'s error, and it lands in the same [`Unusable`] vocabulary.

use std::fs;
use std::path::PathBuf;

use crate::model::{PathKind, Selection, Unusable};

/// What one output will draw, after the configuration and the filesystem have
/// both been consulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputSource {
    /// An image the user selected, which is there and can be opened.
    Image(PathBuf),
    /// The theme background: no image is drawn. Reached by the explicit "no
    /// image" value, and — with a warning — by a bundled default that is not
    /// installed. Not a failure either way.
    ThemeBackground,
    /// The user selected an image that cannot be used. Degrades to the theme
    /// background, carrying the path and the reason the interface names.
    Unavailable {
        /// The image that was asked for.
        path: PathBuf,
        /// Why it cannot be used.
        reason: Unusable,
    },
}

impl OutputSource {
    /// The path this source is about, if it names one.
    #[must_use]
    pub fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::Image(path) => Some(path),
            Self::ThemeBackground => None,
            Self::Unavailable { path, .. } => Some(path),
        }
    }

    /// The reason this source is degraded, if it is.
    #[must_use]
    pub fn reason(&self) -> Option<&Unusable> {
        match self {
            Self::Unavailable { reason, .. } => Some(reason),
            Self::Image(_) | Self::ThemeBackground => None,
        }
    }
}

/// Resolves a selection against the filesystem.
///
/// The selected path is checked, not the file's contents: this is the step that
/// can say "there is nothing at that path" while a decode would only be able to
/// say "those bytes are not an image".
///
/// A bundled wallpaper is looked up, in `dirs` order, as `dir/<file>`; the first
/// readable one wins. The search is what lets the same configuration work
/// whether Inox is installed system-wide or run from a source checkout.
#[must_use]
pub fn resolve(selection: &Selection, dirs: &[PathBuf]) -> OutputSource {
    match selection {
        Selection::ThemeBackground => OutputSource::ThemeBackground,
        Selection::Image(path) => match unusable(path) {
            None => OutputSource::Image(path.clone()),
            Some(reason) => OutputSource::Unavailable {
                path: path.clone(),
                reason,
            },
        },
        Selection::Bundled {
            wallpaper,
            explicit,
        } => resolve_bundled(wallpaper, *explicit, dirs),
    }
}

/// Resolves a bundled wallpaper against the search directories.
///
/// An explicitly chosen wallpaper that is nowhere to be found is a
/// [`OutputSource::Unavailable`] like any other selected-and-missing resource
///; the default the user never asked for falls back to the theme
/// background with only a warning, because a user cannot be expected to act on
/// a wallpaper they did not choose.
fn resolve_bundled(
    wallpaper: &'static inox_config::BundledWallpaper,
    explicit: bool,
    dirs: &[PathBuf],
) -> OutputSource {
    for dir in dirs {
        let candidate = dir.join(wallpaper.file);

        if unusable(&candidate).is_none() {
            return OutputSource::Image(candidate);
        }
    }

    if explicit {
        return OutputSource::Unavailable {
            path: expected_path(wallpaper, dirs),
            reason: Unusable::Missing,
        };
    }

    tracing::warn!(
        wallpaper = wallpaper.id,
        file = wallpaper.file,
        "the bundled default wallpaper is not installed; drawing the theme background"
    );

    OutputSource::ThemeBackground
}

/// The path a bundled wallpaper would have been read from, for a diagnosis.
///
/// The user is told the name of the file that was looked for; the first search
/// directory is where it would be installed, so naming it points a packager at
/// the right place.
fn expected_path(wallpaper: &'static inox_config::BundledWallpaper, dirs: &[PathBuf]) -> PathBuf {
    match dirs.first() {
        Some(dir) => dir.join(wallpaper.file),
        None => PathBuf::from(wallpaper.file),
    }
}

/// Why `path` cannot be used as an image, if it cannot.
fn unusable(path: &std::path::Path) -> Option<Unusable> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if crate::decode::is_not_found(&error) => return Some(Unusable::Missing),
        Err(error) => {
            return Some(Unusable::Unreadable {
                detail: error.to_string(),
            });
        }
    };

    if !metadata.is_file() {
        return Some(Unusable::NotAFile {
            kind: kind_of(&metadata),
        });
    }

    // Opening is the only honest readability check: permissions are not in the
    // metadata, and a `w`-only file is a file the user cannot see.
    match fs::File::open(path) {
        Ok(_) => None,
        Err(error) => Some(Unusable::Unreadable {
            detail: error.to_string(),
        }),
    }
}

/// What a path is, for a reason a user can read.
///
/// The metadata is the followed one, so a symbolic link arrives here as what it
/// points at — a link to a directory is a directory, and a link that resolves to
/// nothing never reaches this function at all.
fn kind_of(metadata: &fs::Metadata) -> PathKind {
    if metadata.file_type().is_dir() {
        PathKind::Directory
    } else {
        PathKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// `resolve` with no search directories, which is all a path selection ever
    /// needs: only a bundled wallpaper is searched for.
    fn resolve(selection: &Selection) -> OutputSource {
        super::resolve(selection, &[])
    }

    fn selection_at(path: impl Into<PathBuf>) -> Selection {
        Selection::Image(path.into())
    }

    #[test]
    fn nothing_configured_is_the_theme_background_and_carries_no_reason() {
        let source = resolve(&Selection::ThemeBackground);

        assert_eq!(source, OutputSource::ThemeBackground);
        assert_eq!(source.path(), None);
        assert_eq!(source.reason(), None);
    }

    #[test]
    fn an_image_that_is_there_and_readable_resolves_to_that_path() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("wall.png");
        fs::write(&path, b"not an image, but it is there").expect("the fixture is written");

        let source = resolve(&selection_at(path.clone()));

        assert_eq!(source, OutputSource::Image(path.clone()));
        assert_eq!(source.path(), Some(&path));
        assert_eq!(source.reason(), None);
    }

    #[test]
    fn a_missing_file_is_reported_rather_than_ignored() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("absent.png");

        let source = resolve(&selection_at(path.clone()));

        assert_eq!(
            source,
            OutputSource::Unavailable {
                path: path.clone(),
                reason: Unusable::Missing,
            }
        );
        assert_eq!(source.reason(), Some(&Unusable::Missing));
    }

    #[test]
    fn a_directory_is_reported_as_something_that_is_not_a_file() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("walls");
        fs::create_dir(&path).expect("the fixture is created");

        match resolve(&selection_at(path)) {
            OutputSource::Unavailable {
                reason: Unusable::NotAFile { kind },
                ..
            } => assert_eq!(kind, PathKind::Directory),
            other => panic!("expected a directory reason, got {other:?}"),
        }
    }

    #[test]
    fn a_symlink_loop_is_reported_instead_of_followed_forever() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("loop.png");
        std::os::unix::fs::symlink(&path, &path).expect("the fixture is created");

        match resolve(&selection_at(path)) {
            OutputSource::Unavailable {
                reason: Unusable::Unreadable { .. },
                ..
            } => {}
            other => panic!("expected an unreadable reason, got {other:?}"),
        }
    }

    #[test]
    fn something_that_is_not_a_file_and_not_a_directory_is_reported_as_such() {
        // "Not a file" is not always "a directory": a socket, a device or a FIFO
        // can be a configured path too, and the reason has to name that case
        // without inventing a word for it in the code.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("wall.sock");
        let _listener =
            std::os::unix::net::UnixListener::bind(&path).expect("the fixture is created");

        match resolve(&selection_at(path)) {
            OutputSource::Unavailable {
                reason: Unusable::NotAFile { kind },
                ..
            } => assert_eq!(kind, PathKind::Other),
            other => panic!("expected a not-a-file reason, got {other:?}"),
        }
    }

    #[test]
    fn a_symlink_to_a_real_file_is_followed() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let real = dir.path().join("real.png");
        let link = dir.path().join("link.png");
        fs::write(&real, b"x").expect("the fixture is written");
        std::os::unix::fs::symlink(&real, &link).expect("the fixture is created");

        assert_eq!(
            resolve(&selection_at(link.clone())),
            OutputSource::Image(link)
        );
    }

    #[test]
    fn the_resolution_of_a_path_is_independent_of_what_is_in_it() {
        // The check is about the path, not the bytes: a note named `.png`
        // resolves and only fails later, at the decode step, with a different
        // reason. Two stages, two diagnoses.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("notes.png");
        fs::write(&path, b"a note").expect("the fixture is written");

        assert!(matches!(
            resolve(&selection_at(path)),
            OutputSource::Image(_)
        ));
    }

    #[test]
    fn resolution_is_deterministic() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("absent.png");
        let selection = selection_at(path);

        for _ in 0..50 {
            assert_eq!(resolve(&selection), resolve(&selection));
        }
    }

    #[test]
    fn a_fixture_path_that_is_not_utf8_still_resolves() {
        // Output names and paths are not required to be UTF-8 by the operating
        // system, and a component must not assume they are.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path: PathBuf = [dir.path().as_os_str(), std::ffi::OsStr::new("wall.png")]
            .iter()
            .collect();

        assert_eq!(
            resolve(&selection_at(path.clone())),
            OutputSource::Unavailable {
                path,
                reason: Unusable::Missing,
            }
        );
        assert!(Path::new("/").exists());
    }

    #[test]
    fn a_bundled_wallpaper_is_found_in_the_first_directory_that_has_it() {
        let wallpaper = inox_config::wallpapers::bundled("fuji").expect("shipped");
        let empty = tempfile::tempdir().expect("a temporary directory");
        let installed = tempfile::tempdir().expect("a temporary directory");
        let file = installed.path().join(wallpaper.file);
        fs::write(&file, b"the image").expect("the fixture is written");

        let source = super::resolve(
            &Selection::Bundled {
                wallpaper,
                explicit: true,
            },
            &[empty.path().to_path_buf(), installed.path().to_path_buf()],
        );

        assert_eq!(source, OutputSource::Image(file));
    }

    #[test]
    fn an_explicitly_chosen_wallpaper_that_is_not_installed_is_reported() {
        let wallpaper = inox_config::wallpapers::bundled("fuji").expect("shipped");
        let dir = tempfile::tempdir().expect("a temporary directory");

        let source = super::resolve(
            &Selection::Bundled {
                wallpaper,
                explicit: true,
            },
            &[dir.path().to_path_buf()],
        );

        assert_eq!(
            source,
            OutputSource::Unavailable {
                path: dir.path().join(wallpaper.file),
                reason: Unusable::Missing,
            }
        );
    }

    #[test]
    fn the_default_wallpaper_missing_degrades_quietly_to_the_theme_background() {
        let wallpaper = inox_config::wallpapers::default_bundled().expect("shipped");
        let dir = tempfile::tempdir().expect("a temporary directory");

        let source = super::resolve(
            &Selection::Bundled {
                wallpaper,
                explicit: false,
            },
            &[dir.path().to_path_buf()],
        );

        assert_eq!(source, OutputSource::ThemeBackground);
        assert_eq!(source.reason(), None, "the user did not ask for it");
    }

    #[test]
    fn a_bundled_wallpaper_with_no_search_directories_is_not_installed() {
        let wallpaper = inox_config::wallpapers::bundled("lake").expect("shipped");

        assert_eq!(
            super::resolve(
                &Selection::Bundled {
                    wallpaper,
                    explicit: true,
                },
                &[],
            ),
            OutputSource::Unavailable {
                path: PathBuf::from(wallpaper.file),
                reason: Unusable::Missing,
            }
        );
    }
}
