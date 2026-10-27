//! Real Wayland clients exercise the private `inox-glass-v1` glass effect.
//!
//! This is the discriminative evidence for the feature. Two scenes are rendered
//! and read back:
//!
//! - the version 1 half: a background layer surface draws a black/white pattern
//!   with a sharp vertical edge, a translucent layer surface ("the pane") is
//!   stacked over that edge, and the pane's client asks the compositor to blur it.
//!   Outside the pane the edge stays sharp, inside it turns into a ramp tens of
//!   columns wide, and the same scene without a glass object (the control) shows
//!   the sharp edge — so "the blur was deleted" fails the test;
//! - the version 2 half: the backdrop is a striped pattern whose first black/white
//!   edge sits 8 px inside the pane, and the pane asks for refraction only
//!   (`radius = 0`). The edge moves outwards by a measurable number of columns, the
//!   pane's interior stays byte-identical to the control (the band really is a
//!   band), the pixels outside the rounded corner stay byte-identical (the SDF mask
//!   really masks), and switching the dispersion bit on separates the red and blue
//!   channels at the edge while the depth bit keeps moving inside the band only.
//!
//! It also pins the protocol's preconditions: the global is advertised at version
//! 2, `capabilities` is [`MAX_BLUR_RADIUS`] and `liquid_capabilities` reports the
//! refraction maxima for a surface that has a layer role, both are 0 for an
//! ordinary `xdg_toplevel`, a version 1 object never receives the version 2 event,
//! and a second `get_glass` for one surface is a fatal protocol error.
//!
//! With `INOX_GLASS_ARTIFACT` set to a path, the final refraction frame is also
//! written there as a binary PPM (P6, 8-bit RGB) so a human can look at the effect
//! without a GPU. Nothing is written when the variable is unset, which is every CI
//! run.
//!
//! One EGL display per process, so this file holds exactly one test, and the
//! compositor and every client are driven from the same thread without sleeps.

