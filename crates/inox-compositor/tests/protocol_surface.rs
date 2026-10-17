//! Integration test for the M3 protocol surface: a running compositor process advertises
//! the globals XWayland interop is built on, and they are bindable.
//!
//! Like `ipc_global.rs` this drives a real `inox-compositor` process, so the test never
//! holds an EGL display itself. The client half is a plain `wayland-client` connection,
//! and the Wayland socket lives in a private `XDG_RUNTIME_DIR`.
//!
//! The client interaction runs on a worker thread so that the blocking wayland roundtrips
//! are bounded by a timeout: a broken compositor then fails the test instead of hanging
//! CI.

use std::{
    fs,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread::{self, sleep},
    time::{Duration, Instant},
};

use inox_protocols::standard::{
    core::client::wl_compositor::WlCompositor,
    wlr::layer_shell::client::zwlr_layer_shell_v1::ZwlrLayerShellV1,
    wp::fractional_scale::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_data_device_manager::WlDataDeviceManager, wl_registry},
};
use wayland_protocols::wp::{
    primary_selection::zv1::client::zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
    viewporter::client::wp_viewporter::WpViewporter,
};

/// The compositor binary built for this test run.
const COMPOSITOR: &str = env!("CARGO_BIN_EXE_inox-compositor");

/// How long the compositor may take to bind its socket before the test gives up.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the compositor may take to exit after being asked to.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the client interaction may take before the test gives up.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

/// The globals every build of the compositor advertises, M1 through M4.
const EXPECTED_GLOBALS: &[&str] = &[
    "wl_compositor",
    "wl_subcompositor",
    "wl_shm",
    "wl_seat",
    "wl_output",
    "xdg_wm_base",
    "zwlr_layer_shell_v1",
    "zxdg_output_manager_v1",
    "inox_ipc",
    // M4, ADR 0020: the exact scale a surface should render at.
    "wp_fractional_scale_manager_v1",
];

/// A private `XDG_RUNTIME_DIR` for one compositor process.
struct RuntimeDir(PathBuf);

