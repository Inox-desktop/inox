//! Acceptance X01 (`docs/quality/acceptance.md` §5): a real X11 client in an
//! Inox headless session — mapped, tiled, resized, focused and fed input.
//!
//! Evidence level E3: this drives the workspace's *own* binaries as processes
//! (`inox-compositor` with the test-only input injection socket, and
//! `inox-xwayland` supervising the real `xwayland-satellite`), then talks to the
//! resulting XWayland over real X11 and real Wayland connections. A third-party
//! headless host is explicitly not enough for X01, so this is the Inox
//! compositor on the Inox session.
//!
//! What the assertions are worth:
//!
//! * **readiness is a real X11 round trip.** `Session::start_service` only
//!   proceeds once `inox-xwayland` has written its readiness file, and that
//!   happens after a complete connection-setup exchange (ADR 0017 decision 9).
//!   The negative case below goes further and binds a socket that accepts
//!   connections and then stays silent, which is exactly the state X01 says must
//!   not count as ready.
//! * **the compositor's tile reaches X11.** The X11 client's `ConfigureNotify`
//!   size is asserted against the geometry `crates/inox-compositor/src/layout.rs`
//!   produces for the 1280x720 output, minus the title bar `xwayland-satellite`
//!   draws itself (M3 advertises no `zxdg_decoration_manager_v1`). That
//!   reservation is *measured* on the first configure and then required to stay
//!   constant, rather than being written down here as a guess.
//! * **the screen is not 0x0.** Xwayland derives its screen size from the
//!   compositor's `wl_output` mode, so the size assertion also proves that the
//!   mode carrying the `Current` flag really is sent.
//! * **input is routed through the satellite**, after a map (focus follows the
//!   newest window), after an explicit Alt+Tab, and after the focused window is
//!   closed. The negative half of each check is asserted explicitly: a window
//!   without the focus must not see the key.
//! * **the window is really composited**, proved by a frame dump rather than by
//!   the compositor's bookkeeping.
//!
//! Opt in with `INOX_M3_E2E=1`; additionally set `INOX_M3_E2E_REAL_APPS=1` to
//! also run `xeyes` and find its window with `xwininfo` (CI sets both).

mod support;

use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

use support::wayland_client::WaylandClient;
use support::x11_client::{Window, X11Client};
use support::{
    CASE_TIMEOUT, GAP, KEY_A, KEY_LEFTALT, KEY_TAB, KILL_GRACE, OUTPUT_HEIGHT, OUTPUT_WIDTH,
    POLL_INTERVAL, Proc, ProcessSignal, Rect, Session, X_KEYCODE_A, assert_group_empty,
    dead_x11_socket, expected_tiles, find_in_group, group_members, ignore, real_apps_enabled,
    require_tools, run_captured, signal_pid, skip_unless_enabled, tile_of_mapped, workspace_binary,
    write_idle_satellite_stand_in,
};

/// Colour of the first X11 window's background, as composited.
const COLOUR_ONE: u32 = 0x0022_88dd;
/// Colour of the second X11 window's background.
const COLOUR_TWO: u32 = 0x00dd_7722;

/// How long a negative assertion is given to be contradicted before it counts.
///
/// The positive half of every routing check is synchronised on the event itself;
/// this window only exists so that a key which should *not* arrive anywhere has a
/// chance to arrive and fail the run. It is a drain, not a readiness sleep.
const DRAIN: Duration = Duration::from_millis(500);

/// The largest title bar the tests accept from the satellite. A larger value
/// would mean the window was not tiled at all and the check below is measuring
/// something else.
const MAX_DECORATION: i32 = 64;

/// The output rectangle the compositor tiles into.
fn work_area() -> Rect {
    Rect::new(0, 0, OUTPUT_WIDTH as i32, OUTPUT_HEIGHT as i32)
}

/// The colour a `0x00RRGGBB` pixel value renders as.
fn rgb(colour: u32) -> [u8; 3] {
    [(colour >> 16) as u8, (colour >> 8) as u8, colour as u8]
}

