//! Integration test for M3's drag-and-drop path: the Wayland-side evidence for X03.
//!
//! Two real `wayland-client` connections are driven against a compositor state on the
//! current thread (the `window_management.rs` pattern). The source client takes an
//! implicit grab with an injected pointer press, starts a drag offering two mime types,
//! and the target client receives the enter, accepts the offer and reads the payload
//! through a socket pair. Two further drags cover the paths that are easy to get wrong:
//! a drag cancelled by dropping outside every surface, and a drag whose target window is
//! destroyed while it is in flight.
//!
//! Everything runs on one thread and every wait is a deadline-bounded pump loop. The
//! drag needs the compositor, the source and the target to make progress in lockstep, so
//! a single thread alternating between "let every client send what it has" and "service
//! the display" is what keeps the scenario deterministic: no sleeps to order the steps,
//! no thread that could deadlock, and an ack that can be trusted because the compositor
//! flushed before it answered.
//!
//! The clients commit a real `wl_shm` buffer for their toplevels. Smithay gives a window
//! the geometry the pointer is hit-tested against only once a surface has a committed
//! buffer, so without one no injected pointer event would ever reach a surface and the
//! test would be asserting against a compositor that never saw a window.
//!
//! Everything lives in a single test function: a process may only ever hold one EGL
//! display (see the `SAFETY` note in `src/render.rs`), and test functions inside one
//! binary run in parallel threads.

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    os::fd::AsFd,
    os::unix::net::UnixStream,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, InoxState, WindowId};
