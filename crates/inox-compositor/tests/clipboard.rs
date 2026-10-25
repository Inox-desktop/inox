//! Integration test for the M3 clipboard access-control property: only the client that
//! holds the keyboard focus is offered the selection, and it is the only one that can
//! read it.
//!
//! The test drives three real `wayland-client` connections against a compositor state on
//! the current thread (the `window_management.rs` pattern): one client takes the
//! clipboard while it is focused, one is connected but never focused, and one receives
//! the focus afterwards and reads the bytes back through a socket pair.
//!
//! Everything runs on one thread on purpose. The compositor has to be serviced for a
//! client to make progress, so a single thread that alternates between "let every client
//! send what it has" and "service the display" needs no synchronisation, no sleeps to
//! order the steps and no thread that could deadlock. The client sockets are switched to
//! non-blocking mode for the same reason: a read that has nothing to return says so
//! instead of stalling the only thread there is.
//!
//! Everything lives in a single test function: a process may only ever hold one EGL
//! display (see the `SAFETY` note in `src/render.rs`), and test functions inside one
//! binary run in parallel threads.

use std::{
    collections::HashMap,
    io::{Read, Write},
    os::fd::AsFd,
    os::unix::net::UnixStream,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, InoxState};
use smithay::reexports::wayland_server::Display;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
    protocol::{
        wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
        wl_data_source, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long a step may take before the test declares the compositor stuck.
const STEP_TIMEOUT: Duration = Duration::from_secs(20);

/// How long one pump-and-service round waits before it looks again.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

/// The mime type the source offers and the reader asks for.
const CLIPBOARD_MIME: &str = "text/plain;charset=utf-8";

/// The payload the source writes and the reader must get back.
const CLIPBOARD_BYTES: &[u8] = b"inox clipboard payload";

/// Service the display once: dispatch client requests and flush outgoing events.
fn service(display: &mut Display<InoxState>, compositor: &mut InoxState) {
    // Errors during teardown (a peer that went away) are not interesting here; the
    // assertions below are what the test is about.
    let _ = display.dispatch_clients(compositor);
    let _ = display.flush_clients();
}

/// The client-side state every connection keeps.
#[derive(Default)]
struct Client {
    /// Registry contents, as `interface -> (name, version)`.
    globals: HashMap<String, (u32, u32)>,
    /// The offer the compositor sent, if this client was offered the selection.
    offer: Option<wl_data_offer::WlDataOffer>,
    /// The mime types advertised on that offer.
    mime_types: Vec<String>,
    /// The data source this client set, kept alive while it owns the selection.
    source: Option<wl_data_source::WlDataSource>,
    /// Whether the source was asked to write its payload.
    sent: bool,
}

impl Client {
    /// Whether this client has been offered the selection at all.
    fn offered(&self) -> bool {
        self.offer.is_some()
    }
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
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Client: ignore wl_data_device_manager::WlDataDeviceManager);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(Client: ignore xdg_surface::XdgSurface);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);

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
            wl_data_device::Event::DataOffer { id } => state.offer = Some(id),
            wl_data_device::Event::Selection { id } => state.offer = id,
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
        // The compositor hands the source client the write end of a socket pair the reader
        // created; writing the payload and closing it is the whole transfer.
        if let wl_data_source::Event::Send { mime_type, fd } = event {
            assert_eq!(mime_type, CLIPBOARD_MIME);
            state.sent = true;

            let mut writer = std::fs::File::from(fd);
            writer
                .write_all(CLIPBOARD_BYTES)
                .expect("failed to write the clipboard payload");
            // Dropping the writer closes the reader's peer, which is its end-of-file.
            drop(writer);
        }
    }
}

/// Attach a small `wl_shm` buffer and commit, so the toplevel draws something.
///
/// The size is irrelevant to the clipboard assertions; what matters is that the
/// compositor has a buffer to map, because a toplevel without one is not a window.
fn attach_test_buffer(
    shm: &wl_shm::WlShm,
    surface: &wl_surface::WlSurface,
    queue: &QueueHandle<Client>,
) -> (wl_buffer::WlBuffer, wl_shm_pool::WlShmPool) {
    const SIZE: i32 = 16;
    let stride = SIZE * 4;
    let file = tempfile::tempfile().expect("failed to create the buffer file");
    file.set_len((stride * SIZE) as u64)
        .expect("failed to size the buffer file");
    let pool = shm.create_pool(file.as_fd(), stride * SIZE, queue, ());
    let buffer = pool.create_buffer(0, SIZE, SIZE, stride, wl_shm::Format::Argb8888, queue, ());
    surface.attach(Some(&buffer), 0, 0);
    surface.commit();
    (buffer, pool)
}

