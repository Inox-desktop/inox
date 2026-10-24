//! Real process lifecycle with a minimal Wayland peer only at the system boundary.
use inox_session::runtime::{CommandSpec, RunOptions, ServiceSpec, SessionError, SessionSpec, run};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

// The peer answers the real client's wl_display.sync request. Socket existence
// without this reply must never start services. No desktop logic is mocked.
const PEER: &str = r#"
import os, socket, struct, signal, time
assert os.environ['WAYLAND_DISPLAY'] == 'wayland-inox'
assert 'WAYLAND_SOCKET' not in os.environ and 'DISPLAY' not in os.environ
for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
    signal.signal(sig, lambda *_: exit(0))
path = os.path.join(os.environ['XDG_RUNTIME_DIR'], 'wayland-inox')
s = socket.socket(socket.AF_UNIX)
s.bind(path)
s.listen(1)
c, _ = s.accept()
data = b''
while len(data) < 12:
    part = c.recv(12 - len(data))
    if not part: exit(1)
    data += part
obj, sizeop, callback = struct.unpack('=III', data)
assert obj == 1 and sizeop == (12 << 16)
if os.environ.get('SILENT') != '1':
    c.sendall(struct.pack('=III', callback, 12 << 16, 1))
if os.environ.get('CRASH') == '1':
    time.sleep(0.3)
    exit(7)
while True: time.sleep(1)
"#;

fn private_runtime() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().expect("runtime root");
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private runtime permissions");
    root
}

fn command(program: &str, arguments: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.into(),
        arguments: arguments.iter().map(OsString::from).collect(),
    }
}
fn service(name: &str, program: &str, args: &[&str], required: bool, oneshot: bool) -> ServiceSpec {
    ServiceSpec {
        name: name.into(),
        command: command(program, args),
        required,
        oneshot,
        environment: BTreeMap::new(),
    }
}
fn spec(root: &Path, services: Vec<ServiceSpec>) -> SessionSpec {
    SessionSpec {
        compositor: command("python3", &["-c", PEER]),
        services,
        environment: BTreeMap::new(),
        runtime_dir: root.to_owned(),
        startup_timeout: Duration::from_secs(2),
    }
}
fn bounded(seconds: u64) -> RunOptions {
    RunOptions {
        exit_after: Some(Duration::from_secs(seconds)),
        ..Default::default()
    }
}
fn no_runtime_left(root: &Path) {
    assert!(
        !root
            .join(format!("inox-session-{}", std::process::id()))
            .exists()
    );
}

#[test]
fn protocol_readiness_crash_isolation_budgets_and_cleanup() {
    let root = private_runtime();
    let marker = root.path().join("starts");
    let script = format!("printf x >> '{}'; exit 1", marker.display());
    let mut session = spec(
        root.path(),
        vec![service(
            "optional",
            "/bin/sh",
            &["-c", &script],
            false,
            false,
        )],
    );
    // Config cannot redirect children to a stale parent display or inherited FD.
    session
        .environment
        .insert("WAYLAND_DISPLAY".into(), "wrong".into());
    session
        .environment
        .insert("WAYLAND_SOCKET".into(), "9999".into());
    run(session, bounded(3)).expect("optional crashes must not terminate the compositor");
    assert_eq!(std::fs::read(&marker).expect("four total starts"), b"xxxx");
    no_runtime_left(root.path());

    std::fs::write(&marker, b"").expect("reset counter");
    let session = spec(
        root.path(),
        vec![service(
            "required",
            "/bin/sh",
            &["-c", &script],
            true,
            false,
        )],
    );
    let start = Instant::now();
    assert!(
        matches!(run(session, bounded(8)), Err(SessionError::Failed(reason)) if reason.contains("restart budget"))
    );
    assert!(start.elapsed() < Duration::from_secs(8));
    assert_eq!(std::fs::read(&marker).expect("bounded restarts"), b"xxxx");
    no_runtime_left(root.path());

    let silent_marker = root.path().join("must-not-start");
    let silent_script = format!("touch '{}'", silent_marker.display());
    let mut session = spec(
        root.path(),
        vec![service(
            "must-not-start",
            "/bin/sh",
            &["-c", &silent_script],
            true,
            true,
        )],
    );
    session.environment.insert("SILENT".into(), "1".into());
    assert!(
        matches!(run(session, bounded(4)), Err(SessionError::Failed(reason)) if reason.contains("readiness timed out"))
    );
    assert!(
        !silent_marker.exists(),
        "socket existence must not start desktop services"
    );
    no_runtime_left(root.path());

    let mut session = spec(root.path(), vec![]);
    session.environment.insert("CRASH".into(), "1".into());
    assert!(
        matches!(run(session, bounded(4)), Err(SessionError::Failed(reason)) if reason.contains("compositor exited"))
    );
    no_runtime_left(root.path());
}