/// Wait until the X11 window's client area is the compositor's tile minus the
/// satellite's title bar.
///
/// The first measurement establishes how much the satellite reserves; every
/// later one has to agree with it, which turns "the configure reached X11" into
/// an assertion about the resize path rather than a one-off.
fn wait_for_tile(
    x11: &mut X11Client,
    window: Window,
    tile: Rect,
    decoration: &mut Option<i32>,
    what: &str,
) {
    let deadline = Instant::now() + CASE_TIMEOUT;
    loop {
        if let Some((width, height)) = x11.configured_size(window) {
            if i32::from(width) == tile.w {
                let reserved = tile.h - i32::from(height);
                match *decoration {
                    Some(known) => assert_eq!(
                        reserved, known,
                        "{what}: window {window} was configured to {width}x{height} inside a \
                         {tile:?} tile, but the satellite's title bar measured {known} pixels on \
                         the previous reconfigure"
                    ),
                    None => {
                        assert!(
                            (0..=MAX_DECORATION).contains(&reserved),
                            "{what}: window {window} was configured to {width}x{height}, {reserved} \
                             pixels short of the {tile:?} tile; that is not a title bar, so the \
                             window was not tiled. Events: {:#?}",
                            x11.events
                        );
                        println!(
                            "{what}: xwayland-satellite reserves {reserved} pixels above the X11 \
                             client area"
                        );
                        *decoration = Some(reserved);
                    }
                }
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "{what}: window {window} was never configured to the {tile:?} tile; \
             observed events: {:#?}",
            x11.events
        );
        x11.poll(POLL_INTERVAL);
    }
}

/// Poll the compositor's frame dump until `colour` covers the client area of
/// `tile` (`decoration` pixels of title bar excluded).
fn wait_for_visible(
    session: &mut Session,
    tile: Rect,
    decoration: i32,
    colour: [u8; 3],
    what: &str,
) {
    // Stay well inside the client area: the title bar, the tile edge and any
    // focus ring must not be part of the measurement.
    let area = tile.below(decoration).below(16);
    let area = Rect::new(
        area.x + 16,
        area.y,
        (area.w - 32).max(1),
        (area.h - 16).max(1),
    );

    let deadline = Instant::now() + CASE_TIMEOUT;
    loop {
        let frame = session.frame();
        let matched = frame.count_matching(area, colour, 2);
        if matched * 2 >= (area.w * area.h) as usize {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: after {CASE_TIMEOUT:?} the composited frame still does not show {colour:?} in \
             {area:?} ({} of {} pixels matched; the dominant colour there is {:?})",
            matched,
            area.w * area.h,
            frame.dominant_colour(area)
        );
        thread::sleep(POLL_INTERVAL);
    }
}

/// Keep dispatching for `duration`, so a key that should not arrive can.
fn drain(x11: &mut X11Client, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        x11.poll(POLL_INTERVAL);
    }
}

/// How many times `window` received the *verification* key (X11 key code 46).
///
/// Counting only [`X_KEYCODE_A`] is what keeps this assertion about the key the
/// test injected: the compositor also forwards keys that are not bound to it —
/// `Alt_L` (`64`) pressed as part of Alt+Tab lands on the focused client, which
/// is correct behaviour and must not be read as a routing failure.
fn verification_key_presses(x11: &X11Client, window: Window) -> usize {
    x11.key_presses(window)
        .iter()
        .filter(|keycode| **keycode == X_KEYCODE_A as u8)
        .count()
}

