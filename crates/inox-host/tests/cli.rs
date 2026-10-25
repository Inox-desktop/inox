//! The command line, driven as a process.
//!
//! Two of these tests are about a refusal that has to happen before this stage owns
//! anything at all: a host that started a compositor and only then discovered there was
//! no authentication boundary would be a login screen that lies.
//! The rest are the ordinary CLI contract — `--help`, `--version`, an argument that is
//! not one of the options, a value that is missing or empty.
//!
//! `LANG`/`LC_ALL` are pinned to `en-US` so that the message assertions do not depend
//! on the locale CI happens to run in; the translations themselves are covered by
//! `tests/catalog.rs` and `src/i18n.rs`.

use std::path::Path;
use std::process::{Command, Output};

/// The binary, as CI builds it.
fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_inox-host"));
    command
        .env("LC_ALL", "en-US.UTF-8")
        .env_remove("LANG")
        .env_remove("LC_MESSAGES")
        .env_remove("INOX_AUTH_SOCKET");
    command
}

/// Everything the process said, both streams.
///
/// `tracing` writes to stdout and the `main` error path writes to stderr; a person
/// reading the journal sees both, so the assertions do too.
fn said(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr)
}

/// A runtime directory for a run that must not need a real one.
fn runtime_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary runtime directory")
}

#[test]
fn help_needs_no_boundary_and_names_every_option() {
    let output = binary().arg("--help").output().expect("the host runs");

    assert!(output.status.success(), "{}", said(&output));

    let text = said(&output);
    for expected in [
        "inox-host",
        "--bin-dir",
        "--socket",
        "--auth-socket",
        "--compositor-arg",
    ] {
        assert!(
            text.contains(expected),
            "`--help` has to mention {expected}: {text}"
        );
    }
}

#[test]
fn the_version_is_available() {
    for flag in ["-V", "--version"] {
        let output = binary().arg(flag).output().expect("the host runs");

        assert!(output.status.success(), "{}", said(&output));
        let text = said(&output);
        assert!(text.contains("inox-host"), "{text}");
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    }
}

#[test]
fn an_unknown_argument_is_refused() {
    let output = binary().arg("--bogus").output().expect("the host runs");

    assert!(!output.status.success());
    assert!(
        said(&output).contains("unrecognized argument"),
        "{}",
        said(&output)
    );
    // A command line that cannot be used has its own code, so a typo in the unit file
    // is never mistaken for a failure of the login chain.
    assert_eq!(output.status.code(), Some(2), "{}", said(&output));
}

#[test]
fn an_option_without_its_value_is_refused() {
    for option in ["--bin-dir", "--socket", "--auth-socket", "--compositor-arg"] {
        let output = binary().arg(option).output().expect("the host runs");

        assert!(
            !output.status.success(),
            "{option} was accepted without a value"
        );
        assert!(
            said(&output).contains(option),
            "the refusal has to name {option}: {}",
            said(&output)
        );
    }
}

#[test]
fn an_empty_value_is_refused() {
    let output = binary()
        .args(["--socket", ""])
        .output()
        .expect("the host runs");

    assert!(!output.status.success());
    assert!(said(&output).contains("--socket"), "{}", said(&output));
}

#[test]
fn the_real_path_refuses_without_an_authentication_boundary() {
    let runtime = runtime_dir();
    let socket = runtime.path().join("no-boundary-here.sock");
    let output = binary()
        .args([
            "--auth-socket",
            &socket.to_string_lossy(),
            // A binary directory whose programs must never run: the refusal happens
            // before anything is started.
            "--bin-dir",
            &runtime.path().join("bin").to_string_lossy(),
        ])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .output()
        .expect("the host runs");

    assert!(!output.status.success(), "{}", said(&output));

    let text = said(&output);
    // the boot login chain
    assert!(
        text.contains("authentication boundary"),
        "the refusal has to name what is missing: {text}"
    );
    assert!(
        text.contains("inox-auth"),
        "and the process that provides it: {text}"
    );
    assert!(
        text.contains(&*socket.to_string_lossy()),
        "and the socket it tried: {text}"
    );
    // The exit code is the one the supervisor records for this refusal, so that
    // `systemctl status` can classify it without the journal.
    assert_eq!(output.status.code(), Some(3), "{}", text);
}

#[test]
fn a_refusal_starts_no_compositor() {
    let runtime = runtime_dir();
    let bin = runtime.path().join("bin");
    std::fs::create_dir(&bin).expect("the binary directory is creatable");
    let marker = runtime.path().join("compositor-started");
    write_executable(
        &bin.join("inox-compositor"),
        &format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.to_string_lossy()),
    );

    let output = binary()
        .args([
            "--bin-dir",
            &bin.to_string_lossy(),
            "--auth-socket",
            &runtime.path().join("missing.sock").to_string_lossy(),
        ])
        .env("XDG_RUNTIME_DIR", runtime.path())
        .output()
        .expect("the host runs");

    assert!(!output.status.success(), "{}", said(&output));
    assert!(
        !marker.exists(),
        "the compositor must not be started without a boundary: {}",
        said(&output)
    );
}

#[test]
fn a_missing_runtime_directory_is_a_refusal_rather_than_a_guess() {
    let output = binary()
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .expect("the host runs");

    assert!(!output.status.success());
    assert!(
        said(&output).contains("XDG_RUNTIME_DIR"),
        "{}",
        said(&output)
    );
}

/// Writes `contents` to `path` and makes it executable.
fn write_executable(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, contents).expect("the script is writable");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("the script is markable executable");
}
