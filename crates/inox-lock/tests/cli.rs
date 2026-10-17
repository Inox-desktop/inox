//! The binary's default path is the one a session script meets, and it must not
//! pretend (ADR 0015, ADR 0018).
//!
//! `inox-lock` with no arguments does **not** start a lock screen: ADR 0015 makes
//! the authentication boundary a human security review item, and a window that
//! looked like a lock screen while the session behind it was unlocked would be
//! worse than no lock screen at all, because a person would believe it. The other
//! protocol test always passes `--take-lock`, so this file is the only place the
//! default path is exercised.

use std::process::Command;

#[test]
fn a_lock_screen_that_cannot_authenticate_refuses_to_start() {
    let binary = env!("CARGO_BIN_EXE_inox-lock");
    let output = Command::new(binary)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("INOX_LOCK_TOKEN")
        .env("LANG", "en-US")
        .env("LC_ALL", "en-US")
        .output()
        .expect("the lock binary runs");

    assert!(
        !output.status.success(),
        "a run that locks nothing must not report success"
    );

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    assert!(
        text.contains("ADR 0015"),
        "the refusal has to name what is blocking it: {text}"
    );
    assert!(
        text.contains("--take-lock"),
        "and has to say what a person exercising the protocol leg can do instead: {text}"
    );
    assert!(
        !text.contains("Enter your password"),
        "a build with no authentication boundary must not put a password prompt on \
         screen: {text}"
    );
}

#[test]
fn the_usage_line_names_the_options_and_needs_no_display() {
    let binary = env!("CARGO_BIN_EXE_inox-lock");

    let help = Command::new(binary)
        .arg("--help")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("the lock binary runs");

    assert!(help.status.success(), "--help must not need a display");
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("--take-lock"), "{text}");

    let bogus = Command::new(binary)
        .arg("--bogus")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("the lock binary runs");

    assert!(
        !bogus.status.success(),
        "an unrecognized argument is a failure, not a silent default"
    );
}
