//! The QML module loading on Qt's offscreen platform, and reporting what it read.
//!
//! This is the half of the evidence a projection test cannot give: the projection
//! proves what the window *should* draw, and this proves that Qt Quick instantiates
//! the module, builds the tree, and that the tree read the projection it was
//! handed. It is the direct successor of the iced render tests' "the window is
//! drawn" assertions — Qt draws now, so the tree itself reports back.
//!
//! `QT_QPA_PLATFORM` has to be in the environment *before* Qt initialises, and
//! setting it from inside this process would call `std::env::set_var`, which
//! edition 2024 makes `unsafe` — and Inox-authored code writes no `unsafe` (root
//! `AGENTS.md` §3.2; the crate's `unsafe_code = "allow"` exists only for the
//! cxx-qt generated glue, the cxx-qt binding). The honest way to choose a platform from a
//! test is therefore to spawn this test binary again with the variable already
//! set — the pattern `docs/qml-migration-contract.md` §7.3 names.
//!
//! The child runs the real boot path (`crate::boot`), so the module it loads is
//! the one the installed binary loads, from the same `qrc:` URL. Readiness is not
//! "a URL was given to `load`": the window's `SettingsSurface` calls
//! `reportReady(title, pages, accessibleItems)` from `Component.onCompleted`, and
//! [`crate::cxxqt_object::qml_ready`] only becomes true when the title, the number
//! of navigation entries and the size of the semantic tree all agree with the
//! projection (root `AGENTS.md` §7.4).

use std::process::{Command, Output};

/// Environment variable the parent sets on the child it spawns.
const CHILD_MARKER: &str = "INOX_SETTINGS_QML_CHILD";

/// What the child prints once the tree has reported ready.
const READY_MARKER: &str = "INOX_SETTINGS_QML_READY";

/// Everything the child process said, both streams.
fn said(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr)
}

/// Spawns this test binary as a process with the offscreen platform selected.
fn run_as_process() -> Output {
    Command::new(std::env::current_exe().expect("the test binary path"))
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "headless_tests::qml::qml_child",
        ])
        .env(CHILD_MARKER, "1")
        .env("QT_QPA_PLATFORM", "offscreen")
        // Qt 6 writes warnings to stderr only when it believes stderr is a
        // console; a test harness is not one, so a QML load failure would
        // otherwise be invisible in the captured output. This is why a broken
        // QML file shows up as "the window did not come up" with no reason.
        .env("QT_ASSUME_STDERR_HAS_CONSOLE", "1")
        .env("QT_FORCE_STDERR_LOGGING", "1")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .output()
        .expect("the child test process runs")
}

/// The child half of [`the_window_loads_offscreen_and_reads_the_projection`].
///
/// It boots the window against a temporary configuration directory and returns
/// from the Qt event loop instead of entering it: the same code path the binary
/// takes, minus the wait for a user.
#[test]
#[ignore = "spawned as a child process by the_window_loads_offscreen_and_reads_the_projection"]
fn qml_child() {
    assert_eq!(
        std::env::var(CHILD_MARKER).as_deref(),
        Ok("1"),
        "this test only runs as the spawned child"
    );

    let directory = tempfile::tempdir().expect("a temporary configuration directory");
    let paths = inox_config::ConfigPaths::new(directory.path());
    let config = inox_config::InoxConfig::load(&paths, inox_config::Appearance::System)
        .expect("the configuration loads");

    // The child talks to the parent through its exit status and the marker it
    // prints, so the window's own log goes where a developer running the child by
    // hand can read it: a readiness mismatch is a `warn!` naming what disagreed.
    let _ = crate::initialise_logging();

    crate::boot(&paths, &config, true)
        .expect("the QML module loads and the tree reports what it built");
    assert!(
        crate::cxxqt_object::qml_ready(),
        "the QML tree must confirm it read the Rust projection"
    );

    // The parent cannot see this process's tracing subscriber, and `println!` is
    // allowed in tests (root `AGENTS.md` §3.5).
    println!("{READY_MARKER}");
}

#[test]
fn the_window_loads_offscreen_and_reads_the_projection() {
    let output = run_as_process();
    let text = said(&output);

    assert!(output.status.success(), "the offscreen load failed: {text}");
    assert!(
        text.contains(READY_MARKER),
        "the child did not report a ready tree: {text}"
    );

    // Qt reports QML errors on stderr even when the process survives, so a
    // window that "loaded" with a broken binding still has to fail here.
    for marker in [
        "is not a type",
        "Cannot assign",
        "Unable to assign",
        "TypeError",
        "Cannot read property",
        "ReferenceError",
    ] {
        assert!(!text.contains(marker), "QML reported an error: {text}");
    }
}
