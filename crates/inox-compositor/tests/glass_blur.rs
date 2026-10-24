//! Real Wayland clients exercise the private `inox-glass-v1` frosted-glass effect.
//!
//! This is the discriminative evidence for the feature: a background layer surface
//! draws a black/white pattern with a sharp vertical edge, a translucent layer
//! surface ("the pane") is stacked over that edge, and the pane's client asks the
//! compositor to frost it. The frame is read back twice — once with the same scene
//! and no glass object at all (the control), once with the effect — and the two
//! are compared column by column:
//!
//! - outside the pane the edge stays sharp, so the compositor did not blur
//!   anything it was not asked to;
//! - inside the pane the edge turns into a ramp tens of columns wide, so the blur
//!   really sampled the composited backdrop rather than the client's pixels;
//! - the pane region differs from the control render, so "frosting" is not a
//!   no-op that happens to leave the sharp scene on screen.
//!
//! It also pins the protocol's preconditions: `capabilities` is
//! [`MAX_BLUR_RADIUS`] for a surface that has a layer role, 0 for an ordinary
//! `xdg_toplevel`, and a second `get_glass` for one surface is a fatal protocol
//! error.
//!
//! One EGL display per process, so this file holds exactly one test, and the
//! compositor and every client are driven from the same thread without sleeps.

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
    glass::inox_glass::client::{inox_glass_manager_v1 as manager, inox_glass_v1 as glass},
    standard::{
        core::client::{
            wl_buffer, wl_callback, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
        },
        wlr::layer_shell::client::{zwlr_layer_shell_v1 as shell, zwlr_layer_surface_v1 as layer},
        xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
    },
};
use smithay::reexports::wayland_server::{Display, DisplayHandle};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

const WIDTH: i32 = 320;
const HEIGHT: i32 = 240;

/// Where the background's black-to-white edge sits.
const EDGE_X: i32 = 60;

/// The pane covers the top-left corner and therefore the edge.
const PANE_WIDTH: i32 = 120;
const PANE_HEIGHT: i32 = 80;

/// A row inside the pane (the pane spans y in `0..PANE_HEIGHT`).
const INSIDE_ROW: usize = 40;

/// A row well below the pane, where the backdrop must still be sharp.
const OUTSIDE_ROW: usize = 200;

/// The requested blur radius, in logical pixels (and physical: scale is 1).
const RADIUS: u32 = 24;

/// The pane's own pixels: premultiplied grey with ~38 % alpha (`wl_shm`'s
/// `Argb8888` is premultiplied), so the frosted backdrop dominates but the pane is
/// genuinely translucent rather than invisible.
const PANE_RGBA: [u8; 4] = [48, 48, 48, 96];

/// A sharp two-colour edge may straddle at most one texel; two is the tolerance
/// the existing pixel tests use for driver rounding.
const MAX_INTERMEDIATE_OUTSIDE_PANE: usize = 2;

#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    configures: HashMap<usize, Vec<(u32, i32, i32)>>,
    /// `capabilities` per glass object, keyed by the user data it was created with.
    max_radius: HashMap<usize, u32>,
    callbacks: usize,
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
            state
                .configures
                .entry(*id)
                .or_default()
                .push((serial, width as i32, height as i32));
            proxy.ack_configure(serial);
        }
    }
}

impl Dispatch<glass::InoxGlassV1, usize> for Client {
    fn event(
        state: &mut Self,
        _proxy: &glass::InoxGlassV1,
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
wayland_client::delegate_noop!(Client: ignore shell::ZwlrLayerShellV1);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(Client: ignore manager::InoxGlassManagerV1);

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

    /// A `wl_shm` buffer of one colour (premultiplied, as `Argb8888` requires).
    fn solid_buffer(&self, width: i32, height: i32, rgba: [u8; 4]) -> wl_buffer::WlBuffer {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            data.extend_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
        }
        self.shm_buffer(width, height, &data)
    }

