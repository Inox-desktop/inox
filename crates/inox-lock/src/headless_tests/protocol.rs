//! A real Wayland wire peer plays the compositor and drives the built binary.
//!
//! This is the E2 half of the session-lock protocol leg: the
//! server side below is a hand-written `ext-session-lock-v1`, `wl_shm` and
//! `inox-ipc`, so what is asserted is what the binary actually put on the wire —
//! the role claim with the session's token, the lock request, a lock surface per
//! output, an acknowledged configure followed by an opaque frame of exactly the
//! configured size, and, when the process is signalled, **no**
//! `unlock_and_destroy`.
//!
//! The last one is the assertion that matters most: this build has no reviewed
//! authentication boundary, so a signal must never mean "the desktop is available
//! again".
//!
//! Moved from `tests/protocol.rs` when the crate became bin-only (contract §2.1).
//! Nothing about the peer changed: it is the same hand-written server, and the
//! frames it reads back are now produced by the QML host instead of tiny-skia.

use std::{
    os::unix::fs::FileExt as _,
    os::unix::net::UnixListener,
    process::{Child, Command},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use inox_protocols::{
    ipc::inox_ipc::server::inox_ipc::{self as ipc, InoxIpc},
    standard::{
        core::server::{wl_compositor, wl_output, wl_shm, wl_shm_pool, wl_surface},
        ext::session_lock::server::{
            ext_session_lock_manager_v1 as manager, ext_session_lock_surface_v1 as lock_surface,
            ext_session_lock_v1 as lock,
        },
    },
};
use rustix::process::{Pid, Signal, kill_process};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource,
    backend::ClientData,
};

/// The size the compositor configures a lock surface to.
const OUTPUT_WIDTH: u32 = 320;
const OUTPUT_HEIGHT: u32 = 240;

/// The size it reconfigures it to, in the fixture that does.
const RESIZED_WIDTH: u32 = 400;
const RESIZED_HEIGHT: u32 = 300;

/// How many times that fixture reconfigures it.
///
/// Three, and not one, because the property under test is about a frame the
/// compositor has *released*: a client only learns that when it commits the frame
/// after the one that replaced it, so the frame destroyed on the third commit is
/// the one the second replaced.
const RECONFIGURES: u32 = 3;

/// The token `inox-session` would have delivered to both sides.
const TOKEN: &str = "test-session-token";

/// What the server observed on the wire, shared with the test thread.
#[derive(Debug, Default)]
struct Observed {
    claims: Vec<String>,
    locks: usize,
    surfaces: usize,
    configures_sent: Vec<u32>,
    acks: Vec<u32>,
    /// The size of every buffer attached to a lock surface, in attach order.
    attached: Vec<Option<(i32, i32)>>,
    /// Every shared-memory frame the client handed over, measured.
    frames: Vec<Frame>,
    /// How many `wl_buffer` objects the client created.
    buffers_created: usize,
    /// How many it asked the compositor to destroy.
    buffers_destroyed: usize,
    commits: usize,
    unlocks: usize,
    destroyed_locks: usize,
}

/// What one frame the client handed over looks like, measured from its pixels.
///
/// The point of measuring rather than trusting: the difference between a lock
/// surface that carries the interface and one that carries a flat colour is
/// exactly "how many different pixels are in it", and that is a fact about the
/// bytes on the wire rather than about the client's own account of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frame {
    /// How many bytes the frame holds.
    bytes: usize,
    /// How many distinct RGBA pixels it holds.
    distinct: usize,
    /// Whether every pixel is opaque.
    opaque: bool,
    /// The pixel at the surface's top-left corner, where the scrim shows.
    corner: [u8; 4],
    /// The pixel at the surface's centre, where the card is drawn.
    centre: [u8; 4],
}

