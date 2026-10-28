//! The locked-session half of `inox-window-manager-v1`'s permission model.
//!
//! ADR 0045 §3 fixes one policy that the process-level test
//! (`tests/window_manager_end_to_end.rs`) cannot reach without the whole lock
//! flow: **while a session lock is held, every request that would change what an
//! ordinary window shows is refused with `not_permitted`**. Bringing a window to
//! the front behind a lock screen would hand the keyboard and the pixels to a
//! client the user is not looking at, which is exactly what a lock exists to
//! prevent.
//!
//! This test drives a real `InoxState` in this process, over real
//! `UnixStream::pair()` connections, with two real clients: an observer of the
//! window-management protocol that owns one mapped window, and a lock holder that
//! claims the trusted role over `inox_ipc` and takes the lock. It asserts:
//!
//! - the lock is really accepted (`InoxState::session_locked`), and the holder
//!   really reaches the protocol's `locked` state after a locked frame is
//!   presented;
//! - all six verbs (`activate`, `minimize`, `close`, `set_mode`,
//!   `move_to_workspace`, `switch_workspace`) answer `request_failed` with
//!   `not_permitted`, carrying the handle the request named;
//! - the refusals are refusals, not silent no-ops: `close` never reaches the
//!   client's `xdg_toplevel`, and no window state changed while locked;
//! - unlocking restores the same verbs, so the refusal is scoped to the lock
//!   rather than to a permanently broken path.
//!
//! Everything lives in one `#[test]` on purpose: a test binary may hold only one
//! EGL display (see the `SAFETY` note in `src/render.rs`), and test functions in
//! one binary run in parallel threads.

use std::{
    collections::HashMap,
    io,
    os::{fd::AsFd, unix::net::UnixStream},
    sync::Arc,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, InoxState};
use inox_protocols::{
    ipc::inox_ipc::client::inox_ipc as ipc,
    standard::{
        core::client::{
            wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
        },
        ext::session_lock::client::{ext_session_lock_manager_v1 as lock_manager, ext_session_lock_v1 as lock},
        xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
    },
    window_manager::inox_window_manager::client::{
        inox_window_list_v1::{self as list, Event as ListEvent},
        inox_window_manager_v1::InoxWindowManagerV1,
    },
};
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

/// The token `inox-session` would have generated and handed to the lock client.
const TOKEN: &str = "test-session-token";

/// Edge length of the `wl_shm` buffer the observer's window draws.
const BUFFER_SIZE: i32 = 64;

/// How long any single wait may take before the test declares the compositor broken.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The `request_failed` reason for a refusal by session policy.
const NOT_PERMITTED: u32 = 1;

/// The handle the protocol reserves for a request that names no window.
const NO_WINDOW_HANDLE: u32 = u32::MAX;

/// A real client connection, with the dispatch state the events land in.
struct Peer {
    connection: Connection,
    queue: EventQueue<Client>,
    state: Client,
    registry: wl_registry::WlRegistry,
}

/// What one client's event streams carried.
#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    /// Every event's name, in arrival order, so the snapshot's contract order can
    /// be asserted on a desktop that actually has a window on it.
    order: Vec<&'static str>,
    /// Every `window` record the observer received, in arrival order.
    windows: Vec<Record>,
    /// Every `request_failed` the observer received.
    failures: Vec<(u32, u32)>,
    /// How many `snapshot_done` events arrived.
    snapshots: usize,
    /// The window records of the most recent snapshot, captured at its
    /// `snapshot_done`. Its length is the number of windows the snapshot held.
    snapshot_records: Vec<Record>,
    /// The number of `window` events the most recent snapshot delivered, so a
    /// second list's snapshot can be measured on its own rather than against the
    /// cumulative event log.
    snapshot_windows: usize,
    /// `xdg_toplevel.close` requests that reached this client.
    closes: usize,
    /// The lock object's events, in arrival order.
    lock_events: Vec<bool>,
    /// The `wl_display.sync` callbacks seen, which orders this test without sleeps.
    callbacks: usize,
}

/// One `window` event, decoded exactly as the protocol documents it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    handle: u32,
    mode: u32,
    minimized: bool,
    visible: bool,
    focused: bool,
}

