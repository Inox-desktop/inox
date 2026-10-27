//! Headless evidence: the projection, the QML window, and the keyboard.
//!
//! Everything lives inside the binary because cxx-qt 0.10 links the bridge's
//! generated C++ archive into every target of the package: an integration test in
//! `tests/` is a separate target and fails to link (`undefined reference to
//! 'cxxbridge1$…Launcher…'`), for the same reason a library target would
//! (KDAB/cxx-qt#770, docs/qml-migration-contract.md §2.1).
//!
//! Four kinds of evidence:
//!
//!   * **projection** — the JSON document QML parses: colours are the token
//!     conversions, sizes are the token derivations, every sentence comes from
//!     Fluent in both catalogs, and no message id leaks into the document;
//!   * **keyboard and focus at the app level** — the actions the QML tree
//!     dispatches go through the same `update` a real window uses, so "typing then
//!     Enter launches once" and "Escape asks to close" are properties of the
//!     state machine, not of a running event loop;
//!   * **the QML window loads** on the offscreen platform, which needs a process
//!     whose environment says `QT_QPA_PLATFORM=offscreen` *before* Qt starts. The
//!     test binary spawns itself as that child: mutating this process's
//!     environment would be `unsafe` in edition 2024, which Inox code may not
//!     write (contract §7.3);
//!   * the scenarios the Iced render tests used to draw (dark, light, no match,
//!     loading, empty, unavailable, a long name, 200 % text) as projection
//!     assertions, since there is no widget tree to rasterise any more.
//!
//! Run with:
//!
//! ```sh
//! QT_QPA_PLATFORM=offscreen cargo test -p inox-launcher
//! ```

use std::process::Command;
use std::rc::Rc;

use inox_common::i18n::Locale;
use inox_config::Theme;
use inox_config::shell::LauncherConfig;

use crate::app::{self, Message, Prepared, State};
use crate::cxxqt_object::action_message;
use crate::index::AppIndex;
use crate::placement::{LogicalSize, Placement};
use crate::theme::{self, TextRole};

/// Environment variable the parent test sets on the child it spawns.
const CHILD_MARKER: &str = "INOX_LAUNCHER_QML_CHILD";

/// Line the child prints once the QML tree has loaded and reported ready.
const SELF_CHECK_MARKER: &str = "INOX_LAUNCHER_QML_SELF_CHECK_OK";

fn prepared(tokens: Theme, locale: &str) -> Prepared {
    let config = LauncherConfig::default();
    prepared_on(tokens, locale, config, LogicalSize::new(1280, 720))
}

fn prepared_on(
    tokens: Theme,
    locale: &str,
    config: LauncherConfig,
    output: LogicalSize,
) -> Prepared {
    let placement = Placement::overlay(&config, &tokens, output, None);
    Prepared {
        tokens,
        localizer: Rc::new(
            crate::i18n::localizer(&Locale::parse(locale).expect("a well-formed tag"))
                .expect("the catalogs load"),
        ),
        config,
        placement,
    }
}

/// Two ordinary applications and a terminal one nobody configured, so a
/// disabled row is in the list too.
fn index() -> AppIndex {
    let mut terminal = AppIndex::test_entry("htop.desktop", "htop", "htop");
    terminal.terminal = true;
    AppIndex::from_entries(
        vec![
            AppIndex::test_entry("firefox.desktop", "Firefox", "true"),
            terminal,
            AppIndex::test_entry("editor.desktop", "Text Editor", "true"),
        ],
        1,
    )
}

fn state(locale: &str) -> State {
    let prepared = prepared(Theme::builtin_dark(), locale);
    State::with_index(&prepared, index(), 1)
}

/// Applies one action document exactly as the QML tree would dispatch it.
///
/// Panics when the action is not understood at all — a control sending garbage is
/// a bug in this crate's own QML, which the test should not tolerate — and does
/// nothing for a well-formed action the launcher has no use for.
fn dispatch(state: &mut State, action: &str) -> Option<Message> {
    let message = action_message(action)
        .unwrap_or_else(|error| panic!("{action} is not an action document: {error}"));
    if let Some(message) = message.clone() {
        drop(app::update(state, message.clone()));
    }
    message
}

