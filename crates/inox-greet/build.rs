//! Build script for the cxx-qt QML preview.
//!
//! Declares the `com.inox.greet.qml` QML module, bundles the module's
//! `main.qml`, and hands the bridge source to cxx-qt (ADR 0035 §实施执行约束:
//! bridges are declared with `.files(...)`).

use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// Configure the cxx-qt build.
fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.greet.qml").qml_file("qml/main.qml"))
        .files(["src/cxxqt_object.rs"])
        .build();
}
