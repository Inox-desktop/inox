//! Integration test for the M2 acceptance criteria plus the window lifecycle:
//! a toplevel becomes a window only when it draws, the focused window is told it
//! is active, and the close chord reaches exactly that window.
//!
//! The test drives a real Wayland client (built with `wayland-client`) over an
//! in-process socket pair against a compositor state on the current thread. The
//! client runs on a second thread while the main thread services the display, so
//! no process boundary is needed.
//!
//! Everything lives in a single test function on purpose: a process may only ever
//! hold one EGL display (see the `SAFETY` note in `src/render.rs`), and test
//! functions inside one binary run in parallel threads.

use std::{
    collections::{HashMap, HashSet},
    os::unix::{io::AsFd, net::UnixStream},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, InoxState};
use smithay::{
    backend::input::{ButtonState, KeyState},
    input::keyboard::Keycode,
    reexports::wayland_server::Display,
    utils::{Logical, Point},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long to service the display before declaring that the client never got
/// through. Generous enough for a busy CI machine.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Edge length of the `wl_shm` buffer each client window draws.
const BUFFER_SIZE: i32 = 64;

/// `evdev` key codes (XKB key codes are the evdev scancode plus 8).
const KEY_LEFT_ALT: u32 = 56 + 8;
const KEY_LEFT_META: u32 = 125 + 8;
const KEY_TAB: u32 = 15 + 8;
const KEY_Q: u32 = 16 + 8;
const KEY_LEFT_SHIFT: u32 = 42 + 8;

/// What the client observed, shared with the test thread.
///
/// The compositor's own state can say what it *thinks* it told the client; only
/// the client's event stream can say what it was actually told. These fields are
/// filled by the client's `Dispatch` implementations from the real events.
#[derive(Debug, Default, Clone)]
struct Observed {
    /// Toplevel indices whose `xdg_surface` has been configured.
    configured: HashSet<u32>,
    /// Last `activated` state received per toplevel index.
    activated: HashMap<u32, bool>,
    /// Number of `xdg_toplevel.close` events received per toplevel index.
    closes: HashMap<u32, usize>,
}

enum Command {
    /// Wait for the initial configure, attach a buffer and commit both toplevels.
    Draw(Sender<()>),
    /// Dispatch whatever the compositor sent so far, then report.
    Observe(Sender<()>),
    /// Finish the thread.
    Stop,
}

/// The client's dispatch state. The test never reads events on the main thread,
/// so everything the client learns is recorded in `observed`.
struct Client {
    observed: Arc<Mutex<Observed>>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The `GlobalList` records the globals; the test only needs to bind them.
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(
        _: &mut Self,
        base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            // An unanswered ping is how a compositor decides a client is gone;
            // this client is alive for as long as the test needs it.
            base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, u32> for Client {
    fn event(
        state: &mut Self,
        surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        index: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            state
                .observed
                .lock()
                .expect("the observed log is not poisoned")
                .configured
                .insert(*index);
        }
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, u32> for Client {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        index: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut observed = state
            .observed
            .lock()
            .expect("the observed log is not poisoned");
        match event {
            xdg_toplevel::Event::Configure { states, .. } => {
                let activated = states_include(&states, xdg_toplevel::State::Activated);
                observed.activated.insert(*index, activated);
            }
            xdg_toplevel::Event::Close => {
                *observed.closes.entry(*index).or_default() += 1;
            }
            _ => {}
        }
    }
}

/// Whether the raw `states` array of a configure carries `wanted`.
///
/// `xdg_toplevel.configure` sends the states as an array of native-endian `u32`,
/// which `wayland-client` hands over as bytes; decoding it here is what lets the
/// test assert on the protocol event instead of on the compositor's own bookkeeping.
fn states_include(states: &[u8], wanted: xdg_toplevel::State) -> bool {
    states
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&chunk| u32::from_ne_bytes(chunk))
        .any(|raw| matches!(xdg_toplevel::State::try_from(raw), Ok(state) if state == wanted))
}

/// One toplevel the client created, with the objects that must stay alive.
struct Toplevel {
    surface: wl_surface::WlSurface,
    _xdg_surface: xdg_surface::XdgSurface,
    _toplevel: xdg_toplevel::XdgToplevel,
    _buffer: Option<wl_buffer::WlBuffer>,
    _pool: Option<wl_shm_pool::WlShmPool>,
}

/// Service the display once: dispatch client requests and flush outgoing events.
fn service(display: &mut Display<InoxState>, state: &mut InoxState) {
    // Errors during teardown (a peer that went away) are not interesting here; the
    // state assertions below are what the test is about.
    let _ = display.dispatch_clients(state);
    let _ = display.flush_clients();
}

/// Service the display until `done` has something to receive, or panic.
fn service_until(display: &mut Display<InoxState>, state: &mut InoxState, done: &Receiver<()>) {
    let deadline = Instant::now() + DISPATCH_TIMEOUT;

    while done.try_recv().is_err() {
        service(display, state);
        assert!(
            Instant::now() < deadline,
            "timed out after {DISPATCH_TIMEOUT:?} waiting for the client"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// Wait until the compositor has seen `count` toplevels that have not drawn yet.
fn wait_for_awaiting(display: &mut Display<InoxState>, state: &mut InoxState, count: usize) {
    let deadline = Instant::now() + DISPATCH_TIMEOUT;

    while state.awaiting_window_count() < count {
        service(display, state);
        assert!(
            Instant::now() < deadline,
            "timed out after {DISPATCH_TIMEOUT:?} with {} toplevels awaiting a buffer",
            state.awaiting_window_count()
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// Wait until the compositor manages at least `count` windows.
fn wait_for_windows(display: &mut Display<InoxState>, state: &mut InoxState, count: usize) {
    let deadline = Instant::now() + DISPATCH_TIMEOUT;

    while state.window_count() < count {
        service(display, state);
        assert!(
            Instant::now() < deadline,
            "timed out after {DISPATCH_TIMEOUT:?} with {} windows managed",
            state.window_count()
        );
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn manages_client_windows_and_input() {
    let event_loop: EventLoop<'static, ()> =
        EventLoop::try_new().expect("failed to create the event loop");
    let mut display: Display<InoxState> =
        Display::new().expect("failed to create the wayland display");
    let mut display_handle = display.handle();

    let mut state = InoxState::new(
        &display_handle,
        &CompositorConfig::default(),
        event_loop.get_signal(),
    )
    .expect("failed to create the compositor state");

    let (server_stream, client_stream) =
        UnixStream::pair().expect("failed to create the socket pair");
    display_handle
        .insert_client(server_stream, Arc::new(ClientState::default()))
        .expect("failed to register the client");

    let client = ClientThread::spawn(client_stream, &mut display, &mut state);

    // --- a toplevel that has not drawn is not a window ---------------------
    //
    // Both toplevels exist on the client side, but neither has a buffer. They must
    // not hold a tile, an Alt+Tab slot or the focus: a client that creates a window
    // and never draws is a window that does not exist yet.
    wait_for_awaiting(&mut display, &mut state, 2);
    for _ in 0..5 {
        service(&mut display, &mut state);
    }
    assert_eq!(
        state.window_count(),
        0,
        "a toplevel without a buffer must not become a window"
    );

    // --- drawing maps them -------------------------------------------------
    client.draw(&mut display, &mut state);
    wait_for_windows(&mut display, &mut state, 2);

    let windows = state.managed_windows().to_vec();
    assert_eq!(windows.len(), 2);

    // The most recently mapped window is the layout master. The client commits
    // them in index order, so the second toplevel is the newest.
    let master = windows[0];
    let stack = windows[1];

    // Default tiling at 1280x720 with a 60% master column and an 8px gap:
    // the master column starts at x = 8 with width 758, the stack column starts
    // at x = 774 with width 498.
    assert_eq!(
        state.window_location(master),
        Some(Point::from((8, 8))),
        "the master window should sit at the top-left of the work area"
    );
    assert_eq!(
        state.window_location(stack),
        Some(Point::from((774, 8))),
        "the stack window should sit right of the master column"
    );

    // --- activation reaches the client -------------------------------------
    //
    // `xdg_toplevel.activated` is what tells a client it is the foreground window.
    // Without it every application draws itself as unfocused no matter what the
    // compositor's own focus state says, so this has to be asserted from the
    // client's event stream, not from `InoxState`.
    assert_eq!(state.focused_window(), Some(master));
    let activation = client.observe(&mut display, &mut state);
    assert_eq!(
        activation.activated.get(&1),
        Some(&true),
        "the focused (newest) toplevel should have been told it is active"
    );
    assert_ne!(
        activation.activated.get(&0),
        Some(&true),
        "the unfocused toplevel must not stay activated"
    );

    // --- input events ------------------------------------------------------

    // Pointer motion is tracked and delivered without a client focus.
    let pointer = Point::<f64, Logical>::from((100.0, 120.0));
    state.inject_pointer_motion(pointer);
    assert_eq!(state.pointer_location(), pointer);

    state.inject_pointer_button(0x110, ButtonState::Pressed);
    state.inject_pointer_button(0x110, ButtonState::Released);

    // Alt+Tab cycles the keyboard focus forward. The focused window starts as the
    // master (the newest window); one step lands on the stack window.
    assert_eq!(
        state.inject_key(Keycode::new(KEY_LEFT_ALT), KeyState::Pressed),
        None
    );
    assert_eq!(
        state.inject_key(Keycode::new(KEY_TAB), KeyState::Pressed),
        Some(inox_compositor::Action::FocusNext)
    );
    assert_eq!(state.focused_window(), Some(stack));

    // Smithay hands the filter a release exactly as it does a press, so a binding
    // that does not check the key state runs twice per chord: the focus would step
    // to the next window and straight back to where it started.
    assert_eq!(
        state.inject_key(Keycode::new(KEY_TAB), KeyState::Released),
        None
    );
    assert_eq!(
        state.focused_window(),
        Some(stack),
        "releasing the bound key must not run the binding a second time"
    );
    state.inject_key(Keycode::new(KEY_LEFT_ALT), KeyState::Released);

    // The activation follows the focus, in the client's event stream too: the
    // window that lost the keyboard must be deactivated, not just the one that
    // gained it.
    let activation = client.observe(&mut display, &mut state);
    assert_eq!(
        activation.activated.get(&0),
        Some(&true),
        "the newly focused toplevel should have been told it is active"
    );
    assert_eq!(
        activation.activated.get(&1),
        Some(&false),
        "the toplevel that lost the focus should have been deactivated"
    );

    // Alt+Shift+Tab walks the focus back. `Shift` moves `Tab` to its second level,
    // which the seat reports as `ISO_Left_Tab` rather than `Tab` — so a binding that
    // matched only `Tab` would leave this direction unreachable.
    assert_eq!(
        state.inject_key(Keycode::new(KEY_LEFT_SHIFT), KeyState::Pressed),
        None
    );
    assert_eq!(
        state.inject_key(Keycode::new(KEY_LEFT_ALT), KeyState::Pressed),
        None
    );
    assert_eq!(
        state.inject_key(Keycode::new(KEY_TAB), KeyState::Pressed),
        Some(inox_compositor::Action::FocusPrev)
    );
    assert_eq!(state.focused_window(), Some(master));

    state.inject_key(Keycode::new(KEY_TAB), KeyState::Released);
    state.inject_key(Keycode::new(KEY_LEFT_ALT), KeyState::Released);
    state.inject_key(Keycode::new(KEY_LEFT_SHIFT), KeyState::Released);
    assert_eq!(
        state.focused_window(),
        Some(master),
        "releasing the chord must not move the focus again"
    );

    // --- closing a window is a request, not a kill -------------------------
    //
    // `Super+Shift+Q` sends `xdg_toplevel.close` to the focused window. The client
    // decides what to do with it; the compositor must not tear the window down
    // behind its back, which is why the window is still managed afterwards.
    assert_eq!(
        state.inject_key(Keycode::new(KEY_LEFT_META), KeyState::Pressed),
        None
    );
    assert_eq!(
        state.inject_key(Keycode::new(KEY_LEFT_SHIFT), KeyState::Pressed),
        None
    );
    assert_eq!(
        state.inject_key(Keycode::new(KEY_Q), KeyState::Pressed),
        Some(inox_compositor::Action::CloseWindow)
    );
    assert_eq!(
        state.inject_key(Keycode::new(KEY_Q), KeyState::Released),
        None,
        "the close binding must only fire on the press"
    );
    state.inject_key(Keycode::new(KEY_LEFT_SHIFT), KeyState::Released);
    state.inject_key(Keycode::new(KEY_LEFT_META), KeyState::Released);

    let closed = client.observe(&mut display, &mut state);
    assert_eq!(
        closed.closes.get(&1),
        Some(&1),
        "the focused (newest) toplevel should have received one close request"
    );
    assert_eq!(
        closed.closes.get(&0),
        None,
        "an unfocused toplevel must not be told to close"
    );
    assert_eq!(
        state.window_count(),
        2,
        "a close request must not unmap the window; only the client can"
    );

    // Super+Q on its own still asks the compositor to shut down cleanly.
    assert_eq!(
        state.inject_key(Keycode::new(KEY_LEFT_META), KeyState::Pressed),
        None
    );
    assert_eq!(
        state.inject_key(Keycode::new(KEY_Q), KeyState::Pressed),
        Some(inox_compositor::Action::Quit)
    );
    assert!(state.exit_requested());

    assert_eq!(
        state.inject_key(Keycode::new(KEY_Q), KeyState::Released),
        None,
        "the quit binding must only fire on the press"
    );

    // Release the client so it drops its protocol objects, then let it finish.
    client.stop();
}

/// Handle on the client thread.
struct ClientThread {
    thread: thread::JoinHandle<()>,
    commands: Sender<Command>,
    observed: Arc<Mutex<Observed>>,
}

impl ClientThread {
    /// Start the client thread and wait until its two toplevels exist.
    ///
    /// The wait services the display: the client cannot get past
    /// `registry_queue_init` until the compositor has answered its registry
    /// requests, so blocking on the thread without doing so would deadlock.
    fn spawn(
        client_stream: UnixStream,
        display: &mut Display<InoxState>,
        state: &mut InoxState,
    ) -> Self {
        let (created_tx, created) = channel();
        let (command_tx, command_rx): (Sender<Command>, Receiver<Command>) = channel();
        let observed = Arc::new(Mutex::new(Observed::default()));
        let client_observed = Arc::clone(&observed);

        let thread = thread::spawn(move || {
            let connection = Connection::from_socket(client_stream).expect("failed to connect");

            let (globals, mut event_queue) = registry_queue_init::<Client>(&connection)
                .expect("failed to initialise the registry");
            let queue = event_queue.handle();
            let mut client = Client {
                observed: client_observed,
            };

            let compositor: wl_compositor::WlCompositor = globals
                .bind(&queue, 1..=4, ())
                .expect("the compositor has no wl_compositor global");
            let xdg: xdg_wm_base::XdgWmBase = globals
                .bind(&queue, 1..=6, ())
                .expect("the compositor has no xdg_wm_base global");
            let shm: wl_shm::WlShm = globals
                .bind(&queue, 1..=1, ())
                .expect("the compositor has no wl_shm global");

            let mut toplevels = Vec::new();
            for index in 0..2u32 {
                let surface = compositor.create_surface(&queue, ());
                let xdg_surface = xdg.get_xdg_surface(&surface, &queue, index);
                let toplevel = xdg_surface.get_toplevel(&queue, index);
                // Request the initial configure. Without a commit the compositor
                // never learns the surface exists; with no buffer it is still not a
                // window.
                surface.commit();
                toplevels.push(Toplevel {
                    surface,
                    _xdg_surface: xdg_surface,
                    _toplevel: toplevel,
                    _buffer: None,
                    _pool: None,
                });
            }
            connection.flush().expect("failed to flush the client");
            let _ = created_tx.send(());

            while let Ok(command) = command_rx.recv() {
                match command {
                    Command::Draw(done) => {
                        let deadline = Instant::now() + DISPATCH_TIMEOUT;
                        loop {
                            let configured = client
                                .observed
                                .lock()
                                .expect("the observed log is not poisoned")
                                .configured
                                .len();
                            if configured == 2 {
                                break;
                            }
                            event_queue
                                .roundtrip(&mut client)
                                .expect("failed to dispatch the initial configure");
                            assert!(
                                Instant::now() < deadline,
                                "the compositor never configured the toplevels"
                            );
                        }

                        for toplevel in &mut toplevels {
                            let stride = BUFFER_SIZE * 4;
                            // An anonymous file of exactly the window's pixels: the
                            // compositor never reads the contents, it only needs the
                            // geometry to composite something.
                            let file =
                                tempfile::tempfile().expect("failed to create the buffer file");
                            file.set_len((stride * BUFFER_SIZE) as u64)
                                .expect("failed to size the buffer file");
                            let pool =
                                shm.create_pool(file.as_fd(), stride * BUFFER_SIZE, &queue, ());
                            let buffer = pool.create_buffer(
                                0,
                                BUFFER_SIZE,
                                BUFFER_SIZE,
                                stride,
                                wl_shm::Format::Argb8888,
                                &queue,
                                (),
                            );
                            toplevel.surface.attach(Some(&buffer), 0, 0);
                            toplevel.surface.commit();
                            toplevel._buffer = Some(buffer);
                            toplevel._pool = Some(pool);
                        }
                        connection.flush().expect("failed to flush the buffers");
                        let _ = done.send(());
                    }
                    Command::Observe(done) => {
                        // A roundtrip both flushes what the client sent and waits for
                        // the compositor to answer, so the log above includes every
                        // event the compositor had already queued.
                        event_queue
                            .roundtrip(&mut client)
                            .expect("failed to observe the compositor");
                        let _ = done.send(());
                    }
                    Command::Stop => break,
                }
            }

            drop((toplevels, xdg, compositor, event_queue, connection));
        });

        // Wait until the client created its toplevels before looking at the
        // compositor: a toplevel that was never created cannot be awaiting a buffer.
        let deadline = Instant::now() + DISPATCH_TIMEOUT;
        while created.try_recv().is_err() {
            service(display, state);
            assert!(
                Instant::now() < deadline,
                "the client never created its toplevels"
            );
            thread::sleep(Duration::from_millis(2));
        }

        Self {
            thread,
            commands: command_tx,
            observed,
        }
    }

    /// Ask the client to wait for its initial configure and attach a buffer.
    fn draw(&self, display: &mut Display<InoxState>, state: &mut InoxState) {
        let (done_tx, done_rx): (Sender<()>, Receiver<()>) = channel();
        self.commands
            .send(Command::Draw(done_tx))
            .expect("client is gone");
        service_until(display, state, &done_rx);
    }

    /// Ask the client to dispatch what it has and return what it observed.
    fn observe(&self, display: &mut Display<InoxState>, state: &mut InoxState) -> Observed {
        let (done_tx, done_rx): (Sender<()>, Receiver<()>) = channel();
        self.commands
            .send(Command::Observe(done_tx))
            .expect("client is gone");
        service_until(display, state, &done_rx);
        self.observed
            .lock()
            .expect("the observed log is not poisoned")
            .clone()
    }

    /// Ask the client to exit and wait for its thread.
    fn stop(self) {
        let _ = self.commands.send(Command::Stop);
        self.thread.join().expect("the client thread panicked");
    }
}
