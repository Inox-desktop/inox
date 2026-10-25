//! The binary's default path is the one a session script meets, and it must not
//! pretend.
//!
//! `inox-lock` with no arguments does **not** start a lock screen: it was started
//! without the token `inox-session` delivers, so it has no capability to lock
//! anything, and a window that looked like a lock screen while the session behind
//! it was unlocked would be worse than no lock screen at all, because a person
//! would believe it. The protocol test always passes `--take-lock`, so this module
//! is the only place the default path is exercised.
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
    run_as_process_with(arguments, &[])
}

/// The same, with environment the child needs to see.
fn run_as_process_with(arguments: &[&str], environment: &[(&str, &str)]) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("the test binary path"));

    command
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
        .env_remove(inox_protocols::control::SOCKET_VARIABLE);

    for (name, value) in environment {
        command.env(name, value);
    }

    command.output().expect("the child test process runs")
}

/// A fake session that answers one control request with `reply`.
fn fake_session(
    directory: &tempfile::TempDir,
    reply: &'static [u8],
) -> (std::path::PathBuf, std::thread::JoinHandle<Vec<u8>>) {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixListener;

    let socket = directory.path().join("control.sock");
    let listener = UnixListener::bind(&socket).expect("the fake session listens");

    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the client connects");
        let mut line = Vec::new();
        let mut byte = [0u8; 1];

        loop {
            match stream.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    line.push(byte[0]);

                    if byte[0] == b'\n' {
                        break;
                    }
                }
                Err(error) => panic!("the request line: {error}"),
            }
        }

        stream.write_all(reply).expect("the reply");

        line
    });

    (socket, handle)
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
fn a_lock_screen_started_without_a_session_refuses_to_start() {
    let output = run_as_process(&[]);

    assert!(
        !output.status.success(),
        "a run that locks nothing must not report success"
    );

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    assert!(
        text.contains("token"),
        "the refusal has to name what is blocking it: {text}"
    );
    assert!(
        text.contains("--take-lock"),
        "and has to say what a person exercising the protocol leg can do instead: {text}"
    );
    assert!(
        !text.contains("Enter your password"),
        "a run that has taken no lock must not put a password prompt anywhere: {text}"
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

#[test]
fn a_request_lock_asks_the_session_in_one_line() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let (socket, session) = fake_session(&root, b"ok\n");

    let output = run_as_process_with(
        &["--request-lock"],
        &[(
            inox_protocols::control::SOCKET_VARIABLE,
            socket.to_str().expect("a UTF-8 path"),
        )],
    );

    let request = session.join().expect("the fake session answers");
    assert_eq!(
        request, b"lock\n",
        "the request is the one verb of the protocol"
    );
    assert!(output.status.success(), "a dispatched request is a success");

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("Asked the session to lock"),
        "the operator is told the request was dispatched, not that the screen is covered: {text}"
    );
    assert!(
        !text.contains("Enter your password"),
        "asking a session to lock is not a lock screen: {text}"
    );
}

#[test]
fn a_session_without_a_lock_service_is_a_failure() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let (socket, session) = fake_session(&root, b"unknown\n");

    let output = run_as_process_with(
        &["--request-lock"],
        &[(
            inox_protocols::control::SOCKET_VARIABLE,
            socket.to_str().expect("a UTF-8 path"),
        )],
    );

    session.join().expect("the fake session answers");

    assert!(
        !output.status.success(),
        "a session with no lock service cannot lock anything, and saying so is a failure"
    );
}

#[test]
fn a_missing_session_is_reported_and_never_becomes_a_lock_screen() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let socket = root.path().join("nothing-here.sock");

    let output = run_as_process_with(
        &["--request-lock"],
        &[(
            inox_protocols::control::SOCKET_VARIABLE,
            socket.to_str().expect("a UTF-8 path"),
        )],
    );

    assert!(!output.status.success(), "there was no session to ask");

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("no session answered"),
        "the operator is told where the request went: {text}"
    );
    assert!(
        !text.contains("Enter your password"),
        "a request with nowhere to go must not draw a lock screen: {text}"
    );
}