impl RuntimeDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("inox-compositor-{label}-{}", std::process::id()));

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create the test runtime directory");

        Self(path)
    }

    fn member(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for RuntimeDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Start a compositor with a private runtime directory and a known socket name.
fn spawn(socket_name: &str) -> (Child, RuntimeDir, PathBuf) {
    let runtime_dir = RuntimeDir::new(socket_name);

    let child = Command::new(COMPOSITOR)
        .args(["--headless", "--socket", socket_name])
        .env("XDG_RUNTIME_DIR", runtime_dir.0.as_os_str())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn inox-compositor");

    let socket = runtime_dir.member(socket_name);

    (child, runtime_dir, socket)
}

/// Block until the compositor has bound its Wayland socket.
fn wait_for_socket(path: &Path) {
    let deadline = Instant::now() + STARTUP_TIMEOUT;

    while Instant::now() < deadline {
        if path.exists() {
            return;
        }

        sleep(Duration::from_millis(20));
    }

    panic!(
        "inox-compositor did not bind {} within {STARTUP_TIMEOUT:?}",
        path.display()
    );
}

/// Wait for the compositor to exit, killing it if it takes too long.
fn wait_with_timeout(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;

    loop {
        if let Some(status) = child.try_wait().expect("failed to poll inox-compositor") {
            return status;
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();

            panic!("inox-compositor did not exit within {SHUTDOWN_TIMEOUT:?}");
        }

        sleep(Duration::from_millis(20));
    }
}

/// Send `SIGTERM` to the compositor.
fn send_sigterm(child: &Child) {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("failed to run kill");

    assert!(status.success(), "kill -TERM exited with {status}");
}

/// The client-side dispatch state. The registry events are what this test reads, and
/// `registry_queue_init` records them; nothing else is dispatched here.
struct RegistryClient;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for RegistryClient {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` already captured the global list.
    }
}

wayland_client::delegate_noop!(RegistryClient: ignore WlCompositor);
wayland_client::delegate_noop!(RegistryClient: ignore WpViewporter);
wayland_client::delegate_noop!(RegistryClient: ignore WpFractionalScaleManagerV1);
wayland_client::delegate_noop!(RegistryClient: ignore WlDataDeviceManager);
wayland_client::delegate_noop!(RegistryClient: ignore ZwpPrimarySelectionDeviceManagerV1);
wayland_client::delegate_noop!(RegistryClient: ignore ZwlrLayerShellV1);

/// What the client observed while talking to the compositor.
#[derive(Debug, Default)]
struct ClientOutcome {
    /// Every advertised global, as `(interface, version)`.
    globals: Vec<(String, u32)>,
}

/// Connect, enumerate the registry and bind the M3 globals.
///
/// Binding is the stronger assertion of the two: a global that is advertised but cannot be
/// bound answers with a protocol error, which fails the roundtrip below.
fn talk_to_compositor(socket_path: &Path) -> anyhow::Result<ClientOutcome> {
    let stream = UnixStream::connect(socket_path)?;
    let connection = Connection::from_socket(stream)?;

    let mut state = RegistryClient;
    let (globals, mut queue) = registry_queue_init::<RegistryClient>(&connection)?;
    let queue_handle = queue.handle();

    let advertised: Vec<(String, u32)> = globals.contents().with_list(|list| {
        list.iter()
            .map(|global| (global.interface.clone(), global.version))
            .collect()
    });

    // v6 is what carries `preferred_buffer_scale`; a client that binds lower
    // cannot be told how large a buffer to render for a scaled output (ADR 0020).
    let _compositor: WlCompositor = globals.bind(&queue_handle, 6..=6, ())?;
    let _fractional: WpFractionalScaleManagerV1 = globals.bind(&queue_handle, 1..=1, ())?;
    let _viewporter: WpViewporter = globals.bind(&queue_handle, 1..=1, ())?;
    let _data_device: WlDataDeviceManager = globals.bind(&queue_handle, 3..=3, ())?;
    let _primary: ZwpPrimarySelectionDeviceManagerV1 = globals.bind(&queue_handle, 1..=1, ())?;
    let _layers: ZwlrLayerShellV1 = globals.bind(&queue_handle, 4..=4, ())?;

    // The compositor answers the sync only if every request above was accepted.
    queue.roundtrip(&mut state)?;

    Ok(ClientOutcome {
        globals: advertised,
    })
}

/// The version of `interface` in `globals`, if it was advertised at all.
fn version_of(globals: &[(String, u32)], interface: &str) -> Option<u32> {
    globals
        .iter()
        .find(|(name, _)| name == interface)
        .map(|(_, version)| *version)
}

#[test]
fn the_protocol_surface_is_advertised_and_bindable() {
    let (mut child, _runtime_dir, socket) = spawn("inox-protocol-surface");
    wait_for_socket(&socket);

    let (sender, receiver) = mpsc::channel();
    let client = thread::spawn(move || {
        let _ = sender.send(talk_to_compositor(&socket));
    });

    let outcome = match receiver.recv_timeout(CLIENT_TIMEOUT) {
        Ok(result) => result.expect("the protocol-surface client interaction failed"),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();

            panic!("the protocol-surface client did not finish within {CLIENT_TIMEOUT:?}");
        }
    };

    // The compositor must stay healthy after serving a client that bound everything.
    send_sigterm(&child);
    let status = wait_with_timeout(&mut child);

    assert_eq!(
        status.code(),
        Some(0),
        "expected a clean exit after serving the registry client, got {status}"
    );

    client
        .join()
        .expect("the protocol-surface client thread panicked");

    let globals = &outcome.globals;

    for interface in EXPECTED_GLOBALS {
        assert!(
            version_of(globals, interface).is_some(),
            "the compositor did not advertise `{interface}`; it advertised {globals:?}"
        );
    }

    // M4: fractional scaling.
    assert_eq!(
        version_of(globals, "wl_compositor"),
        Some(6),
        "`preferred_buffer_scale` needs `wl_compositor` v6"
    );
    assert_eq!(
        version_of(globals, "wp_fractional_scale_manager_v1"),
        Some(1),
        "the exact scale of a non-integer output travels on this manager"
    );

    // M3: what XWayland interop stands on.
    assert_eq!(
        version_of(globals, "wp_viewporter"),
        Some(1),
        "xwayland-satellite needs `wp_viewporter` v1 to scale its windows"
    );
    assert_eq!(
        version_of(globals, "wl_data_device_manager"),
        Some(3),
        "the clipboard and drag-and-drop channel must be v3"
    );
    assert_eq!(
        version_of(globals, "zwp_primary_selection_device_manager_v1"),
        Some(1),
        "middle-click paste needs the primary selection manager"
    );
}

/// `--test-input` must not be accepted by a build without the `test-input` feature.
///
/// This assertion is deliberately compiled out under the feature: `cargo test
/// --all-features` builds the binary *with* it, so only a default-feature run can say
/// anything about the rejection.
#[cfg(not(feature = "test-input"))]
#[test]
fn test_input_is_rejected_without_the_feature() {
    let runtime_dir = RuntimeDir::new("test-input-rejected");
    let socket = runtime_dir.member("test-input.sock");

    let mut child = Command::new(COMPOSITOR)
        .args(["--test-input"])
        .arg(&socket)
        .env("XDG_RUNTIME_DIR", runtime_dir.0.as_os_str())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn inox-compositor");

    let status = wait_with_timeout(&mut child);
    let stderr = child
        .stderr
        .take()
        .map(|mut pipe| {
            let mut text = String::new();
            let _ = std::io::Read::read_to_string(&mut pipe, &mut text);

            text
        })
        .unwrap_or_default();

    assert!(
        !status.success(),
        "a build without the `test-input` feature accepted `--test-input`"
    );
    assert!(
        stderr.contains("test-input"),
        "the rejection does not say what was rejected: {stderr}"
    );
    assert!(
        !socket.exists(),
        "`--test-input` created {} in a build without the feature",
        socket.display()
    );
}