/// Inject evdev code [`KEY_A`] and require that only `expect` receives it.
///
/// Both connections are **settled before the baseline is taken**: this client's
/// socket can still hold events the compositor sent earlier (a forwarded
/// modifier press, for instance), and counting them after the fact would look
/// like a key that arrived during this check. After the settle, the only events
/// that can arrive are the ones this function causes.
///
/// The key is re-injected until the focused window sees it: the satellite moves
/// the X input focus asynchronously, and a key injected during that window is
/// legitimately lost. Once it lands, the *other* window's verification-key count
/// is asserted unchanged, which is the routing property.
fn assert_key_routing(
    session: &mut Session,
    x11: &mut X11Client,
    expect: Window,
    other: Option<Window>,
    what: &str,
) {
    // A settled baseline, not a partially polled one.
    drain(x11, DRAIN);

    let expected_before = verification_key_presses(x11, expect);
    let other_before = other.map(|window| verification_key_presses(x11, window));

    let deadline = Instant::now() + CASE_TIMEOUT;
    let mut attempts = 0;
    loop {
        attempts += 1;
        session.injector.key_tap(KEY_A);
        let attempt_deadline = Instant::now() + Duration::from_secs(2);
        while verification_key_presses(x11, expect) == expected_before {
            assert!(
                Instant::now() < deadline,
                "{what}: after {CASE_TIMEOUT:?} and {attempts} injected keys, window {expect} \
                 never received the verification key. X11 events: {:#?}; compositor log:\n{}\n\
                 service log:\n{}",
                x11.events,
                session.compositor.diagnostics(),
                session
                    .service
                    .as_ref()
                    .map(Proc::diagnostics)
                    .unwrap_or_else(|| "no service in this session".to_owned())
            );
            if Instant::now() >= attempt_deadline {
                break;
            }
            x11.poll(POLL_INTERVAL);
        }
        if verification_key_presses(x11, expect) > expected_before {
            break;
        }
    }

    drain(x11, DRAIN);

    // Exactly one verification key, and its exact key code.
    assert_eq!(
        verification_key_presses(x11, expect),
        expected_before + 1,
        "{what}: the injected evdev key {KEY_A} must arrive as X11 key code {X_KEYCODE_A} exactly \
         once; events: {:#?}",
        x11.events
    );
    if let Some((window, before)) = other.map(|window| (window, other_before.unwrap_or_default())) {
        assert_eq!(
            verification_key_presses(x11, window),
            before,
            "{what}: window {window} does not have the keyboard focus but received the injected \
             key ({} against {before} before); events: {:#?}",
            verification_key_presses(x11, window),
            x11.events
        );
    }
    if attempts > 1 {
        println!("{what}: the key needed {attempts} injections while the focus settled");
    }
}

/// Inject Alt+Tab, the compositor's own "focus the next window" binding.
///
/// The exact sequence from the M3 brief: `key 56 press`, `key 15 press`,
/// `key 15 release`, `key 56 release`.
///
/// `Alt_L` is not itself a binding, so the compositor forwards that press to the
/// focused client, and the bound `Tab` press is consumed. Callers assert on the
/// verification key alone, so a forwarded modifier press cannot be mistaken for
/// a routing failure.
fn alt_tab(session: &mut Session) {
    session.injector.key(KEY_LEFTALT, true);
    session.injector.key(KEY_TAB, true);
    session.injector.key(KEY_TAB, false);
    session.injector.key(KEY_LEFTALT, false);
}

