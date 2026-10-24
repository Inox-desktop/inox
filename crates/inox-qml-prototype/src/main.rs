//! Inox QML prototype executable.
//!
//! Creates a `QGuiApplication` and a `QQmlApplicationEngine` and loads the
//! bundled `main.qml`, letting QML drive the Rust-backed `MyObject` bridge
//! object declared in `cxxqt_object`.
//!
//! The package is deliberately **bin-only** (there is no library target): cxx-qt
//! 0.10 embeds the bridge's generated C++ initializer definitions in the target
//! that declares them, so splitting a bridge between a library and a binary in
//! the same package leaves the initializer definitions unreferenced and the
//! binary fails to link with `undefined reference to 'cxx_qt_init_*'` (KDAB
//! issue #770). Keeping the bridge in the binary, as the upstream
//! `cargo_without_cmake` example does, makes `cargo build` and `cargo test`
//! both link.

mod cxxqt_object;

#[cfg(test)]
mod headless_tests;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

/// Resource URL of the module's root QML file.
///
/// cxx-qt mounts every `QmlModule::qml_file` under `qrc:/qt/qml/<uri as dirs>/`
/// **with the file's own relative path kept as the alias** (see the generated
/// `.qrc` in `OUT_DIR`: `<file alias="qml/main.qml">`). The module URI is
/// `com.inox.prototype.qml`, so the bundled resource is
/// `qrc:/qt/qml/com/inox/prototype/qml/qml/main.qml` — one `qml` segment more
/// than the URI prefix alone. Loading the shorter URL silently leaves the
/// engine without a root object, which is why this constant is named and
/// shared with the headless test instead of being repeated as a literal.
pub const MAIN_QML_URL: &str = "qrc:/qt/qml/com/inox/prototype/qml/qml/main.qml";

/// Entry point: boot Qt and load the QML module's `main.qml`.
fn main() -> anyhow::Result<()> {
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from(MAIN_QML_URL));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }

    Ok(())
}
