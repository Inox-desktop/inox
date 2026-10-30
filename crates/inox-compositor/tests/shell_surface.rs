//! Real Wayland clients exercise `inox-shell-v1` on the software-rendered Inox host.
//!
//! The clients are real `wayland-client` connections, but they are **stand-ins
//! written for this test**, not `inox-panel`. What this file proves is the
//! protocol half and the compositing behaviour ADR 0046 asks for: placement and
//! exclusive-zone reservation shared with `zwlr-layer-shell`, the work area as an
//! event, popups drawn above their parent but not above a higher layer, a click
//! that dismisses a menu *and is consumed*, the popup keyboard grant, and the
//! enter transition the compositor draws itself.
//!
//! It does **not** prove that the panel *appears* like this on a real desktop —
//! that is the cross-L3 E3 evidence, which is still missing and which a headless
//! readback cannot supply.
//!
//! With `INOX_SCREENSHOT_DIR` set, each asserted scene is also written there as a
//! PNG (see [`snapshot`]), so CI can collect a picture next to the numbers.

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
    shell::inox_shell::client::{
        inox_shell_popup_v1 as popup, inox_shell_surface_v1 as role, inox_shell_v1 as shell,
    },
    standard::{
        core::client::{
            wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm,
            wl_shm_pool, wl_surface,
        },
        wlr::layer_shell::client::{zwlr_layer_shell_v1 as layers, zwlr_layer_surface_v1 as layer},
    },
};
use smithay::{
    backend::input::ButtonState,
    reexports::wayland_server::{Display, DisplayHandle},
    utils::Point,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

const WIDTH: i32 = 320;
const HEIGHT: i32 = 240;
const BAR: i32 = 32;
const RED: [u8; 4] = [220, 30, 20, 255];
const GREEN: [u8; 4] = [20, 210, 30, 255];
const BLUE: [u8; 4] = [20, 30, 220, 255];
const YELLOW: [u8; 4] = [220, 210, 20, 255];
const BACKDROP: [u8; 4] = [10, 12, 16, 255];

/// One unique id per surface object a test creates, used as the protocol user
/// data so every configure can be attributed.
type Id = usize;

#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    capabilities: Option<(u32, u32)>,
    configures: HashMap<Id, Vec<(u32, u32, u32)>>,
    workareas: HashMap<Id, (i32, i32, i32, i32)>,
    keyboard_states: HashMap<Id, Vec<u32>>,
    dismissed: Vec<(Id, u32)>,
    closed: usize,
    /// Presses delivered to the client, not releases: the press that dismisses a
    /// menu is consumed, and the release of that same click is still delivered so
    /// the client never believes a button is stuck down.
    presses: usize,
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

impl Dispatch<shell::InoxShellV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &shell::InoxShellV1,
        event: shell::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let shell::Event::Capabilities {
            capabilities,
            max_transition_duration,
        } = event
        {
            state.capabilities = Some((capabilities.into(), max_transition_duration));
        }
    }
}

impl Dispatch<role::InoxShellSurfaceV1, Id> for Client {
    fn event(
        state: &mut Self,
        _: &role::InoxShellSurfaceV1,
        event: role::Event,
        id: &Id,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            role::Event::Configure {
                serial,
                width,
                height,
            } => state
                .configures
                .entry(*id)
                .or_default()
                .push((serial, width, height)),
            role::Event::Workarea {
                x,
                y,
                width,
                height,
            } => {
                state.workareas.insert(*id, (x, y, width, height));
            }
            role::Event::KeyboardState { focused } => {
                state.keyboard_states.entry(*id).or_default().push(focused);
            }
            role::Event::Closed => state.closed += 1,
            _ => {}
        }
    }
}

impl Dispatch<popup::InoxShellPopupV1, Id> for Client {
    fn event(
        state: &mut Self,
        _: &popup::InoxShellPopupV1,
        event: popup::Event,
        id: &Id,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            popup::Event::Configure {
                serial,
                width,
                height,
            } => state
                .configures
                .entry(*id)
                .or_default()
                .push((serial, width, height)),
            popup::Event::Dismissed { reason } => state.dismissed.push((*id, reason.into())),
            popup::Event::KeyboardState { focused } => {
                state.keyboard_states.entry(*id).or_default().push(focused);
            }
            _ => {}
        }
    }
}

