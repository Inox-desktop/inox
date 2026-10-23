//! The waydriver host: a real Qt Quick client composited by the real Inox compositor.
//!
//! ADR 0036 decision 2 asks for a reusable harness that starts a real L3 Qt surface
//! on a real compositor host, screenshots it and asserts on the pixels. This module
//! is that harness; `tests/waydriver_host.rs` is its first user (acceptance M02).
//!
//! It is deliberately built on the same primitives as the M3 harness rather than on
//! a second, parallel one: [`RuntimeDir`] gives every run a private `XDG_RUNTIME_DIR`
//! (mode 0700) with private config/data/cache, [`Proc`] puts every child in its own
//! process group and reaps the group, and [`Injector`] is the compositor's test-only
//! control socket. What this module adds is the *client* side — a real Qt process on
//! `QT_QPA_PLATFORM=wayland` pointed at the host's socket — plus the bounded wait that
//! turns "a frame eventually shows the client" into an assertion.
//!
//! # Why the host is `inox-compositor` and not a third-party compositor
//!
//! `docs/quality/acceptance.md` §1 states it plainly: a third-party headless
//! compositor is only good enough for E2, and a host that does not implement a
//! protocol the component needs cannot stand in for one that does. M02 is E1/E2
//! evidence about *Inox* hosting a real Qt surface, so the host is the workspace's
//! own binary, built with the non-default `test-input` feature that provides
//! `--test-input` (`crates/inox-compositor/src/test_input.rs`).
//!
//! # Layer surfaces are **not** covered here
//!
//! The layer-surface leg of ADR 0036 decision 2 needs LayerShellQt on the Qt side
//! (`LayerShellQt::Window::get(window)`), and the CI runner cannot build it: GitHub's
//! `ubuntu-latest` is Ubuntu 24.04 (noble), whose archive has only the **Qt 5**
//! `layer-shell-qt` source package (`liblayershellqtinterface-dev` 5.27.11-0ubuntu3)
//! and no Qt 6 equivalent. This was verified against Launchpad, not assumed:
//!
//! * `getPublishedSources?source_name=layer-shell-qt` lists 5.27.11 for `noble` and
//!   6.x only for the newer series (`plucky`, `questing`, `resolute`, …);
//! * `getPublishedBinaries?binary_name=liblayershellqtinterface6-dev` returns zero
//!   published binaries for `noble/amd64`.
//!
//! So the surface this harness asserts on is a plain `xdg_toplevel` — a real Qt Quick
//! window on a real Inox compositor, which is genuine M02 evidence for "the harness
//! can drive a Qt surface and assert its pixels" — and the layer-surface leg stays a
//! documented blocker. Per ADR 0036 §"失败、恢复与安全", the tests must not dress a
//! plain window up as a layer placement that took effect: nothing here claims a layer
//! surface was created, and `tests/waydriver_host.rs` repeats the blocker where a
//! reader of the evidence will see it.

