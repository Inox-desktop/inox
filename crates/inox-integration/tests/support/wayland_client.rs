//! Real Wayland clients for the M3 end-to-end tests.
//!
//! These are ordinary `wayland-client` clients speaking the wire protocol to
//! the Inox compositor: no third-party toolkit, no convenience crate, and one
//! `Connection` per client (several clients per test process, each with its own
//! event queue, is fine and is what the clipboard and drag tests need).
//!
//! The plumbing is deliberately explicit, because the M3 evidence depends on
//! *which* protocol events arrive:
//!
//! * a `xdg_toplevel` is mapped with a solid-colour `wl_shm` buffer, so a
//!   `dump-frame` can prove the surface is really composited;
//! * clipboard reads pass the write end of a `UnixStream::pair()` as the fd, so
//!   a transfer is a real pipe and not a mock;
//! * drag and drop is driven through `wl_data_device`/`wl_data_source` with the
//!   serial of an injected pointer press, which is the only serial the
//!   compositor accepts for `start_drag`.
//!
//! Nothing here is a mock of the code under test: the compositor decides the
//! layout, the focus and the offers, and the tests assert on what the clients
//! observe.

#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::fd::AsFd,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    globals::{GlobalList, GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
        wl_data_source, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
        wl_surface,
    },
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use super::{CASE_TIMEOUT, POLL_INTERVAL};

/// Index of a toplevel inside a [`WaylandClient`].
pub type ToplevelId = usize;
/// Index of a data offer inside a [`WaylandClient`].
pub type OfferId = usize;
/// Index of a data source inside a [`WaylandClient`].
pub type SourceId = usize;

/// How a `wl_data_offer` reached this client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferKind {
    /// Offered as the current selection to a focused client.
    Selection,
    /// Offered as the target of a drag and drop.
    Drag,
}

/// One observation made on a client's connection, in arrival order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The compositor configured a toplevel with a concrete size.
    Configure {
        /// Which toplevel.
        toplevel: ToplevelId,
        /// Width in logical pixels.
        width: i32,
        /// Height in logical pixels.
        height: i32,
    },
    /// The compositor asked the toplevel to close.
    Close {
        /// Which toplevel.
        toplevel: ToplevelId,
    },
    /// The keyboard focus entered one of this client's surfaces.
    KeyboardEnter {
        /// The serial to use for selection requests.
        serial: u32,
        /// Which toplevel.
        toplevel: ToplevelId,
    },
    /// The keyboard focus left.
    KeyboardLeave {
        /// Which toplevel.
        toplevel: ToplevelId,
    },
    /// A key reached this client.
    Key {
        /// XKB/evdev-style key code as sent by the compositor.
        key: u32,
        /// Whether the key was pressed (rather than released).
        pressed: bool,
    },
    /// The pointer entered one of this client's surfaces.
    PointerEnter {
        /// Which toplevel.
        toplevel: ToplevelId,
    },
    /// The pointer left.
    PointerLeave {
        /// Which toplevel.
        toplevel: ToplevelId,
    },
    /// A pointer button event, with the serial a drag must quote.
    PointerButton {
        /// The serial of the event.
        serial: u32,
        /// The button code.
        button: u32,
        /// Whether it was pressed.
        pressed: bool,
    },
    /// The selection was offered to this client (it holds the focus).
    SelectionOffered {
        /// Which offer.
        offer: OfferId,
    },
    /// The selection was cleared, or this client lost the focus.
    SelectionCleared,
    /// A drag entered this client's surface.
    DragEnter {
        /// Which offer.
        offer: OfferId,
    },
    /// A drag moved over this client's surface.
    DragMotion,
    /// A drag left this client's surface.
    DragLeave,
    /// The drag was dropped on this client's surface.
    DragDrop,
    /// Source side: the destination asked which mime should be transferred.
    SourceTarget {
        /// The mime the destination prefers, if any.
        mime: Option<String>,
    },
    /// Source side: this client was asked to send data.
    SourceSend {
        /// The mime type that was sent.
        mime: String,
        /// How many bytes were written to the pipe.
        bytes: usize,
    },
    /// Source side: the drag was cancelled.
    SourceCancelled,
    /// Source side: the destination performed the drop.
    SourceDropPerformed,
    /// Source side: the destination finished the transfer.
    SourceFinished,
    /// Source side: the negotiated action changed.
    SourceAction {
        /// The action name.
        action: String,
    },
}

