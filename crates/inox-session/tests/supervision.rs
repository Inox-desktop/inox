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
    let root = tempfile::tempdir().expect("runtime root");
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
    let root = tempfile::tempdir().expect("runtime root");
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
    let root = tempfile::tempdir().expect("runtime root");
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
    let root = tempfile::tempdir().expect("runtime root");
    let binaries = tempfile::tempdir().expect("fake system boundary binaries");
    let compositor = binaries.path().join("inox-compositor");
    std::fs::write(&compositor, format!("#!/usr/bin/env python3\n{PEER}"))
        .expect("compositor boundary");
    std::fs::set_permissions(&compositor, std::fs::Permissions::from_mode(0o700))
        .expect("executable");
    let ready = root.path().join("panel-started");
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
    while !std::fs::read(&ready).is_ok_and(|bytes| bytes.len() == 4) {
        if Instant::now() >= deadline {
            child.kill().expect("stop failed test");
            child.wait().expect("reap failed test");
            panic!("services did not start before deadline");
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
    let root = tempfile::tempdir().expect("runtime root");
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