/// X01: a real X11 client maps, tiles, resizes, takes focus and gets input.
#[test]
fn x11_windows_are_tiled_focused_and_composited() {
    if skip_unless_enabled("X01") {
        return;
    }

    let mut session = Session::start("x01");
    let display = session.display_spec();

    assert_host_prerequisites(&session);

    let mut x11 = X11Client::connect(&display, "x11-x01");
    assert_eq!(
        x11.screen_size(),
        (OUTPUT_WIDTH as u16, OUTPUT_HEIGHT as u16),
        "the X11 screen must be the compositor's output size: Xwayland derives it from the \
         `wl_output` mode, so a compositor that never sends a mode with the `Current` flag leaves \
         X11 clients with a 0x0 screen"
    );

    // --- one window tiles into the whole work area -------------------------
    let single = expected_tiles(1)[0];
    assert_eq!(
        (single.w, single.h),
        (work_area().w - 2 * GAP, work_area().h - 2 * GAP),
        "a single window fills the 1280x720 work area inset by the layout gap on every side          (`LayoutConfig::default` in crates/inox-compositor/src/layout.rs)"
    );

    let window_one = x11.create_window(COLOUR_ONE, 400, 300, "inox-m3-x01-one");
    x11.map_window(window_one);
    let mut decoration = None;
    wait_for_tile(
        &mut x11,
        window_one,
        single,
        &mut decoration,
        "a single X11 window",
    );
    // From here on the reservation is a known constant. The `Option` stays
    // borrowed by the reconfiguration checks, which assert it does not drift.
    let decoration_height = decoration.expect("the first configure measures the title bar");

    assert_key_routing(
        &mut session,
        &mut x11,
        window_one,
        None,
        "the only X11 window has the focus",
    );

    // --- a second window splits the master/stack columns -------------------
    //
    // The compositor's workspace puts every newly mapped window first, so the
    // second window takes the master column and the first one moves to the stack
    // column (`Workspace::add` in crates/inox-compositor/src/workspace.rs).
    let split = expected_tiles(2);
    let (master, stack) = (split[0], split[1]);
    assert_eq!(tile_of_mapped(2, 1), master);
    assert_eq!(tile_of_mapped(2, 0), stack);
    let window_two = x11.create_window(COLOUR_TWO, 300, 200, "inox-m3-x01-two");
    x11.map_window(window_two);
    wait_for_tile(
        &mut x11,
        window_two,
        master,
        &mut decoration,
        "the newly mapped X11 window in the master column",
    );
    wait_for_tile(
        &mut x11,
        window_one,
        stack,
        &mut decoration,
        "the first X11 window, pushed to the stack column",
    );

    // Focus follows the newest window.
    assert_key_routing(
        &mut session,
        &mut x11,
        window_two,
        Some(window_one),
        "a newly mapped window takes the focus",
    );

    // --- both windows are really composited --------------------------------
    wait_for_visible(
        &mut session,
        master,
        decoration_height,
        rgb(COLOUR_TWO),
        "the master X11 window (the second one mapped) is visible",
    );
    wait_for_visible(
        &mut session,
        stack,
        decoration_height,
        rgb(COLOUR_ONE),
        "the stacked X11 window (the first one mapped) is visible",
    );

    // --- closing the focused window hands the focus back -------------------
    //
    // The focus is on the second window here (it took it when it mapped), so the
    // surviving window has to be re-tiled *and* re-focused by the compositor,
    // not merely left as it was.
    x11.destroy_window(window_two);
    wait_for_tile(
        &mut x11,
        window_one,
        single,
        &mut decoration,
        "the remaining X11 window after the other closed",
    );
    assert_key_routing(
        &mut session,
        &mut x11,
        window_one,
        None,
        "the surviving window receives input again",
    );
    wait_for_visible(
        &mut session,
        single,
        decoration_height,
        rgb(COLOUR_ONE),
        "the surviving X11 window is visible again",
    );

    // --- a real X11 application, when the tooling is available -------------
    real_application_leg(&mut session, &display);

    // The service exits cleanly on SIGTERM.
    if let Some(service) = session.service.as_mut() {
        service.signal(ProcessSignal::TERM);
        let status = service.wait_ok(
            "the XWayland service after SIGTERM",
            Duration::from_secs(15),
        );
        assert_eq!(
            status.code(),
            Some(0),
            "inox-xwayland must exit 0 on a clean shutdown, got {status}:\n{}",
            service.diagnostics()
        );
    }
}

/// X01's explicit focus switch: Alt+Tab has to move the focus between two real
/// X11 windows, and the key that follows it must reach only the newly focused
/// one.
///
/// This is a test of its own so that a failure here is named for the behaviour
/// that failed instead of hiding the tiling, resize and compositing assertions in
/// [`x11_windows_are_tiled_focused_and_composited`].
#[test]
fn alt_tab_moves_the_focus_between_x11_windows() {
    if skip_unless_enabled("X01") {
        return;
    }

    let mut session = Session::start("x01-alt-tab");
    let mut x11 = X11Client::connect(&session.display_spec(), "x11-alt-tab");

    let window_one = x11.create_window(COLOUR_ONE, 400, 300, "inox-m3-x11-alt-tab-one");
    x11.map_window(window_one);
    let mut decoration = None;
    wait_for_tile(
        &mut x11,
        window_one,
        expected_tiles(1)[0],
        &mut decoration,
        "the first window of the Alt+Tab scenario",
    );

    let (master, stack) = (expected_tiles(2)[0], expected_tiles(2)[1]);
    let window_two = x11.create_window(COLOUR_TWO, 300, 200, "inox-m3-x11-alt-tab-two");
    x11.map_window(window_two);
    wait_for_tile(
        &mut x11,
        window_two,
        master,
        &mut decoration,
        "the second window of the Alt+Tab scenario",
    );
    wait_for_tile(
        &mut x11,
        window_one,
        stack,
        &mut decoration,
        "the first window after the second one mapped",
    );

    // The newest window takes the focus; that is the starting point of the
    // switch, and asserting it first is what makes the Alt+Tab assertion mean
    // "the focus moved" rather than "the key landed somewhere".
    assert_key_routing(
        &mut session,
        &mut x11,
        window_two,
        Some(window_one),
        "the newest X11 window has the focus before Alt+Tab",
    );

    // Alt+Tab: the compositor's own binding, injected through the test-input
    // channel exactly as a keyboard would deliver it.
    alt_tab(&mut session);
    assert_key_routing(
        &mut session,
        &mut x11,
        window_one,
        Some(window_two),
        "Alt+Tab moves the focus to the other X11 window",
    );

    // ... and back, so the binding is proven to cycle rather than to stick.
    alt_tab(&mut session);
    assert_key_routing(
        &mut session,
        &mut x11,
        window_two,
        Some(window_one),
        "a second Alt+Tab moves the focus back",
    );
}