/// A mapped `xdg_toplevel` with a solid-colour `wl_shm` buffer.
pub struct Toplevel {
    /// The toplevel surface.
    pub surface: wl_surface::WlSurface,
    /// Its `xdg_surface` role holder.
    pub xdg_surface: xdg_surface::XdgSurface,
    /// The toplevel itself.
    pub toplevel: xdg_toplevel::XdgToplevel,
    /// The colour every committed buffer is filled with.
    pub colour: [u8; 3],
    /// The size the compositor last configured.
    pub configured: Option<(i32, i32)>,
    /// Size used before the first configure arrives.
    initial: (i32, i32),
    /// Kept alive so the compositor's mapping stays valid.
    _buffer: Option<wl_buffer::WlBuffer>,
    /// Backing file of the current buffer.
    _file: Option<File>,
}

/// A `wl_data_offer` this client received.
pub struct Offer {
    /// The offer object.
    pub proxy: wl_data_offer::WlDataOffer,
    /// How the offer arrived.
    pub kind: OfferKind,
    /// The mime types the source advertises.
    pub mimes: Vec<String>,
    /// Whether a drop has been observed on this offer.
    pub dropped: bool,
}

/// A `wl_data_source` this client owns.
pub struct Source {
    /// The source object.
    pub proxy: wl_data_source::WlDataSource,
    /// The mime types offered, in the order they were announced.
    pub mimes: Vec<String>,
    /// The bytes to answer a `send` with, per mime type.
    pub payload: BTreeMap<String, Vec<u8>>,
    /// The last mime the destination reported through `target`.
    pub target: Option<String>,
}

/// The dispatch state of one client connection.
pub struct ClientState {
    /// Advertised globals: interface name -> version.
    pub globals: BTreeMap<String, u32>,
    /// Bound core globals.
    pub compositor: wl_compositor::WlCompositor,
    /// Bound shared-memory global.
    pub shm: wl_shm::WlShm,
    /// Bound `xdg_wm_base`.
    pub wm_base: xdg_wm_base::XdgWmBase,
    /// Bound seat.
    pub seat: wl_seat::WlSeat,
    /// Bound data device manager.
    pub data_manager: wl_data_device_manager::WlDataDeviceManager,
    /// This client's data device.
    pub data_device: wl_data_device::WlDataDevice,
    /// The keyboard, once the seat announces it.
    pub keyboard: Option<wl_keyboard::WlKeyboard>,
    /// The pointer, once the seat announces it.
    pub pointer: Option<wl_pointer::WlPointer>,
    /// Every toplevel this client created.
    pub toplevels: Vec<Toplevel>,
    /// Every offer this client received.
    pub offers: Vec<Offer>,
    /// Every source this client created.
    pub sources: Vec<Source>,
    /// The observations, in arrival order.
    pub events: Vec<Event>,
    /// The serial from the most recent keyboard enter.
    pub keyboard_serial: Option<u32>,
    /// Directory for `wl_shm` backing files.
    pub scratch: PathBuf,
    /// A label unique to this client connection, used to make the backing files
    /// unique: several clients live in one test process, so the process id alone
    /// does not separate them.
    client: String,
    /// Counter for unique backing file names.
    counter: usize,
}

impl ClientState {
    /// The toplevel index whose surface is `surface`.
    fn toplevel_of(&self, surface: &wl_surface::WlSurface) -> Option<ToplevelId> {
        self.toplevels
            .iter()
            .position(|candidate| candidate.surface.id() == surface.id())
    }

    /// The offer index for `offer`.
    fn offer_of(&self, offer: &wl_data_offer::WlDataOffer) -> Option<OfferId> {
        self.offers
            .iter()
            .position(|candidate| candidate.proxy.id() == offer.id())
    }

    fn source_of(&self, source: &wl_data_source::WlDataSource) -> Option<SourceId> {
        self.sources
            .iter()
            .position(|candidate| candidate.proxy.id() == source.id())
    }

    /// Create a solid-colour `wl_shm` buffer and attach it to `surface`.
    fn attach_solid(&mut self, qh: &QueueHandle<Self>, index: ToplevelId, width: i32, height: i32) {
        let width = width.max(1);
        let height = height.max(1);
        let colour = self.toplevels[index].colour;
        let stride = width * 4;
        let size = stride * height;

        self.counter += 1;
        // Unique per client *and* per attach. Two clients in the same test
        // process share a pid, so a pid-based name would make them truncate each
        // other's backing file — and a truncate under a live pool mapping makes
        // the compositor fault with "Bad pool size" (SIGBUS).
        let path = self.scratch.join(format!(
            "wl-{}-{}-{}.shm",
            self.client,
            std::process::id(),
            self.counter
        ));
        // Opened read *and* write: the compositor maps this fd for reading, and a
        // write-only fd cannot be mapped with `PROT_READ` (the compositor answers
        // such a pool with the protocol error "Failed to mmap fd").
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("failed to create {}: {error}", path.display()));
        file.set_len(size as u64)
            .unwrap_or_else(|error| panic!("failed to size {}: {error}", path.display()));