impl Dispatch<layer::ZwlrLayerSurfaceV1, Id> for Client {
    fn event(
        state: &mut Self,
        proxy: &layer::ZwlrLayerSurfaceV1,
        event: layer::Event,
        id: &Id,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let layer::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            state
                .configures
                .entry(*id)
                .or_default()
                .push((serial, width, height));
            proxy.ack_configure(serial);
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
        if let wl_pointer::Event::Button {
            state:
                wayland_client::WEnum::Value(wayland_client::protocol::wl_pointer::ButtonState::Pressed),
            ..
        } = event
        {
            state.presses += 1;
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for Client {
    fn event(
        _: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        _: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Client {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore layers::ZwlrLayerShellV1);

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

    fn last_configure(&self, id: Id) -> (u32, i32, i32) {
        let (serial, width, height) = *self.state.configures[&id]
            .last()
            .unwrap_or_else(|| panic!("surface {id} was never configured"));

        (serial, width as i32, height as i32)
    }

    fn surface(&self) -> wl_surface::WlSurface {
        let compositor: wl_compositor::WlCompositor = self.bind(5);

        compositor.create_surface(&self.queue.handle(), ())
    }

    fn shell_surface(&self, id: Id, kind: shell::SurfaceRole) -> ShellSurface {
        let factory: shell::InoxShellV1 = self.bind(1);
        let surface = self.surface();
        let role = factory.get_surface(&surface, kind, &self.queue.handle(), id);

        ShellSurface { surface, role, id }
    }

    fn layer_surface(&self, id: Id, depth: layers::Layer) -> LayerSurface {
        let factory: layers::ZwlrLayerShellV1 = self.bind(4);
        let surface = self.surface();
        let role = factory.get_layer_surface(
            &surface,
            None,
            depth,
            format!("probe-{id}"),
            &self.queue.handle(),
            id,
        );

        LayerSurface { surface, role, id }
    }
}

struct ShellSurface {
    surface: wl_surface::WlSurface,
    role: role::InoxShellSurfaceV1,
    id: Id,
}

impl ShellSurface {
    /// Ask to be a bar across the top edge, `height` thick, reserving that strip.
    fn bar(&self, height: u32) {
        self.role.set_placement(
            role::Layer::Top,
            role::Anchor::Top | role::Anchor::Left | role::Anchor::Right,
            i32::try_from(height).unwrap(),
            0,
            0,
            0,
            0,
            0,
            height,
        );
        self.surface.commit();
    }

    fn attach(&self, peer: &Peer, colour: [u8; 4]) {
        let (_, width, height) = peer.last_configure(self.id);
        let buffer = peer.buffer(width, height, colour);
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage_buffer(0, 0, width, height);
        self.surface.commit();
    }
}

struct LayerSurface {
    surface: wl_surface::WlSurface,
    role: layer::ZwlrLayerSurfaceV1,
    id: Id,
}

impl LayerSurface {
    /// A full-output surface in its layer, which is what makes it a probe for the
    /// work area.
    fn cover(&self) {
        self.role.set_anchor(
            layer::Anchor::Top | layer::Anchor::Bottom | layer::Anchor::Left | layer::Anchor::Right,
        );
        self.surface.commit();
    }

    fn attach(&self, peer: &Peer, colour: [u8; 4]) {
        let (_, width, height) = peer.last_configure(self.id);
        let buffer = peer.buffer(width, height, colour);
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface.damage_buffer(0, 0, width, height);
        self.surface.commit();
    }
}

fn step(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer) {
    peer.pump();
    display.dispatch_clients(state).unwrap();
    display.flush_clients().unwrap();
}

fn until(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peer: &mut Peer,
    what: &str,
    condition: impl Fn(&InoxState, &Peer) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        step(display, state, peer);
        if condition(state, peer) {
            break;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
    }
    for _ in 0..3 {
        step(display, state, peer);
    }
}

fn configured(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer, id: Id) {
    until(display, state, peer, "a configure", |_, peer| {
        peer.state
            .configures
            .get(&id)
            .is_some_and(|list| !list.is_empty())
    });
}

fn settle(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer) {
    for _ in 0..4 {
        step(display, state, peer);
    }
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

fn pixel_is_not(state: &mut InoxState, x: usize, y: usize, unwanted: [u8; 4]) {
    state.render_frame().unwrap();
    let bytes = state.read_frame_rgba().unwrap();
    let offset = (y * WIDTH as usize + x) * 4;
    assert!(
        bytes[offset..offset + 4]
            .iter()
            .zip(unwanted)
            .any(|(actual, unwanted)| actual.abs_diff(unwanted) > 2),
        "pixel ({x}, {y}) is already the final colour: {:?}",
        &bytes[offset..offset + 4]
    );
}

/// Writes a frame this test asserted on, when `INOX_SCREENSHOT_DIR` is set.
///
/// The assertions already prove the scene numerically; the PNG exists so a human
/// can check the part numbers cannot argue about — that the menu hangs off the
/// bar rather than out of the middle of the screen.
fn snapshot(name: &str, state: &mut InoxState) {
    let Ok(directory) = std::env::var("INOX_SCREENSHOT_DIR") else {
        return;
    };
    state.render_frame().unwrap();
    let frame = state.read_frame_rgba().unwrap();
    if let Err(error) = std::fs::create_dir_all(&directory) {
        panic!("the snapshot directory cannot be created: {error}");
    }
    let path = std::path::Path::new(&directory).join(format!("inox-shell-{name}.png"));
    image::RgbaImage::from_raw(WIDTH as u32, HEIGHT as u32, frame)
        .expect("the readback has the output's geometry")
        .save(&path)
        .expect("the snapshot is written");
    println!("shell snapshot: {}", path.display());
}

fn click(state: &mut InoxState, x: f64, y: f64) {
    state.inject_pointer_motion(Point::from((x, y)));
    state.inject_pointer_button(0x110, ButtonState::Pressed);
    state.inject_pointer_button(0x110, ButtonState::Released);
}

fn host() -> (Display<InoxState>, InoxState, Peer) {
    let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
    let mut display = Display::<InoxState>::new().unwrap();
    let mut handle = display.handle();
    let config = CompositorConfig {
        outputs: vec![HeadlessOutput {
            width: WIDTH,
            height: HEIGHT,
            ..HeadlessOutput::default()
        }],
        ..Default::default()
    };
    let mut state = InoxState::new(&handle, &config, event_loop.get_signal()).unwrap();
    let mut peer = Peer::connect(&mut handle);
    until(
        &mut display,
        &mut state,
        &mut peer,
        "the shell global",
        |_, peer| peer.state.globals.contains_key("inox_shell_v1"),
    );

    (display, state, peer)
}

/// The factory reports every bit it implements, and a shell that depends on one
/// of them has to be able to read it before it uses the object.
#[test]
fn the_factory_reports_what_it_implements() {
    let (mut display, mut state, mut peer) = host();
    let _factory: shell::InoxShellV1 = peer.bind(1);
    until(
        &mut display,
        &mut state,
        &mut peer,
        "the capability event",
        |_, peer| peer.state.capabilities.is_some(),
    );
    let (capabilities, duration) = peer.state.capabilities.expect("capabilities arrived");

    let expected = shell::Capability::Transitions
        | shell::Capability::PopupKeyboard
        | shell::Capability::Workarea
        | shell::Capability::NestedPopups;
    assert_eq!(capabilities, u32::from(expected));
    assert_eq!(duration, 250, "the longest transition the compositor draws");
}

/// A panel is placed like a layer surface, reserves its strip, and is told the
/// work area that is left — the three numbers a shell cannot guess.
#[test]
fn a_panel_is_placed_and_reserves_its_strip() {
    let (mut display, mut state, mut peer) = host();

    let panel = peer.shell_surface(1, shell::SurfaceRole::Panel);
    panel.bar(BAR as u32);
    configured(&mut display, &mut state, &mut peer, 1);

    assert_eq!(peer.last_configure(1).1, WIDTH, "the bar spans the output");
    assert_eq!(peer.last_configure(1).2, BAR, "and is as thick as it asked");

    panel.attach(&peer, RED);
    until(
        &mut display,
        &mut state,
        &mut peer,
        "the reserved work area",
        |_, peer| peer.state.workareas.get(&1) == Some(&(0, BAR, WIDTH, HEIGHT - BAR)),
    );

    // The work area is an event, not a re-derivation: after the bar reserves its
    // strip, a third-party layer surface that asks for the whole screen is sized
    // to what is left.
    let probe = peer.layer_surface(2, layers::Layer::Top);
    probe.cover();
    configured(&mut display, &mut state, &mut peer, 2);
    assert_eq!(
        peer.last_configure(2).2,
        HEIGHT - BAR,
        "the panel's strip is not available to anything else"
    );

    probe.attach(&peer, BACKDROP);
    settle(&mut display, &mut state, &mut peer);

    pixel(&mut state, 160, 16, RED);
    pixel(&mut state, 160, 200, BACKDROP);
    snapshot("bar", &mut state);
}

/// A menu hangs off the bar, a submenu hangs off the menu, and a surface in a
/// higher layer covers both: a popup is drawn above its *parent*, never above
/// the whole desktop.
#[test]
fn a_popup_is_drawn_above_its_parent_and_below_a_higher_layer() {
    let (mut display, mut state, mut peer) = host();

    let panel = peer.shell_surface(1, shell::SurfaceRole::Panel);
    panel.bar(BAR as u32);
    configured(&mut display, &mut state, &mut peer, 1);
    panel.attach(&peer, RED);

    // The menu: its top-left corner on the bar's bottom-left corner, offset by
    // the ten pixels a menu leaves for its shadow.
    let menu_surface = peer.surface();
    let menu = panel.role.get_popup(&menu_surface, &peer.queue.handle(), 2);
    menu.set_position(
        popup::Anchor::Bottom | popup::Anchor::Left,
        popup::Anchor::Top | popup::Anchor::Left,
        10,
        5,
    );
    menu.set_size(80, 60);
    menu.set_dismissable(1);
    menu_surface.commit();
    configured(&mut display, &mut state, &mut peer, 2);
    assert_eq!(
        (peer.last_configure(2).1, peer.last_configure(2).2),
        (80, 60),
        "a popup gets the size it asked for"
    );
    let (_, width, height) = peer.last_configure(2);
    let buffer = peer.buffer(width, height, GREEN);
    menu_surface.attach(Some(&buffer), 0, 0);
    menu_surface.damage_buffer(0, 0, width, height);
    menu_surface.commit();
    settle(&mut display, &mut state, &mut peer);

    // (10, 37) .. (90, 97): 80x60 under the bar's bottom-left corner plus the
    // offset the client asked for.
    pixel(&mut state, 50, 50, GREEN);
    pixel(&mut state, 160, 16, RED);
    snapshot("menu", &mut state);

    // A submenu of the menu, hanging off *its* bottom-left corner.
    let submenu_surface = peer.surface();
    let submenu = menu.get_popup(&submenu_surface, &peer.queue.handle(), 3);
    submenu.set_position(
        popup::Anchor::Bottom | popup::Anchor::Left,
        popup::Anchor::Top | popup::Anchor::Left,
        0,
        2,
    );
    submenu.set_size(40, 30);
    submenu_surface.commit();
    configured(&mut display, &mut state, &mut peer, 3);
    let (_, width, height) = peer.last_configure(3);
    let buffer = peer.buffer(width, height, BLUE);
    submenu_surface.attach(Some(&buffer), 0, 0);
    submenu_surface.damage_buffer(0, 0, width, height);
    submenu_surface.commit();
    settle(&mut display, &mut state, &mut peer);

    // The popup is 37..97 tall, so the submenu starts at 99.
    pixel(&mut state, 20, 110, BLUE);
    pixel(&mut state, 20, 50, GREEN);

    // A surface in a higher layer covers the whole menu, and that is what the
    // popup must not paint over.
    let overlay = peer.layer_surface(4, layers::Layer::Overlay);
    overlay.cover();
    configured(&mut display, &mut state, &mut peer, 4);
    overlay.attach(&peer, YELLOW);
    settle(&mut display, &mut state, &mut peer);

    pixel(&mut state, 50, 50, YELLOW);
    pixel(&mut state, 20, 110, YELLOW);
    snapshot("covered", &mut state);
}

/// A click outside a dismissable popup closes it, tells the client why, and is
/// consumed: the surface underneath must not also act on it.
#[test]
fn a_click_outside_dismisses_and_is_swallowed() {
    let (mut display, mut state, mut peer) = host();
    let seat: wl_seat::WlSeat = peer.bind(9);
    let _keyboard = seat.get_keyboard(&peer.queue.handle(), ());
    let _pointer = seat.get_pointer(&peer.queue.handle(), ());

    // Something underneath for the pointer to land on.
    let backdrop = peer.layer_surface(5, layers::Layer::Top);
    backdrop.cover();
    configured(&mut display, &mut state, &mut peer, 5);
    backdrop.attach(&peer, BACKDROP);
    settle(&mut display, &mut state, &mut peer);

    let panel = peer.shell_surface(6, shell::SurfaceRole::Panel);
    panel.bar(BAR as u32);
    configured(&mut display, &mut state, &mut peer, 6);
    panel.attach(&peer, RED);

    let menu_surface = peer.surface();
    let menu = panel.role.get_popup(&menu_surface, &peer.queue.handle(), 7);
    menu.set_position(popup::Anchor::Bottom, popup::Anchor::Top, 0, 0);
    menu.set_size(60, 50);
    menu.set_dismissable(1);
    // A menu wants keys even though its parent never takes focus — that is D01
    // and the popup keyboard request meeting in one object.
    menu.set_keyboard(1);
    menu_surface.commit();
    configured(&mut display, &mut state, &mut peer, 7);
    let (_, width, height) = peer.last_configure(7);
    let buffer = peer.buffer(width, height, GREEN);
    menu_surface.attach(Some(&buffer), 0, 0);
    menu_surface.damage_buffer(0, 0, width, height);
    menu_surface.commit();
    until(
        &mut display,
        &mut state,
        &mut peer,
        "the popup keyboard grant",
        |_, peer| {
            peer.state
                .keyboard_states
                .get(&7)
                .is_some_and(|states| states.first() == Some(&1))
        },
    );

    // The menu is 60x50 under the panel's bottom-left corner, so (200, 180) is
    // outside it.
    click(&mut state, 200.0, 180.0);
    until(
        &mut display,
        &mut state,
        &mut peer,
        "the dismissal",
        |_, peer| !peer.state.dismissed.is_empty(),
    );

    assert_eq!(
        peer.state.dismissed,
        vec![(7, 0)],
        "the client is told it was a click outside"
    );
    assert_eq!(
        peer.state.presses, 0,
        "the click that closed the menu must not reach the surface underneath"
    );
    until(
        &mut display,
        &mut state,
        &mut peer,
        "the keyboard being released",
        |_, peer| {
            peer.state
                .keyboard_states
                .get(&7)
                .is_some_and(|states| states.last() == Some(&0))
        },
    );

    // The same click, with nothing open, does reach the surface underneath —
    // without this the assertion above would also pass on a compositor that
    // never delivered a button at all.
    click(&mut state, 200.0, 180.0);
    until(
        &mut display,
        &mut state,
        &mut peer,
        "a button reaching the window underneath",
        |_, peer| peer.state.presses > 0,
    );
}

/// The compositor draws the enter transition itself: the frame drawn the instant
/// the surface maps is not the final one, and a later frame is.
#[test]
fn the_compositor_draws_the_enter_transition() {
    let (mut display, mut state, mut peer) = host();

    let panel = peer.shell_surface(8, shell::SurfaceRole::Panel);
    panel.role.set_placement(
        role::Layer::Top,
        role::Anchor::Top | role::Anchor::Left | role::Anchor::Right,
        BAR,
        0,
        0,
        0,
        0,
        0,
        BAR as u32,
    );
    panel.role.set_transition(
        role::Effect::Fade,
        role::Effect::Fade,
        250,
        role::Curve::EaseOut,
        role::Anchor::Top,
    );
    panel.surface.commit();
    configured(&mut display, &mut state, &mut peer, 8);
    panel.attach(&peer, RED);
    // The commit has to reach the compositor before the frame that is asserted
    // on: this is the map, and the transition starts with it.
    settle(&mut display, &mut state, &mut peer);

    // The frame drawn the instant the surface maps is still almost entirely
    // background.
    pixel_is_not(&mut state, 160, 16, RED);
    snapshot("fade-in", &mut state);

    std::thread::sleep(Duration::from_millis(400));
    pixel(&mut state, 160, 16, RED);
    snapshot("fade-in-done", &mut state);
}