impl Frame {
    /// Summarise an `Argb8888` frame: the byte order on the wire is `B, G, R, A`.
    fn measure(bytes: &[u8], (width, height): (i32, i32)) -> Self {
        let mut counts: std::collections::HashMap<[u8; 4], usize> =
            std::collections::HashMap::new();

        for pixel in bytes.as_chunks::<4>().0 {
            *counts.entry(*pixel).or_default() += 1;
        }

        let (Ok(width), Ok(height)) = (usize::try_from(width), usize::try_from(height)) else {
            return Self::of(bytes, counts, [0; 4], [0; 4]);
        };

        Self::of(
            bytes,
            counts,
            Self::pixel_at(bytes, width, 0, 0),
            Self::pixel_at(bytes, width, width / 2, height / 2),
        )
    }

    /// The frame read as `bytes`, with `counts` already collected.
    fn of(
        bytes: &[u8],
        counts: std::collections::HashMap<[u8; 4], usize>,
        corner: [u8; 4],
        centre: [u8; 4],
    ) -> Self {
        Self {
            bytes: bytes.len(),
            distinct: counts.len(),
            opaque: bytes
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 0xff),
            corner,
            centre,
        }
    }

    /// One pixel of an `Argb8888` frame `width` pixels wide.
    fn pixel_at(bytes: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let index = (y * width + x) * 4;

        bytes
            .get(index..index + 4)
            .and_then(|pixel| <[u8; 4]>::try_from(pixel).ok())
            .unwrap_or_default()
    }
}

/// The mutex around what the server saw, shared with the test thread.
#[derive(Default)]
struct Observations(Mutex<Observed>);

impl Observations {
    fn record(&self, update: impl FnOnce(&mut Observed)) {
        if let Ok(mut guard) = self.0.lock() {
            update(&mut guard);
        }
    }

    fn read(&self) -> std::sync::MutexGuard<'_, Observed> {
        self.0.lock().expect("the observations are not poisoned")
    }
}

#[derive(Default)]
struct Server {
    observations: Arc<Observations>,
    /// How this fixture answers the lock request.
    answer: Answer,
    lock: Option<lock::ExtSessionLockV1>,
    /// The lock surface, held so a test can configure it a second time.
    lock_surface: Option<lock_surface::ExtSessionLockSurfaceV1>,
    /// Whether the client has committed a frame already, so `locked` is sent once.
    confirmed: bool,
    /// How many reconfigures this fixture still owes the client.
    reconfigures_left: u32,
    /// The size the lock surface was last configured to.
    ///
    /// A shared-memory frame carries pixels, not a shape: the width has to come
    /// from the configure the compositor sent, which is what tells the test where
    /// the card and the scrim are.
    configured: (i32, i32),
    /// The buffer currently on the lock surface, and the one the last attach
    /// offered — held so the replaced one can be released, as a real compositor
    /// releases it when it commits a different buffer.
    current_buffer: Option<wayland_server::protocol::wl_buffer::WlBuffer>,
    pending_buffer: Option<wayland_server::protocol::wl_buffer::WlBuffer>,
}

struct ClientState;

impl ClientData for ClientState {}

impl GlobalDispatch<wl_compositor::WlCompositor, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_compositor::WlCompositor>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_compositor::WlCompositor,
        request: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_compositor::Request::CreateSurface { id } = request {
            init.init(id, ());
        }
    }
}