        // `Xrgb8888` is little endian `0x00RRGGBB`, so the bytes are B, G, R, X.
        let mut row = Vec::with_capacity(stride as usize);
        for _ in 0..width {
            row.extend_from_slice(&[colour[2], colour[1], colour[0], 0xff]);
        }
        for _ in 0..height {
            file.write_all(&row)
                .unwrap_or_else(|error| panic!("failed to fill {}: {error}", path.display()));
        }
        file.flush().ok();
        // Keep the file handle: the compositor maps the fd, and a handle kept
        // open here is what makes the mapping's lifetime obvious.
        file.seek(SeekFrom::Start(0)).ok();

        let pool = self.shm.create_pool(file.as_fd(), size, qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, wl_shm::Format::Xrgb8888, qh, ());
        pool.destroy();

        let surface = self.toplevels[index].surface.clone();
        surface.attach(Some(&buffer), 0, 0);
        if surface.version() >= 4 {
            surface.damage_buffer(0, 0, width, height);
        } else {
            surface.damage(0, 0, width, height);
        }
        surface.commit();

        self.toplevels[index]._buffer = Some(buffer);
        self.toplevels[index]._file = Some(file);
    }
}

/// A Wayland client owned by the test process.
pub struct WaylandClient {
    /// The connection, kept so the socket itself stays alive.
    conn: Connection,
    queue: EventQueue<ClientState>,
    /// Dispatch state.
    pub state: ClientState,
    /// A human-readable label, used in failure messages.
    pub name: String,
}

impl WaylandClient {
    /// Connect to `socket` and bind every global these tests need.
    pub fn connect(socket: &Path, scratch: &Path, name: &str) -> Self {
        let stream = UnixStream::connect(socket).unwrap_or_else(|error| {
            panic!("{name}: failed to connect to {}: {error}", socket.display())
        });
        let conn = Connection::from_socket(stream)
            .unwrap_or_else(|error| panic!("{name}: failed to create a connection: {error}"));
        let (globals, queue) = registry_queue_init::<ClientState>(&conn)
            .unwrap_or_else(|error| panic!("{name}: registry setup failed: {error}"));
        let qh = queue.handle();
        let state = ClientState::bind(&globals, &qh, scratch, name);

        let mut client = Self {
            conn,
            queue,
            state,
            name: name.to_owned(),
        };
        // Flush the binds so later round trips see the objects they depend on.
        client.poll(POLL_INTERVAL);
        client
    }

    /// The version this client bound `interface` with, or `None` if the
    /// compositor does not advertise it.
    pub fn bound_version(&self, interface: &str) -> Option<u32> {
        self.state.globals.get(interface).copied()
    }

    /// Create and map a toplevel with a solid-colour buffer.
    ///
    /// The client acks the compositor's configure and commits a buffer of the
    /// configured size on its own, so the caller only has to wait for the
    /// [`Event::Configure`] observation.
    pub fn create_toplevel(&mut self, colour: [u8; 3], initial: (i32, i32)) -> ToplevelId {
        let qh = self.queue.handle();
        let surface = self.state.compositor.create_surface(&qh, ());
        let xdg_surface = self.state.wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        toplevel.set_title(format!(
            "{} window {}",
            self.name,
            self.state.toplevels.len()
        ));

        let index = self.state.toplevels.len();
        self.state.toplevels.push(Toplevel {
            surface,
            xdg_surface,
            toplevel,
            colour,
            configured: None,
            initial,
            _buffer: None,
            _file: None,
        });
        self.state.toplevels[index].surface.commit();
        self.poll(POLL_INTERVAL);
        index
    }

    /// The size the compositor last configured for `toplevel`.
    pub fn configured_size(&self, toplevel: ToplevelId) -> Option<(i32, i32)> {
        self.state.toplevels[toplevel].configured
    }

    /// The serial from the most recent keyboard enter, needed to request the
    /// selection or to start a drag.
    pub fn keyboard_serial(&self) -> Option<u32> {
        self.state.keyboard_serial
    }

