//! End-to-end evidence for `inox-window-manager-v1`: a real compositor process
//! driven by a real `wayland-client`.
//!
//! This is the E2 half of ADR 0045 §5. The protocol round-trip in
//! `inox-protocols/tests/window_manager_roundtrip.rs` proves the *wire format* —
//! that the bytes the compositor sends are the ones a client decodes — against a
//! stand-in server. This test proves the other half: that the **real** compositor
//! advertises the global, serves a snapshot from its real managed windows, and
//! answers every verb with the state event the protocol promises.
//!
//! What it deliberately does not do: read pixels, take screenshots, or claim
//! anything about a real display. The events carry handles, strings and
//! enumerations only, which is the property the protocol is built on; W05's
//! screenshot evidence and the E3/E4 acceptance remain outstanding and this test
//! must never be cited as them.
//!
//! # Why one process and one test
//!
//! A spawned compositor owns an EGL display, and a test binary may only hold one
//! renderer, so every claim lives in this one `#[test]` against one child process.
//! The client runs on its own thread while the test thread drives the client's
//! event queue; both directions are bounded by deadlines, so a broken compositor
//! fails the test instead of hanging CI.

use std::{
    fs,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

use inox_protocols::{
    standard::{
        core::client::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface},
        xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
    },
    window_manager::inox_window_manager::client::{
        inox_window_list_v1::{self as list, Event as ListEvent, WindowMode},
        inox_window_manager_v1::InoxWindowManagerV1,
    },
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
};

/// The compositor binary built for this test run.
const COMPOSITOR: &str = env!("CARGO_BIN_EXE_inox-compositor");

/// How long the compositor may take to bind its socket.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a single wait may take before the test declares the compositor broken.
const EVENT_TIMEOUT: Duration = Duration::from_secs(20);

/// How many workspaces the compositor is started with.
///
/// More than one, because the workspace verbs are otherwise untestable: with the
/// default of one, every `switch_workspace` is correctly refused, which is a
/// negative case rather than a demonstration of switching.
const WORKSPACES: u32 = 3;

/// Edge length of the `wl_shm` buffer each window draws.
const BUFFER_SIZE: i32 = 64;

/// The handle the protocol reserves for a request that names no window.
const NO_WINDOW_HANDLE: u32 = u32::MAX;

/// A private `XDG_RUNTIME_DIR` for the compositor under test.
struct RuntimeDir(PathBuf);

impl RuntimeDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("inox-wm-{label}-{}", std::process::id()));

        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("failed to create the test runtime directory");

        Self(path)
    }

    fn member(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for RuntimeDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Start the compositor with its own runtime directory and a known socket name.
fn spawn_compositor(socket_name: &str) -> (Child, RuntimeDir, PathBuf) {
    let runtime_dir = RuntimeDir::new(socket_name);

    let child = Command::new(COMPOSITOR)
        .args([
            "--headless",
            "--socket",
            socket_name,
            "--workspaces",
            &WORKSPACES.to_string(),
            "--output-size",
            "1280x720",
        ])
        .env("XDG_RUNTIME_DIR", runtime_dir.0.as_os_str())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn inox-compositor");

    let socket = runtime_dir.member(socket_name);

    (child, runtime_dir, socket)
}

/// Block until the compositor has bound its Wayland socket.
fn wait_for_socket(path: &Path) {
    let deadline = Instant::now() + STARTUP_TIMEOUT;

    while Instant::now() < deadline {
        if path.exists() {
            return;
        }

        sleep(Duration::from_millis(20));
    }

    panic!(
        "inox-compositor did not bind {} within {STARTUP_TIMEOUT:?}",
        path.display()
    );
}

/// Everything the client has been told, in arrival order.
///
/// Deliberately raw: the assertions below decode the same fields a panel would,
/// so a wrong argument order shows up here rather than being papered over by a
/// tidy intermediate type.
#[derive(Debug, Default, Clone)]
struct Seen {
    snapshot_order: Vec<&'static str>,
    generation: Option<u32>,
    workspace: Option<(u32, u32)>,
    windows: Vec<Window>,
    removed: Vec<u32>,
    failures: Vec<(u32, u32)>,
}

/// One `window` event, decoded exactly as the protocol documents it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Window {
    handle: u32,
    app_id: String,
    title: String,
    workspace: u32,
    output: String,
    mode: u32,
    minimized: bool,
    visible: bool,
    focused: bool,
}

