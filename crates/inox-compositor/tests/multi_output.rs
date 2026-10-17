//! Acceptance test for the multi-output topology (O02, ADR 0019).
//!
//! Two headless outputs of different sizes and scales are advertised to a real
//! Wayland client, which maps two toplevels onto them. The test then drives the
//! topology through the transitions O02 asks for — one display disappears, then
//! all of them do, then they come back — and asserts what the compositor's own
//! state says each time: every window stays inside a live work area, the client
//! is never disconnected, and the round trip is repeatable.
//!
//! This is the virtual-topology half of O02 (E1+E3). The hardware half — a real
//! DP/HDMI unplug through udev — is an E4 step and is not asserted here.
//!
//! Everything lives in one test function on purpose: a process may only ever hold
//! one EGL display (see the `SAFETY` note in `src/render.rs`), and test functions
//! inside one binary run in parallel threads.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

use calloop::EventLoop;
use inox_compositor::{ClientState, CompositorConfig, HeadlessOutput, InoxState, OutputDescriptor};
use smithay::{
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::wayland_server::{Display, DisplayHandle},
    utils::{Logical, Point, Rectangle},
};

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_compositor, wl_registry, wl_surface},
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

/// How long to service the display before declaring that the client never got through.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(20);

/// How many times O02 unplugs and replugs a display.
const HOTPLUG_ROUNDS: usize = 20;

/// The names of the two synthetic displays.
const LEFT: &str = "left-1";
const RIGHT: &str = "right-1";

/// The client's dispatch state; the test only needs the registry.
struct Client;

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

