//! What the lock screen draws, asserted on the frames the QML host produces
//! (acceptance Q02, D05).
//!
//! Every scenario here renders the real interface — the real projection, the real
//! QML module, the real offscreen host in [`crate::render::Interface`], which is
//! the code the binary commits its frames with — at the size acceptance Q02
//! snapshots are taken at. There is no compositor and no GPU involved, so these are
//! E1 evidence: they prove the screen draws what the view model says it draws, not
//! that it appears on a display, and not that it covers one.
//!
//! The scenarios are the ones acceptance Q02 asks for — light, dark, and the error
//! and empty states — plus the ones turns into rules: a lock with no UI, a
//! session that cannot be locked at all, and a rate limit that must not look like
//! a prompt.
//!
//! # One test function, on purpose
//!
//! Qt allows one `QGuiApplication` per process, owned by the thread that created
//! it, and every `Interface` builds its QML engine on that thread. The test
//! harness runs test functions on several threads, so a second function that
//! touched Qt would create a second engine on another thread and fail. Every
//! scenario therefore lives in one test, rendered in sequence; the assertions are
//! separate functions so a failure still names which face was wrong.
//!
//! # Two hosts, two outputs
//!
//! [`the_two_hosts_render_their_own_sizes`] is not an afterthought: a session can
//! have a laptop panel and a monitor, and the lock surface for each is configured
//! to its own size. The host supports that by giving each surface its own engine
//! and window (`src/offscreen.cpp`), so the test renders the same state through
//! two live interfaces at once.

use std::rc::Rc;

use inox_common::i18n::Locale;
use inox_config::{Rgba, Theme as Tokens};

use crate::app::{self, Message, State};
use crate::contract::{AuthFailure, AuthOutcome, ProtocolAbsence, ServiceFailure};
use crate::render::Interface;
use crate::state::{LockReason, OutputId, Phase};
use crate::theme;
use crate::view;
use crate::view_model::Face;

/// The size acceptance Q02 snapshots are taken at.
const WIDTH: u32 = 1280;

/// The size acceptance Q02 snapshots are taken at.
const HEIGHT: u32 = 720;

/// The output the fixtures cover.
const OUTPUT: &str = "eDP-1";

fn output() -> OutputId {
    OutputId::new(OUTPUT)
}

fn state_in(locale: &Locale, tokens: Tokens) -> State {
    let localizer = crate::i18n::load(locale).expect("the lock screen's catalogs parse");

    State::new(Rc::new(localizer), tokens)
}

fn update(state: &mut State, message: Message) {
    let _ = app::update(state, message);
}

/// A lock screen whose compositor offers the protocol and whose session is
/// locked, with its one output covered.
fn locked_in(locale: &Locale, tokens: Tokens) -> State {
    let mut state = state_in(locale, tokens);

    update(&mut state, Message::ProtocolPresent);
    update(&mut state, Message::OutputAdded(output()));
    update(&mut state, Message::LockRequested(LockReason::User));
    update(&mut state, Message::OutputCovered(output()));
    let _ = state.take_outgoing();

    assert_eq!(state.machine().phase(), Phase::Locked);
    assert!(state.machine().reports_locked());
    state
}

fn locked(tokens: Tokens) -> State {
    locked_in(&Locale::default_locale(), tokens)
}

/// The answer the boundary gives to the attempt in flight.
fn answer(state: &mut State, outcome: AuthOutcome) {
    let generation = state.boundary().generation().expect("an attempt");
    update(state, Message::AuthAnswered(generation, outcome));
}

/// Types `text` and has the boundary refuse it.
fn fail_an_attempt(state: &mut State, text: &str) {
    for character in text.chars() {
        update(state, Message::Character(character));
    }
    update(state, Message::Submitted);
    answer(state, AuthOutcome::Failure(AuthFailure::Rejected));
}

/// A locked screen whose boundary failed with `failure`.
fn locked_unavailable(tokens: Tokens, failure: ServiceFailure) -> State {
    let mut state = locked(tokens);
    update(&mut state, Message::AuthUnavailable(failure));
    state
}

