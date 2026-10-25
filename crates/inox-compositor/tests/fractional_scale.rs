//! Acceptance test for fractional scaling.
//!
//! One headless output runs at 1.5x with a 960x540 mode, so its logical size is
//! 640x360 and clients that only understand integer scales are told 2. A real
//! Wayland client maps a toplevel carrying a 64x64 solid red buffer; the test
//! renders a frame, reads the physical pixels back and asserts where the red
//! actually landed.
//!
//! The discriminating pixel is physical (100, 100). At 1.5x the buffer covers
//! physical [12, 108), so that point is red; at 1.0x it would cover [8, 72) and
//! the point would still be the background. A compositor that ignored the scale
//! would fail here, which is the whole point — "the global is advertised" is not
//! evidence that anything is scaled.
//!
//! Everything lives in one test function on purpose: a process may only ever hold
//! one EGL display (see the `SAFETY` note in `src/render.rs`), and test functions
//! inside one binary run in parallel threads.

use std::{
    io::Write,
    os::{fd::AsFd, unix::net::UnixStream},
    sync::{
        Arc,
        mpsc::{Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, HeadlessOutput, InoxState};
use smithay::reexports::wayland_server::{Display, DisplayHandle};

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_compositor, wl_output, wl_registry, wl_shm, wl_shm_pool, wl_surface},
};
use wayland_protocols::{
    wp::viewporter::client::{wp_viewport, wp_viewporter},
    xdg::{
        shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base},
        xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1},
    },
};

/// How long to service the display before declaring that the client never got through.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Physical mode of the single output.
const MODE: (i32, i32) = (960, 540);

/// Scale factor the output runs at.
const SCALE: f64 = 1.5;

/// Logical size that follows: `MODE / SCALE`.
const LOGICAL: (i32, i32) = (640, 360);

/// Side of the client's square buffer, in logical pixels.
const BUFFER: i32 = 64;

/// The compositor's background, as `read_frame_rgba` reports it.
const BACKGROUND: [u8; 4] = inox_compositor::render::CLEAR_COLOR_RGBA8;

/// Tolerance of one 8-bit step, covering float-to-unorm rounding differences.
const TOLERANCE: u8 = 1;

/// What the client observed about its output.
#[derive(Debug, Default)]
struct ClientOutcome {
    /// The `wl_output` integer scale the compositor advertised.
    scale: Option<i32>,
    /// The physical mode `wl_output` reported.
    mode: Option<(i32, i32)>,
    /// The logical size `zxdg_output_v1` reported.
    logical: Option<(i32, i32)>,
}

struct Client {
    outcome: ClientOutcome,
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
    }
}

impl Dispatch<wl_output::WlOutput, ()> for Client {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => state.outcome.scale = Some(factor),
            wl_output::Event::Mode { width, height, .. } => {
                state.outcome.mode = Some((width, height))
            }
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_output_v1::Event::LogicalSize { width, height } = event {
            state.outcome.logical = Some((width, height));
        }
    }
}

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(Client: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(Client: ignore xdg_surface::XdgSurface);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);
wayland_client::delegate_noop!(Client: ignore wp_viewporter::WpViewporter);
wayland_client::delegate_noop!(Client: ignore wp_viewport::WpViewport);
wayland_client::delegate_noop!(Client: ignore zxdg_output_manager_v1::ZxdgOutputManagerV1);

/// Service the display once.
fn service(display: &mut Display<InoxState>, state: &mut InoxState) {
    let _ = display.dispatch_clients(state);
    let _ = display.flush_clients();
}

/// The compositor's configured output.
fn output_config() -> CompositorConfig {
    CompositorConfig {
        outputs: vec![HeadlessOutput {
            name: "scaled-1".to_owned(),
            width: MODE.0,
            height: MODE.1,
            scale: SCALE,
            position: Some((0, 0)),
            ..HeadlessOutput::default()
        }],
        ..Default::default()
    }
}

