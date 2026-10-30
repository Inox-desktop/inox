//! The native shell surface on a **real** compositor (ADR 0046 P3, evidence
//! level E2).
//!
//! What this proves, and what it does not:
//!
//! - It proves the halves of P3 that the private ABI makes risky: a real Qt
//!   window takes an `inox_shell_surface_v1` role instead of an `xdg_toplevel`,
//!   the compositor sizes it from the placement the panel sent, the work-area and
//!   capability events arrive, a popup of it is placed and configured, the
//!   compositor dismisses that popup with `parent_hidden` when the panel's
//!   surface unmaps, and a glass object created on Qt's connection is answered
//!   with capabilities.
//! - It proves the **panel loop** the binary runs (`crate::native_run`) does the
//!   same: placed, configured at its configured size, projection sent, work area
//!   received, glass wired up for a pane.
//! - It does **not** prove that the panel *looks* right: no pixels are asserted
//!   here and this run has no GPU. That is the same line `docs/quality` draws
//!   between E2 and E3, and the ADR's "E3 needs a real host screenshot" note
//!   still stands.
//!
//! # Why a real compositor and a child process
//!
//! The compositor is `inox-compositor --headless`, the same binary a session
//! runs, because the point of this test is the interaction between Qt's wayland
//! client and *this* compositor's shell implementation; a socket pair playing
//! both sides would test the protocol twice and neither implementation.
//!
//! The Qt client runs in a child process — this test executable, filtered down to
//! one entry point, the pattern `headless_tests::binary` established. It has to:
//! Qt allows one `QGuiApplication` per process, and each of these entries creates
//! one.
//!
//! # The gate
//!
//! This needs a built `inox-compositor` next to the test binary and a working
//! Wayland socket, so it runs only with `INOX_SHELL_E2E=1` — the same shape as
//! `INOX_M3_E2E` and `INOX_XWAYLAND_E2E`. Without the gate it prints why it did
//! nothing and passes.

use std::{
    fs::File,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    rc::Rc,
    time::{Duration, Instant},
};

use crate::native_surface::{Event, Surface};

/// The entry point that drives a bare surface: the private ABI's own test.
const SURFACE_ENTRY_POINT: &str = "headless_tests::native::the_native_surface_client";

/// The entry point that drives the panel's loop on a taskbar.
const LOOP_ENTRY_POINT: &str = "headless_tests::native::the_panel_loop_client";

/// The entry point that drives the panel's loop on a menu bar, which is a pane
/// and therefore asks for glass.
const GLASS_ENTRY_POINT: &str = "headless_tests::native::the_glass_pane_client";

/// What tells a child that it *is* the client, rather than a normal test run.
const SURFACE_CLIENT: &str = "INOX_NATIVE_SURFACE_CLIENT";

/// What tells a child that it is the panel's loop.
const LOOP_CLIENT: &str = "INOX_NATIVE_PANEL_CLIENT";

/// What tells a child that it is the panel's loop on a glass pane.
const GLASS_CLIENT: &str = "INOX_NATIVE_GLASS_CLIENT";

/// The gate. Absent, these tests report that they did nothing.
const GATE: &str = "INOX_SHELL_E2E";

/// How long the compositor may take to create its socket.
const STARTUP: Duration = Duration::from_secs(15);

/// How long the surface may take to be configured.
const CONFIGURE: Duration = Duration::from_secs(15);

/// The compositor binary, which sits next to the test executable's directory.
///
/// `cargo test -p inox-panel` does not build another crate's binary, so this
/// names the command that does rather than failing with a path nobody can act
/// on.
fn compositor() -> PathBuf {
    let test_executable = std::env::current_exe().expect("the test executable's own path");

    // `target/<profile>/deps/<test>` → `target/<profile>`
    let directory = test_executable
        .parent()
        .and_then(Path::parent)
        .expect("the test executable lives two directories below the target directory");

    let binary = directory.join("inox-compositor");

    assert!(
        binary.is_file(),
        "{} is missing: build it first with `cargo build -p inox-compositor` ({GATE} asks for the \
         real compositor, not a stand-in)",
        binary.display()
    );

    binary
}

/// A runtime directory the compositor and the client both use.
///
/// Wayland refuses a socket in a directory other users can read, so the mode is
/// the one a session's own runtime directory has.
fn runtime_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a temporary runtime directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("the runtime directory has to be private");

    directory
}

