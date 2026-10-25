//! Build script for the cxx-qt bridge and the QML module.
//!
//! Declares the `com.inox.notifications.qml` QML module, bundles the module's
//! `Main.qml`, and hands the bridge source to cxx-qt (//! bridges are declared with `.files(...)`, and `.qt_module("Quick")` links the
//! Qt Quick module the window root needs).

use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// Configure the cxx-qt build.
fn main() {
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.inox.notifications.qml").qml_file("qml/Main.qml"),
    )
    .files(["src/cxxqt_object.rs"])
    .qt_module("Quick")
    .build();
}
