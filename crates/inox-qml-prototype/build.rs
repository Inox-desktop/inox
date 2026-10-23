//! Build script for the cxx-qt QML prototype.
//!
//! Declares the `com.inox.prototype.qml` QML module, registers the module's
//! `main.qml` for bundling, and passes the bridge sources to `cxx-qt`.

use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// Configure the cxx-qt build.
fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new("com.inox.prototype.qml").qml_file("qml/main.qml"))
        .files(["src/cxxqt_object.rs"])
        .build();
}