/// Waits for `path` to exist, or returns how long it waited.
fn wait_for_socket(path: &Path) -> bool {
    let deadline = Instant::now() + STARTUP;

    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }

        std::thread::sleep(Duration::from_millis(20));
    }

    false
}

/// Runs one child entry point against a real `inox-compositor`, and returns what
/// both of them said.
///
/// The compositor is killed before the result is inspected, so a failing
/// assertion can print its log rather than leaving a process behind.
fn round_trip(entry: &str, variable: &str) -> (Output, String) {
    let runtime = runtime_directory();
    let socket = format!("inox-shell-{}", std::process::id());
    let log_path = runtime.path().join("compositor.log");
    let log = File::create(&log_path).expect("a log file for the compositor");

    let mut compositor = Command::new(compositor())
        .arg("--headless")
        .arg("--socket")
        .arg(&socket)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdout(log.try_clone().expect("a copy of the log handle"))
        .stderr(log)
        .spawn()
        .expect("the compositor starts");

    let socket_path = runtime.path().join(&socket);

    if !wait_for_socket(&socket_path) {
        let _ = compositor.kill();
        let mut text = String::new();
        let _ = File::open(&log_path).and_then(|mut file| file.read_to_string(&mut text));

        panic!(
            "the compositor did not create {} within {STARTUP:?}:\n{text}",
            socket_path.display()
        );
    }

    let client = std::env::current_exe().expect("the test executable's own path");

    let output = Command::new(client)
        .arg("--exact")
        .arg(entry)
        .arg("--test-threads=1")
        .arg("--nocapture")
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", &socket)
        // The platform is the subject of these tests, so it is named here rather
        // than inherited: `cargo test -p inox-panel` is normally run with
        // `QT_QPA_PLATFORM=offscreen` (the offscreen host's tests need it), and a
        // client that inherited that would have no Wayland surface at all.
        .env("QT_QPA_PLATFORM", "wayland")
        .env(variable, "1")
        .output()
        .expect("the client runs");

    let _ = compositor.kill();
    let _ = compositor.wait();

    let mut compositor_log = String::new();
    let _ = File::open(&log_path).and_then(|mut file| file.read_to_string(&mut compositor_log));

    (output, compositor_log)
}

/// Asserts that a child ran, and prints everything both sides said when it did
/// not.
fn assert_ran(output: &Output, compositor_log: &str, what: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "{what} failed on a real compositor\n--- client stdout ---\n{stdout}\n\
         --- client stderr ---\n{stderr}\n--- compositor ---\n{compositor_log}"
    );
}

/// The native surface against a real compositor.
#[test]
fn a_real_compositor_places_the_native_surface() {
    if std::env::var_os(GATE).is_none() {
        println!("{GATE} is not set: the native surface's compositor test did nothing");

        return;
    }

    let (output, compositor_log) = round_trip(SURFACE_ENTRY_POINT, SURFACE_CLIENT);

    assert_ran(
        &output,
        &compositor_log,
        "the native surface's role, placement and popup",
    );
}

/// The panel's loop, on a taskbar, against a real compositor.
#[test]
fn the_panel_loop_places_itself() {
    if std::env::var_os(GATE).is_none() {
        println!("{GATE} is not set: the panel loop's compositor test did nothing");

        return;
    }

    let (output, compositor_log) = round_trip(LOOP_ENTRY_POINT, LOOP_CLIENT);

    assert_ran(&output, &compositor_log, "the panel loop");
}

/// The panel's loop, on a menu bar: the pane asks for glass, and the object
/// created on Qt's connection is answered.
#[test]
fn a_glass_pane_is_answered() {
    if std::env::var_os(GATE).is_none() {
        println!("{GATE} is not set: the glass pane's compositor test did nothing");

        return;
    }

    let (output, compositor_log) = round_trip(GLASS_ENTRY_POINT, GLASS_CLIENT);

    assert_ran(&output, &compositor_log, "the panel loop on a glass pane");
}

