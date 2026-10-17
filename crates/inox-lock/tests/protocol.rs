//! A real Wayland wire peer plays the compositor and drives the built binary.
//!
//! This is the E2 half of the session-lock protocol leg (ADR 0022 decision 6): the
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

use std::{
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
    commits: usize,
    unlocks: usize,
    destroyed_locks: usize,
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
    lock: Option<lock::ExtSessionLockV1>,
    /// Whether the client has committed a frame already, so `locked` is sent once.
    confirmed: bool,
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
        _state: &mut Self,
        _: &Client,
        _: &wl_shm::WlShm,
        request: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, fd, .. } = request {
            // The pool's *contents* are the client's business; what the server has
            // to do here is accept the fd and keep the object alive. Closing the
            // received descriptor is correct: the buffer is created from it next.
            drop(fd);
            init.init(id, ());
        }
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for Server {
    fn request(
        _state: &mut Self,
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
            // The dimensions travel with the buffer object, so the commit handler
            // can check what was attached without a side table.
            init.init(id, (width, height));
        }
    }
}

impl Dispatch<wayland_server::protocol::wl_buffer::WlBuffer, (i32, i32)> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wayland_server::protocol::wl_buffer::WlBuffer,
        _: wayland_server::protocol::wl_buffer::Request,
        _: &(i32, i32),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
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
            }
            wl_surface::Request::Commit => {
                state.observations.record(|observed| observed.commits += 1);
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
                lock: None,
                confirmed: false,
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

        let mut command = Command::new(env!("CARGO_BIN_EXE_inox-lock"));
        command
            .arg("--take-lock")
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

        let child = command.spawn().expect("the lock binary starts");

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
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// Configure the surface and confirm the lock once it is covered.
    Confirm,
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

    {
        let observed = fixture.observations.read();
        assert_eq!(observed.claims, [TOKEN]);
        assert_eq!(observed.configures_sent, [7]);
        assert_eq!(observed.unlocks, 0);
        assert_eq!(observed.commits, 1, "one covered frame was committed");
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
        log.contains("stays locked until you log out"),
        "the operator must be told the session is still locked: {log}"
    );
    let _ = status;
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
