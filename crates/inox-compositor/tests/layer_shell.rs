//! Real Wayland clients exercise desktop layers on the software-rendered Inox host.

use std::{
    collections::HashMap,
    io::{self, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::Arc,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, InoxState};
use inox_protocols::standard::{
    core::client::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
        wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
    wlr::layer_shell::client::{zwlr_layer_shell_v1 as shell, zwlr_layer_surface_v1 as layer},
    xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
};
use smithay::{
    backend::input::ButtonState,
    reexports::wayland_server::{Display, DisplayHandle},
    utils::{Point, Rectangle},
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

const WIDTH: i32 = 320;
const HEIGHT: i32 = 240;
const RED: [u8; 4] = [220, 30, 20, 255];
const GREEN: [u8; 4] = [20, 210, 30, 255];
const BLUE: [u8; 4] = [20, 30, 220, 255];
const YELLOW: [u8; 4] = [220, 210, 20, 255];

#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    configures: HashMap<usize, Vec<(u32, i32, i32)>>,
    closed: usize,
    keyboard_focus: Option<wl_surface::WlSurface>,
    pointer_focus: Option<wl_surface::WlSurface>,
    frames: usize,
    auto_ack: bool,
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

impl Dispatch<layer::ZwlrLayerSurfaceV1, usize> for Client {
    fn event(
        state: &mut Self,
        proxy: &layer::ZwlrLayerSurfaceV1,
        event: layer::Event,
        id: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            layer::Event::Configure {
                serial,
                width,
                height,
            } => {
                state.configures.entry(*id).or_default().push((
                    serial,
                    width as i32,
                    height as i32,
                ));
                if state.auto_ack {
                    proxy.ack_configure(serial);
                }
            }
            layer::Event::Closed => state.closed += 1,
            _ => {}
        }
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

impl Dispatch<wl_callback::WlCallback, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.frames += 1;
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
wayland_client::delegate_noop!(Client: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Client: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(Client: ignore shell::ZwlrLayerShellV1);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);

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
            state: Client {
                auto_ack: true,
                ..Default::default()
            },
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

    fn layer(
        &self,
        id: usize,
        depth: shell::Layer,
        output: Option<&wl_output::WlOutput>,
    ) -> Surface {
        let compositor: wl_compositor::WlCompositor = self.bind(5);
        let shell: shell::ZwlrLayerShellV1 = self.bind(4);
        let surface = compositor.create_surface(&self.queue.handle(), ());
        let role = shell.get_layer_surface(
            &surface,
            output,
            depth,
            format!("test-{id}"),
            &self.queue.handle(),
            id,
        );
        Surface { surface, role, id }
    }
}

struct Surface {
    surface: wl_surface::WlSurface,
    role: layer::ZwlrLayerSurfaceV1,
    id: usize,
}

impl Surface {
    fn attach(&self, peer: &Peer, colour: [u8; 4]) {
        let (_, width, height) = *peer.state.configures[&self.id].last().unwrap();
        let buffer = peer.buffer(width, height, colour);
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage_buffer(0, 0, width, height);
        self.surface.commit();
    }

    fn destroy(&self) {
        self.role.destroy();
        self.surface.destroy();
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
        assert!(Instant::now() < deadline, "Wayland scenario timed out");
    }
    // Deliver the acknowledgements queued by the configure handlers.
    for _ in 0..3 {
        step(display, state, peers);
    }
}

fn configured(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer, id: usize) {
    until(display, state, &mut [peer], |_, peers| {
        peers[0].state.configures.contains_key(&id)
    });
}

fn pixel(state: &mut InoxState, x: usize, y: usize, expected: [u8; 4]) {
    state.render_frame().unwrap();
    let bytes = state.read_frame_rgba().unwrap();
    let offset = (y * WIDTH as usize + x) * 4;
    for (actual, expected) in bytes[offset..offset + 4].iter().zip(expected) {
        assert!(
            actual.abs_diff(expected) <= 1,
            "pixel ({x}, {y}): {:?} != {expected:?}",
            &bytes[offset..offset + 4]
        );
    }
}

fn click(state: &mut InoxState, x: f64, y: f64) {
    state.inject_pointer_motion(Point::from((x, y)));
    state.inject_pointer_button(0x110, ButtonState::Pressed);
    state.inject_pointer_button(0x110, ButtonState::Released);
}

fn rejected(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peer: &mut Peer,
    code: layer::Error,
) {
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
        assert!(Instant::now() < deadline, "invalid client was not rejected");
    }
    let error = peer
        .connection
        .protocol_error()
        .expect("expected a protocol error");
    assert_eq!(error.object_interface, "zwlr_layer_surface_v1");
    assert_eq!(error.code, u32::from(code));
}

#[test]
fn layers_render_reserve_focus_and_release_resources() {
    let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
    let mut display = Display::<InoxState>::new().unwrap();
    let mut handle = display.handle();
    let config = CompositorConfig {
        width: WIDTH,
        height: HEIGHT,
        ..Default::default()
    };
    let mut state = InoxState::new(&handle, &config, event_loop.get_signal()).unwrap();
    let mut peer = Peer::connect(&mut handle);
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.globals.contains_key("zwlr_layer_shell_v1")
    });
    assert_eq!(peer.state.globals["zwlr_layer_shell_v1"].1, 4);
    let seat: wl_seat::WlSeat = peer.bind(9);
    let _keyboard = seat.get_keyboard(&peer.queue.handle(), ());
    let _pointer = seat.get_pointer(&peer.queue.handle(), ());
    let output: wl_output::WlOutput = peer.bind(4);

    let compositor: wl_compositor::WlCompositor = peer.bind(5);
    let xdg: xdg_wm_base::XdgWmBase = peer.bind(6);
    let app = compositor.create_surface(&peer.queue.handle(), ());
    let xdg_surface = xdg.get_xdg_surface(&app, &peer.queue.handle(), ());
    let toplevel = xdg_surface.get_toplevel(&peer.queue.handle(), ());
    app.commit();
    until(
        &mut display,
        &mut state,
        &mut [&mut peer],
        |state, peers| state.window_count() == 1 && peers[0].state.keyboard_focus.is_some(),
    );
    let app_buffer = peer.buffer(304, 224, RED);
    app.attach(Some(&app_buffer), 0, 0);
    app.commit();
    for _ in 0..4 {
        step(&mut display, &mut state, &mut [&mut peer]);
    }
    let app_focus = state.keyboard_focus();

    // A neutral surface created first must still avoid a later exclusive panel.
    let notice = peer.layer(0, shell::Layer::Overlay, None);
    notice.role.set_size(40, 20);
    notice
        .role
        .set_anchor(layer::Anchor::Top | layer::Anchor::Right);
    notice.surface.commit();
    configured(&mut display, &mut state, &mut peer, 0);
    notice.attach(&peer, YELLOW);

    let panel = peer.layer(1, shell::Layer::Top, Some(&output));
    panel.role.set_size(0, 30);
    panel
        .role
        .set_anchor(layer::Anchor::Top | layer::Anchor::Left | layer::Anchor::Right);
    panel.role.set_exclusive_zone(30);
    for _ in 0..4 {
        step(&mut display, &mut state, &mut [&mut peer]);
    }
    assert!(
        !peer.state.configures.contains_key(&1),
        "initial configure must follow the empty commit"
    );
    panel.surface.commit();
    configured(&mut display, &mut state, &mut peer, 1);
    assert_eq!(
        state.work_area(),
        Rectangle::from_size((WIDTH, HEIGHT).into()),
        "unbuffered panels do not reserve space"
    );
    assert_eq!(peer.state.configures[&1][0].1, WIDTH);
    panel.attach(&peer, GREEN);
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.layer_count() == 2 && state.work_area().loc.y == 30
    });
    assert_eq!(state.output_size(), (WIDTH, HEIGHT));
    assert_eq!(
        state.window_location(state.managed_windows()[0]),
        Some((8, 38).into())
    );
    assert_eq!(
        state.keyboard_focus(),
        app_focus,
        "None interactivity preserves application focus"
    );

    // The background is deliberately created last to catch creation-order stacking.
    let background = peer.layer(2, shell::Layer::Background, None);
    background.role.set_size(0, 0);
    background.role.set_anchor(layer::Anchor::all());
    background.role.set_exclusive_zone(-1);
    background.surface.commit();
    configured(&mut display, &mut state, &mut peer, 2);
    background.attach(&peer, BLUE);
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.layer_count() == 3
    });
    pixel(&mut state, 10, 10, GREEN);
    pixel(&mut state, 310, 40, YELLOW);
    pixel(&mut state, 20, 80, RED);

    // A buffer sent without acknowledging configure only disconnects its own client.
    let mut invalid = Peer::connect(&mut handle);
    until(&mut display, &mut state, &mut [&mut invalid], |_, peers| {
        peers[0].state.globals.contains_key("zwlr_layer_shell_v1")
    });
    invalid.state.auto_ack = false;
    let unacked = invalid.layer(0, shell::Layer::Overlay, None);
    unacked.role.set_size(20, 20);
    unacked.surface.commit();
    configured(&mut display, &mut state, &mut invalid, 0);
    unacked.attach(&invalid, RED);
    rejected(
        &mut display,
        &mut state,
        &mut invalid,
        layer::Error::InvalidSurfaceState,
    );
    drop(unacked);
    drop(invalid);
    let mut oversized = Peer::connect(&mut handle);
    until(
        &mut display,
        &mut state,
        &mut [&mut oversized],
        |_, peers| peers[0].state.globals.contains_key("zwlr_layer_shell_v1"),
    );
    let huge = oversized.layer(0, shell::Layer::Overlay, None);
    huge.role.set_size(u32::MAX, 20);
    rejected(
        &mut display,
        &mut state,
        &mut oversized,
        layer::Error::InvalidSize,
    );
    drop(huge);
    drop(oversized);
    assert_eq!(state.layer_count(), 3);
    pixel(&mut state, 20, 80, RED);
    pixel(&mut state, 1, 200, BLUE);
    state.inject_pointer_motion((10.0, 10.0).into());
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.pointer_focus.as_ref() == Some(&panel.surface)
    });
    panel.surface.frame(&peer.queue.handle(), ());
    panel.surface.commit();
    for _ in 0..3 {
        step(&mut display, &mut state, &mut [&mut peer]);
    }
    state.render_frame().unwrap();
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.frames == 1
    });

    let launcher = peer.layer(3, shell::Layer::Overlay, None);
    launcher.role.set_size(100, 80);
    launcher
        .role
        .set_keyboard_interactivity(layer::KeyboardInteractivity::Exclusive);
    launcher.surface.commit();
    configured(&mut display, &mut state, &mut peer, 3);
    launcher.attach(&peer, BLUE);
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.keyboard_focus.as_ref() == Some(&launcher.surface)
    });
    click(&mut state, 20.0, 80.0);
    assert_ne!(
        state.keyboard_focus(),
        app_focus,
        "exclusive layer retains focus over application clicks"
    );
    // Detaching a buffer restores focus; remapping requires a fresh configure.
    launcher.surface.attach(None, 0, 0);
    launcher.surface.commit();
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.keyboard_focus() == app_focus
    });
    launcher.role.set_size(100, 80);
    launcher
        .role
        .set_keyboard_interactivity(layer::KeyboardInteractivity::OnDemand);
    let configure_count = peer.state.configures[&3].len();
    launcher.surface.commit();
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.configures[&3].len() > configure_count
    });
    launcher.attach(&peer, BLUE);
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.layer_count() == 4
    });
    assert_eq!(state.keyboard_focus(), app_focus);
    click(&mut state, 160.0, 130.0);
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.keyboard_focus.as_ref() == Some(&launcher.surface)
    });
    click(&mut state, 20.0, 80.0);
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.keyboard_focus() == app_focus
    });
    launcher.destroy();
    panel.destroy();
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.layer_count() == 2 && state.work_area().loc.y == 0
    });
    assert_eq!(
        state.window_location(state.managed_windows()[0]),
        Some((8, 8).into())
    );

    // A second client crashing releases its zone without affecting the live peer.
    let mut doomed = Peer::connect(&mut handle);
    until(&mut display, &mut state, &mut [&mut doomed], |_, peers| {
        peers[0].state.globals.contains_key("zwlr_layer_shell_v1")
    });
    let dock = doomed.layer(0, shell::Layer::Bottom, None);
    dock.role.set_size(0, 20);
    dock.role
        .set_anchor(layer::Anchor::Bottom | layer::Anchor::Left | layer::Anchor::Right);
    dock.role.set_exclusive_zone(20);
    dock.surface.commit();
    configured(&mut display, &mut state, &mut doomed, 0);
    dock.attach(&doomed, GREEN);
    until(&mut display, &mut state, &mut [&mut doomed], |state, _| {
        state.work_area().size.h == HEIGHT - 20
    });
    drop(dock);
    drop(doomed);
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.work_area().size.h == HEIGHT
    });
    pixel(&mut state, 20, 80, RED);

    notice.destroy();
    background.destroy();
    toplevel.destroy();
    xdg_surface.destroy();
    app.destroy();
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.layer_count() == 0 && state.window_count() == 0
    });
}