use std::{
    collections::HashMap,
    io::{self, Write},
    ops::Range,
    os::{fd::AsFd, unix::net::UnixStream},
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{
    ClientState, CompositorConfig, HeadlessOutput, InoxState,
    glass::{MAX_BLUR_RADIUS, MAX_REFRACTION_AMOUNT, MAX_REFRACTION_HEIGHT},
};
use inox_protocols::{
    glass::inox_glass::client::{
        inox_glass_manager_v1 as manager, inox_glass_v1 as glass, inox_glass_v1::RefractionFlag,
    },
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

/// The version 2 pane: taller and wider than the blur one, so its interior is far
/// from every edge.
const LENS_PANE_WIDTH: i32 = 160;
const LENS_PANE_HEIGHT: i32 = 120;

/// A row through the middle of the refraction pane: 60 px from the top and the
/// bottom edges, both of which are well outside the 24 px band.
const LENS_ROW: usize = 60;

/// Only the top-left corner is round, so the readback can tell the four wire
/// values of `set_corner_radii` apart.
const LENS_CORNER_RADIUS: u32 = 32;

/// The refraction band, in logical pixels (and physical: scale is 1).
const LENS_BAND: u32 = 24;

/// The peak displacement. Small enough that the backdrop's first edge stays
/// inside the pane after it is pushed outwards, large enough to move it several
/// columns.
const LENS_AMOUNT: u32 = 8;

/// The striped backdrop of the refraction phase: black bands 8 px wide every
/// 32 px, white in between.
///
/// The first black-to-white edge therefore sits at `x = 8`, eight pixels inside the
/// pane's left outline, and the 32 px period is longer than the band is wide, so
/// "the first edge moved" cannot be confused with the next edge along.
const STRIPE_PERIOD: i32 = 32;
const STRIPE_BLACK: i32 = 8;

#[derive(Default)]
struct Client {
    globals: HashMap<String, (u32, u32)>,
    configures: HashMap<usize, Vec<(u32, i32, i32)>>,
    /// `capabilities` (`max_radius`) per glass object.
    max_radius: HashMap<usize, u32>,
    /// `liquid_capabilities` per glass object: the two refraction maxima and the
    /// flag bits, as `(height, amount, flags)`.
    liquid: HashMap<usize, (u32, u32, u32)>,
    /// The capability events each glass object received, in order: a version 2
    /// object must see `capabilities` first even though it is told to trust the
    /// later one.
    events: HashMap<usize, Vec<&'static str>>,
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
        match event {
            glass::Event::Capabilities { max_radius } => {
                state.max_radius.insert(*id, max_radius);
                state.events.entry(*id).or_default().push("capabilities");
            }
            glass::Event::LiquidCapabilities {
                max_radius,
                max_refraction_height,
                max_refraction_amount,
                flags,
            } => {
                // The flag set is closed, but a compositor may add a bit before
                // this client knows it; the raw bits are what is compared.
                let flags = u32::from(flags);
                state.max_radius.insert(*id, max_radius);
                state
                    .liquid
                    .insert(*id, (max_refraction_height, max_refraction_amount, flags));
                state
                    .events
                    .entry(*id)
                    .or_default()
                    .push("liquid_capabilities");
            }
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

    /// A `wl_shm` buffer of vertical stripes: black for [`STRIPE_BLACK`] columns,
    /// white for the rest of every [`STRIPE_PERIOD`].
    fn stripes_buffer(&self, width: i32, height: i32) -> wl_buffer::WlBuffer {
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..height {
            for x in 0..width {
                let colour = if x % STRIPE_PERIOD < STRIPE_BLACK {
                    [0u8, 0, 0, 255]
                } else {
                    [255u8, 255, 255, 255]
                };
                data.extend_from_slice(&[colour[2], colour[1], colour[0], colour[3]]);
            }
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

    /// Bind the private glass manager at version 1.
    fn glass_manager(&self) -> manager::InoxGlassManagerV1 {
        let (name, advertised) = self.state.globals["inox_glass_manager_v1"];
        self.registry
            .bind(name, advertised.min(1), &self.queue.handle(), ())
    }

    /// Bind the private glass manager at version 2, which is what creates version
    /// 2 glass objects (a child object is created at its parent's version).
    fn glass_manager_v2(&self) -> manager::InoxGlassManagerV1 {
        let (name, advertised) = self.state.globals["inox_glass_manager_v1"];
        assert!(
            advertised >= 2,
            "the refraction requests need a version 2 manager, advertised {advertised}"
        );
        self.registry
            .bind(name, advertised.min(2), &self.queue.handle(), ())
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

/// The four RGBA bytes of one pixel of a readback frame.
fn pixel(frame: &[u8], x: usize, y: usize) -> [u8; 4] {
    let index = (y * WIDTH as usize + x) * 4;
    [
        frame[index],
        frame[index + 1],
        frame[index + 2],
        frame[index + 3],
    ]
}

/// Whether two pixels agree within `tolerance` 8-bit steps, channel by channel.
///
/// Drivers round float→unorm differently, so a one-step tolerance is what the
/// crate's other pixel tests use for "the same pixel".
fn same(left: [u8; 4], right: [u8; 4], tolerance: u8) -> bool {
    left.iter()
        .zip(&right)
        .all(|(a, b)| a.abs_diff(*b) <= tolerance)
}

/// The columns of one row where two frames differ by more than `tolerance` in any
/// channel.
fn differing(
    left: &[u8],
    right: &[u8],
    y: usize,
    columns: Range<usize>,
    tolerance: u8,
) -> Vec<usize> {
    columns
        .filter(|x| !same(pixel(left, *x, y), pixel(right, *x, y), tolerance))
        .collect()
}

/// The first column in `columns` where the row goes from dark to light.
///
/// Every colour in these scenes is grey, so a hard black/white edge is a single
/// step; the midpoint of the two plateaus is the crossing. `None` means the whole
/// range is one side of it — which is itself evidence, so callers decide what it
/// means.
fn first_edge(frame: &[u8], y: usize, columns: Range<usize>) -> Option<usize> {
    let row = row(frame, y);
    let values = &row[columns.clone()];
    let low = *values.iter().min()?;
    let high = *values.iter().max()?;
    let middle = (u16::from(low) + u16::from(high)) / 2;
    let start = columns.start.max(1);
    (start..columns.end).find(|x| u16::from(row[*x]) > middle && u16::from(row[x - 1]) <= middle)
}

/// Writes the final refraction frame as a binary PPM (P6, 8-bit RGB) when
/// `INOX_GLASS_ARTIFACT` names a path.
///
/// The assertions print the numbers they measured, but "the edge is bent outwards"
/// is a claim about a picture, and this is the cheapest way to make it checkable by
/// eye without a GPU. The readback is RGBA8 while PPM wants RGB; alpha is dropped
/// because the headless target is opaque. Nothing is written (and no directory is
/// created) when the variable is unset, which is every CI run.
fn write_artifact(frame: &[u8]) {
    let Ok(path) = std::env::var("INOX_GLASS_ARTIFACT") else {
        return;
    };
    let path = Path::new(&path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).expect("the artifact directory is created");
    }

    let mut ppm = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    ppm.reserve((WIDTH * HEIGHT * 3) as usize);
    for texel in frame.as_chunks::<4>().0 {
        ppm.extend_from_slice(&texel[..3]);
    }
    std::fs::write(path, ppm).expect("the artifact frame is written");
    println!("glass artifact: {}", path.display());
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

/// The version 2 half of the evidence: refraction on a striped backdrop.
///
/// Everything here is measured against a control frame of the same scene with no
/// glass object, so each assertion separates "the compositor refracted the
/// backdrop" from "the client's translucent pixels happened to look like that".
fn refraction_phase(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    peer: &mut Peer,
    background: &Surface,
) {
    // The same background surface, now a pattern whose first black-to-white edge
    // sits 8 px inside the pane: the refraction band has something to move.
    let stripes = peer.stripes_buffer(WIDTH, HEIGHT);
    background.attach(peer, &stripes);
    round_trip(display, state, peer);

    // A fresh pane over the left part of the backdrop, no glass object yet.
    let pane = peer.layer(3, shell::Layer::Top);
    pane.role
        .set_size(LENS_PANE_WIDTH as u32, LENS_PANE_HEIGHT as u32);
    pane.role
        .set_anchor(layer::Anchor::Top | layer::Anchor::Left);
    pane.surface.commit();
    configured(display, state, peer, 3);
    assert_eq!(
        pane.configure_size(peer),
        (LENS_PANE_WIDTH, LENS_PANE_HEIGHT)
    );
    let tint = peer.solid_buffer(LENS_PANE_WIDTH, LENS_PANE_HEIGHT, PANE_RGBA);
    pane.attach(peer, &tint);
    until(display, state, &mut [peer], |state, _| {
        state.layer_count() == 2
    });

    // The control: the same scene with no glass object, so the backdrop is
    // composited untouched.
    state.render_frame().unwrap();
    let control = state.read_frame_rgba().unwrap();

    // A version 2 manager creates a version 2 object, which is told what this
    // compositor refracts — after it is told what it blurs.
    let manager = peer.glass_manager_v2();
    let controller = manager.get_glass(&pane.surface, &peer.queue.handle(), 5);
    until(display, state, &mut [peer], |_, peers| {
        peers[0].state.liquid.contains_key(&5)
    });
    assert_eq!(
        peer.state.events[&5],
        vec!["capabilities", "liquid_capabilities"],
        "a version 2 object still receives the version 1 event first: a client that \
         only listens for it must not find its object mute"
    );
    assert_eq!(peer.state.max_radius[&5], MAX_BLUR_RADIUS);
    assert_eq!(
        peer.state.liquid[&5],
        (
            MAX_REFRACTION_HEIGHT,
            MAX_REFRACTION_AMOUNT,
            (RefractionFlag::Depth | RefractionFlag::Dispersion).bits()
        ),
        "a layer surface is offered the compositor's real refraction maxima and the \
         components it actually draws"
    );
    assert!(
        !peer.state.liquid.contains_key(&1) && !peer.state.liquid.contains_key(&2),
        "the version 1 objects of the first half never receive liquid_capabilities"
    );

    // Refraction without blur: the lens has to do all of the work itself.
    controller.set_radius(0);
    controller.set_region(0, 0, LENS_PANE_WIDTH, LENS_PANE_HEIGHT);
    controller.set_corner_radii(LENS_CORNER_RADIUS, 0, 0, 0);
    controller.set_refraction(LENS_BAND, LENS_AMOUNT, RefractionFlag::empty());
    round_trip(display, state, peer);
    state.render_frame().unwrap();
    let lens = state.read_frame_rgba().unwrap();

    // 1. The outline bends the backdrop outwards. Sampling towards the interior
    //    makes the content near the edge appear displaced away from the pane's
    //    centre, which moves the first edge left.
    let pane_columns = 0..LENS_PANE_WIDTH as usize;
    let control_edge = first_edge(&control, LENS_ROW, pane_columns.clone())
        .expect("the striped backdrop has a black-to-white edge inside the pane");
    assert_eq!(
        control_edge, STRIPE_BLACK as usize,
        "the backdrop's first edge is at x = {STRIPE_BLACK}"
    );
    let lens_edge = first_edge(&lens, LENS_ROW, pane_columns)
        .expect("the refracted edge stays inside the pane: the displacement is small");
    assert!(
        lens_edge < control_edge,
        "refraction must push the backdrop outwards at the outline: the edge moved \
         from {control_edge} to {lens_edge}"
    );
    let shift = control_edge - lens_edge;
    assert!(
        (1..=LENS_AMOUNT as usize + 2).contains(&shift),
        "the edge may move by at most amount + 2 = {} columns, it moved {shift}",
        LENS_AMOUNT + 2
    );

    // 2. Beyond the band the pane is byte-identical to the control: refraction is
    //    an edge effect, not a second blur.
    let interior = 40..(LENS_PANE_WIDTH as usize - LENS_BAND as usize - 4);
    assert_eq!(
        differing(&lens, &control, LENS_ROW, interior.clone(), 1),
        Vec::<usize>::new(),
        "the refraction band must not reach the pane's interior"
    );

    // 3. Outside the rounded corner the backdrop is untouched, while the band that
    //    runs through that same corner did move pixels: the SDF mask is the only
    //    thing keeping the corner from being bent. (3, 3) is the sensitive probe:
    //    it sits outside the shape, and an unmasked pass would sample the backdrop
    //    ~6 px further in — past the stripe edge at x = 8, i.e. white instead of
    //    black — so a broken mask cannot pass this by rounding.
    assert!(
        same(pixel(&lens, 3, 3), pixel(&control, 3, 3), 1),
        "outside the rounded shape the pixel must still be the control pixel: {:?} \
         vs {:?}",
        pixel(&lens, 3, 3),
        pixel(&control, 3, 3)
    );
    let band = differing(&lens, &control, LENS_ROW, 0..LENS_BAND as usize, 1);
    assert!(
        band.len() >= 2,
        "the refraction band must change the backdrop near the outline, only {} \
         columns differ from the control",
        band.len()
    );

    // 4. Dispersion splits the edge into colour: red is displaced furthest and blue
    //    least, so the channels disagree where the gradient is strong and agree
    //    everywhere else.
    controller.set_refraction(LENS_BAND, LENS_AMOUNT, RefractionFlag::Dispersion);
    round_trip(display, state, peer);
    state.render_frame().unwrap();
    let dispersed = state.read_frame_rgba().unwrap();
    let split = |frame: &[u8]| {
        (0..LENS_BAND as usize)
            .map(|x| {
                let texel = pixel(frame, x, LENS_ROW);
                texel[0].abs_diff(texel[2])
            })
            .max()
            .unwrap_or(0)
    };
    assert!(split(&control) <= 1, "the control backdrop is grey");
    assert!(
        split(&lens) <= 1,
        "without the dispersion bit the edge stays achromatic"
    );
    assert!(
        split(&dispersed) >= 3,
        "the dispersion bit must separate red from blue at the edge, the largest \
         split is {}",
        split(&dispersed)
    );

    // 5. Depth moves the sample inside the band and nowhere else.
    controller.set_refraction(
        LENS_BAND,
        LENS_AMOUNT,
        RefractionFlag::Depth | RefractionFlag::Dispersion,
    );
    round_trip(display, state, peer);
    state.render_frame().unwrap();
    let depth = state.read_frame_rgba().unwrap();
    let depth_band = differing(&depth, &dispersed, LENS_ROW, 0..LENS_BAND as usize, 1);
    assert!(
        depth_band.len() >= 2,
        "the depth bit must move the band's sample, only {} columns changed",
        depth_band.len()
    );
    assert_eq!(
        differing(&depth, &control, LENS_ROW, interior, 1),
        Vec::<usize>::new(),
        "the depth component fades out with the band: the interior is still the control"
    );

    println!(
        "glass refraction: backdrop edge at {control_edge}, refracted to {lens_edge} \
         (shift {shift} of at most {}); {band:?} changed in the band; dispersion \
         split {} where the achromatic edge splits {}",
        LENS_AMOUNT + 2,
        split(&dispersed),
        split(&lens)
    );

    // After every assertion, the picture a human can open.
    write_artifact(&depth);
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
        peer.state.globals["inox_glass_manager_v1"].1, 2,
        "the private glass global is advertised at version 2: refraction is part of \
         the same interface"
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
    // The capability rule is about the role, not about the window being mapped: a
    // toplevel that has not drawn is still an ordinary toplevel surface, which the
    // tiling path does not frost.
    until(&mut display, &mut state, &mut [&mut other], |state, _| {
        state.awaiting_window_count() == 1
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
    // rejected client's own undrawn toplevel is gone with it.)
    assert_eq!(
        state.awaiting_window_count(),
        0,
        "the disconnected client's toplevel is released"
    );
    assert_eq!(
        state.window_count(),
        0,
        "the disconnected client never had a managed window"
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

    // The version 2 half: the same compositor, the same client, a lens.
    refraction_phase(&mut display, &mut state, &mut peer, &background);
}