impl Client {
    /// The newest record for `handle`, which is the complete state as the client
    /// currently believes it.
    fn window(&self, handle: u32) -> Option<&Record> {
        self.windows.iter().rev().find(|record| record.handle == handle)
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                state.globals.insert(interface, (name, version));
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.globals.retain(|_, (id, _)| *id != name);
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        _: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Client {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Counted rather than acted on: the point of the locked assertions below is
        // that a refused `close` never reaches this arm at all.
        if let xdg_toplevel::Event::Close = event {
            state.closes += 1;
        }
    }
}

impl Dispatch<InoxWindowManagerV1, ()> for Client {
    fn event(
        _: &mut Self,
        _: &InoxWindowManagerV1,
        _: <InoxWindowManagerV1 as Proxy>::Event,
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
        _: &list::InoxWindowListV1,
        event: ListEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ListEvent::SessionGeneration { .. } => state.order.push("session_generation"),
            ListEvent::WorkspaceState { .. } => state.order.push("workspace_state"),
            ListEvent::Window {
                window,
                mode,
                minimized,
                visible,
                focused,
                ..
            } => {
                state.order.push("window");
                state.windows.push(Record {
                    handle: window,
                    // An unknown discriminant is refused rather than rendered as a
                    // wrong number, the same rule the panel applies.
                    mode: mode.into_result().expect("a known window mode") as u32,
                    minimized: minimized != 0,
                    visible: visible != 0,
                    focused: focused != 0,
                });
            }
            ListEvent::RequestFailed { window, reason } => {
                state.order.push("request_failed");
                state.failures.push((
                    window,
                    reason.into_result().expect("a known request error") as u32,
                ));
            }
            ListEvent::SnapshotDone => {
                state.order.push("snapshot_done");
                state.snapshots += 1;
                // The snapshot's own records are the ones since the previous
                // `snapshot_done`; a second list must be measured on that batch, not
                // on the accumulated log.
                state.snapshot_records = state.windows[state.snapshot_windows..].to_vec();
                state.snapshot_windows = state.windows.len();
            }
            _ => {}
        }
    }
}

impl Dispatch<lock::ExtSessionLockV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &lock::ExtSessionLockV1,
        event: lock::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            lock::Event::Locked => state.lock_events.push(true),
            lock::Event::Finished => state.lock_events.push(false),
            _ => {}
        }
    }
}

impl Dispatch<ipc::InoxIpc, ()> for Client {
    fn event(
        _: &mut Self,
        _: &ipc::InoxIpc,
        _: ipc::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// `wl_display.sync` is the test's clock: a callback proves the compositor
/// processed everything sent before it, so no assertion needs a sleep.
impl Dispatch<wayland_client::protocol::wl_callback::WlCallback, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wayland_client::protocol::wl_callback::WlCallback,
        _: wayland_client::protocol::wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.callbacks += 1;
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore lock_manager::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);

/// One toplevel the observer created, with the objects that keep it alive.
struct Toplevel {
    _surface: wl_surface::WlSurface,
    _xdg_surface: xdg_surface::XdgSurface,
    _toplevel: xdg_toplevel::XdgToplevel,
    _buffer: wl_buffer::WlBuffer,
    _pool: wl_shm_pool::WlShmPool,
}

impl Peer {
    /// Register a new client on the display and open its connection.
    fn connect(handle: &mut DisplayHandle) -> Self {
        let (server, client) = UnixStream::pair().expect("a socket pair");
        client.set_nonblocking(true).expect("a non-blocking client");
        handle
            .insert_client(server, Arc::new(ClientState::default()))
            .expect("the client is registered");
        let connection = Connection::from_socket(client).expect("a client connection");
        let queue = connection.new_event_queue();
        let registry = connection.display().get_registry(&queue.handle(), ());

        Self {
            connection,
            queue,
            state: Client::default(),
            registry,
        }
    }

