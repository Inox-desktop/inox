//! One real frame, through the offscreen QML host.
//!
//! Every other test in this crate asserts on the *projection* — the values the
//! QML renderer is handed. This one asserts that the renderer turns them into
//! pixels: a `QQuickWindow` on the `offscreen` platform, a software scene graph,
//! `grabWindow()`, and the bytes `wl_shm` will carry.
//!
//! It runs in a **child process**, and both reasons are structural:
//!
//! - Qt allows one `QGuiApplication` per process, and it must belong to the main
//!   thread. A test harness that runs its tests in parallel threads cannot
//!   promise either, so the child runs this one test serially (`--test-threads=1`)
//!   and the application is created on the main thread.
//! - The test process cannot rearrange its own environment (edition 2024 makes
//!   writing it `unsafe`, which this crate's hand-written Rust does not do), so
//!   `QT_QPA_PLATFORM` and `QT_QUICK_BACKEND` are set by the parent on the child.
//!
//! The parent runs the child **twice**: once on whatever pixel path the platform
//! gives the host, and once with `INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB` set, which
//! makes the host behave like a Qt whose grab has no alpha channel and rebuild the
//! channel from a black and a white pass. The pixel assertions are the same in
//! both runs — that is the proof the recovery is exact, not approximate — and the
//! forced run also asserts that it really went through the reconstruction, so a
//! second fast-path frame can never pass for a rebuilt one. (Which path the
//! *platform* run takes is the platform's answer: a Qt with an alpha backing store
//! hands the channel over, and one without rebuilds — both are the same frame,
//! which is what the pixel assertions say.)

use std::process::Command;

use inox_common::i18n::Locale;
use inox_config::{PanelSurface, Rgba, ShellConfig, Theme as Tokens};

use crate::app::{self, Message, State};
use crate::dock::DockGeometry;
use crate::layer::Interface;
use crate::tasks::{SessionGeneration, WindowId, WindowKey, WindowState};
use crate::view::PanelGeometry;

/// The environment variable that marks the child process.
const CHILD: &str = "INOX_PANEL_QML_CHILD";

/// The test the child runs.
const TEST: &str = "headless_tests::qml::one_frame_renders_through_the_offscreen_host";

/// The host's test hook: with this set, the offscreen host behaves like a Qt
/// whose grab has no alpha channel and reconstructs the channel from two opaque
/// passes. Nothing in production sets it.
const NO_ALPHA_GRAB: &str = "INOX_PANEL_OFFSCREEN_NO_ALPHA_GRAB";

#[test]
fn one_frame_renders_through_the_offscreen_host() {
    if std::env::var_os(CHILD).is_none() {
        let executable = std::env::current_exe().expect("the test executable's own path");

        // Every frame-drawing assertion runs twice: on the pixel path this Qt
        // gives the host by itself, and on the reconstruction a Qt without an
        // alpha backing store needs. `headless_tests::qml` is the only place
        // that sees pixels, so this is the only place the second path can be
        // proved.
        for forced in [false, true] {
            let mut child = Command::new(&executable);

            child
                .args(["--exact", TEST, "--test-threads=1", "--nocapture"])
                .env(CHILD, "1")
                .env("QT_QPA_PLATFORM", "offscreen")
                .env("QT_QUICK_BACKEND", "software");

            if forced {
                child.env(NO_ALPHA_GRAB, "1");
            }

            let status = child.status().expect("the child process starts");

            assert!(
                status.success(),
                "the offscreen QML host could not render a frame (alpha \
                 reconstruction {}; see the child's output above)",
                if forced {
                    "forced"
                } else {
                    "left to the platform"
                }
            );
        }

        return;
    }

    render_a_taskbar_frame();
    render_a_translucent_dock_frame();
}

/// The pixel path the host took, asserted against the one this child asked for.
///
/// Only the forced run is pinned: it has to have gone through the
/// reconstruction, or the second run would silently repeat the first and the
/// recovery would stay untested on the one machine that can exercise it. The
/// platform run asserts nothing here on purpose — whether the grab carries alpha
/// is the platform's answer, and *both* answers have to pass the pixel
/// assertions above. (On a Qt with an alpha backing store the platform run is the
/// fast path; `Interface::reconstructs_alpha` is what a reader uses to tell which
/// one they got.)
fn assert_pixel_path(interface: &Interface) {
    if std::env::var_os(NO_ALPHA_GRAB).is_some() {
        assert!(
            interface.reconstructs_alpha(),
            "the forced run has to reach the alpha reconstruction, not the platform's grab"
        );
    }
}