/// The satellite's hard host prerequisites, asserted before anything else so a
/// missing global fails with a message instead of a readiness timeout.
fn assert_host_prerequisites(session: &Session) {
    let probe = WaylandClient::connect(
        &session.runtime.wayland_socket_path(),
        session.runtime.path(),
        "probe",
    );
    for interface in [
        // `xwayland-satellite` panics at startup without these.
        "wp_viewporter",
        "xdg_wm_base",
        "wl_compositor",
        "wl_shm",
        "wl_seat",
        // M3's clipboard and drag-and-drop surface.
        "wl_data_device_manager",
        "zwp_primary_selection_device_manager_v1",
    ] {
        assert!(
            probe.bound_version(interface).is_some(),
            "the compositor does not advertise {interface}; the M3 session cannot work without it \
             (ADR 0017 decision 1). Globals: {:#?}",
            probe.state.globals
        );
    }
    assert_eq!(
        probe.bound_version("wl_data_device_manager"),
        Some(3),
        "M3 needs wl_data_device_manager version 3 (drag-and-drop actions)"
    );
}

/// Run `xeyes` and find its window with `xwininfo`, when opted in.
fn real_application_leg(session: &mut Session, display: &str) {
    if !real_apps_enabled() {
        println!(
            "INOX_M3_E2E_REAL_APPS is not set; the real X11 application leg (xeyes/xwininfo) of \
             X01 is skipped"
        );
        return;
    }
    require_tools(&["xeyes", "xwininfo"]);

    let env = session.env();
    let mut command = Command::new("xeyes");
    command
        .args(["-display", display])
        .envs(env.iter().cloned());
    let xeyes = Proc::spawn("xeyes", command, &session.runtime.log_path("xeyes"));

    let deadline = Instant::now() + CASE_TIMEOUT;
    loop {
        let output = run_captured(
            "xwininfo",
            &["-display", display, "-root", "-tree"],
            &env,
            CASE_TIMEOUT,
        );
        if output.status.success() && output.stdout.contains("xeyes") {
            println!("the real X11 application `xeyes` mapped a window in the Inox session");
            return;
        }
        assert!(
            Instant::now() < deadline,
            "xeyes never appeared in the X11 window tree; last `xwininfo` run: status={}, \
             stdout:\n{}\nstderr:\n{}\n{}",
            output.status,
            output.stdout,
            output.stderr,
            xeyes.diagnostics()
        );
        thread::sleep(POLL_INTERVAL);
    }
}