    /// The offer this client currently holds as the selection, if any.
    ///
    /// The selection event is the last word: once the compositor has told this
    /// client the selection was cleared (which is what a client that loses the
    /// focus sees), an earlier offer is stale and must not be returned.
    pub fn selection_offer(&self) -> Option<OfferId> {
        for event in self.state.events.iter().rev() {
            match event {
                Event::SelectionOffered { offer } => return Some(*offer),
                Event::SelectionCleared => return None,
                _ => {}
            }
        }
        None
    }

    /// Wait until the compositor configures `toplevel`, returning its size.
    pub fn wait_for_configure(&mut self, toplevel: ToplevelId, what: &str) -> (i32, i32) {
        let deadline = Instant::now() + CASE_TIMEOUT;
        self.wait_until(deadline, what, |state| {
            state
                .toplevels
                .get(toplevel)
                .is_some_and(|entry| entry.configured.is_some())
        });
        self.configured_size(toplevel).expect("just waited for it")
    }

    /// Wait until the compositor configures `toplevel` to exactly `size`.
    ///
    /// A client that is re-tiled when another window maps only sees the new size
    /// once its own connection is dispatched, so a test that asserts on the tile
    /// has to wait for its own configure, not just for the other client's.
    pub fn wait_for_size(&mut self, toplevel: ToplevelId, size: (i32, i32), what: &str) {
        let deadline = Instant::now() + CASE_TIMEOUT;
        let what = format!("{what} (window {toplevel} configured to {size:?})");
        self.wait_until(deadline, &what, |state| {
            state
                .toplevels
                .get(toplevel)
                .is_some_and(|entry| entry.configured == Some(size))
        });
    }

    /// Wait until this client holds the keyboard focus.
    pub fn wait_for_focus(&mut self, what: &str) -> u32 {
        let deadline = Instant::now() + CASE_TIMEOUT;
        self.wait_until(deadline, what, |state| {
            state.keyboard_serial.is_some()
                && state
                    .events
                    .iter()
                    .rev()
                    .any(|event| matches!(event, Event::KeyboardEnter { .. }))
        });
        self.keyboard_serial().expect("just waited for it")
    }

    /// Which toplevel currently holds this client's keyboard focus.
    pub fn focused_toplevel(&self) -> Option<ToplevelId> {
        for event in self.state.events.iter().rev() {
            match event {
                Event::KeyboardEnter { toplevel, .. } => return Some(*toplevel),
                Event::KeyboardLeave { .. } => return None,
                _ => {}
            }
        }
        None
    }

    /// Every offer of the given kind this client received, in arrival order.
    pub fn offers_of(&self, kind: OfferKind) -> Vec<OfferId> {
        self.state
            .offers
            .iter()
            .enumerate()
            .filter(|(_, offer)| offer.kind == kind)
            .map(|(index, _)| index)
            .collect()
    }

    /// The mime types an offer advertises.
    pub fn offer_mimes(&self, offer: OfferId) -> &[String] {
        &self.state.offers[offer].mimes
    }

    /// How many [`Event::DragDrop`] observations this client made.
    pub fn drop_count(&self) -> usize {
        self.count(|event| matches!(event, Event::DragDrop))
    }

    /// How many events satisfy `predicate`.
    pub fn count(&self, predicate: impl Fn(&Event) -> bool) -> usize {
        self.state
            .events
            .iter()
            .filter(|event| predicate(event))
            .count()
    }

