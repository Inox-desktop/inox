//! Real Inox compositor and desktop child processes, enabled in the dedicated CI job.
use rustix::process::{Pid, Signal, kill_process};
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
        .args(["--headless", "--exit-after-seconds", "20", "--bin-dir"])
        .arg(binaries)
        .env("XDG_RUNTIME_DIR", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .stdout(log.try_clone().expect("log clone"))
        .stderr(log)
        .spawn()
        .expect("session");
    let pid = child.id();
    let startup_deadline = Instant::now() + Duration::from_secs(12);
    let mut stopping = None;
    let status = loop {
        if let Some(status) = child.try_wait().expect("session status") {
            break status;
        }
        let log = std::fs::read_to_string(&log_path).expect("live session log");
        if stopping.is_none()
            && log.contains("compositor protocol ready")
            // The desktop home is two panel instances — one layer surface per
            // process — so readiness means both surfaces are up, not just the
            // first one.
            && log.contains("started session process component=panel-menubar")
            && log.contains("started session process component=panel-dock")
        {
            kill_process(
                Pid::from_raw(pid as i32).expect("session PID"),
                Signal::TERM,
            )
            .expect("request logout after real readiness");
            stopping = Some(Instant::now());
        }
        let expired = stopping.map_or_else(
            || Instant::now() >= startup_deadline,
            |stopped| Instant::now().duration_since(stopped) >= Duration::from_secs(10),
        );
        if expired {
            child.kill().expect("stop timed-out session");
            child.wait().expect("reap timed-out session");
            panic!("session exceeded startup or logout deadline: {log}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let log = std::fs::read_to_string(&log_path).expect("session log");
    assert!(status.success(), "{log}");
    assert!(
        stopping.is_some(),
        "session exited before readiness-triggered logout: {log}"
    );
    assert!(log.contains("compositor protocol ready"), "{log}");
    for component in ["panel-menubar", "panel-dock"] {
        assert!(
            log.contains(&format!("started session process component={component}")),
            "the session must start {component}: {log}"
        );
    }
    assert!(
        !root.path().join(format!("inox-session-{pid}")).exists(),
        "private runtime leaked: {log}"
    );
}