/// One client connection plus the protocol objects it bound.
struct TestClient {
    connection: Connection,
    queue: EventQueue<Client>,
    state: Client,
    registry: wl_registry::WlRegistry,
    compositor: Option<wl_compositor::WlCompositor>,
    xdg: Option<xdg_wm_base::XdgWmBase>,
    manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    shm: Option<wl_shm::WlShm>,
    device: Option<wl_data_device::WlDataDevice>,
    surface: Option<wl_surface::WlSurface>,
    _buffer: Option<wl_buffer::WlBuffer>,
    _pool: Option<wl_shm_pool::WlShmPool>,
    _seat: Option<wl_seat::WlSeat>,
    _xdg_surface: Option<xdg_surface::XdgSurface>,
    _toplevel: Option<xdg_toplevel::XdgToplevel>,
}

impl TestClient {
    /// Open a connection to the in-process compositor and ask for the registry.
    fn connect(display_handle: &mut smithay::reexports::wayland_server::DisplayHandle) -> Self {
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
            manager: None,
            shm: None,
            device: None,
            surface: None,
            _buffer: None,
            _pool: None,
            _seat: None,
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

    /// Bind every global this test needs.
    fn bind_all(&mut self) {
        self.compositor = Some(self.bind::<wl_compositor::WlCompositor, ()>(4));
        self.xdg = Some(self.bind::<xdg_wm_base::XdgWmBase, ()>(6));
        self.manager = Some(self.bind::<wl_data_device_manager::WlDataDeviceManager, ()>(3));
        self.shm = Some(self.bind::<wl_shm::WlShm, ()>(1));

        let seat: wl_seat::WlSeat = self.bind::<wl_seat::WlSeat, ()>(8);
        let manager = self.manager.clone().expect("the manager was just bound");
        self.device = Some(manager.get_data_device(&seat, &self.queue.handle(), ()));
        self._seat = Some(seat);
    }

    /// Map an `xdg_toplevel`, which is what makes this client a candidate for the focus.
    fn map_toplevel(&mut self) {
        let compositor = self.compositor.clone().expect("bind_all has not run");
        let xdg = self.xdg.clone().expect("bind_all has not run");
        let queue = self.queue.handle();

        let surface = compositor.create_surface(&queue, ());
        let xdg_surface = xdg.get_xdg_surface(&surface, &queue, ());
        let toplevel = xdg_surface.get_toplevel(&queue, ());
        surface.commit();

        // A toplevel becomes a window only when it draws: attach a real `wl_shm`
        // buffer, which is what makes this client a focus candidate at all.
        let shm = self.shm.clone().expect("bind_all has not run");
        let (buffer, pool) = attach_test_buffer(&shm, &surface, &queue);
        self._buffer = Some(buffer);
        self._pool = Some(pool);

        self.surface = Some(surface);
        self._xdg_surface = Some(xdg_surface);
        self._toplevel = Some(toplevel);
    }

    /// Take the clipboard with `mime`, owning the payload through a data source.
    fn take_selection(&mut self, mime: &str) {
        let manager = self.manager.clone().expect("bind_all has not run");
        let device = self.device.clone().expect("bind_all has not run");

        let source = manager.create_data_source(&self.queue.handle(), ());
        source.offer(mime.to_owned());
        device.set_selection(Some(&source), 0);

        self.state.source = Some(source);
    }
}

/// The three clients the scenario needs.
struct Clients {
    /// Takes the clipboard and answers the transfer.
    source: TestClient,
    /// Never focused, and therefore never offered anything.
    idle: TestClient,
    /// Focused after the selection was set, and reads it back.
    reader: TestClient,
}

impl Clients {
    /// Every client, mutably.
    fn all_mut(&mut self) -> [&mut TestClient; 3] {
        [&mut self.source, &mut self.idle, &mut self.reader]
    }

