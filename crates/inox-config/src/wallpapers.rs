//! The wallpapers Inox ships with.
//!
//! A *catalogue*, not a file list: nothing here reads a directory or opens an
//! image. It names the wallpapers a session offers out of the box, the file each
//! one is installed as, and who it belongs to — the three facts the settings
//! window needs to draw a chooser and the wallpaper component needs to resolve
//! [`BundledWallpaper::id`] to a path. Where those files actually live is a
//! property of the installation, not of the catalogue, so it is resolved by
//! [`inox_common::paths`] at runtime.
//!
//! The identifiers are the values `wallpaper.image` accepts besides a path;
//! [`BACKGROUND_ID`] is the explicit "no wallpaper" value that keeps the theme
//! background reachable now that an absent `wallpaper.image` means the
//! [`default_bundled`] wallpaper.
//!
//! The images and their credits are listed in `data/wallpapers/CREDITS.md`.

/// One wallpaper that ships with Inox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BundledWallpaper {
    /// The stable identifier written as `wallpaper.image`'s value, and the
    /// option the settings window offers. Never changed once shipped: it is a
    /// value users write into their configuration and tools compare against.
    pub id: &'static str,
    /// The file name below an Inox wallpaper directory
    /// ([`inox_common::paths::WALLPAPERS_DIR`]).
    pub file: &'static str,
    /// The photographer to credit.
    pub author: &'static str,
    /// Where the image came from.
    pub source: &'static str,
    /// The licence the image is distributed under.
    pub license: &'static str,
}

/// The wallpapers the settings window offers, in the order it draws them.
///
/// The order is a curation decision, not a sort: the first entry is the
/// session's default (see [`DEFAULT_ID`]).
pub const BUNDLED: &[BundledWallpaper] = &[
    BundledWallpaper {
        id: "nebula",
        file: "nebula.jpg",
        author: "Scott Lord",
        source: "https://unsplash.com/photos/OHCl_XMUcZ4",
        license: "Unsplash License",
    },
    BundledWallpaper {
        id: "fuji",
        file: "fuji.jpg",
        author: "Daniel Sessler",
        source: "https://unsplash.com/photos/AUPja_8Smd0",
        license: "Unsplash License",
    },
    BundledWallpaper {
        id: "bricks",
        file: "bricks.jpg",
        author: "Joe Dudeck",
        source: "https://unsplash.com/photos/e2MLl-80u4E",
        license: "Unsplash License",
    },
    BundledWallpaper {
        id: "lake",
        file: "lake.jpg",
        author: "Johannes Plenio",
        source: "https://unsplash.com/photos/gWFjkBDYtxg",
        license: "Unsplash License",
    },
    BundledWallpaper {
        id: "rainbow",
        file: "rainbow.jpg",
        author: "Karla Hernandez",
        source: "https://unsplash.com/photos/BkjgNMEcfg8",
        license: "Unsplash License",
    },
    BundledWallpaper {
        id: "monument",
        file: "monument.jpg",
        author: "Tom Gainor",
        source: "https://unsplash.com/photos/XprGan-6wy4",
        license: "Unsplash License",
    },
];

/// The identifier of the wallpaper a session shows when nothing else is chosen.
pub const DEFAULT_ID: &str = "nebula";

/// The `wallpaper.image` value that asks for the theme background.
///
/// A word rather than an empty string, so that "the theme background" and "the
/// default wallpaper" stay distinguishable: an absent `wallpaper.image` means
/// [`default_bundled`], while this value means there is deliberately no image.
pub const BACKGROUND_ID: &str = "none";

/// The bundled wallpaper with this identifier, if there is one.
#[must_use]
pub fn bundled(id: &str) -> Option<&'static BundledWallpaper> {
    BUNDLED.iter().find(|wallpaper| wallpaper.id == id)
}

/// The wallpaper a session falls back to when `wallpaper.image` is absent.
///
/// `None` only if [`BUNDLED`] is empty, which a unit test rules out; a caller
/// that cannot name a default degrades to the theme background rather than
/// failing.
#[must_use]
pub fn default_bundled() -> Option<&'static BundledWallpaper> {
    bundled(DEFAULT_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn there_is_a_default_and_it_is_the_first_entry() {
        let default = default_bundled().expect("a default wallpaper is shipped");

        assert_eq!(default.id, DEFAULT_ID);
        assert_eq!(BUNDLED.first().map(|w| w.id), Some(DEFAULT_ID));
    }

    #[test]
    fn identifiers_are_unique_and_not_reserved_words() {
        let ids: BTreeSet<&str> = BUNDLED.iter().map(|w| w.id).collect();

        assert_eq!(ids.len(), BUNDLED.len(), "two wallpapers share an id");
        assert!(
            !ids.contains(BACKGROUND_ID),
            "`{BACKGROUND_ID}` is reserved for the theme background"
        );
    }

    #[test]
    fn identifiers_are_plain_path_segments_so_they_never_read_as_a_path() {
        for wallpaper in BUNDLED {
            assert!(!wallpaper.id.is_empty(), "{wallpaper:?}");
            assert!(
                !wallpaper.id.contains('/') && !wallpaper.id.contains('\\'),
                "`{}` would look like a path",
                wallpaper.id
            );
            assert!(
                !wallpaper.id.contains('.'),
                "`{}` would look like a file name",
                wallpaper.id
            );
        }
    }

    #[test]
    fn every_wallpaper_names_a_jpeg_and_credits_its_source() {
        for wallpaper in BUNDLED {
            assert!(
                wallpaper.file.ends_with(".jpg"),
                "`{}` is not a JPEG file name",
                wallpaper.file
            );
            assert!(
                !wallpaper.author.trim().is_empty(),
                "`{}` has no author",
                wallpaper.id
            );
            assert!(
                wallpaper.source.starts_with("https://"),
                "`{}` has no source",
                wallpaper.id
            );
            assert!(
                !wallpaper.license.trim().is_empty(),
                "`{}` has no licence",
                wallpaper.id
            );
        }
    }

    #[test]
    fn files_are_unique() {
        let files: BTreeSet<&str> = BUNDLED.iter().map(|w| w.file).collect();

        assert_eq!(files.len(), BUNDLED.len(), "two wallpapers share a file");
    }

    #[test]
    fn lookup_is_by_exact_id() {
        assert_eq!(bundled("nebula").map(|w| w.file), Some("nebula.jpg"));
        assert!(bundled("NEBULA").is_none(), "ids are case-sensitive");
        assert!(bundled("").is_none());
        assert!(
            bundled(BACKGROUND_ID).is_none(),
            "the background is not a file"
        );
    }
}
