//! Build script for the lock screen's QML module and offscreen host.
//!
//! Declares the `com.inox.lock.qml` QML module, bundles the module's
//! `qml/Main.qml`, hands the cxx-qt bridge to cxx-qt, and compiles this crate's
//! C++ offscreen host (`src/offscreen.cpp`, ). `qt_module("Quick")` is
//! named explicitly because the host draws with Qt Quick rather than Qt Core
//! alone; no Wayland Qt module is linked — the lock surface is the crate's own.

/// Configure the cxx-qt build.
fn main() {
    cxx_qt_build::CxxQtBuilder::new_qml_module(
        cxx_qt_build::QmlModule::new("com.inox.lock.qml").qml_file("qml/Main.qml"),
    )
    .files(["src/cxxqt_object.rs"])
    .cpp_file("src/offscreen.cpp")
    .include_dir("src")
    .qt_module("Quick")
    .build();
}