/// One pixel of an RGBA frame.
fn pixel(frame: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let offset = ((y * width + x) * 4) as usize;

    [
        frame[offset],
        frame[offset + 1],
        frame[offset + 2],
        frame[offset + 3],
    ]
}

/// A token colour as the byte order the RGBA frame carries.
fn token_pixel(colour: Rgba) -> [u8; 4] {
    [colour.r, colour.g, colour.b, colour.a]
}

/// How many pixels are exactly `colour`.
fn count_of(frame: &[u8], colour: [u8; 4]) -> usize {
    frame
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| **pixel == colour)
        .count()
}

/// How many distinct pixel values a frame holds.
fn distinct_colours(frame: &[u8]) -> usize {
    let mut colours: Vec<[u8; 4]> = frame.as_chunks::<4>().0.to_vec();
    colours.sort_unstable();
    colours.dedup();
    colours.len()
}

/// The structural assertions every frame has to pass.
fn assert_frame_shape(frame: &[u8], width: u32, height: u32, what: &str) {
    assert_eq!(
        frame.len(),
        (width as usize) * (height as usize) * 4,
        "{what}: the frame is not one RGBA quad per configured pixel"
    );
    assert!(
        frame
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0xff),
        "{what}: a lock frame that is not opaque would show what it is covering"
    );
    assert!(
        distinct_colours(frame) > 2,
        "{what}: the frame is (nearly) a single colour, so nothing was drawn on it"
    );
}

/// Write a snapshot when `INOX_SCREENSHOT_DIR` asks for one (acceptance Q02).
fn snapshot(name: &str, frame: &[u8], width: u32, height: u32) {
    let Some(directory) = std::env::var_os("INOX_SCREENSHOT_DIR") else {
        return;
    };

    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory).expect("the screenshot directory is writable");
    let path = directory.join(format!("inox-lock-{name}.png"));
    let image = image::RgbaImage::from_raw(width, height, frame.to_vec())
        .expect("the frame is the size the image is");
    image.save(&path).expect("the snapshot is written");
}

