//! The binary cannot accidentally treat preview as authenticated login.
use std::process::Command;
#[test]
fn real_login_is_refused_and_help_needs_no_display() {
    let binary = env!("CARGO_BIN_EXE_inox-greet");
    let output = Command::new(binary)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("greeter");
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(text.contains("ADR 0015"));
    assert!(
        Command::new(binary)
            .arg("--help")
            .output()
            .expect("help")
            .status
            .success()
    );
    assert!(
        !Command::new(binary)
            .arg("--bogus")
            .output()
            .expect("invalid argument")
            .status
            .success()
    );
}
