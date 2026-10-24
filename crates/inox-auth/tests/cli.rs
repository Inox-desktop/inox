//! The helper's start-up paths, driven as a process.
//!
//! The facts checked here are the ones an operator meets before anything else
//! happens:
//!
//! - `--help` works with no display, no root and no configuration, because a person
//!   who cannot start the service is exactly the person who needs the usage line;
//! - a mistyped `--session-program` is refused at startup rather than by the first
//!   login that fails after a correct password;
//! - the system socket is refused without root, which is the difference between
//!   "the boundary is not running" and "the boundary is running and cannot possibly
//!   authenticate anyone";
//! - the internal `--session-leader` mode refuses to run without root, refuses an
//!   argument it was not given, and refuses a VT range that names no VT — the three
//!   ways "the arguments come from the root parent" could stop being true.
//!
//! The service itself — the socket, the peer check, the allowance, the session — is
//! covered by `tests/boundary.rs` against the library, so these tests never need to
//! bind `/run` or start a session.

use std::process::Command;

/// The helper, as CI builds it.
fn helper() -> Command {
    Command::new(env!("CARGO_BIN_EXE_inox-auth"))
}

/// Everything the process said, both streams.
fn said(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr)
}

/// Whether this test process is root.
fn is_root() -> bool {
    rustix::process::geteuid().as_raw() == 0
}

#[test]
fn help_needs_no_root_and_no_configuration() {
    let output = helper().arg("--help").output().expect("the helper runs");

    assert!(output.status.success(), "{}", said(&output));
    let text = said(&output);
    assert!(text.contains("inox-auth"), "{text}");
    assert!(text.contains("--socket"), "{text}");
    assert!(text.contains("--session-program"), "{text}");
}

#[test]
fn an_unknown_argument_is_refused() {
    let output = helper()
        .arg("--not-an-option")
        .output()
        .expect("the helper runs");

    assert!(!output.status.success());
    assert!(
        said(&output).contains("unrecognized argument"),
        "{}",
        said(&output)
    );
}

#[test]
fn a_session_program_that_is_not_executable_is_refused_before_serving() {
    let output = helper()
        .args(["--socket", "/tmp/inox-auth-cli-test.sock"])
        .args(["--session-program", "/nonexistent/inox-session"])
        .output()
        .expect("the helper runs");

    assert!(!output.status.success(), "{}", said(&output));
    let text = said(&output);
    assert!(
        text.contains("/nonexistent/inox-session"),
        "the refusal has to name the program that is wrong: {text}"
    );
}

#[test]
fn the_usage_line_names_the_session_placement_flags() {
    let output = helper().arg("--help").output().expect("the helper runs");

    assert!(output.status.success(), "{}", said(&output));
    let text = said(&output);
    for flag in [
        "--session-vt-min",
        "--session-vt-max",
        "--seat",
        "--session-service",
    ] {
        assert!(
            text.contains(flag),
            "an operator has to be able to find out about {flag}: {text}"
        );
    }
}

#[test]
fn a_vt_range_that_names_no_vt_is_refused_before_serving() {
    let output = helper()
        .args(["--socket", "/tmp/inox-auth-cli-range.sock"])
        .args(["--session-vt-min", "6", "--session-vt-max", "2"])
        .output()
        .expect("the helper runs");

    assert!(!output.status.success(), "{}", said(&output));
    let text = said(&output);
    assert!(
        text.contains('6') && text.contains('2'),
        "the refusal has to name the range that is wrong: {text}"
    );
}

#[test]
fn the_session_leader_arguments_are_refused_outside_the_internal_mode() {
    let output = helper()
        .args(["--user", "alice"])
        .output()
        .expect("the helper runs");

    assert!(!output.status.success(), "{}", said(&output));
    assert!(
        said(&output).contains("--session-leader"),
        "the refusal has to say which mode these arguments belong to: {}",
        said(&output)
    );
}

#[test]
fn the_session_leader_refuses_to_run_without_root() {
    if is_root() {
        eprintln!("running as root: the privilege refusal does not apply");
        return;
    }

    // Everything else on this command line is valid, so the refusal that is left
    // is the one about privilege.
    let output = helper()
        .args(["--session-leader", "--user", "nobody", "--vt", "2"])
        .args(["--session-service", "inox-auth-session"])
        .args(["--session-program", "/bin/true"])
        .output()
        .expect("the helper runs");

    assert!(!output.status.success(), "{}", said(&output));
    assert!(
        said(&output).contains("root"),
        "the refusal has to say why the leader needs privilege: {}",
        said(&output)
    );
}

#[test]
fn serving_the_system_socket_without_root_is_refused() {
    if is_root() {
        // As root the refusal does not apply, and this test would be asserting the
        // opposite of the rule. CI runs unprivileged; a developer running the suite
        // as root sees this line instead of a false failure.
        eprintln!("running as root: the privilege refusal does not apply");
        return;
    }

    let output = helper().output().expect("the helper runs");

    assert!(!output.status.success(), "{}", said(&output));
    let text = said(&output);
    assert!(
        text.contains("root"),
        "the refusal has to say why it needs privilege: {text}"
    );
}