    /// Every client, for the read-only predicates the waits are built from.
    fn all(&self) -> [&TestClient; 3] {
        [&self.source, &self.idle, &self.reader]
    }

    fn pump_all(&mut self) {
        for client in self.all_mut() {
            client.pump();
        }
    }

    /// Bind the globals of every client, once the registry has been delivered.
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

/// Run `rounds` pump-and-service rounds, so anything the compositor already queued is
/// delivered without waiting for a condition that may never become true.
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

/// Ask the reader for the clipboard and return what the source wrote.
///
/// The transfer only completes if the compositor forwards the request and the source
/// client answers it, so this keeps the whole compositor and every client running until
/// the reader sees the end of the data.
fn read_clipboard(
    display: &mut Display<InoxState>,
    compositor: &mut InoxState,
    clients: &mut Clients,
    mime: &str,
) -> Vec<u8> {
    let offer = clients
        .reader
        .state
        .offer
        .clone()
        .expect("the reader was never offered the selection");

    // A socket pair stands in for the pipe a real client would use; the compositor only
    // ever forwards the write end to the source.
    let (mut read_end, write_end) =
        UnixStream::pair().expect("failed to create the transfer socket");
    offer.receive(mime.to_owned(), write_end.as_fd());
    // The reader must not hold the write end open, or it would never see end-of-file.
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
            Ok(0) => return bytes,
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            // A read timeout is the normal case while the transfer is still in flight; the
            // compositor is serviced below on every round either way.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => panic!("failed to read the clipboard: {error}"),
        }

        clients.pump_all();
        service(display, compositor);

        assert!(
            Instant::now() < deadline,
            "timed out after {STEP_TIMEOUT:?} reading the clipboard"
        );
    }
}

#[test]
fn only_the_focused_client_is_offered_the_clipboard() {
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
        idle: TestClient::connect(&mut display_handle),
        reader: TestClient::connect(&mut display_handle),
    };

    // --- the registry ------------------------------------------------------

    // Every client binds its data device before anything is focused: `wl_data_device`
    // offers are only ever sent on a *change* of focus or selection, so a device created
    // after its client was focused would never see the current selection.
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "every client to learn the registry",
        |_, clients| {
            clients
                .all()
                .iter()
                .all(|client| client.state.globals.contains_key("wl_data_device_manager"))
        },
    );
    clients.bind_all();
    settle(&mut display, &mut compositor, &mut clients, 4);

    // --- the source takes the clipboard while it is focused ----------------

    clients.source.map_toplevel();
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the source client window",
        |compositor, _| compositor.window_count() == 1,
    );

    clients.source.take_selection(CLIPBOARD_MIME);
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the source client to see its own selection",
        |_, clients| clients.source.state.offered(),
    );

    // The connected-but-unfocused client is offered nothing, no matter how long the
    // compositor runs: this is the access-control property, not a timing artefact.
    settle(&mut display, &mut compositor, &mut clients, 16);
    assert!(
        !clients.idle.state.offered(),
        "a client without the keyboard focus was offered the selection"
    );
    assert!(
        !clients.reader.state.offered(),
        "an unfocused client was offered the selection"
    );

    // --- the reader takes the focus and receives the offer -----------------

    clients.reader.map_toplevel();
    pump_until(
        &mut display,
        &mut compositor,
        &mut clients,
        "the reader client to be offered the selection",
        |_, clients| !clients.reader.state.mime_types.is_empty(),
    );

    assert!(
        clients
            .reader
            .state
            .mime_types
            .iter()
            .any(|mime| mime == CLIPBOARD_MIME),
        "the reader was not offered {CLIPBOARD_MIME}, only {:?}",
        clients.reader.state.mime_types
    );

    // --- the transfer ------------------------------------------------------

    let bytes = read_clipboard(&mut display, &mut compositor, &mut clients, CLIPBOARD_MIME);

    assert!(
        clients.source.state.sent,
        "the source was never asked for its payload"
    );
    assert_eq!(
        bytes, CLIPBOARD_BYTES,
        "the reader did not get the bytes the source offered"
    );

    // The unfocused client still has nothing, even though everything else has run.
    settle(&mut display, &mut compositor, &mut clients, 8);
    assert!(
        !clients.idle.state.offered(),
        "an unfocused client was offered the selection"
    );
}
