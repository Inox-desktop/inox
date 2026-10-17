//! Real Inox compositor and desktop child processes, enabled in the dedicated CI job.
use std::{
    process::Command,
    time::{Duration, Instant},
};
#[test]
fn actual_headless_session_reaches_compositor_ready_and_logs_out() {
    let Some(binaries) = std::env::var_os("INOX_SESSION_E2E_BIN_DIR") else {
        return;
    };
    let root = tempfile::tempdir().expect("runtime");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private runtime permissions");
    let log_path = root.path().join("session.log");
    let log = std::fs::File::create(&log_path).expect("log");
    let mut child = Command::new(env!("CARGO_BIN_EXE_inox-session"))
        .args(["--headless", "--exit-after-seconds", "4", "--bin-dir"])
        .arg(binaries)
        .env("XDG_RUNTIME_DIR", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("RUST_LOG", "info")
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .stdout(log.try_clone().expect("log clone"))
        .stderr(log)
        .spawn()
        .expect("session");
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().expect("session status") {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("stop timed-out session");
            child.wait().expect("reap timed-out session");
            panic!("session failed to log out within its shutdown budget");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let log = std::fs::read_to_string(&log_path).expect("session log");
    assert!(status.success(), "{log}");
    assert!(log.contains("compositor protocol ready"), "{log}");
    assert!(
        log.contains("component=panel") || log.contains("component=\"panel\""),
        "{log}"
    );
    assert!(
        !root.path().join(format!("inox-session-{pid}")).exists(),
        "private runtime leaked: {log}"
    );
}