    /// The keys that reached this client, in arrival order.
    pub fn keys(&self) -> Vec<(u32, bool)> {
        self.state
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Key { key, pressed } => Some((*key, *pressed)),
                _ => None,
            })
            .collect()
    }

    /// The source-side terminal events seen so far.
    ///
    /// A drag ends for the source exactly once: with `cancelled` when the drop
    /// was not accepted, with `drop_performed` when it was (and then `finished`
    /// once the destination confirms the transfer). Listing them lets the tests
    /// assert "exactly one terminal event" without guessing which one.
    pub fn terminal_events(&self) -> Vec<Event> {
        self.state
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    Event::SourceCancelled | Event::SourceDropPerformed | Event::SourceFinished
                )
            })
            .cloned()
            .collect()
    }

    /// Create a source offering `mimes`, answering a `send` with `payload`.
    ///
    /// The same payload is served for every mime type; the clipboard tests use
    /// one payload per mime by creating one source per mime when they need to
    /// distinguish them.
    pub fn create_source(&mut self, mimes: &[&str], payload: &[u8]) -> SourceId {
        self.create_source_with(mimes.iter().map(|mime| (*mime, payload.to_vec())))
    }

    /// Create a source with a distinct payload per mime type.
    pub fn create_source_with<'a>(
        &mut self,
        payloads: impl IntoIterator<Item = (&'a str, Vec<u8>)>,
    ) -> SourceId {
        let qh = self.queue.handle();
        let proxy = self.state.data_manager.create_data_source(&qh, ());
        let mut mimes = Vec::new();
        let mut payload = BTreeMap::new();
        for (mime, bytes) in payloads {
            proxy.offer(mime.to_owned());
            mimes.push(mime.to_owned());
            payload.insert(mime.to_owned(), bytes);
        }

        let index = self.state.sources.len();
        self.state.sources.push(Source {
            proxy,
            mimes,
            payload,
            target: None,
        });
        self.poll(POLL_INTERVAL);
        index
    }

    /// Make `source` the selection, using the serial of this client's keyboard
    /// enter (the compositor rejects a stale or foreign serial).
    pub fn set_selection(&mut self, source: SourceId, serial: u32) {
        let device = self.state.data_device.clone();
        let proxy = self.state.sources[source].proxy.clone();
        // Holding the selection is a `wl_data_device` request, not a
        // `wl_data_source` one.
        device.set_selection(Some(&proxy), serial);
        self.poll(POLL_INTERVAL);
    }

    /// Announce the drag-and-drop actions this source supports.
    pub fn source_set_actions(&mut self, source: SourceId, actions: u32) {
        let proxy = self.state.sources[source].proxy.clone();
        if proxy.version() >= 3 {
            proxy.set_actions(wl_data_device_manager::DndAction::from_bits_truncate(
                actions,
            ));
        }
        self.poll(POLL_INTERVAL);
    }

    /// Start a drag of `source` using `serial`, the serial of the pointer press
    /// that began the interaction.
    pub fn start_drag(
        &mut self,
        source: SourceId,
        origin: ToplevelId,
        icon: Option<ToplevelId>,
        serial: u32,
    ) {
        let device = self.state.data_device.clone();
        let proxy = self.state.sources[source].proxy.clone();
        let origin_surface = self.state.toplevels[origin].surface.clone();
        let icon_surface = icon.map(|index| self.state.toplevels[index].surface.clone());
        device.start_drag(Some(&proxy), &origin_surface, icon_surface.as_ref(), serial);
        self.poll(POLL_INTERVAL);
    }

    /// Accept a drag offer for `mime`, quoting the drag-and-drop serial, and
    /// choose `action`.
    pub fn accept_drag(&mut self, offer: OfferId, serial: u32, mime: Option<&str>, action: u32) {
        let proxy = self.state.offers[offer].proxy.clone();
        if proxy.version() >= 3 {
            let action = wl_data_device_manager::DndAction::from_bits_truncate(action);
            proxy.set_actions(action, action);
        }
        proxy.accept(serial, mime.map(str::to_owned));
        self.poll(POLL_INTERVAL);
    }

    /// Refuse a drag offer: accept `None`, which is how the protocol says "this
    /// mime is not acceptable here".
    pub fn reject_drag(&mut self, offer: OfferId, serial: u32) {
        let proxy = self.state.offers[offer].proxy.clone();
        proxy.accept(serial, None);
        self.poll(POLL_INTERVAL);
    }

    /// Build the read end of a transfer from `offer` for `mime`.
    ///
    /// Returns a handle that yields the bytes once whoever owns the *source*
    /// has dispatched its `send` request; the caller drives that client in the
    /// meantime (see [`transfer`]).
    pub fn request_data(&mut self, offer: OfferId, mime: &str) -> Transfer {
        let (reader, writer) = UnixStream::pair().expect("socketpair must succeed");
        let proxy = self.state.offers[offer].proxy.clone();
        proxy.receive(mime.to_owned(), writer.as_fd());
        drop(writer);
        self.poll(POLL_INTERVAL);

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = reader;
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).ok();
            sender.send(bytes).ok();
        });
        Transfer { receiver }
    }

    /// Finish a drag offer, which is what makes the source see `dnd_finished`.
    pub fn finish_offer(&mut self, offer: OfferId) {
        let proxy = self.state.offers[offer].proxy.clone();
        proxy.finish();
        proxy.destroy();
        self.poll(POLL_INTERVAL);
    }

    /// Destroy a toplevel and its surface (the client-side equivalent of the
    /// window being closed).
    pub fn destroy_toplevel(&mut self, toplevel: ToplevelId) {
        let entry = &mut self.state.toplevels[toplevel];
        if let Some(buffer) = entry._buffer.take() {
            buffer.destroy();
        }
        entry.toplevel.destroy();
        entry.xdg_surface.destroy();
        entry.surface.destroy();
        self.poll(POLL_INTERVAL);
    }

    /// Dispatch everything already queued and flush pending requests.
    pub fn dispatch(&mut self) {
        let _ = self.queue.dispatch_pending(&mut self.state);
        let _ = self.queue.flush();
    }

    /// Wait up to `timeout` for something to read, then dispatch it.
    ///
    /// The wait is a real `poll(2)` on the connection's socket rather than
    /// `blocking_dispatch`, so a compositor that stops answering cannot hang the
    /// caller: the timeout always returns.
    pub fn poll(&mut self, timeout: Duration) {
        self.dispatch();
        if let Some(guard) = self.conn.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let timespec = Timespec {
                tv_sec: timeout.as_secs() as i64,
                tv_nsec: i64::from(timeout.subsec_nanos()),
            };
            let ready = matches!(poll(&mut fds, Some(&timespec)), Ok(count) if count > 0);
            if ready {
                let _ = guard.read();
            }
        }
        self.dispatch();
    }

    /// Poll until `predicate` holds, failing the test on timeout.
    ///
    /// Panics with the observed event log, which is the only useful thing to
    /// look at when a protocol expectation was not met.
    pub fn wait_until(
        &mut self,
        deadline: Instant,
        what: &str,
        predicate: impl Fn(&ClientState) -> bool,
    ) {
        loop {
            if predicate(&self.state) {
                return;
            }
            if Instant::now() >= deadline {
                panic!(
                    "{}: timed out waiting for {what}; observed events: {:#?}",
                    self.name, self.state.events
                );
            }
            self.poll(POLL_INTERVAL);
        }
    }

    /// Wait until at least `counts` more events match `predicate`.
    pub fn wait_for_events(
        &mut self,
        what: &str,
        timeout: Duration,
        predicate: impl Fn(&Event) -> bool,
        counts: usize,
    ) {
        let deadline = Instant::now() + timeout;
        let predicate = &predicate;
        self.wait_until(deadline, what, |state| {
            state.events.iter().filter(|event| predicate(event)).count() >= counts
        });
    }

    /// Read the bytes of `offer` for `mime`, driving this client while waiting.
    ///
    /// Use this when this client *is* the source; when another client owns the
    /// source, use [`transfer`] so both connections are driven.
    pub fn read_offer(&mut self, offer: OfferId, mime: &str, timeout: Duration) -> Vec<u8> {
        let transfer = self.request_data(offer, mime);
        let deadline = Instant::now() + timeout;
        loop {
            self.poll(POLL_INTERVAL);
            if let Some(bytes) = transfer.take() {
                return bytes;
            }
            assert!(
                Instant::now() < deadline,
                "{}: timed out reading {mime} from offer {offer}; events: {:#?}",
                self.name,
                self.state.events
            );
        }
    }
}