#[test]
fn failed_spawn_and_invalid_runtime_leave_no_children_or_directory() {
    let root = private_runtime();
    let mut session = spec(root.path(), vec![]);
    session.compositor = CommandSpec::new("/nonexistent/inox-compositor");
    assert!(matches!(run(session, bounded(1)), Err(SessionError::Io(_))));
    no_runtime_left(root.path());
    let mut session = spec(root.path(), vec![]);
    session.startup_timeout = Duration::ZERO;
    assert!(matches!(
        run(session, bounded(1)),
        Err(SessionError::Invalid(_))
    ));
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755))
        .expect("public runtime");
    assert!(matches!(
        run(spec(root.path(), vec![]), bounded(1)),
        Err(SessionError::Invalid(_))
    ));
}

#[test]
fn successful_bootstrap_runs_once_and_signal_masks_do_not_leak_to_children() {
    let root = private_runtime();
    let marker = root.path().join("bootstrap");
    let script = format!("printf x >> '{}'", marker.display());
    let session = spec(
        root.path(),
        vec![
            service("bootstrap", "/bin/sh", &["-c", &script], true, true),
            service("sleeper", "/bin/sleep", &["60"], true, false),
        ],
    );
    let start = Instant::now();
    // Exercise the real --exec-child path used by the binary. Registering the
    // signal source in a test worker would mask process-wide test signals.
    let options = RunOptions {
        child_launcher: Some(env!("CARGO_BIN_EXE_inox-session").into()),
        ..bounded(1)
    };
    run(session, options).expect("clean logout");
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(std::fs::read(marker).expect("bootstrap"), b"x");
    no_runtime_left(root.path());
}