/// The **shipped binary**, on a real compositor: `main` → [`crate::run`] →
/// [`crate::native_run::run`].
///
/// The tests above drive the pieces; this one drives the process. What it asserts
/// is deliberately coarse — the binary is still running after it has had time to
/// connect, take its role and be configured, and its log says which surface it
/// placed. A panel that could not be placed exits non-zero within that window
/// (`EXIT_NO_SURFACE`), so "still alive" is a real assertion, not a timeout.
#[test]
fn the_panel_binary_places_itself() {
    use std::process::Stdio;

    if std::env::var_os(GATE).is_none() {
        println!("{GATE} is not set: the panel binary's compositor test did nothing");

        return;
    }

    let runtime = runtime_directory();
    let socket = format!("inox-panel-bin-{}", std::process::id());
    let log_path = runtime.path().join("compositor.log");
    let log = File::create(&log_path).expect("a log file for the compositor");

    let mut compositor = Command::new(compositor())
        .arg("--headless")
        .arg("--socket")
        .arg(&socket)
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdout(log.try_clone().expect("a copy of the log handle"))
        .stderr(log)
        .spawn()
        .expect("the compositor starts");

    assert!(
        wait_for_socket(&runtime.path().join(&socket)),
        "the compositor never created its socket"
    );

    // An empty configuration directory: the panel reads the built-in defaults,
    // which is the state a fresh session is in.
    let config = runtime.path().join("config");
    std::fs::create_dir_all(&config).expect("a configuration directory");

    let mut child = crate::headless_tests::binary::panel_command()
        .env("XDG_RUNTIME_DIR", runtime.path())
        .env("WAYLAND_DISPLAY", &socket)
        .env("QT_QPA_PLATFORM", "wayland")
        .env("XDG_CONFIG_HOME", &config)
        // No arguments: the harness's own (`--exact`, `--test-threads`) are not
        // the panel's, so the panel's arrive as an empty list instead.
        .env(crate::headless_tests::binary::ARGUMENTS, "")
        .stdout(Stdio::from(
            File::create(runtime.path().join("panel.log")).expect("a log file for the panel"),
        ))
        .stderr(Stdio::from(
            File::create(runtime.path().join("panel.err")).expect("an error file for the panel"),
        ))
        .spawn()
        .expect("the panel starts");

    // The one line that says the binary placed a shell surface. Polled rather
    // than slept for: a loaded machine takes as long as it takes, and a fixed
    // sleep would make this test a race against whatever else is running.
    let deadline = Instant::now() + CONFIGURE;
    let mut placed = false;

    while Instant::now() < deadline {
        let text = std::fs::read_to_string(runtime.path().join("panel.log")).unwrap_or_default();

        if text.contains("native inox_shell_surface_v1 surface") {
            placed = true;

            break;
        }

        std::thread::sleep(Duration::from_millis(20));
    }

    let exited = child.try_wait().expect("the child's state").is_some();
    let _ = child.kill();
    let _ = child.wait();

    let _ = compositor.kill();
    let _ = compositor.wait();

    let stdout = std::fs::read_to_string(runtime.path().join("panel.log")).unwrap_or_default();
    let stderr = std::fs::read_to_string(runtime.path().join("panel.err")).unwrap_or_default();
    let mut compositor_log = String::new();
    let _ = File::open(&log_path).and_then(|mut file| file.read_to_string(&mut compositor_log));

    assert!(
        placed,
        "the panel never reported a placed surface\n--- stdout ---\n{stdout}\n\
         --- stderr ---\n{stderr}\n--- compositor ---\n{compositor_log}"
    );
    assert!(
        !exited,
        "the panel exited instead of staying on its surface\n--- stdout ---\n{stdout}\n\
         --- stderr ---\n{stderr}\n--- compositor ---\n{compositor_log}"
    );
}