use smithay::{
    backend::input::ButtonState,
    reexports::wayland_server::{Display, DisplayHandle},
    utils::{Logical, Point},
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{
        wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager,
        wl_data_device_manager::DndAction, wl_data_offer, wl_data_source, wl_pointer, wl_registry,
        wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a step may take before the test declares the compositor stuck.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// How long one pump-and-service round waits before it looks again.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// The two mime types the source offers, as a real drag of text would.
const OFFERED_MIMES: [&str; 2] = ["text/plain;charset=utf-8", "text/uri-list"];

/// The mime type the target accepts and reads.
const ACCEPTED_MIME: &str = "text/plain;charset=utf-8";

/// The payload the source writes and the target must get back.
const DRAG_BYTES: &[u8] = b"inox drag and drop payload";

/// The pointer button the drag is started with (evdev `BTN_LEFT`).
const BUTTON_LEFT: u32 = 0x110;

/// The size of the buffer every client commits, in pixels.
const BUFFER_SIZE: i32 = 100;

/// Service the display once: dispatch client requests and flush outgoing events.
fn service(display: &mut Display<InoxState>, compositor: &mut InoxState) {
    // Errors during teardown (a peer that went away) are not interesting here; the
    // assertions below are what the test is about.
    let _ = display.dispatch_clients(compositor);
    let _ = display.flush_clients();
}

/// The client-side state every connection keeps.
///
/// One struct covers both roles: the source and the target are the same kind of client,
/// they just exercise different halves of the protocol.
#[derive(Default)]
struct Client {
    /// Registry contents, as `interface -> (name, version)`.
    globals: HashMap<String, (u32, u32)>,

    // --- the drag source ---
    /// The data source this client is dragging with, kept alive for the drag's duration.
    source: Option<wl_data_source::WlDataSource>,
    /// The serial of the last pointer button press, which is what an implicit grab needs.
    press_serial: Option<u32>,
    /// How often the compositor asked this client for its payload.
    payload_requests: usize,
    /// `wl_data_source.dnd_drop_performed` events, i.e. successful drops.
    drops_performed: usize,
    /// `wl_data_source.cancelled` events, i.e. drags that ended without a drop.
    cancellations: usize,
    /// `wl_data_source.dnd_finished` events.
    finishes: usize,

    // --- the drag target ---
    /// The offer the compositor sent for the drag currently over this client.
    offer: Option<wl_data_offer::WlDataOffer>,
    /// The mime types that offer carries.
    mime_types: Vec<String>,
    /// How often a drag entered this client's surface.
    enters: usize,
    /// How often a drag left it again.
    leaves: usize,
    /// How often a drag was dropped on it.
    drops: usize,
    /// Whether this client should accept the offers it receives.
    accept_offers: bool,

    // --- pointer ---
    /// How often the pointer entered this client's surface.
    pointer_enters: usize,
    /// How often the pointer left it again.
    pointer_leaves: usize,
    /// The buffer the client committed, kept alive so the surface keeps its geometry.
    _buffer: Option<wl_buffer::WlBuffer>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for Client {
    fn event(
        state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            state.globals.insert(interface, (name, version));
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore wl_data_device_manager::WlDataDeviceManager);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);

impl Dispatch<wl_pointer::WlPointer, ()> for Client {
    fn event(
        state: &mut Self,
        _pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { .. } => state.pointer_enters += 1,
            wl_pointer::Event::Leave { .. } => state.pointer_leaves += 1,
            // The serial is the evidence that the compositor installed an implicit grab;
            // it is what `start_drag` has to be sent with.
            wl_pointer::Event::Button {
                serial,
                state: wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => state.press_serial = Some(serial),
            _ => {}
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        _state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        // The compositor offers a tiled size; the client is free to keep its own, but it
        // has to acknowledge the configure either way.
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for Client {
    fn event(
        state: &mut Self,
        _device: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                // A drag that re-enters this client gets a fresh offer, so the mime types
                // of the previous one must not linger.
                state.mime_types.clear();
                state.offer = Some(id);
            }
            wl_data_device::Event::Enter { serial, .. } => {
                state.enters += 1;

                if state.accept_offers
                    && let Some(offer) = state.offer.clone()
                {
                    offer.accept(serial, Some(ACCEPTED_MIME.to_owned()));
                    // The actions the target is willing to perform. Without one of
                    // these the compositor never validates the drop.
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
                }
            }
            wl_data_device::Event::Leave => state.leaves += 1,
            wl_data_device::Event::Drop => state.drops += 1,
            _ => {}
        }
    }

    // `data_offer` and `selection` both create `wl_data_offer` objects, so the queue needs
    // to know which user data to give them.
    wayland_client::event_created_child!(Client, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
        wl_data_device::EVT_SELECTION_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for Client {
    fn event(
        state: &mut Self,
        _offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            state.mime_types.push(mime_type);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for Client {
    fn event(
        state: &mut Self,
        _source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            // The compositor hands the source client the write end of a socket pair the
            // target created; writing the payload and closing it is the whole transfer.
            wl_data_source::Event::Send { mime_type, fd } => {
                assert!(
                    OFFERED_MIMES.contains(&mime_type.as_str()),
                    "the target asked for `{mime_type}`, which the source never offered"
                );
                state.payload_requests += 1;

                let mut writer = File::from(fd);
                writer
                    .write_all(DRAG_BYTES)
                    .expect("failed to write the drag payload");
                // Dropping the writer closes the target's peer, which is its end-of-file.
                drop(writer);
            }
            wl_data_source::Event::DndDropPerformed => state.drops_performed += 1,
            wl_data_source::Event::DndFinished => state.finishes += 1,
            wl_data_source::Event::Cancelled => state.cancellations += 1,
            _ => {}
        }
    }
}

/// One client connection plus the protocol objects it bound.
struct TestClient {
    connection: Connection,
    queue: EventQueue<Client>,
    state: Client,
    registry: wl_registry::WlRegistry,
    compositor: Option<wl_compositor::WlCompositor>,
    xdg: Option<xdg_wm_base::XdgWmBase>,
    shm: Option<wl_shm::WlShm>,
    manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    device: Option<wl_data_device::WlDataDevice>,
    surface: Option<wl_surface::WlSurface>,
    _seat: Option<wl_seat::WlSeat>,
    _pointer: Option<wl_pointer::WlPointer>,
    _xdg_surface: Option<xdg_surface::XdgSurface>,
    _toplevel: Option<xdg_toplevel::XdgToplevel>,
}

impl TestClient {
    /// Open a connection to the in-process compositor and ask for the registry.
    fn connect(display_handle: &mut DisplayHandle) -> Self {
        let (server, client) = UnixStream::pair().expect("failed to create the socket pair");
        client
            .set_nonblocking(true)
            .expect("failed to switch the client socket to non-blocking mode");
        display_handle
            .insert_client(server, Arc::new(ClientState::default()))
            .expect("failed to register the client");

        let connection = Connection::from_socket(client).expect("failed to connect the client");
        let queue = connection.new_event_queue::<Client>();
        let registry = connection.display().get_registry(&queue.handle(), ());

        Self {
            connection,
            queue,
            state: Client::default(),
            registry,
            compositor: None,
            xdg: None,
            shm: None,
            manager: None,
            device: None,
            surface: None,
            _seat: None,
            _pointer: None,
            _xdg_surface: None,
            _toplevel: None,
        }
    }

    /// Send what this client has queued and dispatch what the compositor sent back.
    ///
    /// The socket is non-blocking, so an empty read means "the compositor has nothing for
    /// us yet" rather than an error; the only errors here are the connection going away at
    /// teardown, which the assertions do not depend on.
    fn pump(&mut self) {
        let _ = self.connection.flush();

        if let Some(guard) = self.connection.prepare_read() {
            let _ = guard.read();
        }

        let _ = self.queue.dispatch_pending(&mut self.state);
    }

    /// Bind a global by interface name, capped at `max_version`.
    fn bind<I, U>(&self, max_version: u32) -> I
    where
        I: Proxy + 'static,
        Client: Dispatch<I, U>,
        U: Default + Send + Sync + 'static,
    {
        let name = I::interface().name;
        let (global_name, version) = *self.state.globals.get(name).unwrap_or_else(|| {
            panic!("the compositor did not advertise `{name}`");
        });

        self.registry.bind(
            global_name,
            version.min(max_version),
            &self.queue.handle(),
            U::default(),
        )
    }

    /// Bind every global the drag scenario needs.
    fn bind_all(&mut self) {
        self.compositor = Some(self.bind::<wl_compositor::WlCompositor, ()>(4));
        self.xdg = Some(self.bind::<xdg_wm_base::XdgWmBase, ()>(6));
        self.shm = Some(self.bind::<wl_shm::WlShm, ()>(1));
        self.manager = Some(self.bind::<wl_data_device_manager::WlDataDeviceManager, ()>(3));

        let seat: wl_seat::WlSeat = self.bind::<wl_seat::WlSeat, ()>(8);
        let manager = self.manager.clone().expect("the manager was just bound");
        self.device = Some(manager.get_data_device(&seat, &self.queue.handle(), ()));
        self._pointer = Some(seat.get_pointer(&self.queue.handle(), ()));
        self._seat = Some(seat);
    }

    /// Map an `xdg_toplevel` with a committed buffer, which is what makes it a window the
    /// pointer can reach.
    fn map_toplevel(&mut self) {
        let compositor = self.compositor.clone().expect("bind_all has not run");
        let xdg = self.xdg.clone().expect("bind_all has not run");
        let shm = self.shm.clone().expect("bind_all has not run");
        let queue = self.queue.handle();

        let surface = compositor.create_surface(&queue, ());
        let xdg_surface = xdg.get_xdg_surface(&surface, &queue, ());
        let toplevel = xdg_surface.get_toplevel(&queue, ());
        surface.commit();

        // An anonymous file of exactly `BUFFER_SIZE` squared pixels, all zeroes: the
        // compositor never reads the contents, it only needs the geometry.
        let stride = BUFFER_SIZE * 4;
        let file = tempfile::tempfile().expect("failed to create the buffer file");
        file.set_len((stride * BUFFER_SIZE) as u64)
            .expect("failed to size the buffer file");
        let pool = shm.create_pool(file.as_fd(), stride * BUFFER_SIZE, &queue, ());
        let buffer = pool.create_buffer(
            0,
            BUFFER_SIZE,
            BUFFER_SIZE,
            stride,
            wl_shm::Format::Argb8888,
            &queue,
            (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();

        self.surface = Some(surface);
        self.state._buffer = Some(buffer);
        self._xdg_surface = Some(xdg_surface);
        self._toplevel = Some(toplevel);
    }

    /// Start a drag offering [`OFFERED_MIMES`], using the last button press as the grab.
    ///
    /// # Panics
    ///
    /// Panics when this client has no surface or no press serial; both mean the scenario
    /// got out of order, which is worth failing loudly on.
    fn start_drag(&mut self) {
        let manager = self.manager.clone().expect("bind_all has not run");
        let device = self.device.clone().expect("bind_all has not run");
        let surface = self
            .surface
            .clone()
            .expect("the toplevel has not been mapped");
        let serial = self
            .state
            .press_serial
            .take()
            .expect("the pointer press never reached this client");

        let source = manager.create_data_source(&self.queue.handle(), ());
        for mime in OFFERED_MIMES {
            source.offer(mime.to_owned());
        }
        // Without this the compositor has no action to offer the target, and every drop
        // would come out as a cancellation.
        source.set_actions(DndAction::Copy | DndAction::Move);

        device.start_drag(Some(&source), &surface, None, serial);

        self.state.source = Some(source);
    }

    /// Destroy the toplevel, leaving the connection up.
    fn destroy_toplevel(&mut self) {
        if let Some(toplevel) = self._toplevel.take() {
            toplevel.destroy();
        }
        if let Some(surface) = self.surface.take() {
            surface.destroy();
        }
    }
}

/// The two clients the drag scenario needs.
struct Clients {
    /// The drag source.
    source: TestClient,
    /// The drag target.
    target: TestClient,
}

impl Clients {
    fn all_mut(&mut self) -> [&mut TestClient; 2] {
        [&mut self.source, &mut self.target]
    }

    fn all(&self) -> [&TestClient; 2] {
        [&self.source, &self.target]
    }

    fn pump_all(&mut self) {
        for client in self.all_mut() {
            client.pump();
        }
    }

    fn bind_all(&mut self) {
        for client in self.all_mut() {
            client.bind_all();
        }
    }
}

/// Pump every client and service the compositor until `done` holds.
fn pump_until(
    display: &mut Display<InoxState>,
    compositor: &mut InoxState,
    clients: &mut Clients,
    what: &str,
    done: impl Fn(&InoxState, &Clients) -> bool,
) {
    let deadline = Instant::now() + STEP_TIMEOUT;

    loop {
        clients.pump_all();
        service(display, compositor);

        if done(compositor, clients) {
            return;
        }

        assert!(
            Instant::now() < deadline,
            "timed out after {STEP_TIMEOUT:?} waiting for {what}"
        );

        thread::sleep(POLL_INTERVAL);
    }
}

/// Run `rounds` pump-and-service rounds, so anything already queued is delivered without
/// waiting for a condition that may never become true.
fn settle(
    display: &mut Display<InoxState>,
    compositor: &mut InoxState,
    clients: &mut Clients,
    rounds: usize,
) {
    for _ in 0..rounds {
        clients.pump_all();
        service(display, compositor);
    }
}

/// A point inside the window's hit area.
///
/// The clients commit a [`BUFFER_SIZE`] square buffer, so the area the pointer is
/// hit-tested against is that buffer at the window's tile origin, not the whole tile.
fn inside(compositor: &InoxState, id: WindowId) -> Point<f64, Logical> {
    let location = compositor
        .window_location(id)
        .expect("the window is not mapped");

    Point::from((f64::from(location.x) + 30.0, f64::from(location.y) + 30.0))
}

/// A point on the output that no window covers.
fn outside() -> Point<f64, Logical> {
    // Both windows sit at the top of the work area and are one buffer tall, so the far
    // corner of the 1280x720 output is never part of one.
    Point::from((1270.0, 710.0))
}

/// Move the pointer and wait until `entered` reports the surface it should have reached.
fn move_pointer(
    display: &mut Display<InoxState>,
    compositor: &mut InoxState,
    clients: &mut Clients,
    location: Point<f64, Logical>,
    what: &str,
    entered: impl Fn(&Clients) -> bool,
) {
    compositor.inject_pointer_motion(location);
    pump_until(display, compositor, clients, what, |_, clients| {
        entered(clients)
    });
}

/// Ask the target for the dragged payload and return what the source wrote.
///
/// The transfer only completes if the compositor forwards the request and the source
/// answers it, so this keeps the whole compositor and both clients running until the
/// target sees the end of the data.
fn read_dragged_payload(
    display: &mut Display<InoxState>,
    compositor: &mut InoxState,
    clients: &mut Clients,
) -> Vec<u8> {
    let offer = clients
        .target
        .state
        .offer
        .clone()
        .expect("the target has no drag offer");

    // A socket pair stands in for the pipe a real client would use; the compositor only
    // ever forwards the write end to the source.
    let (mut read_end, write_end) =
        UnixStream::pair().expect("failed to create the transfer socket");
    offer.receive(ACCEPTED_MIME.to_owned(), write_end.as_fd());
    // The target must not hold the write end open, or it would never see end-of-file.
    drop(write_end);
    read_end
        .set_read_timeout(Some(POLL_INTERVAL))
        .expect("failed to bound the transfer read");

    let deadline = Instant::now() + STEP_TIMEOUT;
    let mut chunk = [0u8; 4096];
    let mut bytes = Vec::new();

    loop {
        match read_end.read(&mut chunk) {
            // The source closed its end: the payload is complete.
            Ok(0) => {
                // Telling the source the drag is finished is what keeps it from seeing a
                // cancellation when the offer goes away.
                offer.finish();

                return bytes;
            }
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            // A read timeout is the normal case while the transfer is still in flight; the
            // compositor is serviced below on every round either way.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("failed to read the dragged payload: {error}"),
        }

        clients.pump_all();
        service(display, compositor);

        assert!(
            Instant::now() < deadline,
            "timed out after {STEP_TIMEOUT:?} reading the dragged payload"
        );
    }
}

#[test]
fn drags_transfer_the_payload_and_cancel_cleanly() {
    let event_loop: EventLoop<'static, ()> =
        EventLoop::try_new().expect("failed to create the event loop");
    let mut display: Display<InoxState> =
        Display::new().expect("failed to create the wayland display");
    let mut display_handle = display.handle();

    let mut compositor = InoxState::new(
        &display_handle,
        &CompositorConfig::default(),
        event_loop.get_signal(),
    )
    .expect("failed to create the compositor state");

    let mut clients = Clients {
        source: TestClient::connect(&mut display_handle),
        target: TestClient::connect(&mut display_handle),
    };

    // --- the registry ------------------------------------------------------

    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "both clients to learn the registry",
        |_, clients| {
            clients
                .all()
                .iter()
                .all(|client| client.state.globals.contains_key("wl_data_device_manager"))
        },
    );
    clients.bind_all();
    settle(&mut display, &mut compositor, &mut clients, 4);

    // --- the source window takes the focus and an implicit grab ------------

    clients.source.map_toplevel();
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the source client window",
        |compositor, _| compositor.window_count() == 1,
    );
    let source_id = compositor
        .managed_windows()
        .first()
        .copied()
        .expect("the source window is not managed");

    // Moving onto the source's surface is also what proves the committed buffer gave the
    // window its geometry.
    let point = inside(&compositor, source_id);
    move_pointer(
        &mut display,
        &mut compositor,
        &mut clients,
        point,
        "the pointer to enter the source client's window",
        |clients| clients.source.state.pointer_enters == 1,
    );

    compositor.inject_pointer_button(BUTTON_LEFT, ButtonState::Pressed);
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the source client to see the button press",
        |_, clients| clients.source.state.press_serial.is_some(),
    );

    clients.source.start_drag();
    // Installing the drag grab clears the pointer focus, which is the only observable
    // proof that the compositor accepted the drag: the source stops being the surface the
    // pointer is on.
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the drag to take over the pointer",
        |_, clients| clients.source.state.pointer_leaves == 1,
    );

    // --- the target window appears ----------------------------------------

    clients.target.map_toplevel();
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the target client window",
        |compositor, _| compositor.window_count() == 2,
    );
    let target_id = *compositor
        .managed_windows()
        .first()
        .expect("the target window is not managed");

    // --- the drag reaches the target --------------------------------------

    clients.target.state.accept_offers = true;
    let point = inside(&compositor, target_id);
    move_pointer(
        &mut display,
        &mut compositor,
        &mut clients,
        point,
        "the drag to enter the target client",
        |clients| clients.target.state.enters == 1,
    );

    // The mime types arrive as separate events after the offer, so give the queue a
    // chance to deliver them before asserting on the set.
    settle(&mut display, &mut compositor, &mut clients, 4);

    for mime in OFFERED_MIMES {
        assert!(
            clients
                .target
                .state
                .mime_types
                .iter()
                .any(|offered| offered == mime),
            "the target was not offered `{mime}`, only {:?}",
            clients.target.state.mime_types
        );
    }

    // --- the drop ----------------------------------------------------------

    compositor.inject_pointer_button(BUTTON_LEFT, ButtonState::Released);
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the target to receive the drop",
        |_, clients| clients.target.state.drops == 1,
    );

    let payload = read_dragged_payload(&mut display, &mut compositor, &mut clients);

    assert_eq!(
        payload, DRAG_BYTES,
        "the target did not get the bytes the source offered"
    );
    assert_eq!(
        clients.source.state.payload_requests, 1,
        "the source should have been asked for its payload exactly once"
    );

    settle(&mut display, &mut compositor, &mut clients, 8);
    assert_eq!(
        clients.source.state.drops_performed, 1,
        "a validated drop must be reported to the source"
    );
    assert_eq!(
        clients.source.state.cancellations, 0,
        "a validated drop must not also cancel the drag"
    );

    // --- a drag cancelled by dropping outside every surface ----------------

    let leaves_before = clients.target.state.leaves;
    let drops_before = clients.target.state.drops;

    // This drag is never accepted, so the compositor has nothing to validate the drop
    // against and must cancel it instead.
    clients.target.state.accept_offers = false;

    let point = inside(&compositor, source_id);
    move_pointer(
        &mut display,
        &mut compositor,
        &mut clients,
        point,
        "the pointer to return to the source window",
        |clients| clients.source.state.pointer_enters == 2,
    );
    compositor.inject_pointer_button(BUTTON_LEFT, ButtonState::Pressed);
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the source client to see the second button press",
        |_, clients| clients.source.state.press_serial.is_some(),
    );

    clients.source.start_drag();
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the second drag to take over the pointer",
        |_, clients| clients.source.state.pointer_leaves == 2,
    );

    let point = inside(&compositor, target_id);
    move_pointer(
        &mut display,
        &mut compositor,
        &mut clients,
        point,
        "the second drag to enter the target client",
        |clients| clients.target.state.enters == 2,
    );
    move_pointer(
        &mut display,
        &mut compositor,
        &mut clients,
        outside(),
        "the second drag to leave the target client",
        |clients| clients.target.state.leaves == leaves_before + 1,
    );

    compositor.inject_pointer_button(BUTTON_LEFT, ButtonState::Released);
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the source to see the cancellation",
        |_, clients| clients.source.state.cancellations == 1,
    );

    settle(&mut display, &mut compositor, &mut clients, 8);
    assert_eq!(
        clients.target.state.drops, drops_before,
        "a drag dropped outside every surface must not be delivered"
    );
    assert_eq!(
        clients.target.state.leaves,
        leaves_before + 1,
        "the target must see exactly one leave when the drag moves off it"
    );

    // --- a drag whose target window is destroyed mid-flight ----------------

    let point = inside(&compositor, source_id);
    move_pointer(
        &mut display,
        &mut compositor,
        &mut clients,
        point,
        "the pointer to return to the source window again",
        |clients| clients.source.state.pointer_enters == 3,
    );
    compositor.inject_pointer_button(BUTTON_LEFT, ButtonState::Pressed);
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the source client to see the third button press",
        |_, clients| clients.source.state.press_serial.is_some(),
    );

    clients.source.start_drag();
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the third drag to take over the pointer",
        |_, clients| clients.source.state.pointer_leaves == 3,
    );

    let point = inside(&compositor, target_id);
    move_pointer(
        &mut display,
        &mut compositor,
        &mut clients,
        point,
        "the third drag to enter the target client",
        |clients| clients.target.state.enters == 3,
    );

    // The target window disappears while the drag is over it. The compositor has to
    // survive a drag whose destination is gone: the motion that follows must not be sent
    // to a dead surface, and coming back up must not resurrect the grab.
    clients.target.destroy_toplevel();
    compositor.inject_pointer_motion(outside());
    compositor.inject_pointer_button(BUTTON_LEFT, ButtonState::Released);
    settle(&mut display, &mut compositor, &mut clients, 16);

    assert_eq!(
        compositor.window_count(),
        1,
        "the destroyed target window must be gone"
    );
    assert_eq!(
        compositor.focused_window(),
        Some(source_id),
        "the focus must fall back to the surviving window"
    );

    // The session is still usable: the pointer still reaches the surviving window and a
    // frame still renders.
    let point = inside(&compositor, source_id);
    move_pointer(
        &mut display,
        &mut compositor,
        &mut clients,
        point,
        "the pointer to reach the surviving window",
        |clients| clients.source.state.pointer_enters == 4,
    );
    compositor
        .render_frame()
        .expect("the compositor could not render after a drag lost its target");
}
