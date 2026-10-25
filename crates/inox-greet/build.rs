//! Build script for the greet QML module.
//!
//! Declares the `com.inox.greet.qml` module, bundles `qml/Main.qml` with it and
//! hands the bridge source to cxx-qt (bridges are declared
//! with `.files(...)`). The QML file keeps its own relative path as the resource
//! alias, so the engine loads it from
//! `qrc:/qt/qml/com/inox/greet/qml/qml/Main.qml` — the module path plus that
//! alias.

use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// Configure the cxx-qt build.
fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.greet.qml").qml_file("qml/Main.qml"))
        .files(["src/cxxqt_object.rs"])
        .qt_module("Quick")
        .build();
}