/// The Qt client: run by the child process, not by an ordinary test run.
///
/// It is `#[test]` so the harness can select it with `--exact`; outside the
/// child it returns immediately, like `headless_tests::binary`'s entry point.
#[test]
fn the_native_surface_client() {
    if std::env::var_os(SURFACE_CLIENT).is_none() {
        return;
    }

    use crate::native_surface::{Geometry, KeyboardMode, Layer, Role, anchor};

    let mut surface = Surface::start(Role::Panel).expect("the native surface starts");

    let geometry = Geometry {
        layer: Layer::Top,
        anchor: anchor::TOP | anchor::LEFT | anchor::RIGHT,
        exclusive_zone: 28,
        margin_top: 0,
        margin_right: 0,
        margin_bottom: 0,
        margin_left: 0,
        width: 0,
        height: 28,
    };

    surface.place(&geometry).expect("the placement is sent");
    surface
        .keyboard(KeyboardMode::None)
        .expect("the keyboard mode is sent");
    surface
        .transition(1, 1, 120, 1, 0)
        .expect("the motion hint is sent");

    // The projection is one JSON document, and nothing in the host interprets
    // it: what arrives here is what `view.rs` produced.
    surface
        .set_model(&serde_json::json!({"background": "transparent", "items": []}))
        .expect("the projection is handed over");

    assert!(
        surface.next_action().is_none(),
        "nothing has been clicked, so the renderer has nothing to report"
    );

    let (capabilities, configured, workarea) = drain_until(&mut surface, |seen| {
        seen.configured.is_some() && seen.workarea.is_some() && seen.capabilities.is_some()
    });

    let capabilities = capabilities.expect("the compositor reports its capabilities");

    assert!(
        Surface::offered(capabilities.0, crate::native_surface::capability::WORKAREA),
        "the work area is what a shell lays a menu out against: {capabilities:?}"
    );
    assert_eq!(
        capabilities.1, 250,
        "the longest transition the compositor will draw, milliseconds"
    );

    // 1280 is the headless output's width: the surface is anchored to both
    // horizontal edges, so the compositor gives it the full width and the 28 px
    // thickness the placement asked for.
    assert_eq!(
        configured,
        Some((1280, 28)),
        "the configured size is authoritative, and it is the placement the panel sent"
    );

    let workarea = workarea.expect("the compositor sends the work area");
    assert_eq!(
        workarea.2, 1280,
        "the work area is at least as wide as the panel: {workarea:?}"
    );
    assert!(
        workarea.3 <= 720 && workarea.3 > 0,
        "the work area is inside the 1280x720 headless output: {workarea:?}"
    );

    // A menu: anchored to the panel's bottom-left corner, opening downwards and
    // to the right, and dismissable — which is the whole of what the client
    // declares about a menu's fate.
    let popup = surface
        .open_popup(
            anchor::BOTTOM | anchor::LEFT,
            anchor::TOP | anchor::LEFT,
            (4, 0),
            (180, 240),
            true,
        )
        .expect("a popup of the panel opens");

    let popup_size = drain_popup(&mut surface, popup);

    assert_eq!(
        popup_size,
        Some((180, 240)),
        "the popup is configured at the size it asked for"
    );

    // What is *not* asserted here, and why: the compositor's dismissal
    // (`dismissed(outside | escape | parent_hidden)`) only fires for a popup it
    // considers on screen, which needs a client buffer. This host has no GPU and
    // Qt's wayland client reports the window as not exposed, so it never paints —
    // see the ADR's "P3 open items". The compositor's own
    // `crates/inox-compositor/tests/shell_surface.rs` covers every dismissal
    // reason server-side; what is left for a machine that can paint is the
    // client's reaction to it.
    assert!(
        surface.dismiss_popup(popup),
        "the dismiss request reaches the popup object"
    );

    // Unmapping is the compositor's cue to dismiss whatever is left; the test
    // does it here so the teardown order the panel will use is the one that is
    // exercised.
    surface.hide();
}

/// Loads the panel's catalogs, as `main` does, for a child that runs the loop.
fn localizer() -> Rc<inox_common::i18n::Localizer> {
    use inox_common::i18n::Locale;

    Rc::new(crate::i18n::load(&Locale::default_locale()).expect("the panel's catalogs parse"))
}

/// Runs the panel's state machine until it is configured, and answers it.
///
/// This is the loop `main` runs, minus the two `calloop` sources and the reload
/// host: what is asserted is the panel's own reaction to a real compositor, which
/// is the half that can be wrong in a way no unit test sees.
fn drive(
    shell: inox_config::ShellConfig,
    surface: inox_config::PanelSurface,
) -> crate::native_run::Panel {
    let mut shell = shell;
    shell.panel.surface = surface;

    let mut panel =
        crate::native_run::Panel::start(shell, inox_config::Theme::default(), localizer())
            .expect("the panel starts on a native shell surface");

    assert!(
        panel.drive_until(|panel| panel.configured().is_some(), CONFIGURE),
        "the compositor never configured the panel's surface"
    );

    panel
}

