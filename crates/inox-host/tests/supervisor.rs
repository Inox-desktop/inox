//! The greeter stage, driven as a real process tree.
//!
//! ADR 0037 §验收 asks for the four refusal paths to be shown with fake children —
//! "集成测试用假合成器/假界面脚本（不 mock 宿主自己的规则）" — so this file runs the real
//! `inox-host` binary against shell-script fixtures that behave the way the two real
//! programs can behave: one reports that it is up (and the harness binds the Wayland
//! socket then, because a shell cannot), one never binds it, one exits immediately, one
//! crashes on every start, one stays up long enough to be signalled. Nothing about the host's rules is stubbed: the same binary, the same
//! event loop, the same state machine.
//!
//! What each scenario asserts is the part a unit test cannot see: that the compositor
//! is started with the arguments it was asked for, that the login screen really receives
//! `WAYLAND_DISPLAY` and `INOX_AUTH_SOCKET` with its standard input closed, that a
//! stop request signals the login screen *before* the compositor, that nothing is left
//! running afterwards, and that each refusal ends with the documented exit code.
//!
//! The fixtures record what happened in files rather than on stdout, because the host's
//! children inherit its stdout: waiting for a pipe to close would be waiting for every
//! descendant, which is exactly what these tests are checking has stopped.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process, test_kill_process};

/// How long a well-behaved host gets before a test calls the run a hang.
const LIMIT: Duration = Duration::from_secs(15);

/// The socket wait is ten seconds (ADR 0037 §决策 3.2), so that scenario needs longer.
const SOCKET_TIMEOUT_LIMIT: Duration = Duration::from_secs(25);

/// A greeter stage in a temporary directory: a runtime directory, a binary directory
/// holding the fake children, and a state directory for their markers.
struct Stage {
    root: tempfile::TempDir,
    /// Keeps the fake `auth-v1` boundary listening for the whole test. Its existence is
    /// the only thing the host checks, so it does not have to accept.
    _boundary: UnixListener,
}

impl Stage {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("a temporary stage");
        for directory in ["runtime", "bin", "state"] {
            std::fs::create_dir(root.path().join(directory))
                .expect("the stage directories are creatable");
        }
        let boundary =
            UnixListener::bind(root.path().join("auth.sock")).expect("a unix socket is bindable");

