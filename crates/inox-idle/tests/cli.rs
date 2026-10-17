//! Capability absence must be explicit rather than leaving a silent daemon.
use std::process::Command;
#[test]
fn invalid_timeout_missing_display_and_help_have_finite_outcomes() {
    let binary = env!("CARGO_BIN_EXE_inox-idle");
    for value in ["0", "4294968", "-1", "hello"] {
        assert!(
            !Command::new(binary)
                .args(["--timeout-seconds", value])
                .output()
                .expect("invalid timeout")
                .status
                .success()
        );
    }
    assert!(
        Command::new(binary)
            .arg("--help")
            .output()
            .expect("help")
            .status
            .success()
    );
    let root = tempfile::tempdir().expect("runtime");
    assert!(
        !Command::new(binary)
            .env("XDG_RUNTIME_DIR", root.path())
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("WAYLAND_DISPLAY", "missing")
            .env_remove("WAYLAND_SOCKET")
            .output()
            .expect("missing compositor")
            .status
            .success()
    );
}