/// The panel's loop on a taskbar: run by the child process.
#[test]
fn the_panel_loop_client() {
    if std::env::var_os(LOOP_CLIENT).is_none() {
        return;
    }

    let mut panel = drive(
        inox_config::ShellConfig::default(),
        inox_config::PanelSurface::Taskbar,
    );

    // The default panel is a 32 px strip on the bottom edge, anchored to both
    // horizontal edges: the compositor gives it the output's width.
    assert_eq!(
        panel.configured(),
        Some((1280, 32)),
        "the configured size is the placement the panel sent"
    );

    let scene = panel.scene().expect("the projection was sent to QML");
    assert_eq!(scene.surface, inox_config::PanelSurface::Taskbar);
    assert!(
        scene.json.get("items").is_some(),
        "the projection is what QML draws: {}",
        scene.json
    );

    assert!(
        panel.workarea().is_some(),
        "the work area is the event layer-shell does not have"
    );
    assert_eq!(
        panel.max_transition(),
        250,
        "the compositor's longest transition, which bounds the panel's hint"
    );
    assert!(
        !panel.has_glass(),
        "a taskbar draws the opaque strip token: there is nothing to frost"
    );

    // Nothing was clicked: the renderer has nothing to report that would have
    // changed the projection behind the loop's back.
    assert_eq!(panel.outcome(), crate::layer::Outcome::Stopped);

    panel.teardown();
}

/// The panel's loop on a menu bar: a pane, so a glass object is created and the
/// compositor answers it — run by the child process.
#[test]
fn the_glass_pane_client() {
    if std::env::var_os(GLASS_CLIENT).is_none() {
        return;
    }

    let mut panel = drive(
        inox_config::ShellConfig::default(),
        inox_config::PanelSurface::Menubar,
    );

    assert_eq!(
        panel.configured(),
        Some((1280, 28)),
        "the menu bar's own height, and the output's width"
    );

    assert!(
        panel.has_glass(),
        "a menu bar is a pane: the glass object is created on Qt's connection"
    );

    assert!(
        panel.drive_until(|panel| panel.glass_capabilities().is_some(), CONFIGURE),
        "the compositor answered the glass object's capabilities"
    );

    panel.teardown();
}

/// The capabilities, the configured size and the work area.
type Arrived = (
    Option<(u32, u32)>,
    Option<(u32, u32)>,
    Option<(i32, i32, i32, i32)>,
);

/// What the first few pumps of the surface saw.
#[derive(Default)]
struct Seen {
    capabilities: Option<(u32, u32)>,
    configured: Option<(u32, u32)>,
    workarea: Option<(i32, i32, i32, i32)>,
}

/// Pumps the surface until `done` accepts what has arrived, or the deadline
/// passes.
fn drain_until(surface: &mut Surface, done: impl Fn(&Seen) -> bool) -> Arrived {
    let deadline = Instant::now() + CONFIGURE;
    let mut seen = Seen::default();

    while Instant::now() < deadline {
        surface.pump();

        while let Some(event) = surface.next_event() {
            match event {
                Event::Capabilities {
                    bits,
                    max_transition_ms,
                } => seen.capabilities = Some((bits, max_transition_ms)),
                Event::Configure { width, height } => seen.configured = Some((width, height)),
                Event::Workarea {
                    x,
                    y,
                    width,
                    height,
                } => seen.workarea = Some((x, y, width, height)),
                _ => {}
            }
        }

        if done(&seen) {
            break;
        }

        std::thread::sleep(Duration::from_millis(5));
    }

    (seen.capabilities, seen.configured, seen.workarea)
}

/// Pumps until `popup` is configured.
fn drain_popup(surface: &mut Surface, popup: u32) -> Option<(u32, u32)> {
    let deadline = Instant::now() + CONFIGURE;

    while Instant::now() < deadline {
        surface.pump();

        while let Some(event) = surface.next_event() {
            if let Event::PopupConfigure {
                popup: id,
                width,
                height,
            } = event
                && id == popup
            {
                return Some((width, height));
            }
        }

        std::thread::sleep(Duration::from_millis(5));
    }

    None
}
