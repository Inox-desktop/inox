//! Where the QML assets are, for a host crate's `build.rs`.
//!
//! The components are **assets, not a QML module**: this crate depends on no Qt
//! binding (ADR 0040 §3), and Qt registers QML types per binary, so a QML plugin
//! registered from another crate's binary does not reliably reach the engine that
//! loads it. The host therefore compiles these files into *its own* module, from
//! its own `build.rs`:
//!
//! ```no_run
//! # use std::path::PathBuf;
//! # fn qml_module_files() -> Vec<PathBuf> { Vec::new() }
//! let files: Vec<PathBuf> = inox_glass::qml_files();
//! assert_eq!(files.len(), inox_glass::QML_FILES.len());
//! ```
//!
//! That works from a build script because every path is computed from
//! `env!("CARGO_MANIFEST_DIR")`, the *producer's* manifest directory, at
//! compile time — a build script that ran from the consumer's directory would
//! otherwise resolve a relative path against the wrong crate.

use std::path::{Path, PathBuf};

/// The QML files this crate ships, relative to its `qml/` directory.
///
/// The five components come first, so a host can iterate without thinking about
/// the catalog page; [`QML_FILES`] itself is the complete list, which is what
/// `build.rs` should compile.
pub const QML_FILES: [&str; 6] = [
    "GlassPane.qml",
    "GlassButton.qml",
    "GlassToggle.qml",
    "GlassSlider.qml",
    "GlassTabBar.qml",
    "CatalogWindow.qml",
];

/// The directory the QML assets live in, relative to this crate's root.
const QML_DIR: &str = "qml";

/// The assets with their source text, in [`QML_FILES`] order.
///
/// [`qml_files`] is the route the ADR names, and it is enough for a host whose
/// QML module builder aliases a file by the path it is given. `cxx-qt-build`
/// aliases an absolute path *under* the module prefix (`/qt/qml/<uri>/home/…`),
/// which works — all six files still land in one resource directory, so their
/// relative imports resolve — but it is not a resource path anyone wants to read.
/// A build script that prefers clean paths writes these sources into its own
/// directory (usually `OUT_DIR`) and hands the module builder the relative
/// results; the files are a few kilobytes each, so shipping them as strings costs
/// nothing and saves the consumer a copy of the workspace layout.
const QML_SOURCES: [(&str, &str); 6] = [
    ("GlassPane.qml", include_str!("../qml/GlassPane.qml")),
    ("GlassButton.qml", include_str!("../qml/GlassButton.qml")),
    ("GlassToggle.qml", include_str!("../qml/GlassToggle.qml")),
    ("GlassSlider.qml", include_str!("../qml/GlassSlider.qml")),
    ("GlassTabBar.qml", include_str!("../qml/GlassTabBar.qml")),
    (
        "CatalogWindow.qml",
        include_str!("../qml/CatalogWindow.qml"),
    ),
];

/// Absolute paths of every QML asset, for a host crate's build script.
///
/// The order matches [`QML_FILES`]. Nothing here checks the filesystem: a
/// `build.rs` wants the list to hand to its QML module builder, and the test
/// `every_qml_file_exists_and_is_qml` in this module is what proves the files
/// are really there (a missing asset fails `cargo test`, not a user's session).
pub fn qml_files() -> Vec<PathBuf> {
    QML_FILES.iter().map(|file| qml_dir().join(file)).collect()
}

/// Every QML asset as `(file name, source text)`, in [`QML_FILES`] order.
///
/// Use this when a host's build script wants the assets *inside* its own QML
/// module directory rather than referenced by absolute path; use [`qml_files`]
/// when it can add them in place. Both produce exactly the same six files, and
/// `tests/qml_assets.rs` asserts the two lists agree.
pub fn qml_sources() -> &'static [(&'static str, &'static str)] {
    &QML_SOURCES
}

/// Path of `CatalogWindow.qml`, the page that draws all five components.
///
/// A host's offscreen render test loads exactly this file and hands it a
/// [`crate::catalog::from_theme`] projection, which is the component library's
/// pixel evidence (ADR 0040 §3).
pub fn catalog_qml() -> PathBuf {
    qml_dir().join("CatalogWindow.qml")
}

/// The asset directory, as an absolute path.
fn qml_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(QML_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_qml_file_exists_and_is_qml() {
        for (file, path) in QML_FILES.iter().zip(qml_files()) {
            assert!(
                path.is_absolute(),
                "{file} must resolve to an absolute path, not {path:?}"
            );
            assert!(
                path.ends_with(file),
                "the path for {file} ends with the file name: {path:?}"
            );
            assert!(
                path.extension().is_some_and(|extension| extension == "qml"),
                "every asset is a .qml file: {path:?}"
            );
            assert!(
                path.is_file(),
                "{path:?} is in QML_FILES but missing on disk; the host crate's build.rs \
                 would fail to compile a QML module from it"
            );
        }
    }

    #[test]
    fn the_compiled_in_sources_are_the_files_on_disk() {
        let sources = qml_sources();

        assert_eq!(sources.len(), QML_FILES.len());
        assert_eq!(
            sources.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
            QML_FILES.to_vec(),
            "the two lists must stay in the same order"
        );

        for ((name, source), path) in sources.iter().zip(qml_files()) {
            let on_disk = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));

            assert_eq!(
                *source, on_disk,
                "{name} is compiled in but {path:?} says something else"
            );
            assert!(!source.is_empty(), "{name} is empty");
            assert!(
                source.contains("import QtQuick"),
                "{name} is a QML file, not a placeholder"
            );
        }
    }

    #[test]
    fn the_catalog_page_is_one_of_the_assets() {
        let catalog = catalog_qml();

        assert!(catalog.is_file(), "{catalog:?} is missing");
        assert!(
            qml_files().contains(&catalog),
            "a host that compiles the catalog must not need a second list"
        );
    }

    #[test]
    fn the_file_list_is_unique_and_ordered_components_first() {
        let mut seen = std::collections::BTreeSet::new();

        for file in QML_FILES {
            assert!(seen.insert(file), "{file} is listed twice");
            assert!(file.ends_with(".qml"), "{file} is not a QML file");
        }

        assert_eq!(
            QML_FILES.last(),
            Some(&"CatalogWindow.qml"),
            "the catalog page is the last asset: it is the consumer of the other five"
        );
    }

    #[test]
    fn the_paths_are_computed_from_the_manifest_dir_not_the_working_directory() {
        let expected = Path::new(env!("CARGO_MANIFEST_DIR")).join("qml");

        for path in qml_files() {
            assert!(
                path.starts_with(&expected),
                "{path:?} must live under {expected:?} even when the test runs from the \
                 workspace root"
            );
        }
    }
}