/// The RGBA pixel at `(x, y)` of a tightly packed frame `width` pixels wide.
fn pixel(frame: &[u8], width: i32, x: i32, y: i32) -> [u8; 4] {
    let offset = (y as usize * width as usize + x as usize) * 4;

    frame
        .get(offset..offset + 4)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .unwrap_or_else(|| panic!("the frame is too small for ({x}, {y})"))
}

fn is_red(pixel: [u8; 4]) -> bool {
    pixel[0] > 200 && pixel[1] < 60 && pixel[2] < 60
}

fn is_background(pixel: [u8; 4]) -> bool {
    pixel
        .iter()
        .zip(BACKGROUND.iter())
        .all(|(actual, expected)| actual.abs_diff(*expected) <= TOLERANCE)
}

/// Spawn the client: it maps one solid red toplevel and reports what it was told.
fn spawn_client(stream: UnixStream, ready: Sender<ClientOutcome>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let connection = Connection::from_socket(stream).expect("failed to build the connection");
        let mut state = Client {
            outcome: ClientOutcome::default(),
        };
        let (globals, mut queue) =
            registry_queue_init::<Client>(&connection).expect("failed to enumerate globals");
        let queue_handle = queue.handle();

        let compositor: wl_compositor::WlCompositor = globals
            .bind(&queue_handle, 4..=6, ())
            .expect("the compositor must be bindable");
        let xdg: xdg_wm_base::XdgWmBase = globals
            .bind(&queue_handle, 1..=6, ())
            .expect("xdg_wm_base must be bindable");
        let shm: wl_shm::WlShm = globals
            .bind(&queue_handle, 1..=1, ())
            .expect("wl_shm must be bindable");
        let wl_output: wl_output::WlOutput = globals
            .bind(&queue_handle, 4..=4, ())
            .expect("wl_output must be bindable");
        let xdg_output_manager: zxdg_output_manager_v1::ZxdgOutputManagerV1 = globals
            .bind(&queue_handle, 3..=3, ())
            .expect("zxdg_output_manager_v1 must be bindable");
        let _xdg_output = xdg_output_manager.get_xdg_output(&wl_output, &queue_handle, ());

        // A square of solid red. `Xrgb8888` is little-endian `B, G, R, X`.
        let stride = BUFFER * 4;
        let mut bytes = vec![0u8; (stride * BUFFER) as usize];
        for pixel in bytes.as_chunks_mut::<4>().0 {
            *pixel = [0, 0, 255, 0];
        }
        let mut file = tempfile::tempfile().expect("failed to create the buffer file");
        file.write_all(&bytes).expect("failed to fill the buffer");
        let pool = shm.create_pool(file.as_fd(), stride * BUFFER, &queue_handle, ());
        let buffer = pool.create_buffer(
            0,
            BUFFER,
            BUFFER,
            stride,
            wl_shm::Format::Xrgb8888,
            &queue_handle,
            (),
        );

        let surface = compositor.create_surface(&queue_handle, ());
        let xdg_surface = xdg.get_xdg_surface(&surface, &queue_handle, ());
        let _toplevel = xdg_surface.get_toplevel(&queue_handle, ());
        surface.commit();
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();

        // One roundtrip: the output description and the surface commit both travel
        // on it, so the compositor has the buffer before the test renders.
        queue.roundtrip(&mut state).expect("roundtrip failed");
        ready
            .send(ClientOutcome {
                scale: state.outcome.scale,
                mode: state.outcome.mode,
                logical: state.outcome.logical,
            })
            .expect("the test went away");

        // Hold the surfaces open: dropping them would unmap the window.
        std::thread::park();
        drop((surface, xdg_surface, buffer, pool));
    })
}