impl Seen {
    /// The newest record for `handle`, if one has arrived.
    ///
    /// A `window` event carries the *complete* state, so the latest record for a
    /// handle is the truth and the older ones in the log are history. Every
    /// assertion about "what the client currently believes" goes through here: an
    /// assertion over the whole log would see the state a window was in before its
    /// last update.
    fn window(&self, handle: u32) -> Option<&Window> {
        self.windows
            .iter()
            .rev()
            .find(|window| window.handle == handle)
    }

    /// The handles that are focused according to their latest record.
    fn focused(&self) -> Vec<u32> {
        self.latest()
            .into_iter()
            .filter(|window| window.focused)
            .map(|window| window.handle)
            .collect()
    }

    /// The latest record per handle, in the order the handles first appeared.
    fn latest(&self) -> Vec<Window> {
        let mut handles: Vec<u32> = Vec::new();
        for window in &self.windows {
            if !handles.contains(&window.handle) {
                handles.push(window.handle);
            }
        }

        handles
            .into_iter()
            .filter_map(|handle| self.window(handle).cloned())
            .collect()
    }
}

/// The client's dispatch state.
struct Client {
    seen: Seen,
    /// The toplevels created so far, with the objects that must stay alive.
    toplevels: Vec<Toplevel>,
    /// Whether each toplevel has been configured, so a buffer may be attached.
    configured: usize,
}