/// A taskbar frame: the strip is opaque and the first pixel is the `surface`
/// token, premultiplied `B, G, R, A`.
fn render_a_taskbar_frame() {
    let tokens = Tokens::builtin_dark();
    let mut shell = ShellConfig::default();
    shell.panel.surface = PanelSurface::Taskbar;

    let geometry = PanelGeometry::new(&tokens, &shell.panel);
    let width = 640;
    let height = geometry.height as u32;
    let mut state = state(shell, tokens.clone());
    let mut interface = Interface::new(width, height).expect("a drawable surface");
    let mut frame = Vec::new();

    interface
        .render_argb(&state, &mut frame)
        .expect("the offscreen renderer draws a frame");

    assert_pixel_path(&interface);

    assert_eq!(
        frame.len(),
        width as usize * height as usize * 4,
        "a frame is exactly the size the surface was configured to"
    );

    let first = pixel(&frame, width, 0, 0);
    let surface = tokens.colors.surface;

    assert_eq!(
        first,
        [surface.b, surface.g, surface.r, surface.a],
        "the strip's first pixel is the `surface` token, premultiplied B, G, R, A"
    );
    assert!(
        distinct(&frame) > 2,
        "the frame carries more than one colour, so something was drawn"
    );

    // The projection the renderer was handed and the scene the input half keeps
    // are the same scene: this is what makes `render` and `click` one
    // description of the interface rather than two.
    let drawn = interface
        .input()
        .scene()
        .expect("the frame just drawn is the scene a pointer is matched against");
    assert_eq!(
        drawn,
        &interface.input().project(&state),
        "the scene on the surface is the one projected from the state"
    );
    assert!(
        !drawn.json["items"]
            .as_array()
            .is_none_or(|items| items.is_empty()),
        "the scene QML drew is not empty"
    );

    let _ = state.take_outgoing();
}

/// A dock frame: the strip between the pane and the screen's edge is not
/// painted at all, and inside the pane the glass is really translucent.
fn render_a_translucent_dock_frame() {
    let tokens = Tokens::builtin_dark();
    let mut shell = ShellConfig::default();
    shell.panel.surface = PanelSurface::Dock;
    let geometry = DockGeometry::new(&tokens, &shell.dock);
    let width = 640;
    let height = app::window_height(&shell, &tokens) as u32;
    let mut state = state(shell, tokens.clone());
    let mut interface = Interface::new(width, height).expect("a drawable surface");
    let mut frame = Vec::new();

    interface
        .render_argb(&state, &mut frame)
        .expect("the offscreen renderer draws a dock frame");

    assert_pixel_path(&interface);

    assert_eq!(frame.len(), width as usize * height as usize * 4);
    // The pane is taller than the strip it floats above: `[dock] margin` is the
    // transparent band between the pane's bottom edge and the screen's, and the
    // corner pixel is the pane's own rounded corner, which is a blend.
    assert_eq!(
        pixel(&frame, width, width / 2, height - 1)[3],
        0,
        "the pane floats above the bottom edge, so the strip under it is transparent"
    );

    let translucent = frame
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[3] > 0 && pixel[3] < 0xff);

    assert!(
        translucent,
        "the glass is a translucent tint, not an opaque square: every pixel is either transparent or opaque"
    );

    // The pane's own tint, at the first row inside it, which nothing else is
    // drawn over: the one pixel where the *value* of a translucent colour is
    // asserted rather than just its transparency. It is what both pixel paths
    // have to agree on — the depth of the pane, and the ratio the reconstruction
    // has to recover from two opaque grabs. One 8-bit step of slack, because a
    // pass rounds (`reconstruct_alpha` in `src/offscreen.cpp`).
    let inside_pane = geometry.edge_height.ceil() as u32;
    let pane = pixel(&frame, width, width / 2, inside_pane);
    let tint = premultiplied(tokens.glass.tint);

    for (channel, index) in [("blue", 0), ("green", 1), ("red", 2), ("alpha", 3)] {
        assert!(
            pane[index].abs_diff(tint[index]) <= 1,
            "the pane's {channel} is the `glass.tint` token: {} for {} (the alpha \
             only survives if the renderer hands it back, or rebuilds it)",
            pane[index],
            tint[index]
        );
    }

    let _ = state.take_outgoing();
}

/// A panel state with one window, so a frame has something to draw.
fn state(shell: ShellConfig, tokens: Tokens) -> State {
    let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");
    let mut state = State::with_shell(shell, tokens, std::rc::Rc::new(localizer));

    app::update(
        &mut state,
        Message::Snapshot {
            generation: SessionGeneration::new(1),
            windows: vec![(
                WindowKey::new(WindowId::new(1), SessionGeneration::new(1)),
                WindowState {
                    app_id: "firefox".to_owned(),
                    title: "Inox".to_owned(),
                    ..WindowState::default()
                },
            )],
        },
    );

    state
}

/// A token colour as the compositor's blend leaves it over black: what a
/// translucent rectangle contributes to a premultiplied `B, G, R, A` frame.
fn premultiplied(colour: Rgba) -> [u8; 4] {
    let alpha = u32::from(colour.a);
    let channel = |value: u8| u8::try_from((u32::from(value) * alpha + 127) / 255).unwrap_or(0);

    [
        channel(colour.b),
        channel(colour.g),
        channel(colour.r),
        colour.a,
    ]
}

/// One pixel of a frame, as `B, G, R, A`.
fn pixel(frame: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * width + x) * 4) as usize;

    frame
        .get(index..index + 4)
        .and_then(|pixel| <[u8; 4]>::try_from(pixel).ok())
        .unwrap_or_default()
}

/// How many distinct colours a frame holds.
fn distinct(frame: &[u8]) -> usize {
    let mut seen: Vec<[u8; 4]> = Vec::new();

    for pixel in frame.as_chunks::<4>().0 {
        if !seen.contains(pixel) {
            seen.push(*pixel);
        }
    }

    seen.len()
}
