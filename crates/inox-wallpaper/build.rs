//! Build script for the cxx-qt QML preview.
//!
//! Declares the `com.inox.wallpaper.qml` QML module, bundles the module's
//! `Main.qml`, and hands the bridge source to cxx-qt. Bridges are declared with
//! `.files(...)`, not `.bridge(...)` (docs/qml-migration-contract.md §2.2).

use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// Configure the cxx-qt build.
fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.wallpaper.qml").qml_file("qml/Main.qml"))
        .files(["src/cxxqt_object.rs"])
        .build();
}
