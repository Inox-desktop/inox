//! The compositor-side "waydriver" host harness (`ADR 0036` D02, phase 2d).
//!
//! This is the reusable base every later L3 component migration builds on: it
//! spawns the real `inox-compositor` binary as a subprocess with a private
//! `XDG_RUNTIME_DIR` and a named Wayland socket, waits until the compositor has
//! bound its control socket, connects a control client, asks for a single frame
//! with `dump-frame`, decodes the binary PPM the compositor wrote and asserts
//! the frame against the deterministic scene the headless compositor renders.
//!
//! The scene this harness asserts against is the compositor's own clear colour:
//! with zero clients attached, the headless output renders a uniform 1280x720
//! frame of `render::CLEAR_COLOR_RGBA8` (`[17, 34, 51, 255]`). Asserting every
//! decoded pixel equals that colour means "delete the clear" — break the
//! background the compositor draws — and the assert necessarily fails. No
//! compositor source changes were needed to make the scene deterministic.
//!
//! The control plumbing is the `test-input` feature that `src/test_input.rs`
//! defines; the compositor CI shard builds with `--all-features`, so this test
//! only runs meaningful when that feature (and the `--test-input` socket) is
//! present. It is gated with `#[cfg(feature = "test-input")]` for the same
//! reason `tests/protocol_surface.rs` guards its negative test.