#[test]
fn actual_supervisor_signal_shutdown_is_graceful_and_cleans_its_runtime() {
    use rustix::process::{Pid, Signal, kill_process};
    use std::os::unix::fs::PermissionsExt;
    let root = private_runtime();
    let binaries = tempfile::tempdir().expect("fake system boundary binaries");
    let compositor = binaries.path().join("inox-compositor");
    std::fs::write(&compositor, format!("#!/usr/bin/env python3\n{PEER}"))
        .expect("compositor boundary");
    std::fs::set_permissions(&compositor, std::fs::Permissions::from_mode(0o700))
        .expect("executable");
    let ready = root.path().join("services-started");
    // The components that reach the marker: every non-oneshot service in
    // `src/main.rs`. The desktop home is *two* panel instances (one
    // `wlr-layer-shell` surface per client connection), so the single
    // `inox-panel` binary is started twice and appends two markers. `shell` is
    // a oneshot that exits immediately and must stay out of this list.
    let starters = [
        "panel-menubar",
        "panel-dock",
        "wallpaper",
        "notifications",
        "idle",
    ];
    for component in ["shell", "panel", "wallpaper", "notifications", "idle"] {
        let path = binaries.path().join(format!("inox-{component}"));
        let script = if component == "shell" {
            "#!/bin/sh\nexit 0\n".to_owned()
        } else {
            format!(
                "#!/bin/sh\nprintf x >> '{}'\nexec sleep 60\n",
                ready.display()
            )
        };
        std::fs::write(&path, script).expect("service boundary");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).expect("executable");
    }
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_inox-session"))
        .arg("--headless")
        .arg("--bin-dir")
        .arg(binaries.path())
        .arg("--exit-after-seconds")
        .arg("6")
        .env("XDG_RUNTIME_DIR", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("supervisor");
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !std::fs::read(&ready).is_ok_and(|bytes| bytes.len() == starters.len()) {
        if Instant::now() >= deadline {
            child.kill().expect("stop failed test");
            child.wait().expect("reap failed test");
            panic!(
                "services did not start before deadline: {} of {} markers ({starters:?})",
                std::fs::read(&ready).unwrap_or_default().len(),
                starters.len()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let stopped = Instant::now();
    kill_process(Pid::from_raw(pid as i32).expect("pid"), Signal::TERM).expect("logout");
    loop {
        if let Some(status) = child.try_wait().expect("status") {
            assert!(status.success());
            break;
        }
        if stopped.elapsed() >= Duration::from_secs(3) {
            child.kill().expect("stop hung supervisor");
            child.wait().expect("reap hung supervisor");
            panic!("children inherited blocked SIGTERM or shutdown hung");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!root.path().join(format!("inox-session-{pid}")).exists());
}

#[test]
fn unresponsive_service_is_killed_within_the_logout_budget() {
    let root = private_runtime();
    let session = spec(
        root.path(),
        vec![service(
            "ignores-term",
            "/bin/sh",
            &["-c", "trap '' TERM; exec sleep 60"],
            true,
            false,
        )],
    );
    let start = Instant::now();
    run(session, bounded(1)).expect("forced cleanup still completes logout");
    assert!(start.elapsed() >= Duration::from_secs(4));
    assert!(start.elapsed() < Duration::from_secs(10));
    no_runtime_left(root.path());
}

/// A Wayland peer that also records the arguments it was started with.
///
/// The lock token is delivered to the compositor on its command line (ADR 0022
/// decision 3), so the compositor's own record of its argv is where the test reads
/// the session's token from — the session process never prints it, and must not.
const PEER_RECORDS_ARGV: &str = r#"
import os, socket, struct, signal, time, sys
with open(os.path.join(os.environ['XDG_RUNTIME_DIR'], 'compositor-args'), 'w') as record:
    record.write(' '.join(sys.argv[1:]))
for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
    signal.signal(sig, lambda *_: exit(0))
path = os.path.join(os.environ['XDG_RUNTIME_DIR'], 'wayland-inox')
s = socket.socket(socket.AF_UNIX)
s.bind(path)
s.listen(1)
c, _ = s.accept()
data = b''
while len(data) < 12:
    part = c.recv(12 - len(data))
    if not part: exit(1)
    data += part
obj, sizeop, callback = struct.unpack('=III', data)
c.sendall(struct.pack('=III', callback, 12 << 16, 1))
while True: time.sleep(1)
"#;

#[test]
fn the_session_lock_token_reaches_only_the_compositor_and_the_lock_client() {
    use std::os::unix::fs::PermissionsExt;

    let root = private_runtime();
    let binaries = tempfile::tempdir().expect("fake session binaries");
    let compositor = binaries.path().join("inox-compositor");
    std::fs::write(
        &compositor,
        format!("#!/usr/bin/env python3\n{PEER_RECORDS_ARGV}"),
    )
    .expect("the compositor boundary");
    std::fs::set_permissions(&compositor, std::fs::Permissions::from_mode(0o700))
        .expect("an executable compositor");

    // Every other component the session starts has to exist, or a required one
    // exhausting its restart budget ends the session before the lock client is
    // reached. They are inert here: this test is about one environment variable.
    for name in ["inox-wallpaper", "inox-notifications", "inox-idle"] {
        let path = binaries.path().join(name);
        std::fs::write(&path, "#!/bin/sh\nexec sleep 60\n").expect("a component boundary");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("an executable component");
    }
    let shell = binaries.path().join("inox-shell");
    std::fs::write(&shell, "#!/bin/sh\nexit 0\n").expect("the shell boundary");
    std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700))
        .expect("an executable shell");

    let lock_token_file = root.path().join("lock-token");
    let panel_token_file = root.path().join("panel-token");
    for (name, script) in [
        (
            "inox-lock",
            format!(
                "#!/bin/sh\nprintf '%s' \"${{INOX_LOCK_TOKEN:-absent}}\" > '{0}'\nexec sleep 60\n",
                lock_token_file.display()
            ),
        ),
        (
            "inox-panel",
            format!(
                "#!/bin/sh\nprintf '%s' \"${{INOX_LOCK_TOKEN:-absent}}\" > '{0}'\nexec sleep 60\n",
                panel_token_file.display()
            ),
        ),
    ] {
        let path = binaries.path().join(name);
        std::fs::write(&path, script).expect("the component boundary");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("an executable component");
    }

    let log_path = root.path().join("session.log");
    let log = std::fs::File::create(&log_path).expect("the session log");
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_inox-session"))
        .arg("--headless")
        .arg("--bin-dir")
        .arg(binaries.path())
        .arg("--exit-after-seconds")
        .arg("8")
        .env("XDG_RUNTIME_DIR", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("RUST_LOG", "warn")
        .env("INOX_SESSION_TAKE_LOCK", "1")
        // The session's own output goes to a file rather than to the test's: on a
        // failure the log is what explains it, and on success nobody wants it.
        .stdout(log.try_clone().expect("a log handle"))
        .stderr(log)
        .spawn()
        .expect("the session starts");

    let deadline = Instant::now() + Duration::from_secs(8);
    // The compositor runs inside the session's *private* runtime directory, which
    // is named after the session's process id: that is where its record of its own
    // argv lands.
    let compositor_argv = root
        .path()
        .join(format!("inox-session-{}", child.id()))
        .join("compositor-args");
    let mut argv = String::new();
    let mut lock_token = String::new();
    while lock_token.is_empty() || argv.is_empty() {
        if Instant::now() >= deadline {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            let _ = child.kill();
            let _ = child.wait();
            panic!("the session did not start the lock client and the compositor: {log}");
        }
        argv = std::fs::read_to_string(&compositor_argv).unwrap_or_default();
        lock_token = std::fs::read_to_string(&lock_token_file).unwrap_or_default();
        std::thread::sleep(Duration::from_millis(10));
    }

    let arguments: Vec<&str> = argv.split_whitespace().collect();
    let position = arguments
        .iter()
        .position(|argument| *argument == "--lock-token")
        .expect("the compositor is started with a lock token");
    let delivered = arguments[position + 1];
    assert_eq!(
        delivered.len(),
        32,
        "the token is a 128-bit value in hex: {delivered}"
    );
    assert!(
        delivered
            .chars()
            .all(|character| character.is_ascii_hexdigit()),
        "{delivered}"
    );
    assert_eq!(
        lock_token, delivered,
        "the lock client must be given the same token the compositor was started with"
    );

    // The capability goes to the lock client and nowhere else: a panel that could
    // claim the trusted lock role would be able to take the session lock.
    let panel_token = std::fs::read_to_string(&panel_token_file).unwrap_or_default();
    assert_eq!(
        panel_token, "absent",
        "no component other than the lock client may be handed the lock token"
    );

    let _ = child.wait();
}
