//! Build script for the QML launcher.
//!
//! Declares the `com.inox.launcher.qml` QML module, bundles `qml/Main.qml` into
//! it, and hands the bridge source to cxx-qt. The module's resource URL is
//! `qrc:/qt/qml/com/inox/launcher/qml/qml/Main.qml` — the module URI with dots
//! turned into path segments, plus the file's own relative path (see
//! `docs/qml-migration-contract.md` §2.2 and `src/main.rs`).

use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// Configure the cxx-qt build.
fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.launcher.qml").qml_file("qml/Main.qml"))
        .files(["src/cxxqt_object.rs"])
        .qt_module("Quick")
        .build();
}