    /// Move everything in both directions once.
    fn pump(&mut self) {
        self.connection.flush().expect("the client flush");
        if let Some(guard) = self.connection.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("the client read failed: {error}"),
            }
        }
        self.queue
            .dispatch_pending(&mut self.state)
            .expect("the client dispatch");
    }

    fn bind<I>(&self, version: u32) -> I
    where
        I: Proxy + 'static,
        Client: Dispatch<I, ()>,
    {
        let (name, advertised) = self.state.globals[I::interface().name];
        self.registry
            .bind(name, advertised.min(version), &self.queue.handle(), ())
    }

    fn has_global(&self, name: &str) -> bool {
        self.state.globals.contains_key(name)
    }

    /// Claim the trusted lock role over `inox_ipc`, with `TOKEN`.
    fn claim_lock_role(&self) {
        let ipc: ipc::InoxIpc = self.bind(2);
        ipc.claim_lock_role(TOKEN.to_owned());
    }

    /// Ask for the session lock, returning the object the compositor answers on.
    fn request_lock(&self) -> lock::ExtSessionLockV1 {
        let manager: lock_manager::ExtSessionLockManagerV1 = self.bind(1);
        manager.lock(&self.queue.handle(), ())
    }

    /// Create the observer's list and return it.
    fn window_list(&self) -> list::InoxWindowListV1 {
        let manager: InoxWindowManagerV1 = self.bind(1);
        manager.get_window_list(&self.queue.handle(), ())
    }

    /// One plain xdg toplevel that draws a real buffer, so it becomes a window.
    fn drawable_toplevel(&self) -> Toplevel {
        let compositor: wl_compositor::WlCompositor = self.bind(5);
        let xdg: xdg_wm_base::XdgWmBase = self.bind(6);
        let shm: wl_shm::WlShm = self.bind(1);

        let surface = compositor.create_surface(&self.queue.handle(), ());
        let xdg_surface = xdg.get_xdg_surface(&surface, &self.queue.handle(), ());
        let toplevel = xdg_surface.get_toplevel(&self.queue.handle(), ());
        toplevel.set_app_id("org.example.Locked".to_owned());
        toplevel.set_title("behind a lock".to_owned());
        surface.commit();

        let stride = BUFFER_SIZE * 4;
        let file = tempfile::tempfile().expect("the buffer file");
        file.set_len((stride * BUFFER_SIZE) as u64)
            .expect("the buffer file size");
        let pool = shm.create_pool(
            file.as_fd(),
            stride * BUFFER_SIZE,
            &self.queue.handle(),
            (),
        );
        let buffer = pool.create_buffer(
            0,
            BUFFER_SIZE,
            BUFFER_SIZE,
            stride,
            wl_shm::Format::Argb8888,
            &self.queue.handle(),
            (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();

        Toplevel {
            _surface: surface,
            _xdg_surface: xdg_surface,
            _toplevel: toplevel,
            _buffer: buffer,
            _pool: pool,
        }
    }
}

/// Service every peer once and then the display.
fn step(display: &mut Display<InoxState>, state: &mut InoxState, peers: &mut [&mut Peer]) {
    for peer in peers.iter_mut() {
        peer.pump();
    }
    display.dispatch_clients(state).expect("the display dispatch");
    display.flush_clients().expect("the display flush");
}

/// Run the display until `condition` holds, with a deadline and a reason.
fn until(
    what: &str,
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peers: &mut [&mut Peer],
    condition: impl Fn(&InoxState, &[&mut Peer]) -> bool,
) {
    let deadline = Instant::now() + TIMEOUT;

    loop {
        step(display, state, peers);
        if condition(state, peers) {
            break;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
    }
    // A few extra steps so the events the condition implies have been dispatched.
    for _ in 0..4 {
        step(display, state, peers);
    }
}

/// Round-trip one peer through `wl_display.sync`, so everything it sent has been
/// processed by the compositor by the time this returns.
fn round_trip(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer) {
    let before = peer.state.callbacks;
    peer.connection.display().sync(&peer.queue.handle(), ());
    until(
        "a display round trip",
        display,
        state,
        &mut [peer],
        |_, peers| peers[0].state.callbacks > before,
    );
}

#[test]
fn a_locked_session_refuses_every_window_management_verb() {
    let event_loop: EventLoop<'static, ()> =
        EventLoop::try_new().expect("the event loop");
    let mut display: Display<InoxState> = Display::new().expect("the wayland display");
    let mut display_handle = display.handle();

    // Two workspaces so `switch_workspace(0)` is a workspace that really exists:
    // the refusal below must be the lock's doing, not an out-of-range rejection.
    // The token is what lets the lock holder claim the trusted role.
    let config = CompositorConfig {
        lock_token: Some(TOKEN.to_owned()),
        workspace_count: 2,
        ..CompositorConfig::default()
    };
    let mut state = InoxState::new(&display_handle, &config, event_loop.get_signal())
        .expect("the compositor state");

    // The observer owns the window; the locker takes the session lock, because the
    // claim and the lock have to come from the same connection.
    let mut observer = Peer::connect(&mut display_handle);
    let mut locker = Peer::connect(&mut display_handle);

    until(
        "the clients see their globals",
        &mut display,
        &mut state,
        &mut [&mut observer, &mut locker],
        |_, peers| {
            peers[0].has_global("inox_window_manager_v1")
                && peers[0].has_global("xdg_wm_base")
                && peers[0].has_global("wl_shm")
                && peers[1].has_global("ext_session_lock_manager_v1")
                && peers[1].has_global("inox_ipc")
        },
    );

    // --- the observer's window is mapped and announced ------------------------
    let _toplevel = observer.drawable_toplevel();
    // One list serves both the snapshot and the refusals: `request_failed` goes to
    // the list that asked, which is the protocol's way to let a task button match a
    // failure to the request it has in flight.
    let list = observer.window_list();

    until(
        "the window to be announced to the observer",
        &mut display,
        &mut state,
        &mut [&mut observer],
        |state, peers| state.window_count() == 1 && !peers[0].state.windows.is_empty(),
    );
    round_trip(&mut display, &mut state, &mut observer);

    let handle = {
        // The snapshot of a desktop that *does* have a window still arrives in the
        // contract's order: the generation, then the workspace facts, then one
        // `window` per managed window, then `snapshot_done`. The process-level test
        // asserts this for an empty desktop; the non-empty case is the one where a
        // handle actually exists, so it is the one where the order matters most.
        assert_eq!(
            observer.state.order,
            vec![
                "session_generation",
                "workspace_state",
                "window",
                "snapshot_done"
            ],
            "a one-window snapshot keeps the contract's order"
        );

        let records = &observer.state.snapshot_records;
        assert_eq!(
            records.len(),
            1,
            "the snapshot holds the one mapped window: {records:?}"
        );
        let record = &records[0];
        assert!(record.visible, "the new window is on the shown workspace");
        assert!(record.focused, "the only window has the keyboard");
        assert_eq!(record.mode, 0, "a new window is tiled");
        record.handle
    };

    // --- a second list is served its own fresh snapshot -----------------------
    //
    // The XML promises that a client may create more than one list and that "each
    // one is served independently": a client that drops a list and creates another
    // gets a fresh snapshot, which is how a reconnect rebuilds its view without
    // having to track what it missed. Driving a second list is what proves the
    // promise, and it also proves the service keeps working with more than one
    // observer.
    let _second_list = observer.window_list();
    until(
        "the second list's snapshot",
        &mut display,
        &mut state,
        &mut [&mut observer],
        |_, peers| peers[0].state.snapshots >= 2,
    );
    round_trip(&mut display, &mut state, &mut observer);
    {
        // The second list is independently complete, not a delta against the first:
        // generation, workspace facts, the same window, `snapshot_done`.
        assert!(
            observer.state.snapshots >= 2,
            "a second list receives its own snapshot: {} arrived",
            observer.state.snapshots
        );
        let records = &observer.state.snapshot_records;
        assert_eq!(
            records.len(),
            1,
            "the second list's snapshot carries the same window: {records:?}"
        );
        assert_eq!(
            records[0].handle, handle,
            "both lists describe the same window with the same handle"
        );
    }
    // The second list asked for nothing, so it must not have been told about a
    // failure: `request_failed` goes to the requesting list only.
    assert!(
        observer.state.failures.is_empty(),
        "no request has been refused yet: {:?}",
        observer.state.failures
    );

    // --- the session is locked, and the holder is really told so --------------
    locker.claim_lock_role();
    let held = locker.request_lock();
    until(
        "the session lock to be accepted",
        &mut display,
        &mut state,
        &mut [&mut locker],
        |state, _| state.session_locked(),
    );

    // One frame makes the locked scene present on every output, which is what the
    // protocol's `locked` confirmation waits for. Rendering is also what proves the
    // refusal below is about a *covering* lock, not a half-taken one.
    state.render_frame().expect("a locked frame");
    until(
        "the lock holder to be told it owns the screen",
        &mut display,
        &mut state,
        &mut [&mut locker],
        |_, peers| !peers[0].state.lock_events.is_empty(),
    );
    assert_eq!(
        locker.state.lock_events,
        vec![true],
        "`locked` arrives once the covering frame has been presented"
    );

    // Taking the lock moves the keyboard to the lock surface, so the observer is due
    // one `window` event with `focused = 0` for its window. Settle that before the
    // baseline below is captured, or the assertion "no state changed while locked"
    // would compare against a log that was still growing for an unrelated reason.
    until(
        "the observer to see its window lose the keyboard to the lock",
        &mut display,
        &mut state,
        &mut [&mut observer],
        |_, peers| {
            peers[0]
                .state
                .window(handle)
                .is_some_and(|record| !record.focused)
        },
    );

    // --- every verb is refused, with the handle it named ----------------------
    //
    // These are the requests that would change what an ordinary window shows. The
    // confirmation of a refusal is `request_failed`; there is no window event to
    // read back because nothing was carried out.
    let baseline = observer.state.windows.clone();
    let baseline_closes = observer.state.closes;

    list.activate(handle);
    list.minimize(handle);
    list.close(handle);
    list.set_mode(handle, list::WindowMode::Fullscreen);
    list.move_to_workspace(handle, 1);
    list.switch_workspace(0);

    // Five of the six name the window; `switch_workspace` names the reserved
    // "no window" handle, because a workspace switch has none.
    until(
        "all six refusals to arrive",
        &mut display,
        &mut state,
        &mut [&mut observer],
        |_, peers| peers[0].state.failures.len() >= 6,
    );

    {
        let failures = &observer.state.failures;
        for expected in [
            (handle, NOT_PERMITTED),
            (handle, NOT_PERMITTED),
            (handle, NOT_PERMITTED),
            (handle, NOT_PERMITTED),
            (handle, NOT_PERMITTED),
            (NO_WINDOW_HANDLE, NOT_PERMITTED),
        ] {
            assert!(
                failures.contains(&expected),
                "the refusal {expected:?} is missing from {failures:?}"
            );
        }
        assert!(
            failures.iter().all(|(_, reason)| *reason == NOT_PERMITTED),
            "a locked session refuses with `not_permitted`, never another reason: {failures:?}"
        );
    }

    // --- the refusals really refused nothing ---------------------------------
    //
    // A silent no-op would leave exactly the same state behind, which is why the
    // protocol requires the negative event. These assertions pin the other half:
    // the requests did not take effect behind the lock either.
    assert_eq!(
        observer.state.closes, baseline_closes,
        "a refused `close` must not reach the client's xdg_toplevel"
    );
    assert_eq!(
        observer.state.windows, baseline,
        "no window state may change while the session is locked"
    );
    assert_eq!(
        state.window_count(),
        1,
        "windows stay managed while the session is locked"
    );
    assert!(
        state.session_locked(),
        "the lock still holds after every refused request"
    );

    // --- unlocking restores the verbs ---------------------------------------
    //
    // This is what makes the refusal a policy that is scoped to the lock rather
    // than a broken path: the same `activate` succeeds afterwards, and its
    // confirmation is the window event carrying the new focus.
    held.unlock_and_destroy();
    until(
        "the session lock to be released",
        &mut display,
        &mut state,
        &mut [&mut locker],
        |state, _| !state.session_locked(),
    );

    // The observer needs a list of its own that is still alive; the first one is.
    // (Both lists receive the same events; reusing either is equivalent.)
    list.activate(handle);
    until(
        "the activation to be confirmed by a window event",
        &mut display,
        &mut state,
        &mut [&mut observer],
        |_, peers| {
            peers[0]
                .state
                .window(handle)
                .is_some_and(|record| record.focused)
        },
    );
    assert!(
        observer
            .state
            .window(handle)
            .is_some_and(|record| record.focused && record.visible && !record.minimized),
        "after unlocking, the window is focused, visible and not minimized again"
    );

    // Dropping the peers sends their object teardown; the state must survive it.
    drop(observer);
    drop(locker);
    for _ in 0..4 {
        let _ = display.dispatch_clients(&mut state);
        let _ = display.flush_clients();
    }
}
