//! The two ways the binary can start, driven as a process.
//!
//! Since ADR 0029 the default invocation is the *real* path: it looks for the
//! authentication boundary and refuses to open a window when there is none, because
//! a login screen that collected a username and a password and only then discovered
//! there was nobody to check them would waste the one thing it must not waste. These
//! tests cover that refusal, `--help`, and an argument that is not one of the two
//! modes. `--preview` needs a display, so it is the render tests that cover what it
//! draws, not this file.

use std::process::Command;

/// The binary, as CI builds it.
fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_inox-greet"))
}

/// Everything the process said, both streams.
fn said(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr)
}

#[test]
fn the_real_path_refuses_without_an_authentication_boundary() {
    let output = binary()
        // A path that cannot exist, so the test does not depend on whether this
        // machine happens to be running a boundary.
        .env("INOX_AUTH_SOCKET", "/nonexistent/inox-auth-test.sock")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("the greeter runs");

    assert!(!output.status.success(), "{}", said(&output));

    let text = said(&output);
    assert!(
        text.contains("authentication boundary"),
        "the refusal has to name what is missing: {text}"
    );
    assert!(
        text.contains("inox-auth"),
        "and the process that provides it: {text}"
    );
    assert!(
        !text.contains("Your password"),
        "a refusal is not a login form: {text}"
    );
}

#[test]
fn help_needs_no_display_and_no_boundary() {
    let output = binary().arg("--help").output().expect("the greeter runs");

    assert!(output.status.success(), "{}", said(&output));

    let text = said(&output);
    assert!(text.contains("inox-greet"), "{text}");
    assert!(
        text.contains("--preview"),
        "the mode that opens the interface without a boundary has to be findable: {text}"
    );
}

#[test]
fn an_unknown_argument_is_refused() {
    let output = binary().arg("--bogus").output().expect("the greeter runs");

    assert!(!output.status.success());
    assert!(
        said(&output).contains("unrecognized argument"),
        "{}",
        said(&output)
    );
}
