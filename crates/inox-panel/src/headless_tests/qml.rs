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

use std::process::Command;

use inox_common::i18n::Locale;
use inox_config::{PanelSurface, ShellConfig, Theme as Tokens};

use crate::app::{self, Message, State};
use crate::layer::Interface;
use crate::tasks::{SessionGeneration, WindowId, WindowKey, WindowState};
use crate::view::PanelGeometry;

/// The environment variable that marks the child process.
const CHILD: &str = "INOX_PANEL_QML_CHILD";

/// The test the child runs.
const TEST: &str = "headless_tests::qml::one_frame_renders_through_the_offscreen_host";

#[test]
fn one_frame_renders_through_the_offscreen_host() {
    if std::env::var_os(CHILD).is_none() {
        let executable = std::env::current_exe().expect("the test executable's own path");
        let status = Command::new(executable)
            .args(["--exact", TEST, "--test-threads=1", "--nocapture"])
            .env(CHILD, "1")
            .env("QT_QPA_PLATFORM", "offscreen")
            .env("QT_QUICK_BACKEND", "software")
            .status()
            .expect("the child process starts");

        assert!(
            status.success(),
            "the offscreen QML host could not render a frame (see the child's output above)"
        );

        return;
    }

    render_a_taskbar_frame();
    render_a_translucent_dock_frame();
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
    let width = 640;
    let height = app::window_height(&shell, &tokens) as u32;
    let mut state = state(shell, tokens);
    let mut interface = Interface::new(width, height).expect("a drawable surface");
    let mut frame = Vec::new();

    interface
        .render_argb(&state, &mut frame)
        .expect("the offscreen renderer draws a dock frame");

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