/// A pending clipboard or drag-and-drop transfer.
pub struct Transfer {
    receiver: mpsc::Receiver<Vec<u8>>,
}

impl Transfer {
    /// The bytes, once the source has closed its end of the pipe.
    pub fn take(&self) -> Option<Vec<u8>> {
        self.receiver.try_recv().ok()
    }
}

/// Move `bytes` from `source` to `receiver`, driving both connections.
///
/// Every transfer in these tests is a real pipe: the receiver sends
/// `wl_data_offer.receive` with the write end, the source answers with
/// `wl_data_source.send`, and both clients have to be dispatched for the bytes
/// to move. `timeout` bounds the whole exchange.
pub fn transfer(
    source: &mut WaylandClient,
    receiver: &mut WaylandClient,
    offer: OfferId,
    mime: &str,
    timeout: Duration,
) -> Vec<u8> {
    let pending = receiver.request_data(offer, mime);
    let deadline = Instant::now() + timeout;
    loop {
        source.poll(POLL_INTERVAL);
        receiver.poll(POLL_INTERVAL);
        if let Some(bytes) = pending.take() {
            return bytes;
        }
        assert!(
            Instant::now() < deadline,
            "timed out reading {mime} from {} to {}; receiver events: {:#?}; source events: {:#?}",
            source.name,
            receiver.name,
            receiver.state.events,
            source.state.events
        );
    }
}