impl GlobalDispatch<wl_shm::WlShm, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_shm::WlShm>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<wl_shm::WlShm, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_shm::WlShm,
        request: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, fd, size } = request {
            // The server accepts the fd and creates the buffer object from it; the
            // *contents* are what the client put there, and this test reads them
            // back because "the lock surface carries the interface" is a claim
            // about those bytes. The client writes the frame before it creates the
            // pool, so everything is in the file by the time this runs. Closing the
            // descriptor afterwards is correct: the buffer object holds its own
            // reference.
            let mut frame = vec![0u8; usize::try_from(size).unwrap_or(0)];
            let file = std::fs::File::from(fd);

            // Read *at* an offset rather than from the current one: a descriptor
            // received over the socket shares its file description — and therefore
            // its offset — with the client's, and the client has just written the
            // frame, so the offset is at the end.
            if file.read_exact_at(&mut frame, 0).is_ok() {
                state.observations.record(|observed| {
                    observed
                        .frames
                        .push(Frame::measure(&frame, state.configured))
                });
            }

            init.init(id, ());
        }
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_shm_pool::WlShmPool,
        request: wl_shm_pool::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm_pool::Request::CreateBuffer {
            id,
            width,
            height,
            format,
            ..
        } = request
        {
            assert_eq!(
                format,
                wayland_server::WEnum::Value(wl_shm::Format::Argb8888),
                "a lock frame must be a plain opaque ARGB buffer"
            );
            state
                .observations
                .record(|observed| observed.buffers_created += 1);
            // The dimensions travel with the buffer object, so the commit handler
            // can check what was attached without a side table.
            init.init(id, (width, height));
        }
    }
}

impl Dispatch<wayland_server::protocol::wl_buffer::WlBuffer, (i32, i32)> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wayland_server::protocol::wl_buffer::WlBuffer,
        request: wayland_server::protocol::wl_buffer::Request,
        _: &(i32, i32),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // A client that drops its buffers without saying so leaves the compositor
        // holding a mapping and a file descriptor for each one; this counts the
        // requests that actually release them.
        if let wayland_server::protocol::wl_buffer::Request::Destroy = request {
            state
                .observations
                .record(|observed| observed.buffers_destroyed += 1);
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_surface::WlSurface,
        request: wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_surface::Request::Attach { buffer, .. } => {
                let size = buffer
                    .as_ref()
                    .and_then(|buffer| buffer.data::<(i32, i32)>().copied());
                state
                    .observations
                    .record(|observed| observed.attached.push(size));
                state.pending_buffer = buffer;
            }
            wl_surface::Request::Commit => {
                state.observations.record(|observed| observed.commits += 1);

                // `wl_buffer.release` is the compositor's half of the buffer
                // handshake, and the client's half — destroying a frame it may now
                // reuse — is what this fixture exists to observe. Smithay sends it
                // when a commit replaces the buffer with a different one
                // (`wayland/compositor/handlers.rs`), so a server that never sends
                // it would be testing a client against a compositor that does not
                // exist.
                if let Some(attached) = state.pending_buffer.take() {
                    let replaced = state.current_buffer.replace(attached.clone());

                    match replaced {
                        Some(replaced) if replaced != attached => replaced.release(),
                        _ => {}
                    }
                }

                // A compositor that changes its mind about the size — a resize, a
                // mode change — configures the surface again, and the client has to
                // answer with a frame of the new size. One fixture asks for it, so
                // that "the client redraws on a second configure" and "the frame it
                // replaced is released" are both covered rather than only reasoned
                // about.
                if state.answer == Answer::Reconfigure && state.reconfigures_left > 0 {
                    let serial = 9 + 2 * (RECONFIGURES - state.reconfigures_left);
                    state.reconfigures_left -= 1;
                    state
                        .observations
                        .record(|observed| observed.configures_sent.push(serial));

                    if let Some(role) = state.lock_surface.as_ref() {
                        role.configure(serial, RESIZED_WIDTH, RESIZED_HEIGHT);
                    }
                    state.configured = (RESIZED_WIDTH as i32, RESIZED_HEIGHT as i32);
                }

                // The compositor confirms the lock once a covered frame has been
                // presented on every output; with one output that is this commit.
                if !state.confirmed
                    && state
                        .observations
                        .read()
                        .attached
                        .last()
                        .copied()
                        .flatten()
                        .is_some()
                {
                    state.confirmed = true;
                    if let Some(lock) = state.lock.as_ref() {
                        lock.locked();
                    }
                }
            }
            _ => {}
        }
    }
}