use std::{
    process::{Command, ExitStatus},
    sync::{Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use super::{
    Frame, Injector, OUTPUT_HEIGHT, OUTPUT_WIDTH, POLL_INTERVAL, Proc, ProcessSignal, Rect,
    RuntimeDir, assert_group_empty, wait_for_path, workspace_binary,
};

/// Opt-in switch for the waydriver host tests.
pub const WAYDRIVER_ENV: &str = "INOX_WAYDRIVER_E2";

/// `evdev` code for `Escape` (`KEY_ESC`).
///
/// The injection socket takes `evdev` codes, so this is 1 rather than the 9 an X11
/// client would see.
pub const KEY_ESCAPE: u32 = 1;

/// Budget for the host to come up (compositor socket and control socket).
pub const HOST_START_TIMEOUT: Duration = Duration::from_secs(30);
/// Budget for the Qt client to be mapped *and* composited into a frame.
///
/// A cold CI runner starts the Qt platform plugin, loads the QML module and renders
/// its first frame through llvmpipe; the observed figure on a warm developer machine
/// is 1-3 s, so 90 s is a generous ceiling that still fails a hung client instead of
/// hanging the job. Registered as this case's explicit budget per acceptance B02
/// (a full GUI scenario may use up to 300 s).
pub const QT_READY_TIMEOUT: Duration = Duration::from_secs(90);
/// Budget for the Qt client to end after it was asked to quit.
pub const QT_EXIT_TIMEOUT: Duration = Duration::from_secs(30);
/// Budget for the compositor to end after `SIGTERM`.
pub const HOST_EXIT_TIMEOUT: Duration = Duration::from_secs(15);

/// The environment the Qt client is started with, on top of the host's session env.
///
/// `QT_QPA_PLATFORM=wayland` is explicit rather than inherited: the crate's own
/// headless bridge test sets `offscreen` in-process, and a developer machine may
/// export either value. `QT_FORCE_STDERR_LOGGING=1` matters for the evidence: when
/// Qt's stderr is not a terminal (a redirected log file, as here) Qt 6 routes its
/// diagnostics to the journal instead, so a client that failed to initialise its
/// platform plugin would leave an empty log behind and the failure would be
/// unexplained. With the variable set, the plugin error lands in the log the test
/// prints.
const QT_CLIENT_ENV: [(&str, &str); 2] = [
    ("QT_QPA_PLATFORM", "wayland"),
    ("QT_FORCE_STDERR_LOGGING", "1"),
];

/// Whether the waydriver host tests should run in this process.
pub fn enabled() -> bool {
    std::env::var_os(WAYDRIVER_ENV).is_some()
}

/// Print the "skipping" line and report whether the caller should return early.
///
/// Usage: `if skip_unless_enabled("M02") { return; }`
pub fn skip_unless_enabled(acceptance_id: &str) -> bool {
    if enabled() {
        return false;
    }
    println!(
        "{WAYDRIVER_ENV} is not set; skipping the {acceptance_id} waydriver host test. \
         Run it with `INOX_WAYDRIVER_E2=1 cargo test -p inox-integration --all-features` \
         after `cargo build --workspace --features inox-compositor/test-input`, and with the \
         Qt 6 Wayland platform plugin installed."
    );
    true
}

/// Serialises the waydriver sessions inside one test binary.
///
/// Same reasoning as the M3 sessions: two software-rendered compositors plus two Qt
/// clients on one CI runner would compete for CPU, and `free_display_number` cannot
/// reserve a display number. Tests in different binaries still run in parallel.
static HOST_LOCK: Mutex<()> = Mutex::new(());

/// A live Inox compositor acting as the waydriver host.
pub struct Host {
    runtime: RuntimeDir,
    compositor: Proc,
    injector: Injector,
    /// Held for the whole session, so only one waydriver session runs at a time.
    _serialised: MutexGuard<'static, ()>,
}

impl Host {
    /// Start a headless Inox compositor with the test-input channel and connect to it.
    pub fn start(label: &str) -> Self {
        let serialised = HOST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let runtime = RuntimeDir::with_kind("wd", label);
        let env = runtime.env();

        let mut command = Command::new(workspace_binary("inox-compositor"));
        command
            .args([
                "--backend=headless",
                "--socket",
                runtime.wayland_socket(),
                "--test-input",
                &runtime.inject_socket_path().display().to_string(),
                "--output-size",
                &format!("{OUTPUT_WIDTH}x{OUTPUT_HEIGHT}"),
                "--refresh-rate",
                "60",
            ])
            .envs(env.iter().cloned());

        let mut compositor = Proc::spawn(
            "inox-compositor",
            command,
            &runtime.log_path("inox-compositor"),
        );

        wait_for_path(
            &mut compositor,
            &runtime.wayland_socket_path(),
            "waydriver host Wayland socket",
            HOST_START_TIMEOUT,
        );

        let injector = Injector::connect(
            &runtime.inject_socket_path(),
            &mut compositor,
            HOST_START_TIMEOUT,
        );

        Self {
            runtime,
            compositor,
            injector,
            _serialised: serialised,
        }
    }

    /// The private directory set this host owns.
    pub fn runtime(&self) -> &RuntimeDir {
        &self.runtime
    }

    /// The compositor's input injection and frame-dump client.
    pub fn injector(&mut self) -> &mut Injector {
        &mut self.injector
    }

    /// Capture the compositor's current frame.
    pub fn frame(&mut self) -> Frame {
        let path = self.runtime.scratch_path("waydriver-frame.ppm");
        self.injector.frame(&path)
    }

    /// Poll frames until `predicate` accepts one, failing on timeout or a dead child.
    ///
    /// This is the only way a wait in these tests decides the client has rendered:
    /// the compositor's own frame dump is the evidence, so "the client is ready" is
    /// never a sleep and never a log line. Every iteration also checks that both
    /// children are still alive, so a client that aborted is reported with its log
    /// instead of surfacing as an unexplained timeout.
    pub fn wait_for_frame(
        &mut self,
        client: &mut QtQuickClient,
        what: &str,
        timeout: Duration,
        predicate: impl Fn(&Frame) -> bool,
    ) -> Frame {
        let deadline = Instant::now() + timeout;
        let mut attempts = 0u32;

        loop {
            let frame = self.frame();
            attempts += 1;
            if predicate(&frame) {
                return frame;
            }

            client.assert_running(what);
            self.compositor.assert_alive(what);

            if Instant::now() >= deadline {
                panic!(
                    "{what}: the composited frame did not reach the expected state within \
                     {timeout:?} ({attempts} frame dumps).\n\
                     The last frame was {}x{} with dominant colour {:?}.\n{}\n{}",
                    frame.width,
                    frame.height,
                    frame.dominant_colour(Rect::new(0, 0, frame.width as i32, frame.height as i32)),
                    self.compositor.diagnostics(),
                    client.diagnostics()
                );
            }

            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Ask the compositor to shut down and assert it exits cleanly.
    ///
    /// `SIGTERM` is the compositor's documented clean-shutdown path (acceptance R03),
    /// and the exit status is asserted rather than assumed: a crash during teardown is
    /// a defect even when every pixel assertion before it passed.
    pub fn shutdown(mut self, what: &str) -> ExitStatus {
        let pgid = self.compositor.pid();
        self.compositor.signal(ProcessSignal::TERM);
        let status = self.compositor.wait_ok(what, HOST_EXIT_TIMEOUT);
        assert_eq!(
            status.code(),
            Some(0),
            "{what}: the waydriver host must exit 0 on SIGTERM, got {status}:\n{}",
            self.compositor.diagnostics()
        );
        assert_group_empty(pgid, what, Duration::from_secs(10));
        status
    }
}

/// A real Qt Quick client (`inox-qml-prototype`) running against a [`Host`].
pub struct QtQuickClient {
    proc: Proc,
}

impl QtQuickClient {
    /// Start the Qt client with `WAYLAND_DISPLAY` pointing at `host`.
    pub fn spawn(host: &Host, label: &str) -> Self {
        let runtime = host.runtime();
        let mut command = Command::new(workspace_binary("inox-qml-prototype"));
        command
            .envs(runtime.env().iter().cloned())
            .envs(QT_CLIENT_ENV.iter().copied())
            // The client must find the QML module and the Qt Wayland plugin; both are
            // resolved by Qt itself from the environment it inherits, so nothing else
            // is set here.
            .env("WAYLAND_DISPLAY", runtime.wayland_socket());

        let proc = Proc::spawn(
            "inox-qml-prototype",
            command,
            &runtime.log_path(&format!("inox-qml-prototype-{label}")),
        );

        Self { proc }
    }

    /// Fail with the client's own output if it is no longer running.
    pub fn assert_running(&mut self, what: &str) {
        self.proc.assert_alive(what);
    }

    /// The client's captured output, for failure messages.
    pub fn diagnostics(&self) -> String {
        self.proc.diagnostics()
    }

    /// End the client through its own event loop and assert it exits cleanly.
    ///
    /// `Escape` is injected over the compositor's test channel, so the key travels
    /// the whole way a user's key would: seat → focused surface → Qt → QML. The
    /// prototype's QML quits on it (`crates/inox-qml-prototype/qml/main.qml`), which
    /// returns from `QGuiApplication::exec` and gives a zero exit status. Signalling
    /// the client instead would prove nothing about the event path and would leave
    /// the exit status at the signal's, so this is the shutdown leg the test uses.
    pub fn quit_with_escape(&mut self, host: &mut Host, what: &str) -> ExitStatus {
        host.injector().key(KEY_ESCAPE, true);
        host.injector().key(KEY_ESCAPE, false);

        let status = self.proc.wait_timeout(QT_EXIT_TIMEOUT).unwrap_or_else(|| {
            panic!(
                "{what}: the Qt client did not exit within {QT_EXIT_TIMEOUT:?} after the \
                 injected Escape. Its Escape binding is the documented way it quits, so this \
                 means the key never reached the focused surface.\n{}",
                self.proc.diagnostics()
            )
        });

        assert_eq!(
            status.code(),
            Some(0),
            "{what}: the Qt client must exit 0 when it quits through its own event loop, \
             got {status}:\n{}",
            self.proc.diagnostics()
        );
        assert_group_empty(self.proc.pid(), what, Duration::from_secs(10));
        status
    }
}
