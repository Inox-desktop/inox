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
//!
//! # The glass components
//!
//! `inox-glass` ships its five components and the catalog page as **assets**,
//! not as a QML module: cross-crate static QML module registration is not
//! reliable (KDAB/cxx-qt#770), so the host crate compiles them into its own
//! module instead (ADR 0040 §3). They are added as plain qrc resources with an
//! explicit `qml/` alias rather than through `QmlModule::qml_files()`, because
//! that path aliases each file by the string it is handed — an absolute path
//! (`../../inox-glass/qml/GlassPane.qml`) becomes a qrc path no `import`
//! resolves. The alias puts them in the same resource directory as `Main.qml`,
//! which is what makes them same-directory implicit types (`GlassPane { … }`)
//! for both `Main.qml` and `CatalogWindow.qml`.
//!
//! `qrc_resources` auto-prefixes a QML module's resources with
//! `/qt/qml/<uri as dirs>`, so the files land at
//! `qrc:/qt/qml/com/inox/panel/qml/qml/<name>.qml`.

use cxx_qt_build::{CxxQtBuilder, QResource, QResources, QmlModule};
use qt_build_utils::QResourceFile;

/// The QML module URI, which is also the path the resource URL is built from:
/// `qrc:/qt/qml/com/inox/panel/qml/` + the file's own relative path.
const MODULE: &str = "com.inox.panel.qml";

fn main() {
    #[allow(unused_mut)]
    let mut builder = CxxQtBuilder::new_qml_module(QmlModule::new(MODULE).qml_file("qml/Main.qml"))
        .files(["src/cxxqt_object.rs"])
        .cpp_file("src/offscreen.cpp")
        .include_dir("src")
        .qt_module("Quick")
        .qrc_resources(glass_resources());

    if native_surface() {
        builder = builder.file("src/native_bridge.rs");
    }

    builder.build();
}