impl GlobalDispatch<wl_output::WlOutput, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_output::WlOutput>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Server {
    fn request(
        _state: &mut Self,
        _: &Client,
        _: &wl_output::WlOutput,
        _: wl_output::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl GlobalDispatch<InoxIpc, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<InoxIpc>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<InoxIpc, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &InoxIpc,
        request: ipc::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            ipc::Request::ClaimLockRole { token } => {
                state
                    .observations
                    .record(|observed| observed.claims.push(token));
                resource.version(2);
            }
            ipc::Request::GetVersion => resource.version(2),
            _ => {}
        }
    }
}

impl GlobalDispatch<manager::ExtSessionLockManagerV1, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<manager::ExtSessionLockManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<manager::ExtSessionLockManagerV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &manager::ExtSessionLockManagerV1,
        request: manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let manager::Request::Lock { id } = request {
            state.observations.record(|observed| observed.locks += 1);
            state.lock = Some(init.init(id, ()));
        }
    }
}

impl Dispatch<lock::ExtSessionLockV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &lock::ExtSessionLockV1,
        request: lock::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            lock::Request::GetLockSurface { id, surface, .. } => {
                let role = init.init(id, ());
                state.lock_surface = Some(role.clone());
                state.configured = (OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32);
                let serial = 7;
                state.observations.record(|observed| {
                    observed.surfaces += 1;
                    observed.configures_sent.push(serial);
                });
                // The protocol requires the first configure to precede the first
                // commit, and it is an exact size requirement.
                role.configure(serial, OUTPUT_WIDTH, OUTPUT_HEIGHT);
                let _ = surface;
            }
            lock::Request::UnlockAndDestroy => {
                state.observations.record(|observed| observed.unlocks += 1);
            }
            lock::Request::Destroy => {
                state
                    .observations
                    .record(|observed| observed.destroyed_locks += 1);
            }
            _ => {}
        }
    }
}

impl Dispatch<lock_surface::ExtSessionLockSurfaceV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &lock_surface::ExtSessionLockSurfaceV1,
        request: lock_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let lock_surface::Request::AckConfigure { serial } = request {
            state
                .observations
                .record(|observed| observed.acks.push(serial));
        }
    }
}

/// A running server, its socket directory, and the process under test.
struct Fixture {
    _root: tempfile::TempDir,
    child: Child,
    observations: Arc<Observations>,
}