        Self {
            root,
            _boundary: boundary,
        }
    }

    fn runtime_dir(&self) -> PathBuf {
        self.root.path().join("runtime")
    }

    fn bin_dir(&self) -> PathBuf {
        self.root.path().join("bin")
    }

    fn state_dir(&self) -> PathBuf {
        self.root.path().join("state")
    }

    fn auth_socket(&self) -> PathBuf {
        self.root.path().join("auth.sock")
    }

    /// Where the fake compositor is expected to create its Wayland socket.
    fn wayland_socket(&self) -> PathBuf {
        self.runtime_dir().join("wayland-inox-greet")
    }

    /// Writes an executable fixture into the binary directory.
    fn script(&self, program: &str, body: &str) {
        let path = self.bin_dir().join(program);
        std::fs::write(&path, body).expect("the fixture is writable");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("the fixture is executable");
    }

    /// The host command, pointed at this stage.
    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_inox-host"));
        command
            .arg("--bin-dir")
            .arg(self.bin_dir())
            .arg("--auth-socket")
            .arg(self.auth_socket())
            .args(arguments)
            .env("XDG_RUNTIME_DIR", self.runtime_dir())
            .env("LC_ALL", "en-US.UTF-8")
            .env_remove("LANG")
            .env_remove("LC_MESSAGES")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("INOX_AUTH_SOCKET");
        command
    }

    /// Starts the host and returns a handle that can be signalled and waited on.
    fn spawn(&self, arguments: &[&str], log_name: &str) -> HostRun {
        let log = self.state_dir().join(log_name);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .expect("the log file is creatable");
        let mut command = self.command(arguments);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                file.try_clone().expect("the log file is clonable"),
            ))
            .stderr(Stdio::from(file));

        let child = command.spawn().expect("the host starts");
        HostRun { child, log }
    }

    /// Runs the host to completion.
    fn run(&self, arguments: &[&str], limit: Duration) -> Outcome {
        self.spawn(arguments, "host.log").wait(limit)
    }

    /// Binds the Wayland socket the fake compositor would have bound.
    ///
    /// It has to be a real socket: the host waits for the path to *be a socket*, which
    /// is what stops a leftover regular file from declaring the stage ready. A POSIX
    /// shell cannot bind a unix socket without extra tooling, so the harness does it —
    /// once the fixture has reported that the compositor is up, which keeps the ordering
    /// identical to a real compositor's.
    fn bind_wayland_socket(&self) -> UnixListener {
        UnixListener::bind(self.wayland_socket()).expect("the Wayland socket is bindable")
    }

    /// Waits for a marker file the fixtures write.
    fn wait_for(&self, name: &str, limit: Duration) -> bool {
        let path = self.state_dir().join(name);
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// Waits for a fixture to append `line` to a marker file.
    ///
    /// Waiting for the recorded line rather than for the file that precedes it is what
    /// keeps the assertions race-free: the fixtures write their markers in order.
    fn wait_for_line(&self, name: &str, line: &str, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if self.lines(name).iter().any(|recorded| recorded == line) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// The contents of a marker file, trimmed.
    fn marker(&self, name: &str) -> String {
        std::fs::read_to_string(self.state_dir().join(name))
            .unwrap_or_default()
            .trim()
            .to_owned()
    }

    /// The lines of a marker file that the fixtures append to.
    fn lines(&self, name: &str) -> Vec<String> {
        self.marker(name)
            .lines()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect()
    }
}

/// A running host process.
struct HostRun {
    child: Child,
    log: PathBuf,
}

impl HostRun {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// `SIGTERM`, the way systemd stops the unit.
    fn terminate(&self) {
        self.signal(Signal::TERM);
    }

    /// `SIGINT`, the way an operator's terminal stops it.
    fn interrupt(&self) {
        self.signal(Signal::INT);
    }

    fn signal(&self, signal: Signal) {
        let pid = Pid::from_raw(i32::try_from(self.pid()).expect("a pid fits in an i32"))
            .expect("the host has a pid");
        kill_process(pid, signal).expect("the host can be signalled");
    }

    /// Waits for the host to exit, and reports what it said.
    fn wait(mut self, limit: Duration) -> Outcome {
        let deadline = Instant::now() + limit;
        loop {
            match self
                .child
                .try_wait()
                .expect("the host's status is readable")
            {
                Some(status) => {
                    return Outcome {
                        status,
                        log: self.log_text(),
                        timed_out: false,
                    };
                }
                None if Instant::now() >= deadline => {
                    // A last resort so that a host which never exits cannot hang CI;
                    // every assertion below still reports the timeout.
                    let _ = self.child.kill();
                    let status = self.child.wait().expect("the host is reaped");
                    return Outcome {
                        status,
                        log: self.log_text(),
                        timed_out: true,
                    };
                }
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    }

    fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

/// What a finished host run left behind.
struct Outcome {
    status: ExitStatus,
    log: String,
    timed_out: bool,
}

impl Outcome {
    /// Both streams, as a person reading the journal would see them.
    fn said(&self) -> &str {
        &self.log
    }

    fn code(&self) -> Option<i32> {
        self.status.code()
    }

    fn assert_exit_code(&self, expected: i32) {
        assert!(!self.timed_out, "the host did not exit: {}", self.said());
        assert_eq!(
            self.code(),
            Some(expected),
            "expected exit code {expected}, said:\n{}",
            self.said()
        );
    }
}

/// A pid read from a fixture's record.
fn recorded_pid(path: &Path) -> Pid {
    let text = std::fs::read_to_string(path).expect("the fixture recorded its pid");
    let raw: i32 = text.trim().parse().expect("the pid file holds a number");
    Pid::from_raw(raw).expect("a recorded pid is positive")
}

/// Fails unless `pid` has left the process table.
fn assert_gone(pid: Pid, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match test_kill_process(pid) {
            Err(rustix::io::Errno::SRCH) => return,
            _ if Instant::now() >= deadline => {
                panic!("{what} (pid {}) is still running", pid.as_raw_nonzero());
            }
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// A fake compositor that becomes ready: it records its arguments, reports that it is
/// up, and waits to be stopped.
///
/// The Wayland socket is bound by [`Stage::bind_wayland_socket`] as soon as this report
/// appears; nothing else about the ordering changes.
fn ready_compositor(stage: &Stage) {
    let state = stage.state_dir().display().to_string();
    stage.script(
        "inox-compositor",
        &format!(
            r#"#!/bin/sh
trap 'echo "compositor-term" >> "{state}/events"; exit 0' TERM
echo "$@" > "{state}/compositor.args"
echo "$$" > "{state}/compositor.pid"
echo "compositor-started" >> "{state}/events"
while :; do sleep 0.05; done
"#
        ),
    );
}

/// A fake compositor that never binds its Wayland socket.
fn silent_compositor(stage: &Stage) {
    let state = stage.state_dir().display().to_string();
    stage.script(
        "inox-compositor",
        &format!(
            r#"#!/bin/sh
trap 'echo "compositor-term" >> "{state}/events"; exit 0' TERM
echo "$$" > "{state}/compositor.pid"
while :; do sleep 0.05; done
"#
        ),
    );
}

/// A fake login screen that records what the host handed it and then waits.
fn staying_greeter(stage: &Stage) {
    let state = stage.state_dir().display().to_string();
    stage.script(
        "inox-greet",
        &format!(
            r#"#!/bin/sh
trap 'echo "greeter-term" >> "{state}/events"; exit 0' TERM
echo "$WAYLAND_DISPLAY" > "{state}/greeter.wayland"
echo "$INOX_AUTH_SOCKET" > "{state}/greeter.auth"
if read -r line; then echo "read: $line" > "{state}/greeter.stdin"; else echo "eof" > "{state}/greeter.stdin"; fi
echo "$$" > "{state}/greeter.pid"
echo "greeter-started" >> "{state}/events"
echo "run" >> "{state}/greeter.runs"
while :; do sleep 0.05; done
"#
        ),
    );
}

#[test]
fn the_login_screen_gets_its_display_its_boundary_and_no_standard_input() {
    let stage = Stage::new();
    ready_compositor(&stage);
    staying_greeter(&stage);

    // `--compositor-arg` has to reach the compositor's command line verbatim.
    let run = stage.spawn(&["--compositor-arg", "--headless"], "host.log");
    assert!(
        stage.wait_for_line("events", "compositor-started", LIMIT),
        "the host started no compositor; said:\n{}",
        run.log_text()
    );
    // Held for the rest of the test: the socket has to stay a socket for the login
    // screen to be started at all.
    let _wayland = stage.bind_wayland_socket();
    assert!(
        stage.wait_for_line("events", "greeter-started", LIMIT),
        "the host started no login screen; said:\n{}",
        run.log_text()
    );

    let arguments = stage.marker("compositor.args");
    assert!(
        arguments.contains("--socket wayland-inox-greet"),
        "the compositor has to be told which socket to create: {arguments}"
    );
    assert!(
        arguments.contains("--headless"),
        "extra compositor arguments have to be passed through: {arguments}"
    );
    assert_eq!(
        stage.marker("greeter.wayland"),
        "wayland-inox-greet",
        "the login screen has to be pointed at this stage's display"
    );
    assert_eq!(
        stage.marker("greeter.auth"),
        stage.auth_socket().display().to_string(),
        "the login screen has to be pointed at the boundary this host required"
    );
    assert_eq!(
        stage.marker("greeter.stdin"),
        "eof",
        "the login screen must not read the greeter console"
    );

    run.terminate();
    let outcome = run.wait(LIMIT);
    outcome.assert_exit_code(0);

    // The stop is ordered: the login screen is asked to leave first, and the compositor
    // only once it has.
    assert_eq!(
        stage.lines("events"),
        [
            "compositor-started",
            "greeter-started",
            "greeter-term",
            "compositor-term"
        ],
        "the children have to be stopped in reverse order; said:\n{}",
        outcome.said()
    );
    assert_gone(
        recorded_pid(&stage.state_dir().join("greeter.pid")),
        "the login screen",
    );
    assert_gone(
        recorded_pid(&stage.state_dir().join("compositor.pid")),
        "the compositor",
    );
}

#[test]
fn a_compositor_that_exits_ends_the_stage_without_a_login_screen() {
    let stage = Stage::new();
    let state = stage.state_dir().display().to_string();
    // The compositor leaves without ever creating its socket.
    stage.script(
        "inox-compositor",
        &format!(
            r#"#!/bin/sh
echo "compositor-started" >> "{state}/events"
exit 1
"#
        ),
    );
    staying_greeter(&stage);

    let outcome = stage.run(&[], LIMIT);

    outcome.assert_exit_code(4);
    assert!(
        outcome.said().contains("compositor"),
        "the journal has to say the compositor left: {}",
        outcome.said()
    );
    assert!(
        !stage.state_dir().join("greeter.pid").exists(),
        "no login screen can be started without a display"
    );
    assert!(
        !stage.wayland_socket().exists(),
        "the fixture compositor never created its socket"
    );
}

#[test]
fn a_compositor_that_never_creates_its_socket_is_stopped_and_the_stage_fails() {
    let stage = Stage::new();
    silent_compositor(&stage);
    staying_greeter(&stage);

    let outcome = stage.run(&[], SOCKET_TIMEOUT_LIMIT);

    // ADR 0037 §决策 3.2: ten seconds, then the compositor is stopped and the host exits
    // non-zero. The exit code is the supervisor's, so a stale socket on the next start
    // is distinguishable from this failure in `systemctl status`.
    outcome.assert_exit_code(5);
    assert!(
        !stage.state_dir().join("greeter.pid").exists(),
        "the login screen must not be started without a Wayland socket"
    );
    assert_eq!(
        stage.lines("events"),
        ["compositor-term"],
        "the compositor has to be stopped before the host exits; said:\n{}",
        outcome.said()
    );
    assert_gone(
        recorded_pid(&stage.state_dir().join("compositor.pid")),
        "the compositor",
    );
}

#[test]
fn a_leftover_file_where_the_socket_belongs_does_not_make_the_stage_ready() {
    let stage = Stage::new();
    silent_compositor(&stage);
    staying_greeter(&stage);
    // Somebody else's file with the socket's name: it exists, and it is not a socket.
    std::fs::write(stage.wayland_socket(), b"").expect("the decoy is writable");

    let outcome = stage.run(&[], SOCKET_TIMEOUT_LIMIT);

    // The same refusal as a compositor that bound nothing, because that is what
    // happened: no login screen, the deadline, and a stopped compositor.
    outcome.assert_exit_code(5);
    assert!(
        !stage.state_dir().join("greeter.pid").exists(),
        "a path that is not a socket is not a ready compositor; said:\n{}",
        outcome.said()
    );
    assert_eq!(
        stage.lines("events"),
        ["compositor-term"],
        "the compositor has to be stopped before the host exits; said:\n{}",
        outcome.said()
    );
    assert_gone(
        recorded_pid(&stage.state_dir().join("compositor.pid")),
        "the compositor",
    );
}

#[test]
fn a_login_screen_that_keeps_crashing_spends_the_budget_and_ends_the_stage() {
    let stage = Stage::new();
    ready_compositor(&stage);
    let state = stage.state_dir().display().to_string();
    // Every start records itself and fails immediately: the budget is what has to stop
    // this, not the fixture.
    stage.script(
        "inox-greet",
        &format!(
            r#"#!/bin/sh
echo "run" >> "{state}/greeter.runs"
exit 1
"#
        ),
    );

    let run = stage.spawn(&[], "host.log");
    assert!(
        stage.wait_for_line("events", "compositor-started", LIMIT),
        "the host started no compositor; said:\n{}",
        run.log_text()
    );
    // The socket stays bound for the whole run, so every restart of the crashing login
    // screen happens in a stage that looks ready — which is what the budget is for.
    let _wayland = stage.bind_wayland_socket();

    let outcome = run.wait(LIMIT);
    outcome.assert_exit_code(7);
    assert_eq!(
        stage.lines("greeter.runs").len(),
        4,
        "one start plus three restarts before the budget is spent; said:\n{}",
        outcome.said()
    );
    assert!(
        stage
            .lines("events")
            .contains(&"compositor-term".to_owned()),
        "the compositor has to be stopped when the budget is spent; said:\n{}",
        outcome.said()
    );
    assert_gone(
        recorded_pid(&stage.state_dir().join("compositor.pid")),
        "the compositor",
    );
}

#[test]
fn a_login_screen_that_exits_cleanly_is_restarted_too() {
    let stage = Stage::new();
    ready_compositor(&stage);
    let state = stage.state_dir().display().to_string();
    // The first start exits successfully and the second one stays: ADR 0037 §决策 4 makes
    // the login screen resident after a session starts, so a successful exit is still an
    // exit and still has to come back.
    stage.script(
        "inox-greet",
        &format!(
            r#"#!/bin/sh
trap 'echo "greeter-term" >> "{state}/events"; exit 0' TERM
echo "run" >> "{state}/greeter.runs"
if [ ! -f "{state}/greeter.first" ]; then
    : > "{state}/greeter.first"
    exit 0
fi
echo "$$" > "{state}/greeter.pid"
while :; do sleep 0.05; done
"#
        ),
    );

    let run = stage.spawn(&[], "host.log");
    assert!(
        stage.wait_for_line("events", "compositor-started", LIMIT),
        "the host started no compositor; said:\n{}",
        run.log_text()
    );
    let _wayland = stage.bind_wayland_socket();
    assert!(
        stage.wait_for("greeter.pid", LIMIT),
        "the login screen was not restarted; said:\n{}",
        run.log_text()
    );
    assert_eq!(
        stage.lines("greeter.runs").len(),
        2,
        "a successful exit has to be restarted like a crash"
    );

    run.terminate();
    let outcome = run.wait(LIMIT);
    outcome.assert_exit_code(0);
    assert_gone(
        recorded_pid(&stage.state_dir().join("compositor.pid")),
        "the compositor",
    );
}

#[test]
fn a_stop_request_before_the_socket_exists_stops_the_compositor_cleanly() {
    let stage = Stage::new();
    silent_compositor(&stage);

    let run = stage.spawn(&[], "host.log");
    assert!(
        stage.wait_for("compositor.pid", LIMIT),
        "the host started no compositor; said:\n{}",
        run.log_text()
    );

    run.terminate();
    let outcome = run.wait(LIMIT);

    // A stop that was asked for is a successful stop even though the stage had not
    // finished starting: the unit is not in a failed state.
    outcome.assert_exit_code(0);
    assert_eq!(stage.lines("events"), ["compositor-term"]);
    assert_gone(
        recorded_pid(&stage.state_dir().join("compositor.pid")),
        "the compositor",
    );
}

#[test]
fn an_interrupt_stops_the_stage_the_same_way_a_termination_request_does() {
    let stage = Stage::new();
    silent_compositor(&stage);

    let run = stage.spawn(&[], "host.log");
    assert!(
        stage.wait_for("compositor.pid", LIMIT),
        "the host started no compositor; said:\n{}",
        run.log_text()
    );

    run.interrupt();
    let outcome = run.wait(LIMIT);

    outcome.assert_exit_code(0);
    assert_eq!(stage.lines("events"), ["compositor-term"]);
    assert_gone(
        recorded_pid(&stage.state_dir().join("compositor.pid")),
        "the compositor",
    );
}

#[test]
fn an_executable_that_is_missing_fails_the_stage_instead_of_starting_it() {
    let stage = Stage::new();
    // No `inox-compositor` at all in the binary directory.
    let outcome = stage.run(&[], LIMIT);

    outcome.assert_exit_code(6);
    assert!(
        outcome.said().contains("inox-compositor"),
        "the journal has to name the program that could not be started: {}",
        outcome.said()
    );
}
