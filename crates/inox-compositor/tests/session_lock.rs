//! Real clients exercise the secure session lock on the software-rendered Inox host.
//!
//! This is the compositor half of acceptance S02, and the E2 half of S04: what a
//! client may do with `ext_session_lock_manager_v1`, what the compositor refuses,
//! and what the screen and the seat look like while a lock is held.
//!
//! The test drives three real Wayland clients over in-process socket pairs against
//! a real `InoxState` on one thread, exactly as `tests/layer_shell.rs` does, and
//! asserts on the compositor's observable state and on read-back pixels rather
//! than on logs.
//!
//! Everything lives in one test function on purpose: a process may only ever hold
//! one EGL display (see the `SAFETY` note in `src/render.rs`), and test functions
//! inside one binary run in parallel threads.

use std::{
    collections::HashMap,
    io::{self, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::Arc,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, HeadlessOutput, InoxState};
use inox_protocols::{
    ipc::inox_ipc::client::inox_ipc as ipc,
    standard::{
        core::client::{
            wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
            wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
            wl_shm_pool, wl_surface,
        },
        ext::session_lock::client::{
            ext_session_lock_manager_v1 as manager, ext_session_lock_surface_v1 as lock_surface,
            ext_session_lock_v1 as lock,
        },
        xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
    },
};
use smithay::{
    backend::input::{ButtonState, KeyState},
    input::keyboard::Keycode,
    reexports::wayland_server::{Display, DisplayHandle},
    utils::{Logical, Point},
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

const WIDTH: i32 = 320;
const HEIGHT: i32 = 240;

/// The colour the ordinary client paints its window with.
const WINDOW: [u8; 4] = [220, 30, 20, 255];
/// The colour the lock client paints its lock surface with.
const LOCK_FRAME: [u8; 4] = [20, 210, 30, 255];
/// What the compositor clears to when nobody draws: the locked blank (ADR 0022).
const BLANK: [u8; 4] = [17, 34, 51, 255];

/// The token `inox-session` would have generated and delivered to this compositor.
const TOKEN: &str = "test-session-token";

const KEY_LEFT_META: u32 = 125 + 8;
const KEY_Q: u32 = 16 + 8;
const KEY_A: u32 = 30 + 8;

#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    lock_events: Vec<LockEvent>,
    surface_configures: Vec<(u32, u32, u32)>,
    keyboard_focus: Option<wl_surface::WlSurface>,
    keyboard_keys: usize,
    pointer_focus: Option<wl_surface::WlSurface>,
    /// How many times the compositor has offered this client the selection.
    data_offers: usize,
    device: Option<wl_data_device::WlDataDevice>,
    source: Option<wl_data_source::WlDataSource>,
}

/// What a client saw happen to its session lock object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockEvent {
    Locked,
    Finished,
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

impl Dispatch<lock::ExtSessionLockV1, usize> for Client {
    fn event(
        state: &mut Self,
        _: &lock::ExtSessionLockV1,
        event: lock::Event,
        _: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            lock::Event::Locked => state.lock_events.push(LockEvent::Locked),
            lock::Event::Finished => state.lock_events.push(LockEvent::Finished),
            _ => {}
        }
    }
}

impl Dispatch<lock_surface::ExtSessionLockSurfaceV1, usize> for Client {
    fn event(
        state: &mut Self,
        proxy: &lock_surface::ExtSessionLockSurfaceV1,
        event: lock_surface::Event,
        _: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let lock_surface::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            state.surface_configures.push((serial, width, height));
            proxy.ack_configure(serial);
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_device::Event::DataOffer { .. } = event {
            state.data_offers += 1;
        }
    }

