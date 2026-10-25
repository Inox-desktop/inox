//! The whole session takes the secure session lock, and still logs out.
//!
//! This is the E3 leg of acceptance S02: a real `inox-session`, on a real
//! `inox-compositor`, starting the real `inox-lock --take-lock`, with the token the
//! session generated and delivered. What it proves
//! is the chain, not any one component: the session hands out a capability, the
//! lock client claims the trusted role with it over `inox-ipc`, the compositor
//! covers every output and confirms, and a logout still completes inside the
//! budget the L4 session services fixes.
//!
//! It is enabled by `INOX_SESSION_E2E_BIN_DIR`, like `tests/headless.rs`, and runs
//! in the same CI job: a component that is not built next to the others cannot be
//! exercised this way.

use std::{
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use rustix::process::{Pid, Signal, kill_process};

/// What the session's log and exit status say happened.
fn run_locked_session(binaries: &Path, root: &Path) -> (std::process::ExitStatus, String) {
    let log_path = root.join("session.log");
    let log = std::fs::File::create(&log_path).expect("the session log");
    let mut child = Command::new(env!("CARGO_BIN_EXE_inox-session"))
        .args(["--headless", "--exit-after-seconds", "20", "--bin-dir"])
        .arg(binaries)
        // The one switch that makes the session start the trusted lock client:
        // `--take-lock` has no way to release the lock, so a session never starts
        // it unless it was asked to.
        .env("INOX_SESSION_TAKE_LOCK", "1")
        .env("XDG_RUNTIME_DIR", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        .env("LIBGL_ALWAYS_SOFTWARE", "1")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .stdout(log.try_clone().expect("log clone"))
        .stderr(log)
        .stdin(Stdio::null())
        .spawn()
        .expect("the session starts");
    let pid = child.id();

    let startup_deadline = Instant::now() + Duration::from_secs(20);
    let mut stopping = None;
    let status = loop {
        if let Some(status) = child.try_wait().expect("the session status") {
            break status;
        }
        let log = std::fs::read_to_string(&log_path).expect("the live session log");
        // The lock client says this once the compositor has confirmed the lock,
        // which is only after every output has presented a covered frame.
        if stopping.is_none()
            && log.contains("compositor protocol ready")
            && log.contains("The session is locked")
        {
            kill_process(
                Pid::from_raw(pid as i32).expect("the session pid"),
                Signal::TERM,
            )
            .expect("logout is requested after the session really locked");
            stopping = Some(Instant::now());
        }
        let expired = stopping.map_or_else(
            || Instant::now() >= startup_deadline,
            |stopped| Instant::now().duration_since(stopped) >= Duration::from_secs(10),
        );
        if expired {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            child.kill().expect("stop the timed-out session");
            child.wait().expect("reap the timed-out session");
            panic!("the locked session exceeded its startup or logout deadline: {log}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let log = std::fs::read_to_string(&log_path).expect("the session log");
    assert!(
        stopping.is_some(),
        "the session exited before it ever locked: {log}"
    );
    // The session removes the files it owns — its Wayland socket and that socket's
    // lock — and then the directory. Another library in the session's own processes
    // may leave its private cache there (iced's GTK stack writes `dconf`), which is
    // not a resource leak of this supervisor's; what must never survive is a socket
    // or lock that would keep a stale display name alive.
    let private = root.join(format!("inox-session-{pid}"));
    assert!(
        !private.join("wayland-inox").exists(),
        "the session's Wayland socket leaked: {log}"
    );
    assert!(
        !private.join("wayland-inox.lock").exists(),
        "the session's socket lock leaked: {log}"
    );

    (status, log)
}

#[test]
fn a_real_session_locks_every_output_and_still_logs_out() {
    let Some(binaries) = std::env::var_os("INOX_SESSION_E2E_BIN_DIR") else {
        return;
    };
    let root = tempfile::tempdir().expect("a private runtime directory");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private runtime permissions");
    }

    let (status, log) = run_locked_session(Path::new(&binaries), root.path());

    assert!(
        status.success(),
        "a locked session must still log out cleanly: {log}"
    );
    assert!(log.contains("compositor protocol ready"), "{log}");
    assert!(
        log.contains("started session process component=lock"),
        "the session did not start the trusted lock client: {log}"
    );
    assert!(
        log.contains("The session is locked"),
        "the lock client never reported a confirmed lock: {log}"
    );
    assert!(
        !log.contains("refused the session lock"),
        "the compositor refused the session the session itself authorized: {log}"
    );
    assert!(
        log.contains("stays locked until you log out"),
        "the lock client exited without saying the session is still locked: {log}"
    );
}
