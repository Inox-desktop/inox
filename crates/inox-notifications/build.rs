//! Build script for the cxx-qt bridge and the QML module.
//!
//! Declares the `com.inox.notifications.qml` QML module, bundles the module's
//! `Main.qml`, and hands the bridge source to cxx-qt (bridges are declared with `.files(...)`, and `.qt_module("Quick")` links the
//! Qt Quick module the window root needs).
//!
//! # The glass components
//!
//! A notification card is a pane of the shared liquid-glass material, and the
//! component that draws one (`GlassPane.qml`) lives in `inox-glass` (ADR 0044).
//! `inox-glass` ships its five components and the catalog page as **assets**, not
//! as a QML module: cross-crate static QML module registration is not reliable
//! (KDAB/cxx-qt#770), so the host crate compiles them into its own module
//! instead — exactly the arrangement `inox-panel` uses (ADR 0040 §3). They are
//! added as plain qrc resources with an explicit `qml/` alias rather than
//! through `QmlModule::qml_files()`, because that path aliases each file by the
//! string it is handed — an absolute path (`../../inox-glass/qml/GlassPane.qml`)
//! becomes a qrc path no `import` resolves. The alias puts them in the same
//! resource directory as `Main.qml`, which is what makes them same-directory
//! implicit types (`GlassPane { … }`).
//!
//! `qrc_resources` auto-prefixes a QML module's resources with
//! `/qt/qml/<uri as dirs>`, so the files land at
//! `qrc:/qt/qml/com/inox/notifications/qml/qml/<name>.qml`.

use cxx_qt_build::{CxxQtBuilder, QResource, QResources, QmlModule};
use qt_build_utils::QResourceFile;

/// The QML module URI, which is also the path the resource URL is built from:
/// `qrc:/qt/qml/com/inox/notifications/qml/` + the file's own relative path.
const MODULE: &str = "com.inox.notifications.qml";

/// Configure the cxx-qt build.
fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new(MODULE).qml_file("qml/Main.qml"))
        .files(["src/cxxqt_object.rs"])
        .qt_module("Quick")
        .qrc_resources(glass_resources())
        .build();
}

/// `inox-glass`'s QML assets, aliased into this module's own resource directory.
///
/// The list comes from the crate (`qml_files()` resolves against its
/// `CARGO_MANIFEST_DIR`, so it is path-correct from this build script); the
/// alias is the file name under the module's own `qml/` directory, which makes
/// `GlassPane`, `GlassToggle`, … implicit types next to `Main.qml`.
///
/// A file without a usable name cannot be aliased and is skipped rather than
/// panicking a build: the crate's own tests assert that every entry of
/// `qml_files()` is a `*.qml` file with a name.
fn glass_resources() -> QResources {
    let mut resource = QResource::new();

    for path in inox_glass::qml_files() {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        resource = resource.file(QResourceFile::new(&path).alias(format!("qml/{name}")));
    }

    QResources::new().resource(resource)
}
