//! Build script for the panel's QML module and its offscreen C++ host.
//!
//! The panel's interface is QML rendered offscreen: the QML module
//! below is bundled into a resource, and `src/offscreen.cpp` is the thin
//! `QGuiApplication` + `QQmlApplicationEngine` + `QQuickWindow::grabWindow`
//! host the bridge in `src/cxxqt_object.rs` talks to. `include_dir("src")` lets
//! that bridge say `include!("offscreen.h")`.
//!
//! `qt_module("Quick")` is explicit rather than inferred: the host creates a
//! `QQuickWindow`, which `cxx-qt-build` cannot see from the C++ source alone.

use cxx_qt_build::{CxxQtBuilder, QmlModule};

/// The QML module URI, which is also the path the resource URL is built from:
/// `qrc:/qt/qml/com/inox/panel/qml/` + the file's own relative path.
const MODULE: &str = "com.inox.panel.qml";

fn main() {
    CxxQtBuilder::new_qml_module(QmlModule::new(MODULE).qml_file("qml/Main.qml"))
        .files(["src/cxxqt_object.rs"])
        .cpp_file("src/offscreen.cpp")
        .include_dir("src")
        .qt_module("Quick")
        .build();
}