#[test]
fn a_fractionally_scaled_output_composites_at_its_physical_size() {
    let event_loop: EventLoop<'static, ()> =
        EventLoop::try_new().expect("failed to create the event loop");
    let mut display: Display<InoxState> =
        Display::new().expect("failed to create the wayland display");
    let mut display_handle: DisplayHandle = display.handle();

    let mut state = InoxState::new(&display_handle, &output_config(), event_loop.get_signal())
        .expect("failed to create the compositor state");

    // --- the compositor's own view of the output ----------------------------

    let output = state.output().expect("the output is advertised");
    assert!(
        (output.current_scale().fractional_scale() - SCALE).abs() < f64::EPSILON,
        "the output must run at {SCALE}, got {}",
        output.current_scale().fractional_scale()
    );
    assert_eq!(
        output.current_scale().integer_scale(),
        2,
        "clients that only understand integers are told the ceiling of {SCALE}"
    );
    assert_eq!(
        state.output_size(),
        MODE,
        "the render target is the physical mode in pixels"
    );
    assert_eq!(
        state.work_area(),
        smithay::utils::Rectangle::new((0, 0).into(), LOGICAL.into()),
        "the work area is expressed in logical pixels"
    );

    // --- a real client sees the physical mode and the logical size -----------

    let (server_stream, client_stream) =
        UnixStream::pair().expect("failed to create the socket pair");
    display_handle
        .insert_client(server_stream, Arc::new(ClientState::default()))
        .expect("failed to register the client");

    let (ready_tx, ready_rx) = channel();
    let client = spawn_client(client_stream, ready_tx);

    let deadline = Instant::now() + DISPATCH_TIMEOUT;
    let outcome = loop {
        service(&mut display, &mut state);
        match ready_rx.try_recv() {
            Ok(outcome) => break outcome,
            Err(_) => {
                assert!(
                    Instant::now() < deadline,
                    "the client never completed its roundtrip"
                );
                thread::sleep(Duration::from_millis(2));
            }
        }
    };

    assert_eq!(
        outcome.mode,
        Some(MODE),
        "`wl_output` reports the physical mode"
    );
    assert_eq!(
        outcome.scale,
        Some(2),
        "`wl_output.scale` carries the ceiling of {SCALE}, not the fraction"
    );
    assert_eq!(
        outcome.logical,
        Some(LOGICAL),
        "`zxdg_output_v1` reports the logical size a client should lay out against"
    );

    // --- the pixels: a logical position lands at its physical one -----------

    // The work area is inset by the tiling gap, so the window's buffer starts at
    // logical (8, 8) and covers 64x64 logical pixels: physical [12, 108) at 1.5x.
    let deadline = Instant::now() + DISPATCH_TIMEOUT;
    let frame = loop {
        service(&mut display, &mut state);
        state.render_frame().expect("failed to render a frame");
        let frame = state
            .read_frame_rgba()
            .expect("failed to read the frame back");

        if is_red(pixel(&frame, MODE.0, 100, 100)) {
            break frame;
        }
        assert!(
            Instant::now() < deadline,
            "the client's buffer never reached the frame; centre pixel reads {:?}",
            pixel(&frame, MODE.0, MODE.0 / 2, MODE.1 / 2)
        );
        thread::sleep(Duration::from_millis(2));
    };

    assert_eq!(
        frame.len(),
        (MODE.0 * MODE.1 * 4) as usize,
        "the frame is the output's physical size"
    );
    // Inside the scaled buffer: red only because the 1.5x scale was applied.
    assert!(
        is_red(pixel(&frame, MODE.0, 100, 100)),
        "physical (100, 100) should be inside the 1.5x-scaled buffer, read {:?}",
        pixel(&frame, MODE.0, 100, 100)
    );
    // Just past it: the buffer ends at physical 108, so this is background again.
    assert!(
        is_background(pixel(&frame, MODE.0, 110, 110)),
        "physical (110, 110) should be past the scaled buffer, read {:?}",
        pixel(&frame, MODE.0, 110, 110)
    );
    // The rest of the work area is still the compositor's background.
    assert!(
        is_background(pixel(&frame, MODE.0, MODE.0 - 4, MODE.1 - 4)),
        "the far corner should be background, read {:?}",
        pixel(&frame, MODE.0, MODE.0 - 4, MODE.1 - 4)
    );

    client.thread().unpark();
    client.join().expect("the client thread panicked");
}
