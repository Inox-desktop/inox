//! Build script for the cxx-qt QML window.
//!
//! Declares the `com.inox.settings.qml` module, bundles every QML file the window
//! is made of under it, and hands the bridge source to cxx-qt through
//! `.files(...)`. The QML files are listed one by one on purpose: a file that is
//! not registered is a component the module cannot instantiate, and the offscreen
//! load test (`src/headless_tests/qml.rs`) says so.

use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// Configure the cxx-qt build.
fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.settings.qml").qml_files([
        "qml/Main.qml",
        "qml/SettingsSurface.qml",
        "qml/ControlButton.qml",
        "qml/FieldRow.qml",
        "qml/TextFieldControl.qml",
        "qml/NumberControl.qml",
        "qml/ChoiceRow.qml",
        "qml/FlagControl.qml",
        "qml/CollectionControl.qml",
        "qml/StatusStrip.qml",
        "qml/TrialDialog.qml",
    ]))
    .files(["src/cxxqt_object.rs"])
    .qt_module("Quick")
    .build();
}