    fn layer(&self, id: usize, depth: shell::Layer) -> Surface {
        let compositor: wl_compositor::WlCompositor = self.bind(5);
        let shell: shell::ZwlrLayerShellV1 = self.bind(4);
        let surface = compositor.create_surface(&self.queue.handle(), ());
        let role = shell.get_layer_surface(
            &surface,
            None,
            depth,
            format!("glass-test-{id}"),
            &self.queue.handle(),
            id,
        );
        Surface { surface, role, id }
    }

    /// Bind the private glass manager.
    fn glass_manager(&self) -> manager::InoxGlassManagerV1 {
        let (name, advertised) = self.state.globals["inox_glass_manager_v1"];
        self.registry
            .bind(name, advertised.min(1), &self.queue.handle(), ())
    }
}

struct Surface {
    surface: wl_surface::WlSurface,
    role: layer::ZwlrLayerSurfaceV1,
    id: usize,
}

impl Surface {
    fn configure_size(&self, peer: &Peer) -> (i32, i32) {
        let (_, width, height) = *peer.state.configures[&self.id].last().unwrap();
        (width, height)
    }

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
    for _ in 0..3 {
        step(display, state, peers);
    }
}

fn configured(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer, id: usize) {
    until(display, state, &mut [peer], |_, peers| {
        peers[0].state.configures.contains_key(&id)
    });
}

/// Wait until the server has processed every request already sent on `peer`.
///
/// A `wl_display.sync` callback is processed in order behind them, so this orders
/// the test without a sleep.
fn round_trip(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer) {
    let before = peer.state.callbacks;
    peer.connection.display().sync(&peer.queue.handle(), ());
    until(display, state, &mut [peer], |_, peers| {
        peers[0].state.callbacks > before
    });
}

/// The red channel of one row.
///
/// Every colour in this scene is grey, so the channel carries the whole signal and
/// the readback's `Abgr8888` byte order never has to be guessed.
fn row(frame: &[u8], y: usize) -> Vec<u8> {
    (0..WIDTH as usize)
        .map(|x| frame[(y * WIDTH as usize + x) * 4])
        .collect()
}

/// The crop the magnified evidence uses: the pane's band across the edge.
///
/// The numbers are the pane's own geometry (`PANE_WIDTH` × `PANE_HEIGHT` at the
/// top-left corner) inset by a few pixels, so what is magnified is exactly the
/// part of the frame the assertions measure: a black/white edge about to be
/// softened inside a translucent pane.
const CROP: (u32, u32, u32, u32) = (8, 8, 112, 72);

/// How much the magnified evidence is scaled up.
///
/// Four, with nearest-neighbour sampling: the blur is 24 columns wide, which is
/// legible at 1× once one knows to look, and unmistakable at 4×. Nearest
/// neighbour is deliberate — a smooth scaler would itself blur the evidence and
/// make the two frames harder to tell apart.
const ZOOM: u32 = 4;

/// Writes the frames this test measured, when `INOX_SCREENSHOT_DIR` is set.
///
/// The compositor has no render-test snapshots of its own, and this one exists
/// for a different reason than the L3 components' Q02 screenshots: the numbers the
/// assertions use are already printed, but "the backdrop under the pane is
/// blurred" is a claim about pixels, and a picture of the control and the frosted
/// frame side by side is what makes it checkable by eye. Nothing is written
/// without the variable, so a normal `cargo test` stays artifact-free.
fn snapshot(name: &str, frame: &[u8], zoom: bool) {
    let Ok(directory) = std::env::var("INOX_SCREENSHOT_DIR") else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(&directory) {
        panic!("the snapshot directory cannot be created: {error}");
    }

    let (width, height, pixels) = if zoom {
        let (x0, y0, x1, y1) = CROP;
        let mut pixels = Vec::with_capacity((((x1 - x0) * (y1 - y0) * ZOOM * ZOOM) as usize) * 4);
        for y in y0..y1 {
            let mut line = Vec::new();
            for x in x0..x1 {
                let index = ((y * WIDTH as u32 + x) * 4) as usize;
                let texel = &frame[index..index + 4];
                for _ in 0..ZOOM {
                    line.extend_from_slice(texel);
                }
            }
            for _ in 0..ZOOM {
                pixels.extend_from_slice(&line);
            }
        }
        ((x1 - x0) * ZOOM, (y1 - y0) * ZOOM, pixels)
    } else {
        (WIDTH as u32, HEIGHT as u32, frame.to_vec())
    };

    let path = std::path::Path::new(&directory).join(format!("inox-glass-{name}.png"));
    let image = image::RgbaImage::from_raw(width, height, pixels)
        .expect("the frame has the geometry the snapshot declares");
    image.save(&path).expect("the snapshot is written");
    println!("glass snapshot: {}", path.display());
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
    // A margin of an eighth of the range keeps driver rounding and the
    // translucent pane's own colour from being counted as a blur.
    let margin = (high - low) / 8;
    values
        .iter()
        .filter(|value| **value > low + margin && **value < high - margin)
        .count()
}