impl Fixture {
    /// Start the server and the binary against it.
    ///
    /// `send_locked` decides whether this server ever confirms the lock, which is
    /// what separates "the client took the session" from "the compositor refused".
    fn start(_label: &str, token: Option<&str>, answer: Answer) -> Self {
        let root = tempfile::tempdir().expect("a private runtime directory");
        let socket_path = root.path().join("wayland-lock");
        let listener = UnixListener::bind(&socket_path).expect("the socket binds");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");

        let observations = Arc::new(Observations::default());
        let server_observations = observations.clone();

        let mut display = Display::<Server>::new().expect("a display");
        let mut handle = display.handle();
        handle.create_global::<Server, wl_compositor::WlCompositor, ()>(5, ());
        handle.create_global::<Server, wl_shm::WlShm, ()>(1, ());
        handle.create_global::<Server, wl_output::WlOutput, ()>(4, ());
        handle.create_global::<Server, manager::ExtSessionLockManagerV1, ()>(1, ());
        handle.create_global::<Server, InoxIpc, ()>(2, ());

        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_stop = stop.clone();
        let refusing = answer == Answer::Finished;
        std::thread::spawn(move || {
            let mut state = Server {
                observations: server_observations,
                answer,
                reconfigures_left: RECONFIGURES,
                ..Server::default()
            };
            // A refusing compositor answers the lock request with `finished` as
            // soon as it arrives; one that accepts waits for the client's frames.
            let deadline = Instant::now() + Duration::from_secs(20);
            while !server_stop.load(std::sync::atomic::Ordering::Relaxed)
                && Instant::now() < deadline
            {
                if let Ok((stream, _address)) = listener.accept() {
                    stream
                        .set_nonblocking(true)
                        .expect("a non-blocking client connection");
                    handle
                        .insert_client(stream, Arc::new(ClientState))
                        .expect("the client is registered");
                }
                if display.dispatch_clients(&mut state).is_err() {
                    continue;
                }
                if display.flush_clients().is_err() {
                    continue;
                }
                if refusing && let Some(lock) = state.lock.as_ref() {
                    lock.finished();
                    state.lock = None;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        });

        let log_path = root.path().join("lock.log");
        let log = std::fs::File::create(&log_path).expect("the log file");

        // The crate is bin-only (contract §2.1), so there is no separate
        // executable to run: the test binary spawns itself with one ignored test
        // selected, and that child calls the same `crate::run` `main` does.
        let mut command = Command::new(std::env::current_exe().expect("the test binary path"));
        command
            .args([
                "--ignored",
                "--exact",
                "--nocapture",
                "--test-threads=1",
                "headless_tests::protocol::binary_child",
            ])
            .env("INOX_LOCK_PROTOCOL_CHILD", "1")
            .env("XDG_RUNTIME_DIR", root.path())
            .env("XDG_CONFIG_HOME", root.path().join("config"))
            .env("WAYLAND_DISPLAY", "wayland-lock")
            .env("LANG", "en-US")
            .env("LC_ALL", "en-US")
            .env("RUST_LOG", "info")
            .stdout(log.try_clone().expect("a log handle"))
            .stderr(log);
        match token {
            Some(token) => {
                command.env("INOX_LOCK_TOKEN", token);
            }
            None => {
                command.env_remove("INOX_LOCK_TOKEN");
            }
        }

        let child = {
            // `signalfd` only receives a signal that *every* thread blocks, and a
            // signal the kernel can deliver to an unblocked thread runs its default
            // action instead. The child here is this test binary, so its entry
            // point runs on a test thread rather than on `main`; the thread that
            // spawns it hands down its mask across `exec`, which is why these are
            // blocked in this thread for exactly as long as the spawn takes.
            let mask = calloop::signals::Signals::new(&[
                calloop::signals::Signal::SIGINT,
                calloop::signals::Signal::SIGTERM,
                calloop::signals::Signal::SIGHUP,
            ])
            .expect("the stop signals can be blocked");
            let child = command.spawn().expect("the lock binary starts");
            drop(mask);

            child
        };

        Self {
            _root: root,
            child,
            observations,
        }
    }

    /// The log the binary wrote.
    fn log(&self) -> String {
        std::fs::read_to_string(self._root.path().join("lock.log")).unwrap_or_default()
    }

    /// Wait for `condition`, dispatching nothing: the server thread is already
    /// running.
    fn wait_for(&self, what: &str, condition: impl Fn(&Observed) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if condition(&self.observations.read()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; the child wrote:\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn signal(&self, signal: Signal) {
        kill_process(
            Pid::from_raw(self.child.id() as i32).expect("the child pid"),
            signal,
        )
        .expect("the signal is delivered");
    }

    fn wait_for_exit(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().expect("the child status") {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();

                panic!("the lock binary did not exit");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// How the server answers the lock request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Answer {
    /// Configure the surface and confirm the lock once it is covered.
    #[default]
    Confirm,
    /// The same, and then configure the surface to a second size once the first
    /// frame arrives — a compositor that changed its mind about the output.
    Reconfigure,
    /// Refuse with the protocol's `finished`, as a compositor that will not lock
    /// does.
    Finished,
}

#[test]
fn the_binary_claims_covers_and_never_unlocks_on_a_signal() {
    let mut fixture = Fixture::start("take-lock", Some(TOKEN), Answer::Confirm);

    // The claim carries the session's token, and the lock is requested after it.
    fixture.wait_for("the role claim", |observed| !observed.claims.is_empty());
    fixture.wait_for("the lock request", |observed| observed.locks == 1);
    fixture.wait_for("a lock surface", |observed| observed.surfaces == 1);
    fixture.wait_for("an acknowledged configure", |observed| observed.acks == [7]);

    // What was attached is a real frame of exactly the configured size: a
    // mismatch is a protocol error, and a null buffer is not a cover at all.
    fixture.wait_for("a covered frame", |observed| {
        observed.attached.last().copied().flatten()
            == Some((OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32))
    });
    fixture.wait_for("the lock to be confirmed and reported", |_| {
        fixture.log().contains("The session is locked")
    });

    // And what was committed is the lock screen, not a colour standing in for one.
    // The frame's *contents* are the part of this that only the wire can show: the
    // client's own tests assert on pixels it rendered into memory, this asserts on
    // the bytes it handed to the compositor.
    fixture.wait_for("a frame with pixels in it", |observed| {
        observed
            .frames
            .first()
            .is_some_and(|frame| frame.distinct > 1)
    });

    {
        let observed = fixture.observations.read();
        assert_eq!(observed.claims, [TOKEN]);
        assert_eq!(observed.configures_sent, [7]);
        assert_eq!(observed.unlocks, 0);
        assert_eq!(observed.commits, 1, "one covered frame was committed");

        let frame = observed
            .frames
            .first()
            .copied()
            .expect("a shared-memory frame");
        let pixels = (OUTPUT_WIDTH * OUTPUT_HEIGHT) as usize;

        assert_eq!(
            frame.bytes,
            pixels * 4,
            "the frame is exactly the size the surface was configured to"
        );
        assert!(
            frame.opaque,
            "every pixel of a lock frame is opaque: a translucent cover would show \
             the session it is hiding"
        );
        assert!(
            frame.distinct > 1,
            "the lock frame is a single colour, so nothing was drawn on it"
        );
        // The interface is a card centred on a full-bleed scrim, so its centre and
        // its corner cannot be the same pixel. The card is half the surface wide
        // and taller than a small output, which is why the frame has no majority
        // colour here: what marks it as a drawn screen is the two regions, not
        // which one of them covers more pixels.
        assert_ne!(
            frame.centre, frame.corner,
            "the centre of the lock surface is the card and its corner is the scrim; \
             a frame whose two ends are the same colour carries one flat colour, not \
             the interface"
        );
    }

    // A signal stops the process. It must not release the session: this build has
    // no authentication boundary, so there is nothing that could justify making
    // the desktop available again.
    fixture.signal(Signal::TERM);
    let status = fixture.wait_for_exit();

    let observed = fixture.observations.read();
    assert_eq!(
        observed.unlocks, 0,
        "a signalled lock client must not unlock the session"
    );
    assert_eq!(
        observed.destroyed_locks, 0,
        "the lock object is not destroyed either: the compositor keeps the session locked"
    );
    let log = fixture.log();
    assert!(
        log.contains("Stopped while the session is locked"),
        "the operator must be told the session is still locked ({status:?}): {log}"
    );
}

#[test]
fn a_reconfigure_is_drawn_at_the_new_size_and_the_frame_it_replaced_is_destroyed() {
    // Two things the single-output fixture cannot show, and both are the kind of
    // thing that only shows up in a session that runs for hours:
    //
    // - a second configure has to be answered with a frame of the *new* size, or
    //   the surface is showing a frame the compositor has been told to scale;
    // - the frame the new one replaced has to be destroyed. Dropping a
    //   `wayland-client` proxy sends nothing, so a client that relies on the drop
    //   leaves the compositor holding a shared-memory mapping and a descriptor per
    //   frame — for the whole lock.
    let mut fixture = Fixture::start("reconfigure", Some(TOKEN), Answer::Reconfigure);

    fixture.wait_for("the lock to be confirmed and reported", |_| {
        fixture.log().contains("The session is locked")
    });
    fixture.wait_for("a frame at the resized dimensions", |observed| {
        observed
            .frames
            .iter()
            .any(|frame| frame.bytes == (RESIZED_WIDTH * RESIZED_HEIGHT * 4) as usize)
    });
    fixture.wait_for("the replaced frame to be destroyed", |observed| {
        observed.buffers_destroyed > 0
    });
    // The last of the three reconfigures is answered by the fourth commit, and the
    // assertions below are about the whole exchange rather than about a prefix of
    // it.
    fixture.wait_for("every configure to be answered", |observed| {
        // An ack goes out before the frame that answers the configure, so the
        // exchange is complete only once both counts have caught up.
        observed.acks.len() == 4 && observed.buffers_created == 4
    });

    {
        let observed = fixture.observations.read();
        let resized = observed
            .frames
            .iter()
            .find(|frame| frame.bytes == (RESIZED_WIDTH * RESIZED_HEIGHT * 4) as usize)
            .expect("a frame of the second size");

        assert!(
            resized.opaque && resized.distinct > 1,
            "the redrawn frame is the interface too: {resized:?}"
        );
        assert_eq!(
            observed.configures_sent,
            [7, 9, 11, 13],
            "every configure was sent, so every one is being answered"
        );
        assert_eq!(observed.acks, [7, 9, 11, 13], "and every one was acked");
        assert_eq!(
            observed.buffers_created, 4,
            "one frame per configure, and no more"
        );
        assert!(
            observed.buffers_created - observed.buffers_destroyed
                <= crate::session::MAX_FRAMES_IN_FLIGHT,
            "the client is holding {} frames the compositor has released: \
             created {}, destroyed {}",
            observed.buffers_created - observed.buffers_destroyed,
            observed.buffers_created,
            observed.buffers_destroyed,
        );
    }

    fixture.signal(Signal::TERM);
    let _ = fixture.wait_for_exit();

    assert_eq!(
        fixture.observations.read().unlocks,
        0,
        "a signalled lock client must not unlock the session, however many times \
         it was reconfigured"
    );
}

#[test]
fn a_missing_token_is_refused_before_anything_is_asked_for() {
    let mut fixture = Fixture::start("no-token", None, Answer::Confirm);

    let status = fixture.wait_for_exit();
    assert!(!status.success(), "a run without a token is a failure");

    let observed = fixture.observations.read();
    assert_eq!(
        observed.claims.len(),
        0,
        "without a token there is nothing to claim with, so nothing is claimed"
    );
    assert_eq!(observed.locks, 0, "and no lock is requested");
    drop(observed);

    let log = fixture.log();
    assert!(
        log.contains("no session lock token was delivered"),
        "the failure has to name the missing token: {log}"
    );
}

#[test]
fn a_compositor_that_refuses_the_lock_is_reported_and_exited() {
    let mut fixture = Fixture::start("refused", Some(TOKEN), Answer::Finished);

    fixture.wait_for("the refusal to be observed", |observed| observed.locks == 1);
    let status = fixture.wait_for_exit();

    assert!(
        !status.success(),
        "a refused lock is a failure, not a locked session"
    );
    let log = fixture.log();
    assert!(
        log.contains("refused the session lock"),
        "the refusal must be reported: {log}"
    );
    let observed = fixture.observations.read();
    assert_eq!(observed.unlocks, 0);
}

/// The child half of [`Fixture::start`]: the process-level entry point.
///
/// The protocol leg takes its arguments from `main`'s command line; here the one
/// argument it is exercised with is the flag that runs it.
#[test]
#[ignore = "spawned as a child process by the protocol tests"]
fn binary_child() {
    assert_eq!(
        std::env::var("INOX_LOCK_PROTOCOL_CHILD").as_deref(),
        Ok("1"),
        "this test only runs as the spawned child"
    );

    // The child is this test binary, so the entry point runs on a test thread
    // rather than on `main`; the fixture blocks the stop signals before spawning
    // it so that its `signalfd` is the one that receives them (see `Fixture::start`).
    match crate::run(["--take-lock".to_owned()].into_iter()) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("Error: {error:#}");
            std::process::exit(1);
        }
    }
}