wayland_client::delegate_noop!(Client: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(Client: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(Client: ignore xdg_wm_base::XdgWmBase);
wayland_client::delegate_noop!(Client: ignore xdg_surface::XdgSurface);
wayland_client::delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);

/// Service the display once: dispatch client requests and flush outgoing events.
fn service(display: &mut Display<InoxState>, state: &mut InoxState) {
    let _ = display.dispatch_clients(state);
    let _ = display.flush_clients();
}

/// Wait until `ready` holds, servicing the display meanwhile.
fn until(
    display: &mut Display<InoxState>,
    state: &mut InoxState,
    what: &str,
    ready: impl Fn(&InoxState) -> bool,
) {
    let deadline = Instant::now() + DISPATCH_TIMEOUT;

    while !ready(state) {
        service(display, state);
        assert!(
            Instant::now() < deadline,
            "timed out after {DISPATCH_TIMEOUT:?} waiting for {what}"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// The two synthetic outputs the test drives.
fn two_outputs() -> CompositorConfig {
    CompositorConfig {
        outputs: vec![
            HeadlessOutput {
                name: LEFT.to_owned(),
                width: 800,
                height: 600,
                position: Some((0, 0)),
                ..HeadlessOutput::default()
            },
            HeadlessOutput {
                name: RIGHT.to_owned(),
                width: 400,
                height: 300,
                // 200x150 logical, placed to the right of the first display.
                scale: 2.0,
                position: Some((800, 0)),
                ..HeadlessOutput::default()
            },
        ],
        ..Default::default()
    }
}

/// Describe the outputs currently advertised, with the positions the
/// configuration asked for.
///
/// This mirrors what a backend reports: the configured placement, not the
/// computed one, so re-applying a descriptor set reproduces the arrangement.
fn descriptors(state: &InoxState) -> Vec<OutputDescriptor> {
    state
        .outputs()
        .map(|output| OutputDescriptor {
            position: requested_position(&output.name()),
            output: output.clone(),
            key: output.name(),
        })
        .collect()
}

/// The placement [`two_outputs`] asks for, by name.
fn requested_position(name: &str) -> Option<Point<i32, Logical>> {
    match name {
        LEFT => Some(Point::from((0, 0))),
        RIGHT => Some(Point::from((800, 0))),
        _ => None,
    }
}

/// A second, independent display that advertises the same connector name.
///
/// Two GPUs can each expose a `DP-1`. The name a client sees is then ambiguous,
/// which is exactly why the runtime identity is the backend's key rather than the
/// name (ADR 0019).
fn twin_of(name: &str) -> Output {
    let output = Output::new(
        name.to_owned(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "Test".to_owned(),
            model: "Twin".to_owned(),
        },
    );
    let mode = Mode {
        size: (800, 600).into(),
        refresh: 60_000,
    };
    output.set_preferred(mode);
    output.change_current_state(
        Some(mode),
        None,
        Some(Scale::Integer(1)),
        Some(Point::from((0, 0))),
    );

    output
}

/// The work area of the output named `name`, in global logical coordinates.
fn work_area_of(state: &InoxState, name: &str) -> Option<Rectangle<i32, Logical>> {
    let outputs: Vec<&Output> = state.outputs().collect();
    let index = outputs.iter().position(|output| output.name() == name)?;

    state.work_areas().get(index).map(|(_, area)| *area)
}

/// Assert every managed window sits inside one of `areas`.
fn assert_every_window_inside(state: &InoxState, areas: &[Rectangle<i32, Logical>], round: &str) {
    for id in state.managed_windows() {
        let location = state
            .window_location(id)
            .unwrap_or_else(|| panic!("{round}: window {id:?} has no location"));

        assert!(
            areas.iter().any(|area| area.contains(location)),
            "{round}: window {id:?} at {location:?} is outside every live work area {areas:?}"
        );
    }
}

/// Assert every managed window sits inside the work area of a named live output.
fn assert_inside_named_outputs(state: &InoxState, names: &[&str], round: &str) {
    let areas: Vec<Rectangle<i32, Logical>> = names
        .iter()
        .map(|name| {
            work_area_of(state, name)
                .unwrap_or_else(|| panic!("{round}: output `{name}` is not advertised"))
        })
        .collect();

    assert_every_window_inside(state, &areas, round);
}

/// Spawn the client thread: it creates two toplevels and then stays connected.
fn spawn_client(
    stream: UnixStream,
    ready: Sender<()>,
    release: Receiver<()>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let connection = Connection::from_socket(stream).expect("failed to build the connection");
        let (globals, mut queue) =
            registry_queue_init::<Client>(&connection).expect("failed to enumerate globals");
        let queue_handle = queue.handle();

        let compositor: wl_compositor::WlCompositor = globals
            .bind(&queue_handle, 4..=6, ())
            .expect("the compositor must be bindable");
        let xdg: xdg_wm_base::XdgWmBase = globals
            .bind(&queue_handle, 1..=6, ())
            .expect("xdg_wm_base must be bindable");

        let mut surfaces = Vec::new();
        for _ in 0..2 {
            let surface = compositor.create_surface(&queue_handle, ());
            let xdg_surface = xdg.get_xdg_surface(&surface, &queue_handle, ());
            let toplevel = xdg_surface.get_toplevel(&queue_handle, ());
            surface.commit();
            surfaces.push((surface, xdg_surface, toplevel));
        }

        queue.roundtrip(&mut Client).expect("roundtrip failed");
        ready.send(()).expect("the test went away");

        // Stay connected for the whole test: a topology change must not drop the
        // client, and the surfaces have to outlive every unplug below.
        let _ = release.recv();
        drop(surfaces);
    })
}

#[test]
fn outputs_tile_windows_and_migrate_them_across_hotplug() {
    let event_loop: EventLoop<'static, ()> =
        EventLoop::try_new().expect("failed to create the event loop");
    let mut display: Display<InoxState> =
        Display::new().expect("failed to create the wayland display");
    let mut display_handle: DisplayHandle = display.handle();

    let mut state = InoxState::new(&display_handle, &two_outputs(), event_loop.get_signal())
        .expect("failed to create the compositor state");

    // --- two outputs are advertised, with their own logical geometry ---------

    assert_eq!(
        state.output_count(),
        2,
        "both headless outputs are advertised"
    );
    let left_area = work_area_of(&state, LEFT).expect("the first output is advertised");
    let right_area = work_area_of(&state, RIGHT).expect("the second output is advertised");
    assert_eq!(left_area, Rectangle::new((0, 0).into(), (800, 600).into()));
    // 400x300 at scale 2 is 200x150 logical pixels, placed at the configured x.
    assert_eq!(
        right_area,
        Rectangle::new((800, 0).into(), (200, 150).into())
    );
    assert!(
        !left_area.overlaps(right_area),
        "the two work areas must not overlap: {left_area:?} and {right_area:?}"
    );

    // --- a real client maps two windows, which tile inside the first output ---

    let (server_stream, client_stream) =
        UnixStream::pair().expect("failed to create the socket pair");
    display_handle
        .insert_client(server_stream, Arc::new(ClientState::default()))
        .expect("failed to register the client");

    let (ready_tx, ready_rx) = channel();
    let (release_tx, release_rx) = channel();
    let client = spawn_client(client_stream, ready_tx, release_rx);

    let deadline = Instant::now() + DISPATCH_TIMEOUT;
    while ready_rx.try_recv().is_err() {
        service(&mut display, &mut state);
        assert!(
            Instant::now() < deadline,
            "the client never bound the globals"
        );
        thread::sleep(Duration::from_millis(2));
    }
    until(&mut display, &mut state, "two managed windows", |state| {
        state.window_count() == 2
    });

    // A window that appears while the pointer is at the origin lands on the
    // primary output, so both windows tile inside its work area.
    assert_inside_named_outputs(&state, &[LEFT], "initial mapping");

    // --- both descriptor sets are taken while both displays are advertised ---

    // They are reused for every unplug and replug below, which is what a replug
    // hands back: the same connectors, enumerated again.
    let both = descriptors(&state);
    let single: Vec<OutputDescriptor> = both.iter().skip(1).cloned().collect();

    // --- placement without a configured position follows the first display ---

    let auto: Vec<OutputDescriptor> = both
        .iter()
        .map(|descriptor| OutputDescriptor {
            position: None,
            ..descriptor.clone()
        })
        .collect();
    state
        .apply_outputs(&auto)
        .expect("an unpositioned topology must be accepted");
    assert_eq!(
        work_area_of(&state, RIGHT),
        Some(right_area),
        "an output with no configured position lands to the right of the one before it"
    );
    state
        .apply_outputs(&both)
        .expect("the positioned topology must be accepted");
    service(&mut display, &mut state);

    // --- the primary output disappears: windows migrate to the survivor -----

    state
        .apply_outputs(&single)
        .expect("the surviving output must be accepted");
    service(&mut display, &mut state);

    assert_eq!(state.output_count(), 1, "the removed display is gone");
    assert_eq!(
        state.window_count(),
        2,
        "the windows outlive the display they were on"
    );
    assert_inside_named_outputs(&state, &[RIGHT], "after losing the primary output");

    // --- every output disappears, then comes back ---------------------------

    state
        .apply_outputs(&[])
        .expect("removing every output must be accepted");
    assert_eq!(state.output_count(), 0, "no display is advertised");
    assert_eq!(
        state.window_count(),
        2,
        "windows stay managed while no display exists"
    );
    assert!(
        state
            .managed_windows()
            .iter()
            .all(|id| state.window_output(*id).is_none()),
        "with no output, no window claims one"
    );

    state
        .apply_outputs(&both)
        .expect("restoring both outputs must be accepted");
    service(&mut display, &mut state);
    assert_eq!(state.output_count(), 2, "both displays are back");
    assert_eq!(
        work_area_of(&state, LEFT),
        Some(left_area),
        "a restored display keeps its geometry"
    );
    assert_eq!(
        work_area_of(&state, RIGHT),
        Some(right_area),
        "a restored display keeps its geometry"
    );
    assert_inside_named_outputs(&state, &[LEFT, RIGHT], "after every output came back");

    // --- O02's repeatable round trip ----------------------------------------

    for round in 0..HOTPLUG_ROUNDS {
        let (next, live) = if round % 2 == 0 {
            (single.clone(), vec![RIGHT])
        } else {
            (both.clone(), vec![LEFT, RIGHT])
        };

        state
            .apply_outputs(&next)
            .unwrap_or_else(|error| panic!("round {round}: topology rejected: {error}"));
        service(&mut display, &mut state);

        assert_eq!(
            state.window_count(),
            2,
            "round {round}: the client's windows must survive"
        );
        assert!(
            state.output_active(),
            "round {round}: at least one display must remain"
        );
        assert_inside_named_outputs(&state, &live, &format!("round {round}"));
    }

    // --- the client is still connected, and a no-op change is inert ----------

    assert!(
        !client.is_finished(),
        "the client connection must outlive every hotplug"
    );

    let before = state.managed_windows();
    let locations: Vec<_> = before.iter().map(|id| state.window_location(*id)).collect();
    let unchanged = descriptors(&state);
    let advertised = state.output_count();
    state
        .apply_outputs(&unchanged)
        .expect("re-applying the same outputs must be accepted");
    service(&mut display, &mut state);

    assert_eq!(
        state.output_count(),
        advertised,
        "a no-op change adds or removes no output"
    );
    assert_eq!(
        state.managed_windows(),
        before,
        "a no-op topology change must not disturb the windows"
    );
    assert_eq!(
        before
            .iter()
            .map(|id| state.window_location(*id))
            .collect::<Vec<_>>(),
        locations,
        "a no-op topology change must not move a window"
    );

    // --- two displays may share a connector name without collapsing ---------

    let twins = vec![
        OutputDescriptor::named(
            state
                .outputs()
                .next()
                .expect("an output is advertised")
                .clone(),
            Some(Point::from((0, 0))),
        ),
        OutputDescriptor {
            key: "card1:left-1".to_owned(),
            output: twin_of(LEFT),
            position: Some(Point::from((800, 0))),
        },
    ];
    state
        .apply_outputs(&twins)
        .expect("two displays sharing a name must both be accepted");
    assert_eq!(
        state.output_count(),
        2,
        "a connector name collision must not collapse two displays into one"
    );

    let _ = release_tx.send(());
    client.join().expect("the client thread panicked");
}