fn pane_columns() -> std::ops::Range<usize> {
    0..PANE_WIDTH as usize
}

/// Fail the process as soon as the server reports a protocol error.
fn rejected(display: &mut Display<InoxState>, state: &mut InoxState, peer: &mut Peer) {
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
    let error = peer
        .connection
        .protocol_error()
        .expect("expected a protocol error");
    assert_eq!(error.object_interface, "inox_glass_manager_v1");
    assert_eq!(
        error.code,
        u32::from(manager::Error::AlreadyCreated),
        "a second glass object for one surface is `already_created`"
    );
}

#[test]
fn frosting_blurs_the_backdrop_of_a_layer_surface_only() {
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
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.globals.contains_key("inox_glass_manager_v1")
    });
    assert_eq!(
        peer.state.globals["inox_glass_manager_v1"].1, 1,
        "the private glass global is advertised at its own version"
    );

    // The thing to be blurred: a background layer surface with a sharp black/white
    // vertical edge at EDGE_X.
    let background = peer.layer(0, shell::Layer::Background);
    background.role.set_size(0, 0);
    background.role.set_anchor(
        layer::Anchor::Top | layer::Anchor::Bottom | layer::Anchor::Left | layer::Anchor::Right,
    );
    background.role.set_exclusive_zone(-1);
    background.surface.commit();
    configured(&mut display, &mut state, &mut peer, 0);
    assert_eq!(background.configure_size(&peer), (WIDTH, HEIGHT));
    let pattern = peer.two_tone_buffer(WIDTH, HEIGHT, EDGE_X, [0, 0, 0, 255], [255, 255, 255, 255]);
    background.attach(&peer, &pattern);
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.layer_count() == 1
    });

    // The pane: a translucent top layer surface over the edge, no glass object yet.
    let pane = peer.layer(1, shell::Layer::Top);
    pane.role.set_size(PANE_WIDTH as u32, PANE_HEIGHT as u32);
    pane.role
        .set_anchor(layer::Anchor::Top | layer::Anchor::Left);
    pane.surface.commit();
    configured(&mut display, &mut state, &mut peer, 1);
    assert_eq!(pane.configure_size(&peer), (PANE_WIDTH, PANE_HEIGHT));
    let translucent = peer.solid_buffer(PANE_WIDTH, PANE_HEIGHT, PANE_RGBA);
    pane.attach(&peer, &translucent);
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.layer_count() == 2
    });

    // The control: the very same scene, the same client, but no glass object was
    // ever created, so nothing may be blurred.
    state.render_frame().unwrap();
    let control = state.read_frame_rgba().unwrap();
    snapshot("control", &control, false);
    snapshot("control-zoom", &control, true);

    // Now ask for the effect. The pane already has its layer role, so the
    // compositor must advertise its real maximum.
    let manager = peer.glass_manager();
    let glass = manager.get_glass(&pane.surface, &peer.queue.handle(), 1);
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.max_radius.contains_key(&1)
    });
    assert_eq!(
        peer.state.max_radius[&1], MAX_BLUR_RADIUS,
        "a layer surface is offered the compositor's real maximum"
    );
    glass.set_radius(RADIUS);
    glass.set_region(0, 0, PANE_WIDTH, PANE_HEIGHT);
    round_trip(&mut display, &mut state, &mut peer);

    state.render_frame().unwrap();
    let blurred = state.read_frame_rgba().unwrap();
    snapshot("frosted", &blurred, false);
    snapshot("frosted-zoom", &blurred, true);

    // Outside the pane the edge must still be sharp: the blur is clipped to the
    // region the client asked for.
    let outside = row(&blurred, OUTSIDE_ROW);
    let outside_transitions = intermediate(&outside);
    assert!(
        outside_transitions <= MAX_INTERMEDIATE_OUTSIDE_PANE,
        "the backdrop outside the pane must stay sharp, found {outside_transitions} \
         intermediate columns"
    );

    // Inside the pane the edge must be a ramp. The control pins that the ramp is
    // the effect and not some property of the translucent pane.
    let inside = row(&blurred, INSIDE_ROW)[pane_columns()].to_vec();
    let inside_transitions = intermediate(&inside);
    let control_inside = row(&control, INSIDE_ROW)[pane_columns()].to_vec();
    let control_transitions = intermediate(&control_inside);
    assert!(
        control_transitions <= MAX_INTERMEDIATE_OUTSIDE_PANE,
        "without a glass object the pane shows the sharp scene, found \
         {control_transitions} intermediate columns"
    );
    assert!(
        inside_transitions >= RADIUS as usize / 2,
        "inside the pane the edge must be blurred over at least {} columns, got \
         {inside_transitions} (control: {control_transitions})",
        RADIUS as usize / 2
    );

    // And the frost is not a no-op that leaves the sharp scene visible.
    let changed = inside
        .iter()
        .zip(&control_inside)
        .filter(|(blurred, control)| blurred.abs_diff(**control) > 8)
        .count();
    assert!(
        changed >= RADIUS as usize,
        "the pane region must differ from the control render, only {changed} columns do"
    );

    // The pane's own translucent pixels must be on screen over the frost: its dark
    // plateau is the pane's grey over black (~48), not black itself, and its light
    // one is not white either. Without this the frame could be blurring the
    // backdrop while silently dropping the pane's own element.
    let dark_plateau = inside[0];
    let light_plateau = inside[PANE_WIDTH as usize - 1];
    assert!(
        (44..=52).contains(&dark_plateau),
        "the pane's pixels must be composited over the frost: dark plateau {dark_plateau}, \
         expected the translucent pane over black"
    );
    assert!(
        (203..=211).contains(&light_plateau),
        "the pane's pixels must be composited over the frost: light plateau {light_plateau}, \
         expected the translucent pane over white"
    );

    // The pane's own sharp vertical edge at its right border is the other
    // direction: outside the pane, at the same row, the white backdrop continues.
    let pane_right = (PANE_WIDTH as usize) + 4;
    assert!(
        row(&blurred, INSIDE_ROW)[pane_right] >= 240,
        "the unrequested area beside the pane keeps the composited backdrop"
    );

    // The role precondition: an ordinary toplevel is composited by the tiling path,
    // which does not frost, so it must be told 0 — and asked twice it is a fatal
    // error rather than two controllers for one surface.
    let mut other = Peer::connect(&mut handle);
    until(&mut display, &mut state, &mut [&mut other], |_, peers| {
        peers[0].state.globals.contains_key("inox_glass_manager_v1")
    });
    let compositor: wl_compositor::WlCompositor = other.bind(5);
    let xdg: xdg_wm_base::XdgWmBase = other.bind(6);
    let app = compositor.create_surface(&other.queue.handle(), ());
    let app_surface = xdg.get_xdg_surface(&app, &other.queue.handle(), ());
    let _toplevel = app_surface.get_toplevel(&other.queue.handle(), ());
    app.commit();
    until(&mut display, &mut state, &mut [&mut other], |state, _| {
        state.window_count() == 1
    });
    let other_manager = other.glass_manager();
    let _toplevel_glass = other_manager.get_glass(&app, &other.queue.handle(), 7);
    until(&mut display, &mut state, &mut [&mut other], |_, peers| {
        peers[0].state.max_radius.contains_key(&7)
    });
    assert_eq!(
        other.state.max_radius[&7], 0,
        "an ordinary toplevel must not be offered a blur this compositor will not draw"
    );
    let _second = other_manager.get_glass(&app, &other.queue.handle(), 8);
    rejected(&mut display, &mut state, &mut other);

    // The first client is unaffected: one connection's protocol error is not
    // another's, and the session keeps compositing its frosted pane. (The
    // rejected client's own buffer-less window is gone with it.)
    assert_eq!(
        state.window_count(),
        0,
        "the disconnected client's window is released"
    );
    state.render_frame().unwrap();
    let after = state.read_frame_rgba().unwrap();
    assert!(
        intermediate(&row(&after, INSIDE_ROW)[pane_columns()]) >= RADIUS as usize / 2,
        "the surviving client's glass object is still applying its blur"
    );

    // Frosting ends with the object: the parameters must not outlive it, and the
    // surface must be back to the sharp scene.
    glass.destroy();
    round_trip(&mut display, &mut state, &mut peer);
    state.render_frame().unwrap();
    let object_gone = state.read_frame_rgba().unwrap();
    assert!(
        intermediate(&row(&object_gone, INSIDE_ROW)[pane_columns()])
            <= MAX_INTERMEDIATE_OUTSIDE_PANE,
        "destroying the glass object must end the frost"
    );
    assert!(
        row(&object_gone, INSIDE_ROW)
            .iter()
            .zip(row(&control, INSIDE_ROW).iter())
            .all(|(now, before)| now.abs_diff(*before) <= 1),
        "with the object destroyed the frame is the control frame again"
    );

    // A surface may ask again once its object is gone — the parameters were
    // dropped with the object rather than lingering as a phantom controller.
    let manager = peer.glass_manager();
    let again = manager.get_glass(&pane.surface, &peer.queue.handle(), 2);
    until(&mut display, &mut state, &mut [&mut peer], |_, peers| {
        peers[0].state.max_radius.contains_key(&2)
    });
    assert_eq!(peer.state.max_radius[&2], MAX_BLUR_RADIUS);
    again.set_radius(RADIUS);
    again.set_region(0, 0, PANE_WIDTH, PANE_HEIGHT);
    round_trip(&mut display, &mut state, &mut peer);
    state.render_frame().unwrap();
    let refrosted = state.read_frame_rgba().unwrap();
    assert!(
        intermediate(&row(&refrosted, INSIDE_ROW)[pane_columns()]) >= RADIUS as usize / 2,
        "a second glass object after the first was destroyed frosts again"
    );

    // And it ends with the surface: a destroyed surface is never frosted again.
    pane.role.destroy();
    pane.surface.destroy();
    until(&mut display, &mut state, &mut [&mut peer], |state, _| {
        state.layer_count() == 1
    });
    state.render_frame().unwrap();
    let surface_gone = state.read_frame_rgba().unwrap();
    assert!(
        intermediate(&row(&surface_gone, INSIDE_ROW)) <= MAX_INTERMEDIATE_OUTSIDE_PANE,
        "a destroyed surface leaves no frosted pane behind"
    );

    // The numbers this test is the evidence for.
    println!(
        "glass blur: sharp-edge intermediate columns outside the pane = {outside_transitions}; \
         inside the pane = {inside_transitions} (control = {control_transitions}); \
         columns changed vs control = {changed}; radius = {RADIUS}, max advertised = {MAX_BLUR_RADIUS}"
    );
}
