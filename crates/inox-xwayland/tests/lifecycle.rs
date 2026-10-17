//! Process-level lifecycle tests for the `inox-xwayland` binary.
//!
//! These tests drive the real binary with a stand-in "satellite": a tiny shell
//! script that ignores the display argument the service passes it. That keeps
//! the tests deterministic and independent of a real XWayland installation
//! while still exercising spawning, readiness probing, signal forwarding and
//! the exit-code policy end to end.
//!
//! Readiness is a real X11 connection-setup round trip (acceptance item X01),
//! so the X11 side is a stand-in server too: it either answers every setup
//! request with a valid reply, or accepts the connection and deliberately stays
//! silent so the probe has to time out.

use std::{
    fs,
    io::{self, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use rustix::process::{Pid, Signal, kill_process};
use tempfile::TempDir;

/// The binary under test, provided by Cargo.
const BIN: &str = env!("CARGO_BIN_EXE_inox-xwayland");

/// Slack added to every wait so a slow CI machine cannot turn a pass into a hang.
const SLACK: Duration = Duration::from_secs(5);

/// Poll `predicate` until it holds or `timeout` elapses.
fn wait_until<F>(timeout: Duration, mut predicate: F) -> bool
where
    F: FnMut() -> bool,
{
    let deadline = Instant::now() + timeout;

    loop {
        if predicate() {
            return true;
        }

        if Instant::now() >= deadline {
            return false;
        }

        thread::sleep(Duration::from_millis(25));
    }
}

/// Write an executable stand-in for `xwayland-satellite`.
///
/// The script records its own pid so the test can clean up a leaked child, then
/// runs `body`.
fn write_stand_in(dir: &Path, pid_file: &Path, body: &str) -> PathBuf {
    let script = dir.join("satellite.sh");
    let contents = format!("#!/bin/sh\necho $$ > '{}'\n{body}", pid_file.display());

    fs::write(&script, contents).expect("write the stand-in satellite");

    let mut permissions = fs::metadata(&script)
        .expect("stat the stand-in satellite")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("make the stand-in satellite executable");

    script
}

/// A running `inox-xwayland` process that is killed if a test fails.
struct ServiceProcess {
    child: Child,
}

impl ServiceProcess {
    /// Start the service with `args`.
    fn start(args: &[String]) -> Self {
        let child = Command::new(BIN)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start inox-xwayland");

        Self { child }
    }

    /// Pid of the service, for signal delivery.
    fn pid(&self) -> Pid {
        Pid::from_child(&self.child)
    }

    /// Wait up to `timeout` for the service to exit.
    fn wait_for_exit(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;

        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait on the service") {
                return Some(status);
            }

            if Instant::now() >= deadline {
                return None;
            }

            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for ServiceProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Kill a stand-in satellite that outlived its service.
///
/// The recorded pid is only signalled when `/proc` still reports a `sleep`
/// process for it, so a recycled pid can never be hit by accident. Where
/// `/proc` is unavailable the cleanup is skipped entirely; a leaked stand-in
/// then just exits on its own.
fn kill_stand_in(pid_file: &Path) {
    let Ok(raw) = fs::read_to_string(pid_file) else {
        return;
    };

    let Ok(raw) = raw.trim().parse::<i32>() else {
        return;
    };

    let Ok(cmdline) = fs::read(format!("/proc/{raw}/cmdline")) else {
        return;
    };

    if !cmdline.starts_with(b"sleep") {
        return;
    }

    if let Some(pid) = Pid::from_raw(raw) {
        let _ = kill_process(pid, Signal::KILL);
    }
}

/// A stand-in for the Xwayland side of an X11 socket.
///
/// The readiness probe performs a real connection-setup round trip, so a socket
/// that merely accepts connections no longer makes the service ready. The
/// stand-in either answers every setup request with a valid reply, or accepts
/// the connection and stays silent so `--ready-timeout` has to fire.
struct X11StandIn {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl X11StandIn {
    /// Bind `path` and answer every connection setup with a valid reply.
    fn answering(path: &Path) -> Self {
        Self::start(path, true)
    }

    /// Bind `path` and accept connections without ever answering them.
    fn silent(path: &Path) -> Self {
        Self::start(path, false)
    }

    fn start(path: &Path, answer: bool) -> Self {
        let listener = UnixListener::bind(path).expect("bind the stand-in X11 socket");
        listener
            .set_nonblocking(true)
            .expect("the stand-in listener must be non-blocking");

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || serve(listener, answer, &thread_stop));

        Self {
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for X11StandIn {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);

        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Accept connections and answer (or ignore) their connection requests.
fn serve(listener: UnixListener, answer: bool, stop: &AtomicBool) {
    // Connections accepted while staying silent are kept open, so the probe
    // blocks on its own read timeout instead of seeing a closed socket.
    let mut accepted = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) if answer => answer_setup(stream),
            Ok((stream, _)) => accepted.push(stream),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return,
        }
    }
}

/// Read the 12-byte connection request and answer it with a valid setup reply.
fn answer_setup(mut stream: UnixStream) {
    // Every read is bounded so a stalled client can never block the stand-in
    // thread, whose `Drop` joins it.
    if stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .is_err()
    {
        return;
    }

    let mut request = [0u8; 12];
    if stream.read_exact(&mut request).is_err() {
        return;
    }

    // The probe may have given up on this connection already; a write that
    // fails just means it sees no reply here and retries on the next poll.
    let _ = stream.write_all(&setup_reply());
}

/// A syntactically valid X11 setup reply.
///
/// It carries the fixed setup fields and a vendor string; pixmap formats and
/// screens are omitted because the probe only requires the round trip to end in
/// a well formed successful reply.
fn setup_reply() -> Vec<u8> {
    // The setup body's fixed fields end where the vendor string starts.
    const VENDOR_OFFSET: usize = 32;

    // The probe only checks the round trip, so the vendor string is the one
    // payload that has to be well formed.
    const VENDOR: &str = "The Inox test stand-in";

    let mut body = vec![0u8; VENDOR_OFFSET];
    body[0..4].copy_from_slice(&1u32.to_le_bytes()); // release number
    body[16..18].copy_from_slice(
        &u16::try_from(VENDOR.len())
            .expect("the vendor string is short")
            .to_le_bytes(),
    );
    body.extend_from_slice(VENDOR.as_bytes());

    while !body.len().is_multiple_of(4) {
        body.push(0);
    }

    let mut reply = vec![1u8, 0]; // status 1: success, no failure reason
    reply.extend_from_slice(&11u16.to_le_bytes()); // protocol major
    reply.extend_from_slice(&0u16.to_le_bytes()); // protocol minor
    reply.extend_from_slice(
        &u16::try_from(body.len() / 4)
            .expect("the setup body fits in 4-byte units")
            .to_le_bytes(),
    );
    reply.extend_from_slice(&body);
    reply
}

/// A private temp dir plus a private X11 socket directory.
struct Layout {
    _temp: TempDir,
    socket_dir: PathBuf,
    ready_file: PathBuf,
    pid_file: PathBuf,
}

impl Layout {
    fn new() -> Self {
        let temp = TempDir::new().expect("temp dir");
        let socket_dir = temp.path().join("x11-unix");

        fs::create_dir_all(&socket_dir).expect("create the socket dir");

        Self {
            socket_dir,
            ready_file: temp.path().join("ready"),
            pid_file: temp.path().join("satellite.pid"),
            _temp: temp,
        }
    }

    fn script_dir(&self) -> &Path {
        self._temp.path()
    }

    fn socket_path(&self, display: u16) -> PathBuf {
        self.socket_dir.join(format!("X{display}"))
    }

    fn base_args(&self, display: u16, satellite: &Path) -> Vec<String> {
        vec![
            "--display".to_owned(),
            format!(":{display}"),
            "--satellite".to_owned(),
            satellite.display().to_string(),
            "--x11-socket-dir".to_owned(),
            self.socket_dir.display().to_string(),
        ]
    }
}

#[test]
fn the_service_becomes_ready_and_shuts_down_cleanly_on_sigterm() {
    let layout = Layout::new();
    let display = 97;

    // The X11 side the readiness probe performs its setup round trip against.
    let _stand_in = X11StandIn::answering(&layout.socket_path(display));

    let satellite = write_stand_in(layout.script_dir(), &layout.pid_file, "exec sleep 300\n");

    let mut args = layout.base_args(display, &satellite);
    args.extend([
        "--ready-timeout".to_owned(),
        "10".to_owned(),
        "--shutdown-grace".to_owned(),
        "5".to_owned(),
        "--ready-file".to_owned(),
        layout.ready_file.display().to_string(),
    ]);

    let mut service = ServiceProcess::start(&args);

    assert!(
        wait_until(Duration::from_secs(10), || layout.ready_file.exists()),
        "the service never wrote its readiness file"
    );

    let contents = fs::read_to_string(&layout.ready_file).expect("read the readiness file");
    assert_eq!(contents.trim(), format!(":{display}"));

    kill_process(service.pid(), Signal::TERM).expect("signal the service");

    let status = service
        .wait_for_exit(SLACK + Duration::from_secs(6))
        .expect("the service did not exit after SIGTERM");

    assert_eq!(
        status.code(),
        Some(0),
        "expected a clean shutdown, got {status:?}"
    );

    kill_stand_in(&layout.pid_file);
}

#[test]
fn a_missing_x_socket_times_out_with_a_non_zero_exit() {
    let layout = Layout::new();
    let display = 98;

    // No listener is created: Xwayland never becomes ready.
    let satellite = write_stand_in(layout.script_dir(), &layout.pid_file, "exec sleep 300\n");

    let mut args = layout.base_args(display, &satellite);
    args.extend([
        "--ready-timeout".to_owned(),
        "1".to_owned(),
        "--shutdown-grace".to_owned(),
        "5".to_owned(),
    ]);

    let mut service = ServiceProcess::start(&args);

    let status = service
        .wait_for_exit(SLACK + Duration::from_secs(6))
        .expect("the service did not give up on readiness");

    assert_ne!(
        status.code(),
        Some(0),
        "expected a failure exit code, got {status:?}"
    );

    kill_stand_in(&layout.pid_file);
}

#[test]
fn a_socket_that_never_answers_the_setup_times_out_with_a_non_zero_exit() {
    let layout = Layout::new();
    let display = 95;

    // The socket accepts connections but never answers the X11 setup request:
    // with the round-trip probe this is not ready, so the deadline must fire.
    let _stand_in = X11StandIn::silent(&layout.socket_path(display));

    let satellite = write_stand_in(layout.script_dir(), &layout.pid_file, "exec sleep 300\n");

    let mut args = layout.base_args(display, &satellite);
    args.extend([
        "--ready-timeout".to_owned(),
        "1".to_owned(),
        "--shutdown-grace".to_owned(),
        "5".to_owned(),
    ]);

    let mut service = ServiceProcess::start(&args);

    let status = service
        .wait_for_exit(SLACK + Duration::from_secs(6))
        .expect("the service did not give up on readiness");

    assert_ne!(
        status.code(),
        Some(0),
        "expected a failure exit code, got {status:?}"
    );

    kill_stand_in(&layout.pid_file);
}

#[test]
fn an_early_satellite_exit_is_reported_through_the_exit_code() {
    let layout = Layout::new();
    let display = 99;

    let satellite = write_stand_in(layout.script_dir(), &layout.pid_file, "exit 7\n");

    let mut args = layout.base_args(display, &satellite);
    args.extend([
        "--ready-timeout".to_owned(),
        "10".to_owned(),
        "--shutdown-grace".to_owned(),
        "5".to_owned(),
    ]);

    let mut service = ServiceProcess::start(&args);

    let status = service
        .wait_for_exit(SLACK + Duration::from_secs(6))
        .expect("the service did not notice the satellite exit");

    assert_eq!(
        status.code(),
        Some(7),
        "expected the satellite's exit code, got {status:?}"
    );
}

#[test]
fn a_signalled_satellite_is_reported_as_a_failure() {
    let layout = Layout::new();
    let display = 96;

    // The stand-in kills itself, so the service sees a child without an exit
    // code (killed by a signal) and must not report success.
    let satellite = write_stand_in(layout.script_dir(), &layout.pid_file, "kill -KILL $$\n");

    let mut args = layout.base_args(display, &satellite);
    args.extend([
        "--ready-timeout".to_owned(),
        "10".to_owned(),
        "--shutdown-grace".to_owned(),
        "5".to_owned(),
    ]);

    let mut service = ServiceProcess::start(&args);

    let status = service
        .wait_for_exit(SLACK + Duration::from_secs(6))
        .expect("the service did not notice the signalled child");

    assert_ne!(
        status.code(),
        Some(0),
        "a signalled satellite must not look like a clean shutdown, got {status:?}"
    );
}
