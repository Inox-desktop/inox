//! Real clients exercise the secure session lock on the software-rendered Inox host.
//!
//! This is the compositor half of acceptance S02, and the E2 half of S04: what a
//! client may do with `ext_session_lock_manager_v1`, what the compositor refuses,
//! and what the screen and the seat look like while a lock is held.
//!
//! The test drives five real Wayland clients over in-process socket pairs against
//! a real `InoxState` on one thread, exactly as `tests/layer_shell.rs` does, and
//! asserts on the compositor's observable state and on read-back pixels rather
//! than on logs.
//!
//! The first scenario is the compositor half of the glass protocol's acceptance table: a
//! background layer with a sharp black/white edge and a translucent top-layer
//! "pane" whose client holds a real `inox_glass_v1` object has its backdrop
//! frosted, the session is then locked, and the frame is read back. The frost is
//! live in the unlocked frame and cannot appear in the locked one, a glass object
//! created *after* `locked` is told `capabilities(max_radius = 0)`, and the lock
//! survives every glass request. That table listed this case as 待补, pinned only
//! by the pure `glass::collect_panes(locked = true, ..)` unit test and the early
//! return in `scene_elements`; this is the end-to-end evidence with pixels.
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
use inox_compositor::{
    ClientState, CompositorConfig, HeadlessOutput, InoxState, glass::MAX_BLUR_RADIUS,
};
use inox_protocols::{
    glass::inox_glass::client::{inox_glass_manager_v1 as glass_manager, inox_glass_v1 as glass},
    ipc::inox_ipc::client::inox_ipc as ipc,
    standard::{
        core::client::{
            wl_buffer, wl_callback, wl_compositor, wl_data_device, wl_data_device_manager,
            wl_data_offer, wl_data_source, wl_keyboard, wl_output, wl_pointer, wl_registry,
            wl_seat, wl_shm, wl_shm_pool, wl_surface,
        },
        ext::session_lock::client::{
            ext_session_lock_manager_v1 as manager, ext_session_lock_surface_v1 as lock_surface,
            ext_session_lock_v1 as lock,
        },
        wlr::layer_shell::client::{zwlr_layer_shell_v1 as shell, zwlr_layer_surface_v1 as layer},
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
/// What the compositor clears to when nobody draws: the locked blank.
const BLANK: [u8; 4] = [17, 34, 51, 255];

/// The token `inox-session` would have generated and delivered to this compositor.
const TOKEN: &str = "test-session-token";

const KEY_LEFT_META: u32 = 125 + 8;
const KEY_Q: u32 = 16 + 8;
const KEY_A: u32 = 30 + 8;

// The frosted-glass scene, the same geometry `tests/glass_blur.rs` measures: a
// black-to-white edge at `GLASS_EDGE_X` behind a translucent 120x80 pane that
// covers it, with a 24-column blur.
/// Where the background layer's black-to-white edge sits.
const GLASS_EDGE_X: i32 = 60;
/// Width of the frosted pane, in logical pixels.
const PANE_WIDTH: i32 = 120;
/// Height of the frosted pane, in logical pixels.
const PANE_HEIGHT: i32 = 80;
/// A row inside the pane; the pane spans y in `0..PANE_HEIGHT`.
const GLASS_ROW_INSIDE: usize = 40;
/// A row well below the pane, where the backdrop must still be sharp.
const GLASS_ROW_OUTSIDE: usize = 200;
/// The requested blur radius, in logical pixels (and physical: scale is 1).
const GLASS_RADIUS: u32 = 24;
/// The pane's own pixels: premultiplied grey with ~38 % alpha (`Argb8888` is
/// premultiplied), so the frosted backdrop dominates but the pane is genuinely
/// translucent rather than invisible.
const PANE_RGBA: [u8; 4] = [48, 48, 48, 96];
/// The two colours of the structured backdrop.
const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
/// A sharp two-colour edge may straddle at most one texel; two is the tolerance
/// the existing pixel tests use for driver rounding.
const MAX_INTERMEDIATE_OUTSIDE_PANE: usize = 2;

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
    /// Layer-surface configures, keyed by the user data the surface was created
    /// with (the `tests/glass_blur.rs` shape, which a separate binary cannot
    /// share).
    layer_configures: HashMap<usize, Vec<(u32, i32, i32)>>,
    /// `capabilities` per glass object, keyed by the user data it was created with.
    max_radius: HashMap<usize, u32>,
    /// `wl_display.sync` callbacks seen, which is what orders a test without a sleep.
    callbacks: usize,
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

impl Dispatch<layer::ZwlrLayerSurfaceV1, usize> for Client {
    fn event(
        state: &mut Self,
        proxy: &layer::ZwlrLayerSurfaceV1,
        event: layer::Event,
        id: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let layer::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            state.layer_configures.entry(*id).or_default().push((
                serial,
                width as i32,
                height as i32,
            ));
            proxy.ack_configure(serial);
        }
    }
}

impl Dispatch<glass::InoxGlassV1, usize> for Client {
    fn event(
        state: &mut Self,
        _: &glass::InoxGlassV1,
        event: glass::Event,
        id: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let glass::Event::Capabilities { max_radius } = event {
            state.max_radius.insert(*id, max_radius);
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.callbacks += 1;
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
wayland_client::delegate_noop!(Client: ignore shell::ZwlrLayerShellV1);
wayland_client::delegate_noop!(Client: ignore glass_manager::InoxGlassManagerV1);
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

    /// A `wl_shm` buffer of `width` x `height` filled with `data` (BGRA rows).
    fn shm_buffer(&self, width: i32, height: i32, data: &[u8]) -> wl_buffer::WlBuffer {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(data).unwrap();
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

    /// A `wl_shm` buffer split at `edge_x` into `left` and `right`.
    fn two_tone_buffer(
        &self,
        width: i32,
        height: i32,
        edge_x: i32,
        left: [u8; 4],
        right: [u8; 4],
    ) -> wl_buffer::WlBuffer {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..height {
            for x in 0..width {
                let colour = if x < edge_x { left } else { right };
                // `Argb8888` is BGRA in memory on a little-endian machine.
                data.extend_from_slice(&[colour[2], colour[1], colour[0], colour[3]]);
            }
        }
        self.shm_buffer(width, height, &data)
    }

    /// A layer surface in `depth`, created with user data `id`.
    fn layer(&self, id: usize, depth: shell::Layer) -> LayerSurface {
        let compositor: wl_compositor::WlCompositor = self.bind(5);
        let shell: shell::ZwlrLayerShellV1 = self.bind(4);
        let surface = compositor.create_surface(&self.queue.handle(), ());
        let role = shell.get_layer_surface(
            &surface,
            None,
            depth,
            format!("session-lock-glass-{id}"),
            &self.queue.handle(),
            id,
        );
        LayerSurface { surface, role, id }
    }

    /// Bind the private glass manager.
    fn glass_manager(&self) -> glass_manager::InoxGlassManagerV1 {
        let (name, advertised) = self.state.globals["inox_glass_manager_v1"];
        self.registry
            .bind(name, advertised.min(1), &self.queue.handle(), ())
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

/// A real layer surface, with the geometry the compositor configured it to.
struct LayerSurface {
    surface: wl_surface::WlSurface,
    role: layer::ZwlrLayerSurfaceV1,
    id: usize,
}

impl LayerSurface {
    fn configure_size(&self, peer: &Peer) -> (i32, i32) {
        let (_, width, height) = *peer.state.layer_configures[&self.id].last().unwrap();
        (width, height)
    }

    /// Paint the surface with a buffer, once the compositor has sized it.
    fn attach(&self, peer: &Peer, buffer: &wl_buffer::WlBuffer) {
        let (width, height) = self.configure_size(peer);
        self.surface.attach(Some(buffer), 0, 0);
        self.surface.damage_buffer(0, 0, width, height);
        self.surface.commit();
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

/// Wait for the first configure the compositor sends `peer`'s layer surface `id`.
fn await_layer_configure(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peer: &mut Peer,
    id: usize,
) {
    until(
        "a layer surface configure",
        display,
        state,
        &mut [peer],
        |_, peers| peers[0].state.layer_configures.contains_key(&id),
    );
}

/// Wait until the server has processed every request already sent on `peer`.
///
/// A `wl_display.sync` callback is processed in order behind them, so this orders
/// the test without a sleep.
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

/// The red channel of one row.
///
/// Every colour in the glass scene is grey or a two-tone edge, so the channel
/// carries the whole signal and the readback's `Abgr8888` byte order never has to
/// be guessed.
fn row(frame: &[u8], y: usize) -> Vec<u8> {
    (0..WIDTH as usize)
        .map(|x| frame[(y * WIDTH as usize + x) * 4])
        .collect()
}

/// The pane's own columns: the pane is anchored to the top-left corner.
fn pane_columns() -> std::ops::Range<usize> {
    0..PANE_WIDTH as usize
}

/// How many samples lie strictly between the row's two extremes.
///
/// A sharp two-colour edge has (almost) none; a blurred one has many. This is the
/// measurement that separates "the composited backdrop was blurred" from "the
/// client's translucent pixels were drawn over the sharp scene" — the two produce
/// very different counts and identical plateaus.
fn intermediate(values: &[u8]) -> usize {
    let (Some(&low), Some(&high)) = (values.iter().min(), values.iter().max()) else {
        return 0;
    };
    if high.saturating_sub(low) < 8 {
        return 0;
    }
    // A margin of an eighth of the range keeps driver rounding and the translucent
    // pane's own colour from being counted as a blur.
    let margin = (high - low) / 8;
    values
        .iter()
        .filter(|value| **value > low + margin && **value < high - margin)
        .count()
}

/// How many pixels of `frame` are `probe` within the one-step tolerance drivers
/// need for float-to-unorm rounding.
fn count_probe(frame: &[u8], probe: [u8; 4]) -> usize {
    frame
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| {
            pixel
                .iter()
                .zip(probe)
                .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
        })
        .count()
}

/// How many pixels the two frames disagree on by more than one 8-bit step.
fn differing_pixels(before: &[u8], after: &[u8]) -> usize {
    assert_eq!(before.len(), after.len(), "frames must have the same size");
    before
        .as_chunks::<4>()
        .0
        .iter()
        .zip(after.as_chunks::<4>().0)
        .filter(|(left, right)| {
            left.iter()
                .zip(right.iter())
                .any(|(left, right)| left.abs_diff(*right) > 1)
        })
        .count()
}

/// The largest per-channel distance between any pixel and `probe`.
///
/// Zero means the frame is `probe` exactly; one is the tolerance float-to-unorm
/// rounding needs, which is what "every pixel is the lock surface" is asserted
/// with.
fn max_channel_distance(frame: &[u8], probe: [u8; 4]) -> u8 {
    frame
        .as_chunks::<4>()
        .0
        .iter()
        .map(|pixel| {
            pixel
                .iter()
                .zip(probe)
                .map(|(actual, expected)| actual.abs_diff(expected))
                .max()
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
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
    // it happens on the request.
    assert_eq!(app.state.globals["ext_session_lock_manager_v1"].1, 1);

    // --- frosting is live before a lock, and cannot survive one --------------
    //
    // the glass protocol's acceptance table left exactly this case open: the "a locked
    // session contributes no panes" rule was pinned only by the pure
    // `glass::collect_panes(locked = true, ..)` unit test and the early return in
    // `scene_elements`. This is the end-to-end half, with the same scene
    // `tests/glass_blur.rs` measures — a background layer carrying a sharp
    // black/white edge, a translucent top-layer pane over it, and a real
    // `inox_glass_v1` object with a radius and a region — and it is placed before
    // the ordinary desktop exists so the only content on screen is the glass
    // scene itself.
    let mut glassy = Peer::connect(&mut handle);
    let mut locker = Peer::connect(&mut handle);
    until(
        "the glass client and the lock holder see their globals",
        &mut display,
        &mut state,
        &mut [&mut glassy, &mut locker],
        |_, peers| {
            peers.iter().all(|peer| {
                peer.has_global("zwlr_layer_shell_v1") && peer.has_global("inox_glass_manager_v1")
            })
        },
    );

    // The structured backdrop: a background layer surface with a sharp edge at
    // GLASS_EDGE_X, exactly as in `tests/glass_blur.rs`.
    let backdrop = glassy.layer(0, shell::Layer::Background);
    backdrop.role.set_size(0, 0);
    backdrop.role.set_anchor(
        layer::Anchor::Top | layer::Anchor::Bottom | layer::Anchor::Left | layer::Anchor::Right,
    );
    backdrop.role.set_exclusive_zone(-1);
    backdrop.surface.commit();
    await_layer_configure(&mut display, &mut state, &mut glassy, 0);
    assert_eq!(backdrop.configure_size(&glassy), (WIDTH, HEIGHT));
    let pattern = glassy.two_tone_buffer(WIDTH, HEIGHT, GLASS_EDGE_X, BLACK, WHITE);
    backdrop.attach(&glassy, &pattern);
    until(
        "the background layer is mapped",
        &mut display,
        &mut state,
        &mut [&mut glassy],
        |state, _| state.layer_count() == 1,
    );

    // The pane: a translucent top layer surface over the edge.
    let pane = glassy.layer(1, shell::Layer::Top);
    pane.role.set_size(PANE_WIDTH as u32, PANE_HEIGHT as u32);
    pane.role
        .set_anchor(layer::Anchor::Top | layer::Anchor::Left);
    pane.surface.commit();
    await_layer_configure(&mut display, &mut state, &mut glassy, 1);
    assert_eq!(pane.configure_size(&glassy), (PANE_WIDTH, PANE_HEIGHT));
    let translucent = glassy.buffer(PANE_WIDTH, PANE_HEIGHT, PANE_RGBA);
    pane.attach(&glassy, &translucent);
    until(
        "the pane layer is mapped",
        &mut display,
        &mut state,
        &mut [&mut glassy],
        |state, _| state.layer_count() == 2,
    );

    // The pane already has its layer role, so the compositor must offer it the
    // real maximum, and the effect must really be applied.
    let manager = glassy.glass_manager();
    let pane_glass = manager.get_glass(&pane.surface, &glassy.queue.handle(), 1);
    until(
        "the pane's glass object is told its maximum",
        &mut display,
        &mut state,
        &mut [&mut glassy],
        |_, peers| peers[0].state.max_radius.contains_key(&1),
    );
    let unlocked_max_radius = glassy.state.max_radius[&1];
    assert_eq!(
        unlocked_max_radius, MAX_BLUR_RADIUS,
        "an unlocked layer surface is offered the compositor's real maximum"
    );
    pane_glass.set_radius(GLASS_RADIUS);
    pane_glass.set_region(0, 0, PANE_WIDTH, PANE_HEIGHT);
    round_trip(&mut display, &mut state, &mut glassy);

    let frosted = settle_until(
        &mut display,
        &mut state,
        &mut [&mut glassy],
        "the live frost before the lock",
        |rendered| {
            intermediate(&row(rendered, GLASS_ROW_INSIDE)[pane_columns()])
                >= GLASS_RADIUS as usize / 2
        },
    );
    let live_ramp = intermediate(&row(&frosted, GLASS_ROW_INSIDE)[pane_columns()]);
    let live_pattern = count_probe(&frosted, WHITE);
    let sharp_below_pane = intermediate(&row(&frosted, GLASS_ROW_OUTSIDE));
    assert!(
        live_ramp >= GLASS_RADIUS as usize / 2,
        "the frost must be live before the lock, otherwise the locked frame proves nothing: \
         {live_ramp} intermediate columns"
    );
    assert!(
        live_pattern > 0 && frame_contains(&frosted, WHITE),
        "the structured backdrop must be on screen before the lock: {live_pattern} white pixels"
    );
    assert!(
        sharp_below_pane <= MAX_INTERMEDIATE_OUTSIDE_PANE,
        "the backdrop below the pane must stay sharp, otherwise the ramp above it is not the \
         effect: {sharp_below_pane} intermediate columns"
    );

    // The lock holder claims the trusted role and covers the glass scene. The
    // keyboard is bound so the lock client is the full real thing, not a client
    // that merely holds the object.
    locker.claim(TOKEN);
    let locker_seat: wl_seat::WlSeat = locker.bind(9);
    let _locker_keyboard = locker_seat.get_keyboard(&locker.queue.handle(), ());
    let held = locker.request_lock();
    until(
        "the glass scene is behind a lock",
        &mut display,
        &mut state,
        &mut [&mut locker],
        |state, _| state.session_locked(),
    );
    let lock_frame = locker.create_lock_surface(&held);
    let (_, width, height) = await_configure(&mut display, &mut state, &mut locker);
    assert_eq!((width, height), (WIDTH as u32, HEIGHT as u32));
    locker.paint_lock_surface(&lock_frame, (width, height), LOCK_FRAME);
    let locked = settle_until(
        &mut display,
        &mut state,
        &mut [&mut locker],
        "the lock surface over the glass scene",
        |rendered| max_channel_distance(rendered, LOCK_FRAME) <= 1,
    );

    // The locked frame is the lock surface and nothing else: not one pixel of the
    // pattern, the pane or the frost survives, within the one-step rounding
    // tolerance.
    assert!(
        max_channel_distance(&locked, LOCK_FRAME) <= 1,
        "every pixel of the locked frame must be the lock surface, found a channel up to {} \
         steps away",
        max_channel_distance(&locked, LOCK_FRAME)
    );
    assert_eq!(
        count_probe(&locked, WHITE),
        0,
        "the background pattern must be gone from a locked screen"
    );
    let locked_ramp = intermediate(&row(&locked, GLASS_ROW_INSIDE)[pane_columns()]);
    assert_eq!(
        locked_ramp, 0,
        "the frosted edge must be gone from a locked screen"
    );
    let locked_sharp = intermediate(&row(&locked, GLASS_ROW_OUTSIDE));
    assert_eq!(
        locked_sharp, 0,
        "the sharp edge below the pane must be gone from a locked screen too"
    );
    let hidden = differing_pixels(&frosted, &locked);
    assert!(
        hidden >= (WIDTH * HEIGHT) as usize / 2,
        "the locked frame must differ from the pre-lock frame it replaced: only {hidden} pixels"
    );
    assert!(state.session_locked());

    // `locked` arrives once the covered frame has been presented; only then does
    // the "created while locked" case below mean what it says.
    until(
        "the lock holder is told it owns the screen",
        &mut display,
        &mut state,
        &mut [&mut locker],
        |_, peers| !peers[0].state.lock_events.is_empty(),
    );
    assert_eq!(
        locker.state.lock_events,
        vec![LockEvent::Locked],
        "`locked` arrives once the locked frame has been presented"
    );

    // --- a glass object created while locked is told 0 -----------------------
    //
    // The compositor still answers the layer-shell and glass protocols while a
    // lock is held; what it must not do is promise a blur it will not draw. The
    // new surface is committed and mapped exactly like the pre-lock pane — same
    // layer role, same size, same pixels — so the lock is the only reason to
    // answer 0 and the only reason it may not change the frame.
    let late = glassy.layer(2, shell::Layer::Top);
    late.role.set_size(PANE_WIDTH as u32, PANE_HEIGHT as u32);
    late.role
        .set_anchor(layer::Anchor::Top | layer::Anchor::Left);
    late.surface.commit();
    await_layer_configure(&mut display, &mut state, &mut glassy, 2);
    assert_eq!(late.configure_size(&glassy), (PANE_WIDTH, PANE_HEIGHT));
    // It is attached and mapped: the pixel assertion below is about a pane that is
    // really part of the scene, not about a surface that never had content.
    late.attach(&glassy, &translucent);
    until(
        "the pane created under the lock is mapped",
        &mut display,
        &mut state,
        &mut [&mut glassy],
        |state, _| state.layer_count() == 3,
    );
    let late_manager = glassy.glass_manager();
    let late_glass = late_manager.get_glass(&late.surface, &glassy.queue.handle(), 2);
    until(
        "the locked scene's glass object is told its maximum",
        &mut display,
        &mut state,
        &mut [&mut glassy],
        |_, peers| peers[0].state.max_radius.contains_key(&2),
    );
    let locked_max_radius = glassy.state.max_radius[&2];
    assert_eq!(
        locked_max_radius, 0,
        "a glass object created while the session is locked must be told 0"
    );
    assert_eq!(
        glassy.state.max_radius[&1], unlocked_max_radius,
        "`capabilities` is sent once: the pre-lock object keeps the maximum it was promised, \
         and the compositor simply does not apply it while locked"
    );
    late_glass.set_radius(GLASS_RADIUS);
    late_glass.set_region(0, 0, PANE_WIDTH, PANE_HEIGHT);
    round_trip(&mut display, &mut state, &mut glassy);

    state.render_frame().unwrap();
    let after_glass_traffic = state.read_frame_rgba().unwrap();
    assert_eq!(
        differing_pixels(&locked, &after_glass_traffic),
        0,
        "a pane created under the lock must not change a single pixel of the locked frame"
    );
    assert_eq!(
        count_probe(&after_glass_traffic, WHITE),
        0,
        "no pattern may appear because a locked client asked for frosting"
    );

    // --- the held lock survives the glass traffic ----------------------------
    assert_eq!(
        locker.state.lock_events,
        vec![LockEvent::Locked],
        "glass traffic must not disturb the lock: one `locked`, no `finished`"
    );
    assert!(state.session_locked());
    state.render_frame().unwrap();
    let still_locked = state.read_frame_rgba().unwrap();
    assert!(
        max_channel_distance(&still_locked, LOCK_FRAME) <= 1,
        "the frame read back after the glass traffic must still be a locked frame"
    );

    // --- the same scene without the lock ------------------------------------
    //
    // The discriminative half: the lock is released and the very same surfaces
    // come back, so the empty locked frame above cannot be explained by a scene
    // that was never drawn.
    held.unlock_and_destroy();
    until(
        "the lock is released",
        &mut display,
        &mut state,
        &mut [&mut locker, &mut glassy],
        |state, _| !state.session_locked(),
    );
    let restored = settle_until(
        &mut display,
        &mut state,
        &mut [&mut glassy],
        "the frosted scene after the unlock",
        |rendered| count_probe(rendered, WHITE) > 0,
    );
    let restored_pattern = count_probe(&restored, WHITE);
    assert!(
        restored_pattern > 0,
        "the same scene must come back when the lock is released, otherwise the locked frame \
         proved nothing"
    );

    // Release the scene so the rest of this test measures the ordinary desktop.
    drop(glassy);
    drop(locker);
    until(
        "the glass scene is released",
        &mut display,
        &mut state,
        &mut [&mut app, &mut keeper, &mut stranger],
        |state, _| state.layer_count() == 0 && state.window_count() == 0,
    );
    state.render_frame().unwrap();
    let cleaned = state.read_frame_rgba().unwrap();
    assert_eq!(
        count_probe(&cleaned, WHITE),
        0,
        "the disconnected glass client leaves no pattern behind"
    );

    println!(
        "session lock + glass: capabilities pre-lock = {unlocked_max_radius}, mid-lock = \
         {locked_max_radius}; frosted row intermediate columns pre-lock = {live_ramp} (sharp row \
         below the pane = {sharp_below_pane}, max {MAX_INTERMEDIATE_OUTSIDE_PANE} outside a pane), \
         locked = {locked_ramp} (sharp row = {locked_sharp}); white pattern pixels pre-lock = \
         {live_pattern}, locked = 0, restored = {restored_pattern}; pixels differing pre-lock vs \
         locked = {hidden} of {}",
        WIDTH * HEIGHT
    );

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
    // hand a held lock to another client.
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
