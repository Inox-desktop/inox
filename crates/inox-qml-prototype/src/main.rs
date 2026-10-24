//! Inox QML prototype executable.
//!
//! Creates a `QGuiApplication` and a `QQmlApplicationEngine` and loads the
//! bundled `main.qml`, letting QML drive the Rust-backed `MyObject` bridge
//! object defined in this crate's library.

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QUrl};

// Cargo always links the library rlib into this binary; this explicit import
// also makes the intent clear that the generated cxx-qt C++/QML type
// registration must be pulled in here.
#[allow(unused_imports)]
use inox_qml_prototype::cxxqt_object;

/// Entry point: boot Qt and load the QML module's `main.qml`.
fn main() -> anyhow::Result<()> {
    let mut app = QGuiApplication::new();
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/com/inox/prototype/qml/main.qml"));
    }
    if let Some(app) = app.as_mut() {
        app.exec();
    }

    Ok(())
}
