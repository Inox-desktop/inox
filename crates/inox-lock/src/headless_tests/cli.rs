//! The binary's default path is the one a session script meets, and it must not
//! pretend.
//!
//! `inox-lock` with no arguments does **not** start a lock screen: makes
//! the authentication boundary a human security review item, and a window that
//! looked like a lock screen while the session behind it was unlocked would be
//! worse than no lock screen at all, because a person would believe it. The
//! protocol test always passes `--take-lock`, so this module is the only place the
//! default path is exercised.
//!
//! The crate is bin-only, so there is no `CARGO_BIN_EXE_inox-lock` to run: the
//! test binary spawns itself with one ignored test selected, and that child calls
//! [`crate::run`] — the same function `main` calls, with the same arguments.

use std::process::{Command, Output};

/// Environment variable that marks the spawned child.
const CLI_CHILD_MARKER: &str = "INOX_LOCK_CLI_CHILD";

/// Environment variable carrying the child's arguments, separated by `\u{1f}`.
const CLI_CHILD_ARGS: &str = "INOX_LOCK_CLI_ARGS";

/// Spawn this test binary as a process that behaves like `main`.
fn run_as_process(arguments: &[&str]) -> Output {
    Command::new(std::env::current_exe().expect("the test binary path"))
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "--test-threads=1",
            "headless_tests::cli::cli_child",
        ])
        .env(CLI_CHILD_MARKER, "1")
        .env(CLI_CHILD_ARGS, arguments.join("\u{1f}"))
        .env("LANG", "en-US")
        .env("LC_ALL", "en-US")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("INOX_LOCK_TOKEN")
        .output()
        .expect("the child test process runs")
}

/// The child half of [`run_as_process`]: the process-level entry point.
#[test]
#[ignore = "spawned as a child process by the command-line tests"]
fn cli_child() {
    assert_eq!(
        std::env::var(CLI_CHILD_MARKER).as_deref(),
        Ok("1"),
        "this test only runs as the spawned child"
    );

    let raw = std::env::var(CLI_CHILD_ARGS).unwrap_or_default();
    let arguments: Vec<String> = if raw.is_empty() {
        Vec::new()
    } else {
        raw.split('\u{1f}').map(str::to_owned).collect()
    };

    match crate::run(arguments.into_iter()) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            // The same shape `main`'s `anyhow::Result` produces.
            eprintln!("Error: {error:#}");
            std::process::exit(1);
        }
    }
}

#[test]
fn a_lock_screen_that_cannot_authenticate_refuses_to_start() {
    let output = run_as_process(&[]);

    assert!(
        !output.status.success(),
        "a run that locks nothing must not report success"
    );

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    assert!(
        text.contains(""),
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
    let help = run_as_process(&["--help"]);

    assert!(help.status.success(), "--help must not need a display");
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("--take-lock"), "{text}");

    let bogus = run_as_process(&["--bogus"]);

    assert!(
        !bogus.status.success(),
        "an unrecognized argument is a failure, not a silent default"
    );
}