    wayland_client::event_created_child!(Client, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
        wl_data_device::EVT_SELECTION_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
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

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => state.keyboard_focus = Some(surface),
            wl_keyboard::Event::Leave { .. } => state.keyboard_focus = None,
            wl_keyboard::Event::Key { .. } => state.keyboard_keys += 1,
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => state.pointer_focus = Some(surface),
            wl_pointer::Event::Leave { .. } => state.pointer_focus = None,
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        _: &mut Self,
        proxy: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            proxy.ack_configure(serial);
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(Client: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(Client: ignore manager::ExtSessionLockManagerV1);
wayland_client::delegate_noop!(Client: ignore wl_data_device_manager::WlDataDeviceManager);
wayland_client::delegate_noop!(Client: ignore wl_data_offer::WlDataOffer);
wayland_client::delegate_noop!(Client: ignore wl_data_source::WlDataSource);

struct Peer {
    connection: Connection,
    queue: EventQueue<Client>,
    state: Client,
    registry: wl_registry::WlRegistry,
}

impl Peer {
    fn connect(handle: &mut DisplayHandle) -> Self {
        let (server, client) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        handle
            .insert_client(server, Arc::new(ClientState::default()))
            .unwrap();
        let connection = Connection::from_socket(client).unwrap();
        let queue = connection.new_event_queue();
        let registry = connection.display().get_registry(&queue.handle(), ());
        Self {
            connection,
            queue,
            registry,
            state: Client::default(),
        }
    }

    fn pump(&mut self) {
        self.connection.flush().unwrap();
        if let Some(guard) = self.connection.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("client read failed: {error}"),
            }
        }
        self.queue.dispatch_pending(&mut self.state).unwrap();
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

    fn buffer(&self, width: i32, height: i32, rgba: [u8; 4]) -> wl_buffer::WlBuffer {
        let mut file = tempfile::tempfile().unwrap();
        let pixel = [rgba[2], rgba[1], rgba[0], rgba[3]];
        file.write_all(&pixel.repeat((width * height) as usize))
            .unwrap();
        let shm: wl_shm::WlShm = self.bind(1);
        let pool = shm.create_pool(file.as_fd(), width * height * 4, &self.queue.handle(), ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            width * 4,
            wl_shm::Format::Argb8888,
            &self.queue.handle(),
            (),
        );
        pool.destroy();
        buffer
    }

    /// Claim the trusted lock role over `inox_ipc`.
    fn claim(&self, token: &str) {
        let ipc: ipc::InoxIpc = self.bind(2);
        ipc.claim_lock_role(token.to_owned());
    }

    /// Take the selection, the way an application does when the user copies.
    ///
    /// Smithay only lets the *focused* client set a selection, which is what makes
    /// this the observable form of "a locked session grants the clipboard to
    /// nobody": the same call succeeds again once the lock is released.
    fn set_selection(&mut self) {
        let manager: wl_data_device_manager::WlDataDeviceManager = self.bind(3);
        let device = self
            .state
            .device
            .clone()
            .expect("bind_data_device must run before the client is focused");
        let source = manager.create_data_source(&self.queue.handle(), ());
        source.offer("text/plain;charset=utf-8".to_owned());
        device.set_selection(Some(&source), 0);
        self.state.source = Some(source);
    }

    /// Create this client's data device.
    ///
    /// It has to exist *before* the client is focused: the compositor only reports a
    /// selection change to the data devices that already exist, so a device created
    /// after the focus was granted would never see one.
    fn bind_data_device(&mut self) {
        let manager: wl_data_device_manager::WlDataDeviceManager = self.bind(3);
        let seat: wl_seat::WlSeat = self.bind(9);
        self.state.device = Some(manager.get_data_device(&seat, &self.queue.handle(), ()));
    }

    /// Ask for the session lock, returning the object the compositor answers on.
    fn request_lock(&self) -> lock::ExtSessionLockV1 {
        let manager: manager::ExtSessionLockManagerV1 = self.bind(1);

        manager.lock(&self.queue.handle(), 0)
    }

    /// Create a lock surface for the default output.
    fn create_lock_surface(&self, lock: &lock::ExtSessionLockV1) -> wl_surface::WlSurface {
        let compositor: wl_compositor::WlCompositor = self.bind(5);
        let surface = compositor.create_surface(&self.queue.handle(), ());
        let output: wl_output::WlOutput = self.bind(4);
        let role = lock.get_lock_surface(&surface, &output, &self.queue.handle(), 0);
        let _ = role;

        surface
    }

    /// Paint a lock surface with the size the compositor configured it to.
    fn paint_lock_surface(
        &self,
        surface: &wl_surface::WlSurface,
        configured: (u32, u32),
        rgba: [u8; 4],
    ) {
        let (width, height) = (configured.0 as i32, configured.1 as i32);
        let buffer = self.buffer(width, height, rgba);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, width, height);
        surface.commit();
    }

    /// One plain xdg toplevel, painted with `rgba`.
    fn window(&self, rgba: [u8; 4]) -> wl_surface::WlSurface {
        let compositor: wl_compositor::WlCompositor = self.bind(5);
        let xdg: xdg_wm_base::XdgWmBase = self.bind(6);
        let surface = compositor.create_surface(&self.queue.handle(), ());
        let xdg_surface = xdg.get_xdg_surface(&surface, &self.queue.handle(), ());
        let _toplevel = xdg_surface.get_toplevel(&self.queue.handle(), ());
        surface.commit();
        let buffer = self.buffer(WIDTH / 2, HEIGHT / 2, rgba);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, WIDTH / 2, HEIGHT / 2);
        surface.commit();

        surface
    }
}

