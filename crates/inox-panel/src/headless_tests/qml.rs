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
use crate::offscreen::Host;
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
    render_the_glass_catalog_page();
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

/// The shared glass component library's only pixel evidence.
///
/// `inox-glass` links no Qt, so it cannot draw by itself: it ships
/// `CatalogWindow.qml` and the five stateless components as assets, and this
/// crate compiles them into its own QML module (ADR 0040 §3). The page is
/// rendered here through the same offscreen host, the same platform plugin and
/// the same `grabWindow()` path as the panel's own frame, from
/// `inox_glass::catalog_model(&tokens)` — one projection whose every rectangle
/// carries its own geometry, so this test reads the coordinates it samples out of
/// the model instead of guessing a layout.
///
/// What is asserted is what makes a glass component a glass component: the pane
/// is the `glass.tint` token and nothing outside its rounded corner is painted,
/// a checked toggle's track is the accent token, and the slider's fill ends where
/// its `value` says.
fn render_the_glass_catalog_page() {
    let tokens = Tokens::builtin_dark();
    let model: serde_json::Value = serde_json::from_str(&inox_glass::catalog_model(&tokens))
        .expect("the catalog projection is JSON");
    let width = number(&model, "", "width");
    let height = number(&model, "", "height");

    let mut host =
        Host::with_url(crate::offscreen::CATALOG_URL).expect("the catalog page loads and renders");
    host.set_model(&model)
        .expect("the catalog page takes its projection");

    let frame = host
        .frame(width, height)
        .expect("the catalog page renders a frame");

    assert_eq!(
        frame.len(),
        width as usize * height as usize * 4,
        "a frame is exactly the size of the page it draws"
    );

    // The page's own fill is the structural "no colour" sentinel: the corners the
    // components do not reach are transparent, not a clear colour the host chose.
    for (x, y) in [(0, 0), (width - 1, 0), (width - 1, height - 1)] {
        assert_eq!(
            rgba(&frame, width, x, y)[3],
            0,
            "the page paints nothing at ({x}, {y}), so the frame is transparent there"
        );
    }

    // The pane: the tint token at its centre, and no paint outside its rounded
    // corner — the corner is what the compositor's SDF mask has to agree with.
    let (pane_x, pane_y) = (number(&model, "pane", "x"), number(&model, "pane", "y"));
    let (pane_w, pane_h) = (
        number(&model, "pane", "width"),
        number(&model, "pane", "height"),
    );

    assert_close(
        rgba(&frame, width, pane_x + pane_w / 2, pane_y + pane_h / 2),
        premultiplied_rgba(tokens.glass.tint),
        "the pane's centre is the `glass.tint` token over the transparent page",
    );
    assert_eq!(
        rgba(&frame, width, pane_x + 1, pane_y + 1)[3],
        0,
        "the pane's rounded corner is not painted: the shape is the radius, not a rectangle"
    );

    // The toggle: the catalog shows it checked, and a checked track is the accent
    // token the projection named — not a colour of the component's own.
    let (toggle_x, toggle_y) = (number(&model, "toggle", "x"), number(&model, "toggle", "y"));
    let (toggle_w, toggle_h) = (
        number(&model, "toggle", "width"),
        number(&model, "toggle", "height"),
    );
    let toggle_track = (toggle_x + toggle_w / 4, toggle_y + toggle_h / 2);

    assert_close(
        rgba(&frame, width, toggle_track.0, toggle_track.1),
        opaque_rgba(tokens.colors.accent),
        "a checked toggle is drawn in the accent token",
    );

    // The slider: the fill is the accent token and ends where `value` says, and
    // the rest of the track is the `surface_variant` token.
    let (slider_x, slider_y) = (number(&model, "slider", "x"), number(&model, "slider", "y"));
    let (slider_w, slider_h) = (
        number(&model, "slider", "width"),
        number(&model, "slider", "height"),
    );
    let value = model["slider"]["value"]
        .as_f64()
        .expect("the slider projects its value");
    let mid = slider_y + slider_h / 2;
    let along = |fraction: f64| slider_x + (f64::from(slider_w) * fraction) as u32;

    assert_close(
        rgba(&frame, width, along(value / 2.0), mid),
        opaque_rgba(tokens.colors.accent),
        "the filled half of the track is the accent token",
    );
    assert_close(
        rgba(&frame, width, along((value + 1.0) / 2.0), mid),
        opaque_rgba(tokens.colors.surface_variant),
        "and the empty half is the track token",
    );

    // Move the value and the fill follows it: the switch happens where `value`
    // is, not at a fraction the component chose.
    let mut fuller = model.clone();
    fuller["slider"]["value"] = serde_json::json!(0.75);
    host.set_model(&fuller)
        .expect("the catalog page takes the moved slider");

    let frame = host
        .frame(width, height)
        .expect("the catalog page renders the moved slider");

    assert_close(
        rgba(&frame, width, along(0.6), mid),
        opaque_rgba(tokens.colors.accent),
        "a value of 0.75 fills the track past the 0.6 mark",
    );
    assert_eq!(
        rgba(&frame, width, along(0.85), mid)[3],
        0xff,
        "and the far end of the track is still painted, not blank"
    );
}

/// One integer of the catalog projection; `part` is a component name or empty for
/// the page itself.
fn number(model: &serde_json::Value, part: &str, key: &str) -> u32 {
    let value = if part.is_empty() {
        &model[key]
    } else {
        &model[part][key]
    };

    u32::try_from(value.as_u64().expect("a projected integer")).expect("a u32 dimension")
}

/// One pixel of a host frame, as the host hands it over: `R, G, B, A`,
/// premultiplied. (The layer's frames are `B, G, R, A` — see [`pixel`].)
fn rgba(frame: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    pixel(frame, width, x, y)
}

/// A token colour as a translucent pane contributes it over the transparent page,
/// in the host's `R, G, B, A` order.
fn premultiplied_rgba(colour: Rgba) -> [u8; 4] {
    let alpha = u32::from(colour.a);
    let channel = |value: u8| u8::try_from((u32::from(value) * alpha + 127) / 255).unwrap_or(0);

    [
        channel(colour.r),
        channel(colour.g),
        channel(colour.b),
        colour.a,
    ]
}

/// An opaque token colour, in the host's `R, G, B, A` order.
fn opaque_rgba(colour: Rgba) -> [u8; 4] {
    [colour.r, colour.g, colour.b, colour.a]
}

/// Assert a pixel is a token, allowing the one 8-bit step a premultiplied pass
/// rounds by (`reconstruct_alpha` in `src/offscreen.cpp`).
fn assert_close(actual: [u8; 4], expected: [u8; 4], why: &str) {
    for (index, channel) in ["red", "green", "blue", "alpha"].iter().enumerate() {
        assert!(
            actual[index].abs_diff(expected[index]) <= 1,
            "{why}: {channel} is {} for {} (pixel {actual:?}, token {expected:?})",
            actual[index],
            expected[index]
        );
    }
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