/// X01's negative half: a satellite that never becomes ready has to hit
/// `--ready-timeout`, exit non-zero and take its process group with it.
///
/// The socket this test binds exists *and accepts connections*; it just never
/// answers an X11 connection-setup request. X01 requires exactly that state not
/// to count as ready.
#[test]
fn an_unready_satellite_times_out_and_leaves_nothing_behind() {
    if skip_unless_enabled("X01") {
        return;
    }

    let session = Session::start_compositor("x01-timeout");
    let display = session.runtime.display_number();
    let socket_dir = session.runtime.scratch_path("x11-silent");
    let silent = dead_x11_socket(&socket_dir, display);
    assert!(silent.path.exists(), "the silent socket must exist on disk");

    let stand_in = write_idle_satellite_stand_in(session.runtime.path(), "idle-satellite.sh");
    let ready_file = session.runtime.scratch_path("never-ready");

    let mut command = Command::new(workspace_binary("inox-xwayland"));
    command
        .args([
            "--display",
            &format!(":{display}"),
            "--satellite",
            &stand_in.display().to_string(),
            "--x11-socket-dir",
            &socket_dir.display().to_string(),
            "--ready-timeout",
            "3",
            "--shutdown-grace",
            "2",
            "--ready-file",
            &ready_file.display().to_string(),
        ])
        .envs(session.runtime.env().iter().cloned());
    let mut service = Proc::spawn(
        "inox-xwayland (silent satellite)",
        command,
        &session.runtime.log_path("inox-xwayland-silent"),
    );
    let pgid = service.pid();

    // Timeout (3 s) plus shutdown grace (2 s) plus slack for process startup.
    let status = service.wait_ok(
        "inox-xwayland with a satellite that is never ready",
        Duration::from_secs(20),
    );
    assert_eq!(
        status.code(),
        Some(1),
        "a readiness timeout must fail with status 1 (`ExitCode::FAILURE`), got {status}:\n{}",
        service.diagnostics()
    );
    assert!(
        !ready_file.exists(),
        "the service must not report readiness for a socket that never completes an X11 setup \
         round trip, but {} exists",
        ready_file.display()
    );

    assert_group_empty(
        pgid,
        "the timed-out service",
        KILL_GRACE + Duration::from_secs(5),
    );
    drop(silent);
}

/// X01's supervision half: killing the satellite ends the service with a
/// defined status and leaves nothing behind, while the Inox session survives.
#[test]
fn killing_the_satellite_ends_the_service_but_not_the_session() {
    if skip_unless_enabled("X01") {
        return;
    }

    let mut session = Session::start("x01-kill");
    let service_pid = session
        .service
        .as_ref()
        .expect("the session runs the XWayland service")
        .pid();

    let satellite = find_in_group(service_pid, "xwayland-satel").unwrap_or_else(|| {
        panic!(
            "no satellite process in the service's process group {service_pid}; members: {:?}",
            group_members(service_pid)
        )
    });
    signal_pid(satellite.pid, ProcessSignal::KILL);

    let service = session.service.as_mut().expect("checked above");
    let status = service.wait_ok(
        "inox-xwayland after its satellite was killed",
        Duration::from_secs(15),
    );
    // A satellite killed by a signal has no exit code of its own, which
    // `inox-xwayland` maps to `ExitCode::FAILURE` (see its `main.rs`).
    assert_eq!(
        status.code(),
        Some(1),
        "killing the satellite must end the service with a defined failure status, got \
         {status}:\n{}",
        service.diagnostics()
    );

    assert_group_empty(
        service_pid,
        "the service whose satellite was killed",
        Duration::from_secs(10),
    );

    // The session itself survives: the compositor is alive and a new Wayland
    // client can still map a window in it.
    session.assert_compositor_alive("after the satellite was killed");
    let mut client = WaylandClient::connect(
        &session.runtime.wayland_socket_path(),
        session.runtime.path(),
        "after-kill",
    );
    let toplevel = client.create_toplevel([0x40, 0xc0, 0x40], (320, 240));
    let deadline = Instant::now() + CASE_TIMEOUT;
    client.wait_until(deadline, "a configure after the satellite died", |state| {
        state
            .toplevels
            .get(toplevel)
            .is_some_and(|entry| entry.configured.is_some())
    });
    let (width, height) = client
        .configured_size(toplevel)
        .expect("just waited for it");
    assert_eq!(
        (width, height),
        (expected_tiles(1)[0].w, expected_tiles(1)[0].h),
        "the surviving session must still tile a Wayland client into the work area"
    );

    // The compositor's teardown is exercised by `Proc::drop`; nothing else is
    // asserted about it here (R03 owns that).
    ignore(session.compositor.wait_timeout(Duration::from_millis(1)));
}