fn step(display: &mut Display<InoxState>, state: &mut InoxState, peers: &mut [&mut Peer]) {
    for peer in peers.iter_mut() {
        peer.pump();
    }
    display.dispatch_clients(state).unwrap();
    display.flush_clients().unwrap();
}

fn until(
    what: &str,
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peers: &mut [&mut Peer],
    condition: impl Fn(&InoxState, &[&mut Peer]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        step(display, state, peers);
        if condition(state, peers) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Wayland scenario timed out: {what}"
        );
    }
    for _ in 0..3 {
        step(display, state, peers);
    }
}

/// Wait for the next lock-surface configure and return its `(serial, width, height)`.
fn await_configure(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peer: &mut Peer,
) -> (u32, u32, u32) {
    let before = peer.state.surface_configures.len();
    until(
        "a lock surface configure",
        display,
        state,
        &mut [peer],
        |_, peers| peers[0].state.surface_configures.len() > before,
    );

    *peer
        .state
        .surface_configures
        .last()
        .expect("a configure was just awaited")
}

/// The frame the compositor would put on the default output right now.
fn frame(state: &mut InoxState) -> Vec<u8> {
    state.render_frame().unwrap();
    state.read_frame_rgba().unwrap()
}

/// Render until the output shows what `accept` describes.
///
/// A client's buffer commit and the compositor's next frame are separate steps,
/// so a single render after a `step` is a race: under concurrent load the commit
/// can still be queued, and the assertion would then be about a frame the client
/// never asked for. Every wait here therefore has an explicit deadline and a
/// reason, and no fixed sleep.
fn settle_until(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peers: &mut [&mut Peer],
    what: &str,
    accept: impl Fn(&[u8]) -> bool,
) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let rendered = frame(state);
        if accept(&rendered) {
            return rendered;
        }
        assert!(
            Instant::now() < deadline,
            "the output never showed {what}: {rendered:?}"
        );
        step(display, state, peers);
    }
}

fn frame_contains(frame: &[u8], probe: [u8; 4]) -> bool {
    frame.as_chunks::<4>().0.iter().any(|pixel| {
        pixel
            .iter()
            .zip(probe)
            .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
    })
}

fn assert_frame_is(frame: &[u8], probe: [u8; 4], what: &str) {
    assert!(!frame.is_empty(), "the frame has no pixels at all: {what}");
    let mut counts: HashMap<[u8; 4], usize> = HashMap::new();
    for pixel in frame.as_chunks::<4>().0 {
        let key = [pixel[0], pixel[1], pixel[2], pixel[3]];
        *counts.entry(key).or_default() += 1;
    }
    let matching = counts.get(&probe).copied().unwrap_or(0);
    assert_eq!(
        matching,
        counts.values().sum::<usize>(),
        "expected every pixel to be {probe:?} ({what}); the frame is {counts:?}"
    );
}