impl ClientState {
    /// Bind the globals the M3 tests need and record every advertised global.
    fn bind(globals: &GlobalList, qh: &QueueHandle<Self>, scratch: &Path, name: &str) -> Self {
        let advertised: BTreeMap<String, u32> = globals
            .contents()
            .clone_list()
            .into_iter()
            .map(|global| (global.interface, global.version))
            .collect();

        let compositor = globals
            .bind::<wl_compositor::WlCompositor, Self, ()>(qh, 4..=6, ())
            .unwrap_or_else(|error| panic!("{name}: cannot bind wl_compositor: {error}"));
        let shm = globals
            .bind::<wl_shm::WlShm, Self, ()>(qh, 1..=1, ())
            .unwrap_or_else(|error| panic!("{name}: cannot bind wl_shm: {error}"));
        let wm_base = globals
            .bind::<xdg_wm_base::XdgWmBase, Self, ()>(qh, 2..=6, ())
            .unwrap_or_else(|error| panic!("{name}: cannot bind xdg_wm_base: {error}"));
        let seat = globals
            .bind::<wl_seat::WlSeat, Self, ()>(qh, 1..=7, ())
            .unwrap_or_else(|error| panic!("{name}: cannot bind wl_seat: {error}"));
        let data_manager = globals
            .bind::<wl_data_device_manager::WlDataDeviceManager, Self, ()>(qh, 3..=3, ())
            .unwrap_or_else(|error| panic!("{name}: cannot bind wl_data_device_manager: {error}"));

        let data_device = data_manager.get_data_device(&seat, qh, ());

        Self {
            globals: advertised,
            compositor,
            shm,
            wm_base,
            seat,
            data_manager,
            data_device,
            keyboard: None,
            pointer: None,
            toplevels: Vec::new(),
            offers: Vec::new(),
            sources: Vec::new(),
            events: Vec::new(),
            keyboard_serial: None,
            scratch: scratch.to_path_buf(),
            client: name.to_owned(),
            counter: 0,
        }
    }
}