/// One toplevel the client created, with the objects that keep it alive.
struct Toplevel {
    surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    toplevel: xdg_toplevel::XdgToplevel,
    _buffer: Option<wl_buffer::WlBuffer>,
    _pool: Option<wl_shm_pool::WlShmPool>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` captured the global list already.
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(
        _state: &mut Self,
        base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, u32> for Client {
    fn event(
        state: &mut Self,
        _surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &u32,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            // Acknowledge first, then note it: the compositor only lets the client
            // attach a buffer once the configure has been answered.
            if let Some(toplevel) = state.toplevels.get(*index as usize) {
                toplevel.xdg_surface.ack_configure(serial);
            }
            state.configured = state.configured.max(*index as usize + 1);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, u32> for Client {
    fn event(
        state: &mut Self,
        _toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &u32,
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        // A close request is answered by destroying the toplevel, exactly as an
        // application that honoured it would. That is what turns `close` into
        // `window_removed` — the protocol's only confirmation of a close.
        if let xdg_toplevel::Event::Close = event
            && let Some(toplevel) = state.toplevels.get(*index as usize)
        {
            toplevel.toplevel.destroy();
            toplevel.xdg_surface.destroy();
            toplevel.surface.destroy();
        }
    }
}

impl Dispatch<InoxWindowManagerV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &InoxWindowManagerV1,
        _: <InoxWindowManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The manager has no events.
    }
}

impl Dispatch<list::InoxWindowListV1, ()> for Client {
    fn event(
        state: &mut Self,
        _list: &list::InoxWindowListV1,
        event: ListEvent,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            ListEvent::SessionGeneration { generation } => {
                state.seen.snapshot_order.push("session_generation");
                state.seen.generation = Some(generation);
            }
            ListEvent::WorkspaceState { active, count } => {
                state.seen.snapshot_order.push("workspace_state");
                state.seen.workspace = Some((active, count));
            }
            ListEvent::Window {
                window,
                app_id,
                title,
                workspace,
                output,
                mode,
                minimized,
                visible,
                focused,
            } => {
                state.seen.snapshot_order.push("window");
                state.seen.windows.push(Window {
                    handle: window,
                    app_id,
                    title,
                    workspace,
                    output,
                    // An unknown discriminant is refused rather than rendered as a
                    // wrong number, the same rule the panel applies.
                    mode: mode.into_result().expect("a known window mode") as u32,
                    minimized: minimized != 0,
                    visible: visible != 0,
                    focused: focused != 0,
                });
            }
            ListEvent::WindowRemoved { window } => {
                state.seen.snapshot_order.push("window_removed");
                state.seen.removed.push(window);
            }
            ListEvent::SnapshotDone => {
                state.seen.snapshot_order.push("snapshot_done");
            }
            ListEvent::RequestFailed { window, reason } => {
                state.seen.snapshot_order.push("request_failed");
                state.seen.failures.push((
                    window,
                    reason.into_result().expect("a known request error") as u32,
                ));
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);

/// The client half: a connection, a queue, and the objects to observe with.
struct Leg {
    connection: Connection,
    queue: wayland_client::EventQueue<Client>,
    client: Client,
    compositor: wl_compositor::WlCompositor,
    xdg: xdg_wm_base::XdgWmBase,
    shm: wl_shm::WlShm,
    manager: InoxWindowManagerV1,
    list: list::InoxWindowListV1,
}

impl Leg {
    /// Connect, bind what the test needs, and create the observer list.
    fn connect(socket: &Path) -> Self {
        let stream = UnixStream::connect(socket).expect("failed to connect to inox-compositor");
        let connection = Connection::from_socket(stream).expect("failed to wrap the socket");
        let (globals, mut queue) =
            registry_queue_init::<Client>(&connection).expect("failed to read the registry");
        let qh = queue.handle();

        let compositor: wl_compositor::WlCompositor = globals
            .bind(&qh, 1..=6, ())
            .expect("the compositor advertises wl_compositor");
        let xdg: xdg_wm_base::XdgWmBase = globals
            .bind(&qh, 1..=6, ())
            .expect("the compositor advertises xdg_wm_base");
        let shm: wl_shm::WlShm = globals
            .bind(&qh, 1..=1, ())
            .expect("the compositor advertises wl_shm");
        // The variable this whole test is about. A missing global is a bind error,
        // which is exactly the assertion "the compositor advertises it".
        let manager: InoxWindowManagerV1 = globals
            .bind(&qh, 1..=1, ())
            .expect("the compositor advertises inox_window_manager_v1 v1");
        let list = manager.get_window_list(&qh, ());

        let mut client = Client {
            seen: Seen::default(),
            toplevels: Vec::new(),
            configured: 0,
        };
        connection.flush().expect("failed to flush the client");

        // The first roundtrip is what brings the snapshot; the compositor serves it
        // synchronously from `get_window_list`, so it is already on the wire.
        queue
            .roundtrip(&mut client)
            .expect("failed to receive the snapshot");

        Self {
            connection,
            queue,
            client,
            compositor,
            xdg,
            shm,
            manager,
            list,
        }
    }

    /// Create one toplevel with its title and app id set, and commit it empty.
    ///
    /// An empty commit is deliberate: the compositor learns the surface exists
    /// without it becoming a window, which is what makes the "never drawn" case
    /// testable.
    fn create_toplevel(&mut self, index: u32, app_id: &str, title: &str) -> u32 {
        let qh = self.queue.handle();
        let surface = self.compositor.create_surface(&qh, ());
        let xdg_surface = self.xdg.get_xdg_surface(&surface, &qh, index);
        let toplevel = xdg_surface.get_toplevel(&qh, index);
        toplevel.set_app_id(app_id.to_owned());
        toplevel.set_title(title.to_owned());
        surface.commit();

        self.client.toplevels.push(Toplevel {
            surface,
            xdg_surface,
            toplevel,
            _buffer: None,
            _pool: None,
        });
        self.connection.flush().expect("failed to flush the client");

        index
    }

    /// Wait for `index` to be configured, then attach a buffer to make it a window.
    fn draw(&mut self, index: u32) {
        self.wait_until("the toplevel to be configured", |client| {
            client.configured > index as usize
        });

        let qh = self.queue.handle();
        let stride = BUFFER_SIZE * 4;
        // An anonymous file of exactly the window's pixels; the compositor never
        // reads the contents, it only needs the geometry to composite something.
        let file = tempfile::tempfile().expect("failed to create the buffer file");
        file.set_len((stride * BUFFER_SIZE) as u64)
            .expect("failed to size the buffer file");

        use std::os::fd::AsFd;
        let pool = self
            .shm
            .create_pool(file.as_fd(), stride * BUFFER_SIZE, &qh, ());
        let buffer = pool.create_buffer(
            0,
            BUFFER_SIZE,
            BUFFER_SIZE,
            stride,
            wl_shm::Format::Argb8888,
            &qh,
            (),
        );

        let toplevel = &mut self.client.toplevels[index as usize];
        toplevel.surface.attach(Some(&buffer), 0, 0);
        toplevel.surface.commit();
        toplevel._buffer = Some(buffer);
        toplevel._pool = Some(pool);
        self.connection.flush().expect("failed to flush the client");

        // The commit has to reach the compositor and be promoted before the window
        // is announced; one roundtrip is enough because the compositor processes
        // requests in order.
        self.roundtrip();
    }

    /// Dispatch whatever is pending and merge it into the observation log.
    fn roundtrip(&mut self) {
        self.queue
            .roundtrip(&mut self.client)
            .expect("failed to dispatch compositor events");
    }

    /// Dispatch until `predicate` holds, up to the event timeout.
    fn wait_until(&mut self, what: &str, predicate: impl Fn(&Client) -> bool) {
        let deadline = Instant::now() + EVENT_TIMEOUT;

        loop {
            if predicate(&self.client) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out after {EVENT_TIMEOUT:?} waiting for {what}; \\
                 seen so far: {:?}",
                self.client.seen
            );
            self.roundtrip();
            sleep(Duration::from_millis(2));
        }
    }

    fn seen(&self) -> &Seen {
        &self.client.seen
    }
}

impl Drop for Leg {
    fn drop(&mut self) {
        // Tear the objects down in order so the compositor sees its client leave
        // voluntarily rather than through a protocol error.
        self.list.destroy();
        self.manager.destroy();
        let _ = self.connection.flush();
    }
}

/// The whole contract, against one real compositor and one real client.
#[test]
fn the_compositor_serves_and_drives_window_management_over_the_protocol() {
    let (mut child, _runtime_dir, socket) = spawn_compositor("window-manager");

    wait_for_socket(&socket);
    let mut leg = Leg::connect(&socket);

    // ---------------------------------------------------------------- snapshot
    //
    // An empty desktop still answers completely: the generation, the workspace
    // facts and `snapshot_done`, in that order. A panel that received a bare
    // `snapshot_done` could not tell an empty desktop from a snapshot it had not
    // finished receiving.
    {
        let seen = leg.seen();
        assert_eq!(
            seen.snapshot_order,
            vec!["session_generation", "workspace_state", "snapshot_done"],
            "an empty desktop is announced in the contract's order"
        );
        assert!(
            seen.generation.is_some(),
            "the generation comes before any handle"
        );
        assert_eq!(
            seen.workspace,
            Some((0, WORKSPACES)),
            "the snapshot reports the configured workspace count"
        );
        assert!(seen.windows.is_empty(), "no window exists yet");
    }

    // ------------------------------------------------- two mapped windows, one not
    //
    // The third toplevel is created and committed empty but never draws. It must
    // never be announced: a task button for a window that may never appear is the
    // exact thing the map gate exists to prevent.
    let first = leg.create_toplevel(0, "org.example.Editor", "notes.txt");
    let second = leg.create_toplevel(1, "org.example.Terminal", "shell");
    // Created, committed empty, and never drawn: it must never become a window.
    let _never_drawn = leg.create_toplevel(2, "org.example.Hidden", "helper");
    leg.draw(first);
    leg.draw(second);

    leg.wait_until("both mapped windows to be announced", |client| {
        ["notes.txt", "shell"].iter().all(|title| {
            client
                .seen
                .windows
                .iter()
                .any(|window| &window.title == title)
        })
    });

    let (first_handle, second_handle) = {
        let seen = leg.seen();
        let by_title = |title: &str| {
            seen.windows
                .iter()
                .rev()
                .find(|window| window.title == title)
                .map(|window| window.handle)
                .unwrap_or_else(|| panic!("no window titled {title} was announced"))
        };
        let first_handle = by_title("notes.txt");
        let second_handle = by_title("shell");

        // The drawn windows carry everything the task bar draws.
        let window = seen
            .window(first_handle)
            .expect("the first window's record is in the log");
        assert_eq!(window.app_id, "org.example.Editor");
        assert_eq!(window.title, "notes.txt");
        assert!(!window.minimized, "a new window is not put away");
        assert!(window.visible, "a new window is on the shown workspace");
        assert_eq!(window.workspace, 0, "a new window lands on the shown one");
        assert!(
            !window.output.is_empty(),
            "a mapped window on a live headless output reports its connector"
        );
        assert_eq!(window.mode, 0, "a new window is tiled");

        // Exactly one window has the keyboard, and it is one of the two.
        let focused = seen.focused();
        assert_eq!(focused.len(), 1, "exactly one window is focused");

        // The toplevel that never drew is absent from every announcement. It has no
        // handle at all, so the assertion is on its title: a task button for a
        // window that may never appear is exactly what the map gate prevents.
        assert!(
            seen.windows.iter().all(|window| window.title != "helper"),
            "a toplevel that never drew is not a window"
        );

        (first_handle, second_handle)
    };

    // ------------------------------------------------------------------ activate
    //
    // There is no success event: the confirmation is the state event showing the
    // new focus. Activating the *other* window must move `focused` off this one.
    leg.list.activate(first_handle);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the activated window to become focused", |client| {
        client
            .seen
            .window(first_handle)
            .is_some_and(|window| window.focused)
    });
    {
        let seen = leg.seen();
        assert_eq!(
            seen.focused(),
            vec![first_handle],
            "activation moves the focus, and only the confirmation says so"
        );
        assert!(
            seen.window(second_handle)
                .is_some_and(|window| !window.focused),
            "the previously focused window is told it is not focused"
        );
    }

    // ----------------------------------------------------------------- set_mode
    //
    // Every mode is confirmed the same way, so a task button learns the truth from
    // the compositor rather than from the request it sent.
    for (mode, expected) in [
        (WindowMode::Fullscreen, 3u32),
        (WindowMode::Maximized, 2),
        (WindowMode::Floating, 1),
        (WindowMode::Tiled, 0),
    ] {
        leg.list.set_mode(first_handle, mode);
        leg.connection.flush().expect("failed to flush the request");
        leg.wait_until("the mode change to be announced", |client| {
            client
                .seen
                .window(first_handle)
                .is_some_and(|window| window.mode == expected)
        });
        assert_eq!(
            leg.seen()
                .window(first_handle)
                .expect("the window is still managed")
                .mode,
            expected,
            "the compositor reports the mode it applied"
        );
    }

    // ---------------------------------------------------------------- minimize
    leg.list.minimize(first_handle);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the window to be put away", |client| {
        client
            .seen
            .window(first_handle)
            .is_some_and(|window| window.minimized)
    });
    {
        let window = leg
            .seen()
            .window(first_handle)
            .expect("a minimized window keeps its handle");
        assert!(window.minimized, "minimize is reported");
        assert!(!window.visible, "a minimized window is not composited");
        assert_ne!(
            leg.seen().focused(),
            vec![first_handle],
            "the keyboard does not stay on a window that is not on screen"
        );
    }

    // Activating it again brings it back, which is what a task click does.
    leg.list.activate(first_handle);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the window to come back", |client| {
        client
            .seen
            .window(first_handle)
            .is_some_and(|window| !window.minimized && window.focused)
    });

    // --------------------------------------------------------------- workspaces
    //
    // Switching is a visibility change, not a relayout: the windows of the
    // workspace being left report `visible = 0` and keep their state.
    leg.list.switch_workspace(1);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the workspace switch to be announced", |client| {
        client.seen.workspace == Some((1, WORKSPACES))
    });
    {
        let seen = leg.seen();
        assert_eq!(seen.workspace, Some((1, WORKSPACES)));
        for handle in [first_handle, second_handle] {
            let window = seen
                .window(handle)
                .expect("a window on another workspace is still managed");
            assert_eq!(
                window.workspace, 0,
                "switching does not move a window between workspaces"
            );
            assert!(
                !window.visible,
                "a window on a hidden workspace is invisible"
            );
        }
        assert!(
            seen.focused().is_empty(),
            "no window on a hidden workspace can hold the keyboard"
        );
    }

    // Back, and the windows are composited again.
    leg.list.switch_workspace(0);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the windows to be visible again", |client| {
        client.seen.workspace == Some((0, WORKSPACES))
            && client
                .seen
                .window(first_handle)
                .is_some_and(|window| window.visible)
    });

    // Moving a window to another workspace changes its workspace field and hides it.
    leg.list.move_to_workspace(second_handle, 2);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the workspace move to be announced", |client| {
        client
            .seen
            .window(second_handle)
            .is_some_and(|window| window.workspace == 2 && !window.visible)
    });

    // ------------------------------------------------------- negative outcomes
    //
    // A request that cannot be carried out reports why. A silent no-op would let a
    // task button stay disabled for ever with nothing to log.
    leg.list.activate(u32::MAX - 1);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the unknown-window refusal", |client| {
        client.seen.failures.contains(&(u32::MAX - 1, 0u32))
    });

    leg.list.switch_workspace(WORKSPACES);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the out-of-range workspace refusal", |client| {
        client.seen.failures.contains(&(NO_WINDOW_HANDLE, 1u32))
    });

    leg.list.move_to_workspace(first_handle, WORKSPACES);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the out-of-range move refusal", |client| {
        client.seen.failures.contains(&(first_handle, 1u32))
    });

    // --------------------------------------------------------------------- close
    //
    // `close` asks the application. The client in this test honours it by
    // destroying the toplevel, and *that* is what removes the window: the
    // confirmation is `window_removed`, not a success event.
    leg.list.close(second_handle);
    leg.connection.flush().expect("failed to flush the request");
    leg.wait_until("the closed window to be removed", |client| {
        client.seen.removed.contains(&second_handle)
    });
    {
        let seen = leg.seen();
        assert!(
            seen.removed.contains(&second_handle),
            "a close is confirmed by the window going away"
        );
        assert!(
            seen.window(second_handle).is_some(),
            "the last record for the removed window is still the state it was in"
        );
        assert!(
            seen.focused().len() <= 1,
            "the focus invariant survives a window going away"
        );
    }

    drop(leg);

    // A clean shutdown is part of the M1 contract this binary keeps; the test
    // reuses it so a crash in the new code path fails here rather than hanging.
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("failed to run kill");
    assert!(status.success(), "kill -TERM exited with {status}");

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("failed to poll inox-compositor") {
            assert!(
                status.success(),
                "the compositor must exit cleanly after SIGTERM, got {status}"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "inox-compositor did not exit within {STARTUP_TIMEOUT:?}"
        );
        sleep(Duration::from_millis(20));
    }
}