#[test]
fn the_session_lock_covers_the_screen_refuses_strangers_and_survives_its_client() {
    let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
    let mut display = Display::<InoxState>::new().unwrap();
    let mut handle = display.handle();
    let config = CompositorConfig {
        outputs: vec![HeadlessOutput {
            width: WIDTH,
            height: HEIGHT,
            ..HeadlessOutput::default()
        }],
        lock_token: Some(TOKEN.to_owned()),
        ..Default::default()
    };
    let mut state = InoxState::new(&handle, &config, event_loop.get_signal()).unwrap();

    let mut app = Peer::connect(&mut handle);
    let mut keeper = Peer::connect(&mut handle);
    let mut stranger = Peer::connect(&mut handle);
    until(
        "step 1",
        &mut display,
        &mut state,
        &mut [&mut app, &mut keeper, &mut stranger],
        |_, peers| {
            peers
                .iter()
                .all(|peer| peer.has_global("ext_session_lock_manager_v1"))
        },
    );
    // The global is advertised to every client; the decision about who may *use*
    // it happens on the request (ADR 0022 decision 3).
    assert_eq!(app.state.globals["ext_session_lock_manager_v1"].1, 1);

    let seat: wl_seat::WlSeat = app.bind(9);
    let _keyboard = seat.get_keyboard(&app.queue.handle(), ());
    let _pointer = seat.get_pointer(&app.queue.handle(), ());
    // The lock client needs a keyboard of its own: the compositor can only hand
    // focus to a client that has one, and the whole point of the lock is that the
    // keys go there instead of to the desktop.
    let keeper_seat: wl_seat::WlSeat = keeper.bind(9);
    let _keeper_keyboard = keeper_seat.get_keyboard(&keeper.queue.handle(), ());
    let _keeper_pointer = keeper_seat.get_pointer(&keeper.queue.handle(), ());
    app.bind_data_device();
    for _ in 0..3 {
        step(&mut display, &mut state, &mut [&mut app]);
    }
    let _window = app.window(WINDOW);
    let window_surface = _window.clone();
    until(
        "step 2",
        &mut display,
        &mut state,
        &mut [&mut app],
        |state, peers| {
            state.window_count() == 1
                && peers[0].state.keyboard_focus.as_ref() == Some(&window_surface)
        },
    );
    let ordinary = settle_until(
        &mut display,
        &mut state,
        &mut [&mut app],
        "the ordinary window before the lock",
        |rendered| frame_contains(rendered, WINDOW),
    );
    assert!(
        frame_contains(&ordinary, WINDOW),
        "the ordinary window must be on screen before the lock, otherwise the rest of this test \
         proves nothing"
    );
    assert!(!state.session_locked(), "nothing has asked for a lock yet");

    // The focused client takes the selection, the way an application does when the
    // user copies something. This is the baseline the lock is measured against:
    // the same call is refused while the session is locked, and honoured again
    // after the unlock, so what the lock suppresses is a property of the lock
    // rather than of a client that cannot ask.
    app.set_selection();
    until(
        "the focused client is offered the selection",
        &mut display,
        &mut state,
        &mut [&mut app],
        |_, peers| peers[0].state.data_offers == 1,
    );

    // --- a client that has not claimed the role cannot lock -------------------
    let _refused = app.request_lock();
    until(
        "step 3",
        &mut display,
        &mut state,
        &mut [&mut app],
        |_, peers| !peers[0].state.lock_events.is_empty(),
    );
    assert_eq!(
        app.state.lock_events,
        vec![LockEvent::Finished],
        "an unauthorized lock request is refused with `finished`, not accepted"
    );
    assert!(!state.session_locked());
    assert_eq!(state.lock_refusals(), 1);

    // --- a claim with the wrong token does not authorize ----------------------
    stranger.claim("not-the-session-token");
    until(
        "step 4",
        &mut display,
        &mut state,
        &mut [&mut stranger],
        |state, _| state.lock_claim_refusals() == 1,
    );
    let refused = stranger.request_lock();
    until(
        "step 5",
        &mut display,
        &mut state,
        &mut [&mut stranger],
        |_, peers| !peers[0].state.lock_events.is_empty(),
    );
    assert_eq!(stranger.state.lock_events, vec![LockEvent::Finished]);
    assert_eq!(state.lock_refusals(), 2);
    assert_eq!(state.lock_claim_refusals(), 1);
    drop(refused);

    // --- the session's own lock client may lock -------------------------------
    keeper.claim(TOKEN);
    let held = keeper.request_lock();
    until(
        "step 6",
        &mut display,
        &mut state,
        &mut [&mut keeper],
        |state, _| state.session_locked(),
    );
    assert!(
        keeper.state.lock_events.is_empty(),
        "the client must not be told it owns the screen before a locked frame is presented"
    );

    // It creates its surface for the output while the lock is being established.
    let lock_frame = keeper.create_lock_surface(&held);
    let (_, width, height) = await_configure(&mut display, &mut state, &mut keeper);
    assert_eq!(
        (width, height),
        (WIDTH as u32, HEIGHT as u32),
        "a lock surface is configured to exactly its output's logical size"
    );
    keeper.paint_lock_surface(&lock_frame, (width, height), LOCK_FRAME);
    for _ in 0..3 {
        step(&mut display, &mut state, &mut [&mut keeper]);
    }
    assert!(
        keeper.state.lock_events.is_empty(),
        "still not locked: the compositor has not presented a locked frame"
    );

    let locked_frame = settle_until(
        &mut display,
        &mut state,
        &mut [&mut app, &mut keeper],
        "the lock surface",
        |rendered| frame_contains(rendered, LOCK_FRAME),
    );
    assert_frame_is(
        &locked_frame,
        LOCK_FRAME,
        "the locked frame is the lock surface and nothing else",
    );
    assert!(
        !frame_contains(&locked_frame, WINDOW),
        "the ordinary client's pixels must be gone from a locked screen"
    );
    until(
        "step 7",
        &mut display,
        &mut state,
        &mut [&mut keeper],
        |_, peers| !peers[0].state.lock_events.is_empty(),
    );
    assert_eq!(
        keeper.state.lock_events,
        vec![LockEvent::Locked],
        "`locked` arrives once the covered frame has been presented"
    );
    assert!(state.session_locked());

    // The client is still connected and held its selection when the session
    // locked; a locked session offers the clipboard to nobody, because the data
    // device focus is cleared while the lock is held.
    app.set_selection();
    for _ in 0..3 {
        step(&mut display, &mut state, &mut [&mut app, &mut keeper]);
    }
    assert_eq!(
        app.state.data_offers, 1,
        "a locked session must not grant the selection to an ordinary client"
    );

    // --- input reaches the lock surface and no ordinary client ---------------
    until(
        "step 8",
        &mut display,
        &mut state,
        &mut [&mut app, &mut keeper],
        |state, peers| {
            state.keyboard_focus().is_some()
                && peers[1].state.keyboard_focus.is_some()
                && peers[0].state.keyboard_focus.is_none()
        },
    );
    assert_eq!(
        keeper.state.keyboard_focus.as_ref(),
        Some(&lock_frame),
        "the lock surface holds the keyboard"
    );
    assert_eq!(
        app.state.keyboard_focus, None,
        "the ordinary client lost the keyboard when the session locked"
    );

    // The compositor's own chords are not intercepted while locked: `Super+Q`
    // would otherwise close the focused window behind a locked screen.
    state.inject_key(Keycode::new(KEY_LEFT_META), KeyState::Pressed);
    assert_eq!(
        state.inject_key(Keycode::new(KEY_Q), KeyState::Pressed),
        None,
        "no compositor action may fire while the session is locked"
    );
    state.inject_key(Keycode::new(KEY_Q), KeyState::Released);
    state.inject_key(Keycode::new(KEY_LEFT_META), KeyState::Released);
    let plain_before = app.state.keyboard_keys;
    state.inject_key(Keycode::new(KEY_A), KeyState::Pressed);
    state.inject_key(Keycode::new(KEY_A), KeyState::Released);
    for _ in 0..3 {
        step(&mut display, &mut state, &mut [&mut app, &mut keeper]);
    }
    assert_eq!(
        app.state.keyboard_keys, plain_before,
        "keys typed on a locked screen must not reach an ordinary client"
    );
    assert!(
        keeper.state.keyboard_keys > 0,
        "keys typed on a locked screen go to the lock surface"
    );

    // Pointer events are hit-tested against lock surfaces only.
    state.inject_pointer_motion(Point::<f64, Logical>::from((40.0, 40.0)));
    state.inject_pointer_button(0x110, ButtonState::Pressed);
    state.inject_pointer_button(0x110, ButtonState::Released);
    for _ in 0..3 {
        step(&mut display, &mut state, &mut [&mut app, &mut keeper]);
    }
    assert_eq!(
        app.state.pointer_focus, None,
        "an ordinary client must not be reached by the pointer while the session is locked"
    );

    // --- the lock's own client may release it --------------------------------
    held.unlock_and_destroy();
    until(
        "step 10",
        &mut display,
        &mut state,
        &mut [&mut keeper],
        |state, _| !state.session_locked(),
    );
    let restored = settle_until(
        &mut display,
        &mut state,
        &mut [&mut app, &mut keeper],
        "the restored desktop",
        |rendered| frame_contains(rendered, WINDOW),
    );
    assert!(
        frame_contains(&restored, WINDOW),
        "the ordinary scene comes back when the lock is released"
    );

    // The same call the lock refused above is honoured again, which is what makes
    // the refusal a property of the lock rather than of a broken client.
    app.set_selection();
    until(
        "the selection is granted again after the unlock",
        &mut display,
        &mut state,
        &mut [&mut app, &mut keeper],
        |_, peers| peers[0].state.data_offers == 2,
    );

    // --- locking again, then losing the UI ----------------------------------
    keeper.state.lock_events.clear();
    let held = keeper.request_lock();
    until(
        "step 11",
        &mut display,
        &mut state,
        &mut [&mut keeper],
        |state, _| state.session_locked(),
    );
    let second_frame = keeper.create_lock_surface(&held);
    let (_, width, height) = await_configure(&mut display, &mut state, &mut keeper);
    keeper.paint_lock_surface(&second_frame, (width, height), LOCK_FRAME);
    for _ in 0..3 {
        step(&mut display, &mut state, &mut [&mut keeper]);
    }
    let second = settle_until(
        &mut display,
        &mut state,
        &mut [&mut app, &mut keeper],
        "the second lock surface",
        |rendered| frame_contains(rendered, LOCK_FRAME),
    );
    assert_frame_is(
        &second,
        LOCK_FRAME,
        "the second lock covers the screen like the first",
    );

    // The lock UI dies. The session must stay locked, and this compositor does not
    // hand a held lock to another client (ADR 0022 decision 2).
    drop(keeper);
    for _ in 0..5 {
        step(&mut display, &mut state, &mut [&mut app, &mut stranger]);
    }
    assert!(
        state.session_locked(),
        "a lock UI that dies must not unlock the session"
    );
    let orphaned = settle_until(
        &mut display,
        &mut state,
        &mut [&mut app, &mut stranger],
        "the locked blank after the UI died",
        |rendered| !frame_contains(rendered, LOCK_FRAME) && !frame_contains(rendered, WINDOW),
    );
    assert_frame_is(
        &orphaned,
        BLANK,
        "with no live lock surface the output falls back to the opaque blank",
    );
    assert!(!frame_contains(&orphaned, LOCK_FRAME));
    assert!(
        !frame_contains(&orphaned, WINDOW),
        "the ordinary client must not reappear just because the lock UI died"
    );

    // This client has claimed the role with the session's token, so what refuses
    // it is the held lock itself and not a missing capability.
    stranger.claim(TOKEN);
    stranger.state.lock_events.clear();
    let takeover = stranger.request_lock();
    until(
        "step 12",
        &mut display,
        &mut state,
        &mut [&mut stranger],
        |_, peers| !peers[0].state.lock_events.is_empty(),
    );
    assert_eq!(
        stranger.state.lock_events,
        vec![LockEvent::Finished],
        "an orphaned lock is not handed over: the session stays locked"
    );
    assert!(state.session_locked());
    drop(takeover);

    // --- a stranger can neither take a held lock over nor end one -------------
    // `unlock_and_destroy` on an object that was never locked is the protocol's
    // `invalid_unlock`. Smithay's own arm posts that error and then unlocks
    // anyway; this compositor refuses the request before it reaches that arm, so
    // the session stays locked and the offending connection is the one that goes
    // away — not the lock.
    stranger.state.lock_events.clear();
    let orphan = stranger.request_lock();
    until(
        "the stranger's lock request is refused",
        &mut display,
        &mut state,
        &mut [&mut stranger],
        |_, peers| !peers[0].state.lock_events.is_empty(),
    );
    assert_eq!(
        stranger.state.lock_events,
        vec![LockEvent::Finished],
        "the held lock is not handed to another client"
    );
    assert!(state.session_locked());
    orphan.unlock_and_destroy();
    let error = protocol_error(&mut display, &mut state, &mut stranger);
    // `unlock_and_destroy` is a destructor: the client removes the object as it
    // sends the request, so by the time the server's error names that object the
    // client can no longer resolve its interface (wayland-backend reports an empty
    // one). The code is what identifies the refusal, and the interface is asserted
    // where the object is still known — see the tests in `inox-lock`.
    assert_eq!(
        error.code,
        u32::from(lock::Error::InvalidUnlock),
        "expected the protocol's invalid_unlock refusal, got {error:?}"
    );
    assert!(
        state.session_locked(),
        "an unlock from a connection that does not hold the lock must not unlock the session"
    );
    let still = settle_until(
        &mut display,
        &mut state,
        &mut [&mut app, &mut stranger],
        "the session staying covered",
        |rendered| !frame_contains(rendered, WINDOW),
    );
    assert_frame_is(
        &still,
        BLANK,
        "the session is still locked and the output is still covered",
    );
}

/// Pump until the client's connection is closed with a protocol error.
fn protocol_error(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peer: &mut Peer,
) -> wayland_client::backend::protocol::ProtocolError {
    peer.connection.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        display.dispatch_clients(state).unwrap();
        display.flush_clients().unwrap();
        if let Some(guard) = peer.connection.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
        }
        if peer.queue.dispatch_pending(&mut peer.state).is_err() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the invalid client was not rejected"
        );
    }

    peer.connection
        .protocol_error()
        .expect("expected a protocol error")
}