#[test]
fn the_qml_actions_type_into_the_search_field() {
    let mut state = state("en-US");
    assert_eq!(state.query, "");

    dispatch(&mut state, r#"{"type":"set-query","value":"fire"}"#);
    assert_eq!(state.query, "fire");
    assert_eq!(state.model.tiles.len(), 1);
    assert_eq!(state.model.tiles[0].label, "Firefox");

    dispatch(&mut state, r#"{"type":"clear-query"}"#);
    assert_eq!(state.query, "");
    assert_eq!(state.model.tiles.len(), 3);
}

#[test]
fn the_qml_actions_launch_exactly_once_per_selection() {
    let mut state = state("en-US");
    dispatch(&mut state, r#"{"type":"set-query","value":"fire"}"#);

    let selection = state.selection_generation;
    for _ in 0..3 {
        dispatch(&mut state, r#"{"type":"press","key":"enter"}"#);
    }

    assert!(state.activator.is_tracked(selection));
    assert_eq!(
        state.activator.tracked(),
        1,
        "three presses of Enter before the first has a result are one launch",
    );
    assert_eq!(
        state.activation.as_ref().map(|it| it.entry.as_str()),
        Some("firefox.desktop"),
    );
}

#[test]
fn the_qml_actions_step_the_grid_without_wrapping() {
    let mut state = state("en-US");
    let columns = state.placement.grid.columns;
    assert!(columns >= 2, "the fixture grid has more than one column");

    dispatch(&mut state, r#"{"type":"press","key":"up"}"#);
    dispatch(&mut state, r#"{"type":"press","key":"left"}"#);
    assert_eq!(state.selected, 0, "the first tile is already selected");

    dispatch(&mut state, r#"{"type":"press","key":"right"}"#);
    assert_eq!(state.selected, 1, "right is one tile");

    for _ in 0..9 {
        dispatch(&mut state, r#"{"type":"press","key":"down"}"#);
    }
    assert_eq!(
        state.selected,
        state.model.tiles.len() - 1,
        "the last tile is a wall, not a wrap-around",
    );
}

/// The projection carries the position of the selection, which is the only
/// number the QML grid scrolls to, and the column count the keyboard steps by.
///
/// The scroll itself happens in QML (`positionViewAtIndex(…, GridView.Contain)`)
/// and this crate has no QML interaction harness, so what is proved here is the
/// half that Rust owns: after real key presses through the same `update` a window
/// uses, the document names the selected tile — and `null` when the screen has no
/// selection at all. The grid following that number is asserted by inspection of
/// `qml/Main.qml`, not by a test.
#[test]
fn the_projection_carries_the_tile_the_grid_has_to_keep_visible() {
    let mut state = state("en-US");
    let document = crate::view::project(&state);

    assert_eq!(document["tiles"].as_array().map(Vec::len), Some(3));
    assert_eq!(document["selected"].as_u64(), Some(0));
    assert_eq!(
        document["grid"]["columns"].as_u64(),
        Some(state.placement.grid.columns as u64),
        "the window draws the columns the keyboard moves by",
    );

    dispatch(&mut state, r#"{"type":"press","key":"down"}"#);
    let document = crate::view::project(&state);
    assert_eq!(state.selected, 2);
    assert_eq!(
        document["selected"].as_u64(),
        Some(2),
        "the projection names the last tile the grid has to keep visible",
    );
    assert_eq!(document["tiles"][2]["selected"].as_bool(), Some(true));
    assert_eq!(document["tiles"][0]["selected"].as_bool(), Some(false));

    // A query that matches nothing leaves no tile to scroll to.
    let mut filtered = self::state("en-US");
    dispatch(&mut filtered, r#"{"type":"set-query","value":"zzzz"}"#);
    assert!(crate::view::project(&filtered)["selected"].is_null());
}

#[test]
fn the_qml_actions_ask_the_launcher_to_close_on_escape() {
    let mut state = state("en-US");
    assert!(!state.close_requested());

    let message = dispatch(&mut state, r#"{"type":"press","key":"escape"}"#);
    assert_eq!(message, Some(Message::Dismissed));
    assert!(state.close_requested());
}

#[test]
fn a_held_key_is_not_a_second_press() {
    let mut state = state("en-US");
    let moved = dispatch(
        &mut state,
        r#"{"type":"press","key":"right","repeat":false}"#,
    );
    assert_eq!(moved, Some(Message::SelectionMoved(app::Step::Right)));

    let repeated = dispatch(
        &mut state,
        r#"{"type":"press","key":"right","repeat":true}"#,
    );
    assert_eq!(repeated, None, "holding a key is one press, not a stream");
    assert_eq!(state.selected, 1);
}

#[test]
fn a_pointer_choice_launches_the_row_it_landed_on() {
    let mut state = state("en-US");
    dispatch(&mut state, r#"{"type":"choose","id":"editor.desktop"}"#);

    assert_eq!(state.selected, 2);
    assert_eq!(
        state.activation.as_ref().map(|it| it.entry.as_str()),
        Some("editor.desktop")
    );
}

#[test]
fn an_action_the_launcher_does_not_understand_is_reported_not_panicked() {
    assert!(action_message("{not json").is_err());
    assert!(action_message(r#"{"type":"dance"}"#).is_err());
    assert!(matches!(
        action_message(r#"{"type":"press","key":"tab"}"#),
        Ok(None)
    ));
    assert_eq!(
        action_message(r#"{"type":"press","key":"enter"}"#).expect("an action document"),
        Some(Message::Activated)
    );
}

#[test]
fn the_launcher_focuses_the_search_field_as_it_opens() {
    let mut state = state("en-US");
    assert_eq!(state.focus_serial, 0);

    // What `boot` does before the first frame.
    app::focus_search(&mut state);
    assert_eq!(state.focus_serial, 1);

    let projected = crate::view::project(&state);
    assert_eq!(
        projected["focus"]["field"].as_str(),
        Some(app::SEARCH_INPUT_ID)
    );
    assert_eq!(projected["focus"]["serial"].as_u64(), Some(1));
}

#[test]
fn the_projection_is_the_theme_and_the_catalogs_in_both_languages() {
    for tag in ["en-US", "zh-CN"] {
        let state = state(tag);
        let document = crate::view::project(&state);

        assert_eq!(
            document["theme"]["colors"]["background"].as_str(),
            Some(theme::qml_color(state.tokens.colors.background).as_str()),
            "{tag}: the window colour is not the token",
        );
        assert_eq!(
            document["theme"]["font"]["body"].as_f64().map(f64::round),
            Some(f64::from(
                theme::role_size(&state.tokens, TextRole::Body).round()
            )),
            "{tag}: the body size is not the token derivation",
        );
        assert_eq!(
            document["title"].as_str(),
            Some(state.localizer.message("launcher-window-title").as_str()),
            "{tag}: the title is not the catalog's",
        );
        assert!(
            document["hint"]
                .as_str()
                .is_some_and(|hint| !hint.is_empty()),
            "{tag}: the key hint is missing",
        );
    }

    let english = crate::view::project(&state("en-US"));
    let chinese = crate::view::project(&state("zh-CN"));
    assert_ne!(
        english["title"], chinese["title"],
        "the zh-CN catalog must reach the projection",
    );
    assert_ne!(
        english["hint"], chinese["hint"],
        "the key hints are translated in both catalogs",
    );
}

#[test]
fn the_projection_never_shows_a_message_id() {
    let state = state("en-US");
    let document = crate::view::to_json(&state);
    let ids = inox_common::i18n::message_ids(&[crate::i18n::EN_US]).expect("the catalog parses");

    assert!(!ids.is_empty(), "the catalog defines messages");
    for id in ids {
        assert!(
            !document.contains(&id),
            "{id} reached the document instead of its text",
        );
    }
}

#[test]
fn the_two_builtin_themes_project_different_documents() {
    let dark = Theme::builtin_dark();
    let light = Theme::builtin_light();
    let dark_document = crate::view::project(&State::with_index(
        &prepared(dark.clone(), "en-US"),
        index(),
        1,
    ));
    let light_document = crate::view::project(&State::with_index(
        &prepared(light.clone(), "en-US"),
        index(),
        1,
    ));

    assert_ne!(
        dark_document["theme"]["colors"], light_document["theme"]["colors"],
        "a light theme that projects the dark colours is not a theme",
    );
    assert_eq!(
        dark_document["theme"]["styles"]["window"]["background"].as_str(),
        Some(theme::qml_color(dark.colors.background).as_str())
    );
    assert_eq!(
        light_document["theme"]["colors"]["background"].as_str(),
        Some(theme::qml_color(light.colors.background).as_str())
    );
}

#[test]
fn the_states_the_acceptance_criteria_name_project_their_own_screens() {
    use crate::contract::ServiceState;

    let ready = state("en-US");
    assert_eq!(ready.model.banner, None);
    assert!(
        !ready.model.tiles.is_empty(),
        "the resting state is the grid of applications",
    );
    assert_eq!(
        ready.model.hint,
        ready.localizer.message("launcher-search-hint"),
        "and its hint names the keys the grid binds",
    );

    let mut no_match = state("en-US");
    dispatch(&mut no_match, r#"{"type":"set-query","value":"zzzz"}"#);
    assert_eq!(
        no_match.model.banner.as_ref().map(|banner| banner.kind),
        Some(crate::view_model::BannerKind::NoMatch)
    );

    let mut loading = state("en-US");
    loading.service = ServiceState::Loading { generation: 2 };
    loading.refresh_model();
    assert_eq!(
        loading.model.banner.as_ref().map(|banner| banner.kind),
        Some(crate::view_model::BannerKind::Loading)
    );
    assert!(!loading.model.can_retry_index());

    let mut unavailable = state("en-US");
    unavailable.service = ServiceState::Unavailable {
        generation: 2,
        failure: crate::contract::Failure::service(
            crate::contract::FailureKind::IndexUnavailable {
                detail: "no HOME".to_owned(),
            },
        ),
    };
    unavailable.refresh_model();
    assert!(unavailable.model.can_retry_index());

    let mut empty = State::with_index(
        &prepared(Theme::builtin_dark(), "en-US"),
        AppIndex::from_entries(Vec::new(), 1),
        1,
    );
    empty.refresh_model();
    assert_eq!(
        empty.model.banner.as_ref().map(|banner| banner.kind),
        Some(crate::view_model::BannerKind::Empty)
    );
}

#[test]
fn a_long_name_is_truncated_to_the_tile_it_is_drawn_in() {
    // A small output: the placement gives a tile's label less room, which is the
    // case a long name has to survive.
    let prepared = prepared_on(
        Theme::builtin_dark(),
        "en-US",
        LauncherConfig::default(),
        LogicalSize::new(520, 400),
    );
    let mut long = AppIndex::test_entry(
        "long.desktop",
        "An Application With A Name That Would Not Fit In Any Reasonable Row At All",
        "true",
    );
    long.icon = None;
    let mut state = State::with_index(&prepared, AppIndex::from_entries(vec![long], 1), 1);
    dispatch(&mut state, r#"{"type":"set-query","value":"application"}"#);

    let projected = crate::view::project(&state);
    let tile = &projected["tiles"][0];
    assert_eq!(tile["truncated"].as_bool(), Some(true));
    assert!(
        tile["label"]
            .as_str()
            .is_some_and(|label| label.ends_with('…')),
        "{:?}",
        tile["label"],
    );
    assert_eq!(
        tile["label"].as_str().map(|label| label.chars().count()),
        Some(state.placement.label_chars),
        "a truncated label is exactly as wide as the tile's label",
    );
}

#[test]
fn a_two_hundred_percent_text_scale_grows_the_projected_type() {
    let mut tokens = Theme::builtin_dark();
    tokens.typography.scale = 2.0;
    let scaled = State::with_index(&prepared(tokens.clone(), "en-US"), index(), 1);
    let normal = state("en-US");

    let scaled_document = crate::view::project(&scaled);
    let normal_document = crate::view::project(&normal);

    assert_eq!(
        scaled_document["theme"]["font"]["heading"].as_f64(),
        normal_document["theme"]["font"]["heading"]
            .as_f64()
            .map(|size| (size * 2.0 * 100.0).round() / 100.0),
        "200 % text doubles every role",
    );
    assert!(
        scaled_document["theme"]["metrics"]["field_height"]
            .as_f64()
            .is_some_and(|height| height > 0.0)
    );
    assert!(
        scaled.placement.label_chars > 0,
        "a 200 % scale may not leave a tile with no room for a name",
    );
    assert!(
        scaled_document["grid"]["cell_width"].as_f64()
            > normal_document["grid"]["cell_width"].as_f64(),
        "the tile grows with the type rather than cutting the label",
    );
    assert!(
        scaled.placement.grid.columns <= normal.placement.grid.columns,
        "wider tiles mean fewer columns in the same surface",
    );
}

/// The child half of [`the_qml_window_loads_offscreen`].
///
/// Ignored so a normal `cargo test` does not run it in-process; the parent spawns
/// the test binary with `QT_QPA_PLATFORM=offscreen` already set, which is the only
/// way to select a Qt platform without mutating the environment.
#[test]
#[ignore = "spawned as a child process by the_qml_window_loads_offscreen"]
fn qml_child_self_check() {
    assert_eq!(
        std::env::var(CHILD_MARKER).as_deref(),
        Ok("1"),
        "this test only runs as the spawned child"
    );

    // The warnings are the only place a refused `reportReady` explains itself,
    // and the parent reads this process's stderr — install the subscriber the
    // binary installs in `main`, so a failure is diagnosable from the parent's
    // assertion message instead of a bare exit status.
    let _ = inox_common::logging::init();

    // The same boot `main` runs, then the window's own load path.
    let boot = crate::boot().expect("the launcher boots without a display");
    assert!(
        crate::install_boot(boot),
        "the child installs the boot once"
    );
    crate::run_window(true).expect("the QML module loads and the tree reports ready");
    assert!(
        crate::cxxqt_object::qml_ready(),
        "the QML tree must confirm it read the Rust projection",
    );

    // The parent cannot see this process's tracing subscriber, so the success
    // marker goes to stdout. `println!` is allowed in tests (root AGENTS.md §3.5).
    println!("{SELF_CHECK_MARKER}");
}

#[test]
fn the_qml_window_loads_offscreen() {
    let binary = std::env::current_exe().expect("the test binary path");
    let output = Command::new(binary)
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "headless_tests::qml_child_self_check",
        ])
        .env(CHILD_MARKER, "1")
        .env("QT_QPA_PLATFORM", "offscreen")
        // Qt suppresses its own messages when stderr is a pipe (which it is here),
        // and a QML type error is the one thing that explains a failed load; force
        // them out so the assertion below carries the reason.
        .env("QT_ASSUME_STDERR_HAS_CONSOLE", "1")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .output()
        .expect("the child test process runs");

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the offscreen launcher self-check failed: {text}"
    );
    assert!(
        text.contains(SELF_CHECK_MARKER),
        "the child did not report a successful self-check: {text}"
    );
    // Qt reports QML errors on stderr even when the process survives.
    for marker in [
        "is not a type",
        "Cannot assign",
        "Unable to assign",
        "TypeError",
        "Cannot read property",
        "ReferenceError",
    ] {
        assert!(!text.contains(marker), "QML reported an error: {text}");
    }
}