/// The native shell surface (`src/native/`), which needs what `cxx-qt-build`
/// will not do for us: `wayland-scanner` generating C client bindings for
/// `inox-shell-v1`, and `QtWaylandClient`'s private headers on the include path.
///
/// # Why a build-time probe and not a feature
///
/// ADR 0046 §10 makes the private ABI conditional on Qt >= 6.5 **and** the
/// private headers being installed. A cargo feature would not express that: this
/// workspace validates with `--all-features`, so a feature named for the native
/// path would be on for every CI job — including the ones whose Qt is older,
/// where it can only fail. The question "can this host carry the private ABI?" is
/// a property of the build host, so the build script answers it once and the
/// crate is compiled with `cfg(native_surface)` or without it.
///
/// A host without the headers gets the offscreen path and a `cargo:warning`
/// naming exactly what is missing. Nothing silently degrades at *runtime*: the
/// ADR's rule is that a panel which cannot be placed exits non-zero, and this is
/// about which renderer the binary contains, not about where it is placed.
///
/// The C bindings are generated rather than checked in: a checked-in copy is a
/// second source of truth for a protocol whose XML is the first one, and the
/// `inox-protocols` tests would keep passing while the panel spoke an older
/// revision. Returns whether the native path can be built.
fn native_surface() -> bool {
    use std::{path::PathBuf, process::Command};

    println!("cargo:rustc-check-cfg=cfg(native_surface)");
    println!("cargo:rerun-if-changed=src/native");
    println!("cargo:rerun-if-changed=../inox-protocols/protocol/inox-shell-v1.xml");
    println!("cargo:rerun-if-changed=../inox-protocols/protocol/inox-glass-v1.xml");
    println!("cargo:rerun-if-env-changed=INOX_NATIVE_SURFACE");

    if std::env::var_os("INOX_NATIVE_SURFACE").is_some_and(|value| value == "0") {
        println!(
            "cargo:warning=INOX_NATIVE_SURFACE=0: inox-panel is built with the offscreen host"
        );

        return false;
    }

    let version = match qt_version() {
        Some(version) => version,
        None => {
            println!(
                "cargo:warning=no `qmake -query QT_VERSION`; inox-panel is built with the \
                 offscreen host and the native shell surface is not compiled (ADR 0046 §10)"
            );

            return false;
        }
    };

    if version < (6, 5) {
        println!(
            "cargo:warning=Qt {}.{} is older than 6.5, so inox-panel is built with the offscreen \
             host: the native shell surface uses QtWaylandClient's private ABI and cannot be \
             compiled against this Qt (ADR 0046 §10)",
            version.0, version.1
        );

        return false;
    }

    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let generated = out.join("native");

    std::fs::create_dir_all(&generated).expect("creating the generated-binding directory");

    // `code`, not `private-code`: private code marks the interface symbols
    // hidden, which is right for a shared object that is the only user of the
    // protocol and wrong here — the panel's C++ and the generated C end up in
    // different objects of one archive, and a hidden symbol in one object cannot
    // be resolved from another.
    //
    // `inox-glass-v1` is generated for the same reason `inox-shell-v1` is: a
    // glass object has to be created on Qt's connection, because a protocol
    // proxy belongs to the connection whose `wl_surface` it names (ADR 0046 §8),
    // and Rust cannot reach that connection. Both are generated rather than
    // checked in, so the XML stays the only source of truth for either.
    let protocols = [
        ("inox-shell-v1.xml", "inox-shell", "inox_shell_protocol"),
        ("inox-glass-v1.xml", "inox-glass", "inox_glass_protocol"),
    ];

    for (xml_name, prefix, library) in protocols {
        let protocol_xml = PathBuf::from("../inox-protocols/protocol").join(xml_name);
        let header = generated.join(format!("{prefix}-client.h"));
        let code = generated.join(format!("{prefix}-protocol.c"));

        for (flag, output) in [("client-header", &header), ("public-code", &code)] {
            let status = Command::new("wayland-scanner")
                .arg(flag)
                .arg(&protocol_xml)
                .arg(output)
                .status()
                .unwrap_or_else(|error| {
                    panic!(
                        "running `wayland-scanner {flag}` for {xml_name}: {error}. \
                         Install the Wayland development tools (`libwayland-bin` on Debian/Ubuntu): \
                         the panel's shell role and its glass object are protocols, and their \
                         bindings are generated from `crates/inox-protocols/protocol/`."
                    )
                });

            assert!(
                status.success(),
                "`wayland-scanner {flag}` failed for {xml_name}"
            );
        }

        // The protocol's C is compiled **as C**. `cc`'s `cpp(true)` would compile
        // a `.c` file with the C++ compiler, and in C++ a `const` object at
        // namespace scope has internal linkage: the interface table would be a
        // private symbol in one object and undefined in the one that uses it.
        let mut protocol = cc::Build::new();
        protocol
            .include(&generated)
            .file(&code)
            .warnings(false)
            .compile(library);
    }

    let version_string = qt_query("QT_VERSION");
    let headers = PathBuf::from(qt_query("QT_INSTALL_HEADERS"));

    // Qt's private headers live under `include/<Module>/<version>/<Module>`, and
    // a private header in one module includes another's by bare path
    // (`<qpa/qplatformwindow_p.h>`), which is why both layouts are on the path.
    let modules = ["QtCore", "QtGui", "QtWaylandClient", "QtWaylandGlobal"];
    let mut includes = Vec::new();

    for module in modules {
        let private = headers.join(module).join(&version_string);

        if !private.is_dir() {
            println!(
                "cargo:warning=the private headers of Qt module {module} are missing at {}, so \
                 inox-panel is built with the offscreen host: the native shell surface needs \
                 `qt6-wayland-private-dev` (and qtbase's private headers) on Qt >= 6.5 \
                 (ADR 0046 §10)",
                private.display()
            );

            return false;
        }

        includes.push(private.clone());
        includes.push(private.join(module));
    }

    // The protocol's C is compiled **as C**. `cc`'s `cpp(true)` would compile a
    // `.c` file with the C++ compiler, and in C++ a `const` object at namespace
    // scope has internal linkage: the interface table would be a private symbol
    // in one object and undefined in the one that uses it. That is why each
    // protocol has its own `cc::Build` above and the C++ below has another.
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include(&generated)
        .include("src")
        .include(&headers)
        .include(headers.join("QtCore"))
        .include(headers.join("QtGui"))
        .include(headers.join("QtQml"))
        .include(headers.join("QtQuick"))
        .include(headers.join("QtWaylandClient"))
        .define("QT_WAYLANDCLIENT_LIB", None)
        .warnings(false)
        .file("src/native/qt_shell_surface.cpp")
        .file("src/native/host.cpp");

    for include in &includes {
        build.include(include);
    }

    build.compile("inox_panel_native");

    println!("cargo:rustc-link-lib=dylib=Qt6WaylandClient");
    println!("cargo:rustc-link-lib=dylib=wayland-client");
    println!("cargo:rustc-cfg=native_surface");

    true
}

/// `QMAJOR.QMINOR` from `qmake -query`, if a Qt can be found at all.
fn qt_version() -> Option<(u32, u32)> {
    let text = qt_query_optional("QT_VERSION")?;
    let mut parts = text.split('.');

    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;

    Some((major, minor))
}

/// One `qmake -query` answer, which is where Qt's own paths come from.
fn qt_query(key: &str) -> String {
    qt_query_optional(key).unwrap_or_else(|| {
        panic!(
            "`qmake -query {key}` failed: the native surface needs a Qt installation it can locate \
             (ADR 0046 §10)"
        )
    })
}

/// The same, for the answers that decide whether the native path is buildable.
fn qt_query_optional(key: &str) -> Option<String> {
    use std::process::Command;

    for qmake in ["qmake6", "qmake"] {
        let Ok(output) = Command::new(qmake).args(["-query", key]).output() else {
            continue;
        };

        if !output.status.success() {
            continue;
        }

        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();

        if !value.is_empty() {
            return Some(value);
        }
    }

    None
}

/// `inox-glass`'s QML assets, aliased into the panel's own resource directory.
///
/// The list comes from the crate (`qml_files()` resolves against its
/// `CARGO_MANIFEST_DIR`, so it is path-correct from this build script); the
/// alias is the file name under the module's own `qml/` directory, which makes
/// `GlassPane`, `GlassToggle`, … implicit types next to `Main.qml`.
///
/// A file without a usable name cannot be aliased and is skipped rather than
/// panicking a build: the crate's own tests assert that every entry of
/// `qml_files()` is a `*.qml` file with a name.
fn glass_resources() -> QResources {
    let mut resource = QResource::new();

    for path in inox_glass::qml_files() {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        resource = resource.file(QResourceFile::new(&path).alias(format!("qml/{name}")));
    }

    QResources::new().resource(resource)
}