wayland_client::delegate_noop!(ClientState: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(ClientState: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(ClientState: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(ClientState: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(ClientState: ignore wl_buffer::WlBuffer);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for ClientState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `registry_queue_init` already collected the initial global list, which
        // is where `bind` reads it from.
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for ClientState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            let WEnum::Value(capabilities) = capabilities else {
                return;
            };
            if capabilities.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            if capabilities.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter {
                serial, surface, ..
            } => {
                state.keyboard_serial = Some(serial);
                if let Some(toplevel) = state.toplevel_of(&surface) {
                    state.events.push(Event::KeyboardEnter { serial, toplevel });
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                state.keyboard_serial = None;
                if let Some(toplevel) = state.toplevel_of(&surface) {
                    state.events.push(Event::KeyboardLeave { toplevel });
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                let pressed = matches!(key_state, WEnum::Value(wl_keyboard::KeyState::Pressed));
                state.events.push(Event::Key { key, pressed });
            }
            // `keymap` carries an fd we only need to keep open until the
            // keymap has been delivered; the tests assert on raw key codes.
            wl_keyboard::Event::Keymap { .. }
            | wl_keyboard::Event::Modifiers { .. }
            | wl_keyboard::Event::RepeatInfo { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface, .. } => {
                if let Some(toplevel) = state.toplevel_of(&surface) {
                    state.events.push(Event::PointerEnter { toplevel });
                }
            }
            wl_pointer::Event::Leave { surface, .. } => {
                if let Some(toplevel) = state.toplevel_of(&surface) {
                    state.events.push(Event::PointerLeave { toplevel });
                }
            }
            wl_pointer::Event::Button {
                serial,
                button,
                state: button_state,
                ..
            } => {
                let pressed =
                    matches!(button_state, WEnum::Value(wl_pointer::ButtonState::Pressed));
                state.events.push(Event::PointerButton {
                    serial,
                    button,
                    pressed,
                });
            }
            wl_pointer::Event::Motion { .. }
            | wl_pointer::Event::Axis { .. }
            | wl_pointer::Event::Frame
            | wl_pointer::Event::AxisSource { .. }
            | wl_pointer::Event::AxisStop { .. }
            | wl_pointer::Event::AxisDiscrete { .. }
            | wl_pointer::Event::AxisValue120 { .. }
            | wl_pointer::Event::AxisRelativeDirection { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for ClientState {
    fn event(
        _: &mut Self,
        _: &wl_data_device_manager::WlDataDeviceManager,
        _: wl_data_device_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // `wl_data_device_manager` has no events.
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.offers.push(Offer {
                    proxy: id,
                    // The kind is decided by the event that follows: `selection`
                    // for a clipboard offer, `enter` for a drag offer.
                    kind: OfferKind::Selection,
                    mimes: Vec::new(),
                    dropped: false,
                });
            }
            wl_data_device::Event::Selection { id } => match id {
                Some(id) => {
                    if let Some(offer) = state.offer_of(&id) {
                        state.offers[offer].kind = OfferKind::Selection;
                        state.events.push(Event::SelectionOffered { offer });
                    }
                }
                None => state.events.push(Event::SelectionCleared),
            },
            wl_data_device::Event::Enter { id, .. } => {
                let Some(id) = id else {
                    return;
                };
                if let Some(offer) = state.offer_of(&id) {
                    state.offers[offer].kind = OfferKind::Drag;
                    state.events.push(Event::DragEnter { offer });
                }
            }
            wl_data_device::Event::Motion { .. } => state.events.push(Event::DragMotion),
            wl_data_device::Event::Leave => state.events.push(Event::DragLeave),
            wl_data_device::Event::Drop => state.events.push(Event::DragDrop),
            _ => {}
        }
    }

    // `wl_data_device.data_offer` is the one core-protocol event that creates a
    // new object, so the queue needs to know which interface and user data the
    // child gets. Without this the dispatch panics with "Missing
    // event_created_child specialization".
    wayland_client::event_created_child!(ClientState, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for ClientState {
    fn event(
        state: &mut Self,
        offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Some(index) = state.offer_of(offer)
        {
            state.offers[index].mimes.push(mime_type);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for ClientState {
    fn event(
        state: &mut Self,
        source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state.source_of(source) else {
            return;
        };
        match event {
            wl_data_source::Event::Target { mime_type } => {
                state.sources[index].target = mime_type.clone();
                state.events.push(Event::SourceTarget { mime: mime_type });
            }
            wl_data_source::Event::Send { mime_type, fd } => {
                let bytes = state.sources[index]
                    .payload
                    .get(&mime_type)
                    .cloned()
                    .unwrap_or_default();
                let written = bytes.len();
                // Writing on this thread could block once the pipe buffer is
                // full (a 100 KiB clipboard payload is more than one pipe
                // buffer), so the source writes from its own thread while the
                // test keeps dispatching both connections.
                thread::spawn(move || {
                    let mut file = File::from(fd);
                    file.write_all(&bytes).ok();
                    file.flush().ok();
                });
                state.events.push(Event::SourceSend {
                    mime: mime_type,
                    bytes: written,
                });
            }
            wl_data_source::Event::Cancelled => state.events.push(Event::SourceCancelled),
            wl_data_source::Event::DndDropPerformed => {
                state.events.push(Event::SourceDropPerformed);
            }
            wl_data_source::Event::DndFinished => state.events.push(Event::SourceFinished),
            wl_data_source::Event::Action { dnd_action } => {
                let action = match dnd_action {
                    WEnum::Value(action) => format!("{action:?}"),
                    WEnum::Unknown(raw) => format!("unknown({raw})"),
                };
                state.events.push(Event::SourceAction { action });
            }
            _ => {}
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for ClientState {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            // Not answering a ping is a protocol error the compositor punishes
            // by disconnecting the client.
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for ClientState {
    fn event(
        state: &mut Self,
        xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            let Some(index) = state
                .toplevels
                .iter()
                .position(|candidate| candidate.xdg_surface.id() == xdg_surface.id())
            else {
                return;
            };
            // A toplevel that has never been configured gets one with 0x0, which
            // means "you choose"; anything else is the compositor's tiling size.
            let (width, height) = match state.toplevels[index].configured {
                Some(size) => size,
                None => state.toplevels[index].initial,
            };
            state.attach_solid(qh, index, width, height);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ()> for ClientState {
    fn event(
        state: &mut Self,
        toplevel: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(index) = state
            .toplevels
            .iter()
            .position(|candidate| candidate.toplevel.id() == toplevel.id())
        else {
            return;
        };
        match event {
            xdg_toplevel::Event::Configure { width, height, .. } => {
                let width = if width > 0 {
                    width
                } else {
                    state.toplevels[index].initial.0
                };
                let height = if height > 0 {
                    height
                } else {
                    state.toplevels[index].initial.1
                };
                state.toplevels[index].configured = Some((width, height));
                state.events.push(Event::Configure {
                    toplevel: index,
                    width,
                    height,
                });
                // The configure always arrives with a matching `xdg_surface`
                // configure; committing the buffer here keeps the client's
                // size in step with the compositor's tile.
                state.attach_solid(qh, index, width, height);
            }
            xdg_toplevel::Event::Close => state.events.push(Event::Close { toplevel: index }),
            xdg_toplevel::Event::ConfigureBounds { .. }
            | xdg_toplevel::Event::WmCapabilities { .. } => {}
            _ => {}
        }
    }
}