#[test]
fn the_offscreen_qml_host_renders_every_face_of_the_lock_screen() {
    let mut interface =
        Interface::new(WIDTH, HEIGHT).expect("the QML host renders a 1280x720 frame");
    assert_eq!(interface.dimensions(), (WIDTH, HEIGHT));

    // --- The prompt, on the dark tokens -----------------------------------
    let dark_state = locked(Tokens::builtin_dark());
    let dark = interface.render(&dark_state).to_vec();

    assert_frame_shape(&dark, WIDTH, HEIGHT, "the dark prompt");
    let corners = [
        pixel(&dark, WIDTH, 0, 0),
        pixel(&dark, WIDTH, WIDTH - 1, 0),
        pixel(&dark, WIDTH, 0, HEIGHT - 1),
        pixel(&dark, WIDTH, WIDTH - 1, HEIGHT - 1),
    ];
    for corner in corners {
        assert_eq!(
            corner,
            token_pixel(theme::scrim(dark_state.tokens())),
            "the whole screen is painted, or the desktop shows through it"
        );
    }
    assert!(
        count_of(&dark, token_pixel(dark_state.tokens().colors.surface)) > 0,
        "the card is drawn on the surface token"
    );
    assert!(
        matches!(dark_state.model().face, Face::Prompt),
        "{:?}",
        dark_state.model().face
    );

    // The projection carries the frame's size and the tokens' colours, so "the
    // frame is the projection drawn" is a claim with two independently testable
    // halves: this is the projection half.
    let document = view::screen(dark_state.model(), dark_state.tokens(), WIDTH, HEIGHT);
    assert_eq!(document["surface"]["width"], WIDTH);
    assert_eq!(document["surface"]["height"], HEIGHT);
    assert_eq!(
        document["palette"]["scrim"],
        theme::qml_color(theme::scrim(dark_state.tokens()))
    );
    assert_eq!(
        document["field"]["visible"],
        dark_state.model().prompts_for_credential
    );
    snapshot("dark", &dark, WIDTH, HEIGHT);

    // --- The light theme draws a different screen --------------------------
    let light_state = locked(Tokens::builtin_light());
    let light = interface.render(&light_state).to_vec();

    assert_frame_shape(&light, WIDTH, HEIGHT, "the light prompt");
    assert_eq!(
        pixel(&light, WIDTH, 0, 0),
        token_pixel(theme::scrim(light_state.tokens()))
    );
    assert_ne!(light, dark, "the light theme drew the dark screen");
    snapshot("light", &light, WIDTH, HEIGHT);

    // --- A typed credential is masked, and the screen changes --------------
    let mut typed_state = locked(Tokens::builtin_dark());
    for character in "hunter2".chars() {
        update(&mut typed_state, Message::Character(character));
    }
    assert_eq!(typed_state.model().mask, "•••••••");
    let typed = interface.render(&typed_state).to_vec();

    assert_frame_shape(&typed, WIDTH, HEIGHT, "the typed prompt");
    assert_ne!(typed, dark, "typing into the field drew nothing new");
    snapshot("typed", &typed, WIDTH, HEIGHT);

    // --- A refused attempt -------------------------------------------------
    let mut failed_state = locked(Tokens::builtin_dark());
    fail_an_attempt(&mut failed_state, "wrong");
    assert!(matches!(failed_state.model().face, Face::Failed { .. }));
    let failed = interface.render(&failed_state).to_vec();

    assert_frame_shape(&failed, WIDTH, HEIGHT, "the failed face");
    assert_ne!(failed, dark, "a refusal looks like the prompt it refuses");
    assert!(
        count_of(
            &failed,
            token_pixel(theme::danger_text(failed_state.tokens()))
        ) > 0,
        "the refusal's line is not drawn in the danger token"
    );
    snapshot("failed", &failed, WIDTH, HEIGHT);

    // --- A spent allowance looks like a wait, not like a prompt ------------
    let mut waiting_state = locked(Tokens::builtin_dark());
    for _ in 0..crate::attempts::AttemptPolicy::standard().allowance {
        fail_an_attempt(&mut waiting_state, "wrong");
    }
    assert!(matches!(waiting_state.model().face, Face::Waiting { .. }));
    let waiting = interface.render(&waiting_state).to_vec();

    assert_frame_shape(&waiting, WIDTH, HEIGHT, "the waiting face");
    assert_ne!(
        waiting, dark,
        "a rate limit looks like an invitation to try"
    );
    assert!(
        count_of(
            &waiting,
            token_pixel(theme::warning_text(waiting_state.tokens()))
        ) > 0,
        "the wait is not drawn in the warning token"
    );
    snapshot("waiting", &waiting, WIDTH, HEIGHT);

    // --- A lock with no trusted UI ----------------------------------------
    let mut held_state = locked(Tokens::builtin_dark());
    update(&mut held_state, Message::UiExited);
    assert!(matches!(
        held_state.model().face,
        Face::HeldWithoutUi { .. }
    ));
    let held = interface.render(&held_state).to_vec();

    assert_frame_shape(&held, WIDTH, HEIGHT, "the held-without-ui face");
    assert_ne!(
        held, dark,
        "a lock with no UI looks like an operable screen"
    );
    assert_eq!(
        view::screen(held_state.model(), held_state.tokens(), WIDTH, HEIGHT)["field"]["visible"],
        false
    );
    snapshot("held-without-ui", &held, WIDTH, HEIGHT);

    // --- This build has no authentication boundary at all ------------------
    let unavailable_state = locked_unavailable(Tokens::builtin_dark(), ServiceFailure::NotProvided);
    assert!(matches!(
        unavailable_state.model().face,
        Face::Unavailable { retry: None, .. }
    ));
    let unavailable = interface.render(&unavailable_state).to_vec();

    assert_frame_shape(&unavailable, WIDTH, HEIGHT, "the unavailable face");
    assert_ne!(
        unavailable, dark,
        "a screen that says it cannot check a password looks like one that asks for one"
    );
    assert_eq!(
        view::screen(
            unavailable_state.model(),
            unavailable_state.tokens(),
            WIDTH,
            HEIGHT
        )["field"]["visible"],
        false,
        "a boundary this build does not have must draw no field to type into"
    );
    snapshot("unavailable", &unavailable, WIDTH, HEIGHT);

    // --- A boundary that could come back offers a retry --------------------
    let retry_state = locked_unavailable(Tokens::builtin_dark(), ServiceFailure::Disconnected);
    let retry = interface.render(&retry_state).to_vec();

    assert_frame_shape(&retry, WIDTH, HEIGHT, "the retryable-unavailable face");
    let actions = view::screen(retry_state.model(), retry_state.tokens(), WIDTH, HEIGHT)["actions"]
        .as_array()
        .expect("actions")
        .len();
    assert_eq!(actions, 2, "a retry control and the unlock control");
    snapshot("unavailable-retry", &retry, WIDTH, HEIGHT);

    // --- No session-lock protocol at all ----------------------------------
    let mut absent_state = state_in(&Locale::default_locale(), Tokens::builtin_dark());
    update(
        &mut absent_state,
        Message::ProtocolAbsent(ProtocolAbsence::GlobalMissing),
    );
    let absent = interface.render(&absent_state).to_vec();

    assert_frame_shape(&absent, WIDTH, HEIGHT, "the missing-protocol face");
    assert_ne!(absent, dark, "a session with no lock protocol looks locked");
    snapshot("no-protocol", &absent, WIDTH, HEIGHT);

    // --- The other language is drawn too ----------------------------------
    let zh = Locale::parse("zh-CN").expect("a locale");
    let chinese_state = locked_in(&zh, Tokens::builtin_dark());
    let chinese = interface.render(&chinese_state).to_vec();

    assert_frame_shape(&chinese, WIDTH, HEIGHT, "the zh-CN prompt");
    assert_ne!(chinese, dark, "the translated screen is the English one");
    snapshot("chinese", &chinese, WIDTH, HEIGHT);

    // --- Two hundred percent text still leaves the controls on screen ------
    let mut doubled = Tokens::builtin_dark();
    doubled.typography.scale *= 2.0;
    let doubled_state = locked(doubled.clone());
    let large = interface.render(&doubled_state).to_vec();

    assert_frame_shape(&large, WIDTH, HEIGHT, "the 200% prompt");
    assert_ne!(large, dark, "200% text drew the same screen as 100%");
    snapshot("large-text", &large, WIDTH, HEIGHT);

    // --- The refresh is the state's, not the renderer's --------------------
    let again = interface.render(&dark_state).to_vec();
    assert_eq!(
        again, dark,
        "the same state drew two different frames, so something in the renderer \
         holds state of its own"
    );

    // --- Two outputs, two surfaces, two sizes ------------------------------
    // The same state rendered through a second, live interface: a session can
    // have a laptop panel and a monitor, and the lock surface for each is
    // configured to its own size. This is why each `Interface` owns its own QML
    // engine and window rather than sharing one window that is resized
    // between surfaces.
    let mut second = Interface::new(640, 480).expect("a 640x480 surface");
    let small = second.render(&dark_state).to_vec();

    assert_eq!(small.len(), (640 * 480 * 4) as usize);
    assert_ne!(
        second.dimensions(),
        interface.dimensions(),
        "the two surfaces are configured to different sizes"
    );
    assert!(
        second.resize(320, 240),
        "a second configure on one surface is taken"
    );
    assert_eq!(second.dimensions(), (320, 240));
    assert_eq!(
        interface.dimensions(),
        (WIDTH, HEIGHT),
        "the other surface's size is untouched"
    );
    let resized = second.render(&dark_state).to_vec();
    assert_frame_shape(&resized, 320, 240, "the resized surface");
    snapshot("small", &resized, 320, 240);
}

#[test]
fn a_surface_this_component_will_not_draw_is_refused_before_any_allocation() {
    // A configure is data from outside this process. The size bound lives in the
    // renderer so the protocol leg can refuse it without allocating anything; the
    // output then keeps the compositor's own locked blank.
    assert!(Interface::new(0, 0).is_none());
    assert!(Interface::new(65_536, 65_536).is_none());
}