use std::{
    fs,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

mod support;

use support::{Ppm, decode_ppm};

/// The private runtime directory for one compositor process.
///
/// A fresh directory keeps the named Wayland socket and the frame dump from
/// colliding with a real session, and is removed when the host is dropped.
struct RuntimeDir {
    path: PathBuf,
}

impl RuntimeDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("inox-waydriver-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create the harness runtime directory");
        Self { path }
    }

    fn member(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for RuntimeDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A running compositor subprocess.
struct Host {
    child: Child,
    runtime: RuntimeDir,
}

impl Host {
    /// Terminate the compositor gracefully, killing it if it lingers.
    fn stop(&mut self) {
        if self
            .child
            .try_wait()
            .expect("failed to poll the compositor")
            .is_none()
        {
            let status = Command::new("kill")
                .args(["-TERM", &self.child.id().to_string()])
                .status()
                .expect("failed to run kill");
            assert!(status.success(), "kill -TERM exited with {status}");
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .expect("failed to poll the compositor")
            {
                assert!(
                    status.success(),
                    "the compositor exited with {status} instead of a clean shutdown"
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the compositor did not exit within 30s"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Spawn `inox-compositor --headless --socket <name> --test-input <control>` with a
/// private `XDG_RUNTIME_DIR`.
fn spawn_host(socket_name: &str) -> Host {
    let runtime = RuntimeDir::new(socket_name);
    let control = runtime.member("waydriver.sock");

    let child = Command::new(env!("CARGO_BIN_EXE_inox-compositor"))
        .args(["--headless", "--socket", socket_name, "--test-input"])
        .arg(&control)
        .env("XDG_RUNTIME_DIR", runtime.path.as_os_str())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn inox-compositor");

    Host { child, runtime }
}

/// Block until the compositor has bound its control socket.
fn wait_for_control_socket(path: &Path, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if path.exists() {
            return;
        }
        assert!(
            child
                .try_wait()
                .expect("failed to poll the compositor")
                .is_none(),
            "the compositor exited before binding the control socket"
        );
        assert!(
            Instant::now() < deadline,
            "the compositor did not bind {} within 30s",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A blocking client on the compositor's control socket.
struct ControlClient {
    stream: UnixStream,
}

impl ControlClient {
    /// Connect after the socket is known to exist.
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).expect("failed to connect the control socket");
        // Bound the per-read wait so a hung compositor fails the test with a clear
        // panic instead of blocking it forever; a live compositor replies within a
        // few milliseconds.
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .expect("failed to set the control-socket read timeout");
        Self { stream }
    }

    /// Send one command and read back the compositor's `ok`/`err …` reply.
    /// Returns `true` for `ok`, `false` for `err`, and panics on a short read or
    /// when the compositor stays silent past the deadline.
    fn command(&mut self, line: &str) -> bool {
        // The compositor only parses a complete line, so every command is
        // terminated with a newline (test_input.rs `read_available`).
        write!(self.stream, "{line}\n").expect("failed to write the control command");
        // The reply is at most a handful of bytes — "ok\n" or "err <message>\n" —
        // always one line. Loop until the newline, retrying a timed-out or
        // interrupted read, and fail the test once the deadline passes.
        let mut reply = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let mut chunk = [0u8; 64];
            match self.stream.read(&mut chunk) {
                Ok(0) => panic!("the compositor closed the control socket before replying"),
                Ok(n) => {
                    reply.extend_from_slice(&chunk[..n]);
                    if reply.contains(&b'\n') {
                        break;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    assert!(
                        Instant::now() < deadline,
                        "the compositor did not reply within 30s"
                    );
                    continue;
                }
                Err(e) => panic!("failed to read the control socket: {e}"),
            }
        }
        let text = String::from_utf8(reply).expect("the reply must be UTF-8");
        match text.as_str() {
            "ok\n" => true,
            _ if text.starts_with("err ") => false,
            other => panic!("unexpected control reply: {other:?}"),
        }
    }
}

/// The deterministic scene the headless compositor renders with zero clients.
///
/// Mirrored from `src/state.rs`: for a headless output the render target is
/// cleared to `render::CLEAR_COLOR_RGBA8` (`[17, 34, 51, 255]`) before any
/// client surface is composited on top, so an empty client list yields this
/// uniform colour across the whole 1280x720 frame.
mod scene {
    /// The headless compositor's default mode (state.rs `create_headless_outputs`).
    pub const DEFAULT_MODE: (u32, u32) = (1280, 720);
    /// The clear colour, from `render::CLEAR_COLOR_RGBA8`.
    pub const CLEAR_RGBA: [u8; 4] = [17, 34, 51, 255];
}

/// Compare a decoded frame against the deterministic clear-colour scene.
///
/// `TOLERANCE` is one 8-bit step, matching `tests/headless_render.rs` and the
/// float-to-unorm rounding difference between GL drivers.
fn assert_matches_clear_scene(ppm: &Ppm, width: u32, height: u32, tolerance: u8) {
    assert_eq!(
        (ppm.width, ppm.height),
        (width, height),
        "the frame geometry must match the headless output"
    );
    let expected = scene::CLEAR_RGBA;
    for (pixel_index, pixel) in ppm.pixels.chunks_exact(3).enumerate() {
        for channel in 0..3 {
            let actual = pixel[channel];
            let want = expected[channel];
            assert!(
                actual.abs_diff(want) <= tolerance,
                "pixel {pixel_index} channel {channel}: expected {want}±{tolerance}, got {actual}"
            );
        }
    }
}

#[test]
#[cfg_attr(not(feature = "test-input"), ignore)]
fn waydriver_host_dumps_and_asserts_the_deterministic_scene() {
    let socket_name = "wayland-waydriver";
    let mut host = spawn_host(socket_name);
    let control = host.runtime.member("waydriver.sock");
    wait_for_control_socket(&control, &mut host.child);

    let frame_path = host.runtime.member("frame.ppm");
    let mut client = ControlClient::connect(&control);

    // Ask the live compositor for a frame. The `ok` acknowledgement means the
    // command was applied and the file written.
    let ok = client.command(&format!(
        "dump-frame {}",
        frame_path.to_str().expect("the frame path must be UTF-8")
    ));
    assert!(ok, "the compositor rejected the dump-frame command");

    let bytes = fs::read(&frame_path).expect("the compositor should have written the frame");
    let ppm = decode_ppm(&bytes).expect("the dumped frame must decode as a P6 PPM");

    let (width, height) = scene::DEFAULT_MODE;
    assert_matches_clear_scene(&ppm, width, height, 1);

    // Dropping here sends SIGTERM and waits for a clean exit.
    host.stop();
}

#[test]
#[cfg_attr(not(feature = "test-input"), ignore)]
fn waydriver_host_rejects_a_bad_dump_path() {
    let socket_name = "wayland-waydriver-bad";
    let mut host = spawn_host(socket_name);
    let control = host.runtime.member("waydriver.sock");
    wait_for_control_socket(&control, &mut host.child);

    let mut client = ControlClient::connect(&control);
    // An unwritable path (the enclosing directory does not exist) must produce an
    // `err` reply, proving the control channel surfaces command failure.
    let missing = host.runtime.member("no-such-dir").join("frame.ppm");
    let ok = client.command(&format!(
        "dump-frame {}",
        missing.to_str().expect("the frame path must be UTF-8")
    ));
    assert!(
        !ok,
        "the compositor accepted a dump-frame to an unwritable path"
    );

    host.stop();
}
