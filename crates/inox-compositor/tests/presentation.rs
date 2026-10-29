//! `wp_presentation` and `zxdg-decoration`: what a real client is told about its
//! frame, and about its window's frame.
//!
//! Both protocols exist to replace a guess. `wp_presentation` tells a client when
//! its content actually reached an output, so it can time the next frame instead
//! of assuming; `zxdg-decoration` tells it whether the compositor draws a title
//! bar, so a toolkit that would otherwise ask for server-side decorations does not
//! end up with a window that has none.
//!
//! The test drives both from one in-process client so that the frame the
//! compositor renders is the frame the feedback is about: a subprocess test could
//! only assert that *an* event arrived.

use std::{
    collections::HashMap,
    io::{self, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::Arc,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, HeadlessOutput, InoxState};
use inox_protocols::standard::{
    core::client::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    wp::presentation_time::client::{
        wp_presentation::{self, WpPresentation},
        wp_presentation_feedback::{self, WpPresentationFeedback},
    },
    xdg::{
        decoration::client::zxdg_toplevel_decoration_v1::{self, ZxdgToplevelDecorationV1},
        shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
    },
};
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

/// Side of the square the test window renders, in pixels.
const SIZE: i32 = 64;

/// The value `wp_presentation.clock_id` must name: `CLOCK_MONOTONIC`.
///
/// Anything else would make the timestamps incomparable with the ones a client
/// gets from `wl_surface.frame`, which is the whole point of advertising a clock.
const CLOCK_MONOTONIC: u32 = 1;

#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    /// The clock the compositor named when the global was bound.
    clock_id: Option<u32>,
    /// Every decoration mode the compositor sent.
    decoration_modes: Vec<WEnum<zxdg_toplevel_decoration_v1::Mode>>,
    /// Serial of the last `xdg_surface.configure`, so the test can acknowledge it.
    xdg_configure: Option<u32>,
    /// The feedbacks the compositor answered, as `(refresh, vsync)`.
    presented: Vec<(u32, bool)>,
    /// Feedbacks the compositor discarded instead of presenting.
    discarded: usize,
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

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(
        _state: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        // A compositor that pings and is not answered disconnects the client.
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, ()> for Client {
    fn event(
        state: &mut Self,
        _surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            state.xdg_configure = Some(serial);
        }
    }
}

impl Dispatch<ZxdgToplevelDecorationV1, ()> for Client {
    fn event(
        state: &mut Self,
        _decoration: &ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Event::Configure { mode } = event {
            state.decoration_modes.push(mode);
        }
    }
}

impl Dispatch<WpPresentation, ()> for Client {
    fn event(
        state: &mut Self,
        _presentation: &WpPresentation,
        event: wp_presentation::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        if let wp_presentation::Event::ClockId { clk_id } = event {
            state.clock_id = Some(clk_id);
        }
    }
}

impl Dispatch<WpPresentationFeedback, ()> for Client {
    fn event(
        state: &mut Self,
        _feedback: &WpPresentationFeedback,
        event: wp_presentation_feedback::Event,
        _data: &(),
        _connection: &Connection,
        _queue_handle: &QueueHandle<Self>,
    ) {
        match event {
            wp_presentation_feedback::Event::Presented { refresh, flags, .. } => {
                let vsync = matches!(
                    flags,
                    WEnum::Value(kind) if kind.contains(wp_presentation_feedback::Kind::Vsync)
                );
                state.presented.push((refresh, vsync));
            }
            wp_presentation_feedback::Event::Discarded => state.discarded += 1,
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore
    inox_protocols::standard::xdg::decoration::client::zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);

/// One client connection to the in-process compositor.
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

    fn buffer(&self, width: i32, height: i32) -> wl_buffer::WlBuffer {
        let mut file = tempfile::tempfile().unwrap();
        let pixel = [0_u8, 0, 255, 255];
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
}

/// One exchange with the compositor: send what the client queued, then deliver
/// what the compositor answered.
fn step(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer) {
    peer.pump();
    display.dispatch_clients(state).unwrap();
    display.flush_clients().unwrap();
}

/// Step and render until `condition` holds, or fail after a generous deadline.
///
/// Rendering inside the loop, rather than once at the end, is what makes the
/// feedback meaningful: `wp_presentation` answers the frame a commit produced, and
/// this compositor only composites when something asked it to.
fn until(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peer: &mut Peer,
    condition: impl Fn(&Client) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);

    loop {
        step(display, state, peer);

        if condition(&peer.state) {
            break;
        }

        state.render_frame().unwrap();
        assert!(
            Instant::now() < deadline,
            "the scenario timed out; the compositor advertised {:?}",
            peer.state.globals.keys().collect::<Vec<_>>()
        );
    }

    // Deliver whatever the acknowledgements queued.
    for _ in 0..3 {
        step(display, state, peer);
    }
}

#[test]
fn a_toplevel_is_told_it_draws_its_own_frame_and_when_its_frame_was_shown() {
    let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
    let mut display = Display::<InoxState>::new().unwrap();
    let mut handle = display.handle();
    let config = CompositorConfig {
        outputs: vec![HeadlessOutput {
            width: SIZE * 4,
            height: SIZE * 4,
            ..HeadlessOutput::default()
        }],
        ..Default::default()
    };
    let mut state = InoxState::new(&handle, &config, event_loop.get_signal()).unwrap();

    let mut peer = Peer::connect(&mut handle);
    until(&mut display, &mut state, &mut peer, |client| {
        [
            "wp_presentation",
            "zxdg_decoration_manager_v1",
            "xdg_wm_base",
            "wl_shm",
        ]
        .iter()
        .all(|interface| client.globals.contains_key(*interface))
    });

    let queue_handle = peer.queue.handle();
    let compositor: wl_compositor::WlCompositor = peer.bind(6);
    let wm_base: xdg_wm_base::XdgWmBase = peer.bind(6);
    let presentation: WpPresentation = peer.bind(2);
    let decoration_manager: inox_protocols::standard::xdg::decoration::client::zxdg_decoration_manager_v1::ZxdgDecorationManagerV1 =
        peer.bind(1);

    let surface = compositor.create_surface(&queue_handle, ());
    let xdg = wm_base.get_xdg_surface(&surface, &queue_handle, ());
    let toplevel = xdg.get_toplevel(&queue_handle, ());
    toplevel.set_title("presentation".to_owned());
    toplevel.set_app_id("inox.test".to_owned());

    // Requested before the toplevel's first commit, which is what version 1
    // requires of a client.
    let decoration = decoration_manager.get_toplevel_decoration(&toplevel, &queue_handle, ());

    // The initial commit carries no buffer: it asks the compositor for a size.
    surface.commit();
    until(&mut display, &mut state, &mut peer, |client| {
        client.xdg_configure.is_some()
    });

    let serial = peer.state.xdg_configure.expect("no configure arrived");
    xdg.ack_configure(serial);

    // Feedback is requested *before* the commit: the protocol ties it to the frame
    // that commit produces.
    let _feedback = presentation.feedback(&surface, &queue_handle, ());

    let buffer = peer.buffer(SIZE, SIZE);
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, SIZE, SIZE);
    surface.commit();

    until(&mut display, &mut state, &mut peer, |client| {
        !client.presented.is_empty()
    });

    // The decoration negotiation: one answer, and it is the one that matches a
    // compositor drawing no title bar.
    assert_eq!(
        peer.state.decoration_modes,
        vec![WEnum::Value(zxdg_toplevel_decoration_v1::Mode::ClientSide)],
        "a compositor that draws no title bar must say so"
    );

    // The clock, and the feedback.
    assert_eq!(
        peer.state.clock_id,
        Some(CLOCK_MONOTONIC),
        "`presented` timestamps are only comparable if the clock is the monotonic one"
    );
    assert_eq!(
        peer.state.presented.len(),
        1,
        "one request, one answer: {:?}",
        peer.state.presented
    );
    assert_eq!(
        peer.state.discarded, 0,
        "the frame was presented, not dropped"
    );

    let (refresh, vsync) = peer.state.presented[0];
    // `presented` carries the period in nanoseconds, which is what a client needs
    // to place its next frame; the output's refresh rate is the way to derive it.
    let expected_period =
        1_000_000_000_u32 / u32::try_from(HeadlessOutput::default().refresh_hz).unwrap();
    assert_eq!(
        refresh, expected_period,
        "the reported period must be the output's own, not a guess"
    );
    assert!(
        vsync,
        "a composited frame is presented on the vertical blank"
    );

    decoration.destroy();
    buffer.destroy();
    surface.destroy();

    for _ in 0..3 {
        step(&mut display, &mut state, &mut peer);
    }
}
