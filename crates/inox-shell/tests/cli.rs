//! The coordinator's command line, as a script or a pipe sees it.
//!
//! Root `AGENTS.md` §3.5 forbids `println!`, and this crate's `AGENTS.md` adds
//! that `--help`'s usage text is a `tracing::info!` line rather than a write to
//! stdout. `tracing_subscriber::fmt` writes to stdout until it is told
//! otherwise, so both rules are about *which stream* a message reaches and
//! cannot be checked from inside the process: this file runs the real binary
//! and looks at its two streams.

use std::process::Command;

/// The coordinator, built by Cargo for this test.
const SHELL: &str = env!("CARGO_BIN_EXE_inox-shell");

/// Runs the coordinator with a fixed locale and no inherited log filter, so
/// that the streams do not depend on the machine the test runs on.
fn run(arguments: &[&str]) -> std::process::Output {
    Command::new(SHELL)
        .args(arguments)
        .env("LC_ALL", "en-US")
        .env_remove("LC_MESSAGES")
        .env_remove("LANG")
        .env_remove("RUST_LOG")
        .output()
        .expect("the coordinator runs")
}

#[test]
fn the_usage_text_goes_to_stderr_and_stdout_stays_empty() {
    let output = run(&["--help"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stderr}");
    assert!(
        output.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(stderr.contains("Usage: inox-shell"), "stderr: {stderr}");
}

#[test]
fn a_rejected_argument_is_reported_on_stderr_and_fails() {
    let output = run(&["--config", "/tmp/inox"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success(), "stderr: {stderr}");
    assert!(
        output.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    // The localized line names the argument, and the usage text follows it.
    assert!(stderr.contains("--config"), "stderr: {stderr}");
    assert!(stderr.contains("Usage: inox-shell"), "stderr: {stderr}");
}
