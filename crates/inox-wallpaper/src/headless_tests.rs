//! Headless evidence for the QML preview.
//!
//! Everything lives inside the binary because cxx-qt 0.10 links the bridge's
//! generated C++ archive into every target of the package: an integration test
//! in `tests/` is a separate target and fails to link (`undefined reference to
//! 'cxxbridge1$…WallpaperPreview…'`), for the same reason a library target would
//! (KDAB/cxx-qt #770). The binary is therefore the only place that can hold
//! evidence.
//!
//! Four kinds of evidence:
//!
//!   * the **projection** QML reads: token-derived colours and sizes, the Fluent
//!     strings in `en-US` and `zh-CN`, the wallpaper geometry, and the accessible
//!     item count the tree has to agree with;
//!   * the **catalogs**, checked against each other (acceptance Q03);
//!   * the **command line**, through the same argument handling `main` uses and
//!     a child process that maps the result onto the exit status `main` produces;
//!   * the **QML module actually loading** on the offscreen platform, together
//!     with Qt decoding the projected image. That needs a process whose
//!     environment sets `QT_QPA_PLATFORM` *before* Qt starts, so the test binary
//!     spawns itself as that child: mutating this process's environment would be
//!     `unsafe` in edition 2024, which Inox code may not write. The child also
//!     points `XDG_CONFIG_HOME` at a fixture whose `[wallpaper]` names a real PNG,
//!     so the run exercises config → decode → projection → QML image.
//!
//! Run with:
//!
//! ```sh
//! QT_QPA_PLATFORM=offscreen cargo test -p inox-wallpaper --all-features
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;

use inox_common::i18n::{Locale, Localizer};
use inox_config::{Appearance, Theme, WallpaperMode};
use serde_json::Value;

use crate::app::{Message, PREVIEW_SIZE, Preview, State, update};
use crate::apply;
use crate::i18n::{self, CHINESE_TAG, MESSAGES};
use crate::model::{OutputName, Selection, Unusable};
use crate::theme;
use crate::view::{self, Projection};
use crate::view_model;

/// Environment variable the parent test sets on the QML child.
const CHILD_MARKER: &str = "INOX_WALLPAPER_QML_CHILD";

/// Line the QML child prints once the tree reported a matching projection.
const SELF_CHECK_MARKER: &str = "INOX_WALLPAPER_QML_SELF_CHECK_OK";

/// Environment variable that turns the test binary into a command-line child.
const CLI_CHILD_MARKER: &str = "INOX_WALLPAPER_CLI_CHILD";

/// Environment variable carrying the child's arguments, separated by `\u{1f}`.
const CLI_CHILD_ARGS: &str = "INOX_WALLPAPER_CLI_ARGS";

/// The output every scenario previews.
const OUTPUT: &str = "HDMI-A-1";

/// A localizer for one of the catalogs the crate ships.
fn localizer(tag: &str) -> Rc<Localizer> {
    let locale = Locale::parse(tag).expect("a well-formed tag");

    Rc::new(i18n::localizer(locale).expect("the shipped catalogs"))
}

/// The built-in tokens, optionally at a font-scale preference.
fn tokens(appearance: Appearance, scale: f32) -> Arc<Theme> {
    let mut tokens = match appearance {
        Appearance::Light => Theme::builtin_light(),
        _ => Theme::builtin_dark(),
    };
    tokens.typography.scale = scale;

    Arc::new(tokens)
}

/// A state that has not loaded anything yet.
fn boot(
    tokens: Arc<Theme>,
    localizer: Rc<Localizer>,
    selection: Selection,
    mode: WallpaperMode,
    configuration: Option<Unusable>,
) -> State {
    State::new(
        tokens,
        localizer,
        Preview {
            output: OutputName::from(OUTPUT),
            size: PREVIEW_SIZE,
            mode,
            selection,
            wallpaper_dirs: Arc::from(Vec::<PathBuf>::new()),
        },
        configuration,
    )
}

/// A state that went through the request the runtime would have made.
fn requested(state: State, mode: WallpaperMode) -> State {
    let mut state = state;

    if let Some(request) = state.take_request() {
        let preparation =
            apply::prepare(request.output, &request.selection, mode, PREVIEW_SIZE, &[]);
        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: request.generation,
                preparation,
            },
        );
    }

    state
}

/// A PNG fixture of `width` × `height` in a colour no theme uses.
fn fixture(dir: &Path, width: u32, height: u32) -> PathBuf {
    let path = dir.join("wall.png");
    image::RgbaImage::from_pixel(width, height, image::Rgba([1, 2, 3, 255]))
        .save(&path)
        .expect("the fixture is written");

    path
}

/// The projection and the document QML parses for `state`.
fn projected(state: &State) -> (Projection, Value) {
    let projection = view::project(state);
    let document = serde_json::from_str(projection.json()).expect("the projection is JSON");

    (projection, document)
}

/// The number of accessible items the document asks the QML tree for, counted
/// from what it draws.
fn counted_accessible_items(document: &Value) -> i32 {
    let lines = |value: &Value| -> i32 {
        value
            .as_array()
            .map_or(0, |lines| i32::try_from(lines.len()).unwrap_or(i32::MAX))
    };

    let mut count = 1; // the content pane
    count += lines(&document["card"]["lines"]);
    count += i32::from(!document["footer"].is_null());

    if !document["notice"].is_null() {
        count += lines(&document["notice"]["lines"]);
        count += 1; // the control
        count += i32::from(!document["notice"]["action"]["reason"].is_null());
    }

    count
}

/// Every string a document carries, for the "no message id reaches the screen"
/// assertions.
fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                strings(item, out);
            }
        }
        Value::Object(fields) => {
            for field in fields.values() {
                strings(field, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// A document's text values.
fn text_values(document: &Value) -> Vec<String> {
    let mut out = Vec::new();
    strings(document, &mut out);

    out
}

#[test]
fn the_projection_carries_the_tokens_as_qml_values() {
    let tokens = tokens(Appearance::Light, 1.0);
    let state = boot(
        Arc::clone(&tokens),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let (_, document) = projected(&state);
    let tokens_json = &document["tokens"];

    // Colours are the token conversion, never a literal QML could have written.
    for (field, color) in [
        ("background", tokens.colors.background),
        ("surface", tokens.colors.surface),
        ("surfaceVariant", tokens.colors.surface_variant),
        ("border", tokens.colors.border),
        ("text", tokens.colors.text),
        ("textMuted", tokens.colors.text_muted),
        ("accent", tokens.colors.accent),
        ("onAccent", tokens.colors.on_accent),
        ("warning", tokens.colors.warning),
        ("success", tokens.colors.success),
        ("danger", tokens.colors.danger),
    ] {
        assert_eq!(
            tokens_json["colors"][field].as_str(),
            Some(theme::qml_color(color).as_str()),
            "{field} is not the token colour"
        );
    }

    // Sizes are the token arithmetic, in whole logical pixels.
    assert_eq!(
        tokens_json["font"]["size"]["title"],
        theme::px(theme::title_size(&tokens))
    );
    assert_eq!(
        tokens_json["font"]["size"]["heading"],
        theme::px(theme::heading_size(&tokens))
    );
    assert_eq!(
        tokens_json["font"]["size"]["body"],
        theme::px(theme::body_size(&tokens))
    );
    assert_eq!(
        tokens_json["font"]["size"]["caption"],
        theme::px(theme::caption_size(&tokens))
    );
    assert_eq!(tokens_json["spacing"]["sm"], theme::gap(&tokens));
    assert_eq!(tokens_json["spacing"]["lg"], theme::section_gap(&tokens));
    assert_eq!(
        tokens_json["sizes"]["cardWidth"],
        theme::card_width(&tokens)
    );
    assert_eq!(
        tokens_json["sizes"]["windowPadding"],
        theme::window_padding(&tokens)
    );
    assert_eq!(
        tokens_json["sizes"]["controlHeight"],
        theme::control_height(&tokens)
    );
    assert_eq!(
        tokens_json["sizes"]["controlPadding"]["left"],
        theme::control_padding(&tokens).left
    );
    assert_eq!(
        tokens_json["card"]["background"].as_str(),
        Some(theme::card(&tokens).background.as_str())
    );
    assert_eq!(
        tokens_json["card"]["borderWidth"],
        theme::card(&tokens).border_width
    );

    // The window is the preview the screenshots are taken at (acceptance Q02).
    assert_eq!(tokens_json["window"]["width"], PREVIEW_SIZE.width);
    assert_eq!(tokens_json["window"]["height"], PREVIEW_SIZE.height);
    assert_eq!(document["title"], "Inox wallpaper");
}

#[test]
fn the_projected_strings_are_the_fluent_catalogs() {
    for (tag, title, heading, scaling, footer) in [
        (
            "en-US",
            "Inox wallpaper",
            "Output HDMI-A-1",
            "Scaling: fill",
            "Previewing in a window",
        ),
        (
            CHINESE_TAG,
            "Inox 壁纸",
            "输出 HDMI-A-1",
            "缩放：填充",
            "当前以窗口方式预览",
        ),
    ] {
        let state = boot(
            tokens(Appearance::Dark, 1.0),
            localizer(tag),
            Selection::ThemeBackground,
            WallpaperMode::Fill,
            None,
        );
        let (_, document) = projected(&state);
        let lines = document["card"]["lines"]
            .as_array()
            .expect("the card has lines");

        assert_eq!(document["title"], title, "{tag}");
        assert_eq!(lines[0]["text"], heading, "{tag}");
        // The card reads heading, summary, source, scaling, phase, hint; the empty
        // state has no source line, so the scaling line is found rather than
        // indexed — the order is `view_model::StatusCard::lines`, not this test's.
        assert!(
            lines.iter().any(|line| line["text"] == scaling),
            "{tag}: the scaling sentence is missing from {lines:?}"
        );
        assert!(
            document["footer"]["text"]
                .as_str()
                .is_some_and(|text| text.contains(footer)),
            "{tag}: {:?}",
            document["footer"]["text"]
        );

        for text in text_values(&document) {
            assert!(
                !MESSAGES.contains(&text.as_str()),
                "`{text}` is a message id"
            );
            assert!(!text.contains('$'), "`{text}` has a placeholder left in it");
        }
    }
}

#[test]
fn a_loaded_output_projects_the_frame_the_rect_and_the_phase() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 640, 480);
    let state = requested(
        boot(
            tokens(Appearance::Light, 1.0),
            localizer("en-US"),
            Selection::Image(path),
            WallpaperMode::Fill,
            None,
        ),
        WallpaperMode::Fill,
    );
    let (_, document) = projected(&state);
    let wallpaper = &document["wallpaper"];

    assert!(
        wallpaper["dataUrl"]
            .as_str()
            .is_some_and(|url| url.starts_with("data:image/png;base64,")),
        "QML is handed a data URL: {wallpaper:?}"
    );
    assert_eq!(wallpaper["sourceWidth"], 640);
    assert_eq!(wallpaper["sourceHeight"], 480);
    assert_eq!(wallpaper["tiled"], false);

    // The rectangle is the geometry's answer, not something QML computes.
    let first = crate::scaling::place(
        WallpaperMode::Fill,
        crate::model::Size::new(640, 480),
        PREVIEW_SIZE,
    )
    .expect("a placement")
    .first();
    assert_eq!(wallpaper["rect"]["x"], first.x);
    assert_eq!(wallpaper["rect"]["y"], first.y);
    assert_eq!(wallpaper["rect"]["width"], first.width);
    assert_eq!(wallpaper["rect"]["height"], first.height);

    let lines = document["card"]["lines"]
        .as_array()
        .expect("the card has lines");
    assert!(
        lines.iter().any(|line| line["text"] == "640×480 image"),
        "the card says what it is drawing: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line["text"]
            .as_str()
            .is_some_and(|text| text.contains("wall.png"))),
        "the card names the file: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line["text"] == "Applied"),
        "a committed frame is in effect: {lines:?}"
    );
    assert!(document["notice"].is_null());
}

#[test]
fn a_tiled_projection_asks_qt_to_tile_the_whole_output() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 320, 240);
    let state = requested(
        boot(
            tokens(Appearance::Dark, 1.0),
            localizer("en-US"),
            Selection::Image(path),
            WallpaperMode::Tile,
            None,
        ),
        WallpaperMode::Tile,
    );
    let (_, document) = projected(&state);
    let wallpaper = &document["wallpaper"];

    assert_eq!(wallpaper["tiled"], true);
    assert_eq!(wallpaper["rect"]["x"], 0);
    assert_eq!(wallpaper["rect"]["y"], 0);
    assert_eq!(wallpaper["rect"]["width"], PREVIEW_SIZE.width);
    assert_eq!(wallpaper["rect"]["height"], PREVIEW_SIZE.height);
    assert!(
        document["card"]["lines"]
            .as_array()
            .is_some_and(|lines| lines.iter().any(|line| line["text"] == "Scaling: tile")),
        "the mode is on screen"
    );
}

#[test]
fn the_empty_projection_says_so_and_offers_nothing_to_press() {
    let state = boot(
        tokens(Appearance::Light, 1.0),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let (_, document) = projected(&state);
    let lines = document["card"]["lines"]
        .as_array()
        .expect("the card has lines");

    assert!(document["wallpaper"].is_null());
    assert!(
        document["notice"].is_null(),
        "a chosen state is not a report"
    );
    assert!(
        lines
            .iter()
            .any(|line| line["text"] == "The theme background"),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line["kind"] == "idle" && line["text"] == "No image"),
        "the phase is projected with its kind: {lines:?}"
    );
    assert!(
        lines.iter().any(|line| line["text"]
            .as_str()
            .is_some_and(|text| text.contains("wallpaper.image is set to none"))),
        "the empty state says what would change it: {lines:?}"
    );
    assert_eq!(
        document["accessibleItems"],
        counted_accessible_items(&document)
    );
}

#[test]
fn a_missing_image_is_projected_as_a_problem_with_an_enabled_retry() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let missing = dir.path().join("gone.png");
    let state = requested(
        boot(
            tokens(Appearance::Dark, 1.0),
            localizer("en-US"),
            Selection::Image(missing),
            WallpaperMode::Fill,
            None,
        ),
        WallpaperMode::Fill,
    );
    let (_, document) = projected(&state);
    let notice = &document["notice"];

    assert_eq!(notice["kind"], "problem");
    assert_eq!(
        notice["lines"][0]["text"],
        "The selected wallpaper could not be used"
    );
    assert_eq!(notice["action"]["enabled"], true);
    assert_eq!(notice["action"]["label"], "Try again");
    assert!(notice["action"]["reason"].is_null(), "the control works");
    assert!(
        notice["lines"].as_array().is_some_and(|lines| lines
            .iter()
            .any(|line| line["text"] == "the file does not exist")),
        "{notice:?}"
    );

    // The enabled drawing is the primary role's active state, from tokens.
    assert_eq!(
        notice["action"]["style"]["background"],
        theme::qml_color(state.theme().colors.accent)
    );
    assert_eq!(
        notice["action"]["styles"]["disabled"]["background"],
        theme::qml_color(state.theme().colors.surface_variant)
    );
    assert!(
        document["card"]["lines"]
            .as_array()
            .is_some_and(|lines| lines.iter().any(|line| line["kind"] == "degraded")),
        "the card says the image is not applied"
    );
}

#[test]
fn a_configuration_that_could_not_be_read_projects_a_disabled_control_with_its_reason() {
    let state = boot(
        tokens(Appearance::Light, 1.0),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        Some(Unusable::Unreadable {
            detail: String::from("permission denied"),
        }),
    );
    let (_, document) = projected(&state);
    let action = &document["notice"]["action"];

    assert_eq!(document["notice"]["kind"], "problem");
    assert!(
        document["notice"]["lines"][0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("The configuration could not be read")),
        "{:?}",
        document["notice"]["lines"][0]
    );
    assert_eq!(action["enabled"], false);
    assert_eq!(
        action["reason"]["text"],
        "The configuration is read once, when the component starts."
    );
    let disabled = theme::button_style(
        state.theme(),
        theme::ButtonRole::Primary,
        theme::ButtonState::Disabled,
    );
    assert_eq!(
        action["style"],
        serde_json::json!({
            "background": disabled.background,
            "text": disabled.text,
            "border": disabled.border,
            "borderWidth": disabled.border_width,
            "radius": disabled.radius,
        })
    );
}

#[test]
fn a_load_in_flight_projects_its_notice_and_suppresses_a_second_phase_line() {
    let state = boot(
        tokens(Appearance::Dark, 1.0),
        localizer("en-US"),
        Selection::Image(PathBuf::from("/walls/wall.png")),
        WallpaperMode::Fill,
        None,
    );
    let (_, document) = projected(&state);
    let lines = document["card"]["lines"]
        .as_array()
        .expect("the card has lines");

    assert_eq!(document["notice"]["kind"], "loading");
    assert_eq!(
        document["notice"]["lines"][0]["text"],
        "Loading the wallpaper…"
    );
    assert_eq!(document["notice"]["action"]["enabled"], true);
    assert_eq!(
        lines.len(),
        2,
        "the notice already says the load is running: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line["kind"] == "loading"),
        "the phase line would repeat the notice"
    );
    assert!(document["wallpaper"].is_null());
}

#[test]
fn the_accessible_item_count_is_the_number_of_lines_the_tree_draws() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 64, 64);
    let scenarios = [
        (
            "loaded",
            requested(
                boot(
                    tokens(Appearance::Dark, 1.0),
                    localizer("en-US"),
                    Selection::Image(path),
                    WallpaperMode::Fill,
                    None,
                ),
                WallpaperMode::Fill,
            ),
        ),
        (
            "empty",
            boot(
                tokens(Appearance::Dark, 1.0),
                localizer("en-US"),
                Selection::ThemeBackground,
                WallpaperMode::Fill,
                None,
            ),
        ),
        (
            "missing",
            requested(
                boot(
                    tokens(Appearance::Dark, 1.0),
                    localizer("en-US"),
                    Selection::Image(PathBuf::from("/walls/gone.png")),
                    WallpaperMode::Fill,
                    None,
                ),
                WallpaperMode::Fill,
            ),
        ),
        (
            "loading",
            boot(
                tokens(Appearance::Dark, 1.0),
                localizer("en-US"),
                Selection::Image(PathBuf::from("/walls/wall.png")),
                WallpaperMode::Fill,
                None,
            ),
        ),
    ];

    for (name, state) in scenarios {
        let (projection, document) = projected(&state);

        assert_eq!(
            document["accessibleItems"],
            counted_accessible_items(&document),
            "{name}: the projected count and the drawn lines disagree"
        );
        assert_eq!(
            projection.accessible_items(),
            counted_accessible_items(&document),
            "{name}: the readiness report is checked against a different number"
        );
        assert!(projection.accessible_items() > 0, "{name}");
    }
}

#[test]
fn the_retry_action_projects_the_id_the_state_machine_parses() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let state = requested(
        boot(
            tokens(Appearance::Dark, 1.0),
            localizer("en-US"),
            Selection::Image(dir.path().join("gone.png")),
            WallpaperMode::Fill,
            None,
        ),
        WallpaperMode::Fill,
    );
    let (_, document) = projected(&state);
    let id = document["notice"]["action"]["id"]
        .as_str()
        .expect("the control carries an action id");

    assert!(
        matches!(
            view::parse_action(&format!(r#"{{"type":"{id}"}}"#)),
            Some(Message::Reload)
        ),
        "`{id}` is not an action the state machine accepts"
    );
}

#[test]
fn a_two_hundred_percent_font_scale_grows_every_projected_line() {
    let normal = boot(
        tokens(Appearance::Dark, 1.0),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let large = boot(
        tokens(Appearance::Dark, 2.0),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let (_, normal) = projected(&normal);
    let (_, large) = projected(&large);

    assert_eq!(normal["tokens"]["font"]["size"]["title"], 24);
    assert_eq!(large["tokens"]["font"]["size"]["title"], 48);
    assert_eq!(normal["card"]["lines"][0]["size"], 24);
    assert_eq!(large["card"]["lines"][0]["size"], 48);
    let normal_height = normal["tokens"]["sizes"]["controlHeight"]
        .as_f64()
        .expect("a projected control height");
    let large_height = large["tokens"]["sizes"]["controlHeight"]
        .as_f64()
        .expect("a projected control height");
    assert!(
        large_height > normal_height,
        "a control grows with the text instead of clipping it"
    );
}

#[test]
fn the_two_appearances_project_different_drawings() {
    let light = boot(
        tokens(Appearance::Light, 1.0),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let dark = boot(
        tokens(Appearance::Dark, 1.0),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let (_, light) = projected(&light);
    let (_, dark) = projected(&dark);

    assert_ne!(
        light["tokens"]["colors"]["background"],
        dark["tokens"]["colors"]["background"]
    );
    assert_ne!(
        light["tokens"]["card"]["background"],
        dark["tokens"]["card"]["background"]
    );
    assert_ne!(
        light["tokens"]["colors"]["text"],
        dark["tokens"]["colors"]["text"]
    );
}

#[test]
fn the_same_state_projects_the_same_document() {
    let state = boot(
        tokens(Appearance::Dark, 1.0),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );

    let (first, _) = projected(&state);
    let (second, _) = projected(&state);

    assert_eq!(first.json(), second.json());
}

#[test]
fn the_view_model_keeps_its_own_projection() {
    // The JSON document is built from this projection, so the strings the UI
    // tests above assert are the strings the state machine decided on.
    let state = boot(
        tokens(Appearance::Dark, 1.0),
        localizer("en-US"),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let model = view_model::project(&state);

    assert_eq!(model.title, "Inox wallpaper");
    assert!(model.wallpaper.is_none());
    assert_eq!(model.card.heading, "Output HDMI-A-1");
    assert_eq!(model.card.scaling, "Scaling: fill");
}

#[test]
fn the_two_catalogs_define_the_same_messages_with_the_same_placeholders() {
    let diff = inox_common::i18n::compare_catalogs(&[i18n::EN_US], &[i18n::ZH_CN])
        .expect("both catalogs parse");

    assert!(
        diff.is_clean(),
        "the catalogs differ: missing {:?}, extra {:?}, placeholders {:?}",
        diff.missing_ids,
        diff.extra_ids,
        diff.placeholder_mismatches
    );
}

#[test]
fn the_list_the_code_formats_from_is_exactly_what_the_catalogs_define() {
    let mut defined: Vec<String> = inox_common::i18n::message_ids(&[i18n::EN_US])
        .expect("the default catalog parses")
        .into_iter()
        .collect();
    defined.sort();

    let mut listed: Vec<String> = MESSAGES.iter().map(|id| (*id).to_owned()).collect();
    listed.sort();

    assert_eq!(
        listed, defined,
        "every id the component formats must exist, and no catalog entry may be dead"
    );
}

#[test]
fn both_locales_load_and_name_the_window() {
    for tag in ["en-US", "zh-CN", "de-DE", "zh-Hans"] {
        let locale = Locale::parse(tag).expect("a well-formed tag");
        let localizer = i18n::localizer(locale).expect("the shipped catalogs");
        let title = localizer.message("wallpaper-window-title");

        assert!(!title.is_empty());
        assert_ne!(title, "wallpaper-window-title", "`{tag}` renders a title");
    }
}

#[test]
fn the_chinese_catalog_is_actually_chinese() {
    let localizer =
        i18n::localizer(Locale::parse(CHINESE_TAG).expect("a tag")).expect("the shipped catalogs");
    let title = localizer.message("wallpaper-window-title");

    assert!(
        !title.is_ascii(),
        "`{title}` should not be the English string"
    );
}

/// Spawn this test binary as a process that behaves like `main`.
///
/// The binary target itself is not built by `cargo test`, and an integration
/// test cannot link the cxx-qt bridge, so the child re-runs [`crate::run`] and
/// translates its result into the exit status `main` would produce. That keeps
/// the process-level contract — non-zero status plus a readable message —
/// observable from a unit test.
fn run_as_process(arguments: &[&str]) -> std::process::Output {
    Command::new(std::env::current_exe().expect("the test binary path"))
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "headless_tests::cli_child",
        ])
        .env(CLI_CHILD_MARKER, "1")
        .env(CLI_CHILD_ARGS, arguments.join("\u{1f}"))
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("the child test process runs")
}

/// The child half of [`run_as_process`].
#[test]
#[ignore = "spawned as a child process by the command-line tests"]
fn cli_child() {
    assert_eq!(
        std::env::var(CLI_CHILD_MARKER).as_deref(),
        Ok("1"),
        "this test only runs as the spawned child"
    );
    crate::init_logging();

    let raw = std::env::var(CLI_CHILD_ARGS).unwrap_or_default();
    let arguments: Vec<String> = if raw.is_empty() {
        Vec::new()
    } else {
        raw.split('\u{1f}').map(str::to_owned).collect()
    };

    match crate::run(arguments.into_iter()) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            // The same shape `main`'s `anyhow::Result` produces.
            eprintln!("Error: {error:#}");
            std::process::exit(1);
        }
    }
}

#[test]
fn the_command_line_keeps_its_exit_codes() {
    let help = run_as_process(&["--help"]);
    let text =
        String::from_utf8_lossy(&help.stdout).to_string() + &String::from_utf8_lossy(&help.stderr);
    assert!(help.status.success(), "--help must succeed: {text}");
    assert!(text.contains("usage: inox-wallpaper"), "{text}");

    let bogus = run_as_process(&["--bogus"]);
    let text = String::from_utf8_lossy(&bogus.stdout).to_string()
        + &String::from_utf8_lossy(&bogus.stderr);
    assert!(!bogus.status.success(), "an unknown argument must fail");
    assert!(text.contains("unrecognized argument: --bogus"), "{text}");
}

#[test]
fn the_arguments_are_parsed_without_a_display() {
    // `run` returns before Qt for both of these, which is what makes the
    // argument handling testable in-process.
    crate::run([String::from("--help")].into_iter()).expect("help succeeds");

    let error = crate::run([String::from("--bogus")].into_iter())
        .expect_err("an unknown argument is an error, not a silent fallback");
    assert!(
        format!("{error:#}").contains("unrecognized argument"),
        "{error:#}"
    );
}

/// The child half of [`the_qml_preview_loads_offscreen`].
///
/// Ignored so a normal `cargo test` does not run it in-process; the parent
/// spawns the test binary with `QT_QPA_PLATFORM=offscreen` already set, which
/// is the only way to select a Qt platform without mutating the environment.
#[test]
#[ignore = "spawned as a child process by the_qml_preview_loads_offscreen"]
fn qml_preview_child_self_check() {
    assert_eq!(
        std::env::var(CHILD_MARKER).as_deref(),
        Ok("1"),
        "this test only runs as the spawned child"
    );
    crate::init_logging();

    crate::run([String::from("--self-check")].into_iter())
        .expect("the QML module loads and the tree reports a matching projection");
    assert!(
        crate::cxxqt_object::qml_ready(),
        "the QML tree must confirm it read the Rust projection"
    );
    // The parent cannot see this process's tracing subscriber, so the success
    // marker goes to stdout. `println!` is allowed in tests (root AGENTS.md §3.5).
    println!("{SELF_CHECK_MARKER}");
}

#[test]
fn the_qml_preview_loads_offscreen() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let image = fixture(directory.path(), 64, 64);
    let config_home = directory.path().join("config");
    let inox_config = config_home.join("inox");
    std::fs::create_dir_all(&inox_config).expect("the fixture configuration directory");
    std::fs::write(
        inox_config.join("shell.toml"),
        format!(
            "[wallpaper]\nmode = \"fill\"\nimage = \"{}\"\n",
            image.display()
        ),
    )
    .expect("the fixture configuration");

    let output = Command::new(std::env::current_exe().expect("the test binary path"))
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "headless_tests::qml_preview_child_self_check",
        ])
        .env(CHILD_MARKER, "1")
        .env("QT_QPA_PLATFORM", "offscreen")
        .env("QT_QUICK_BACKEND", "software")
        .env("XDG_CONFIG_HOME", &config_home)
        .env("HOME", directory.path())
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .env_remove("INOX_WALLPAPER_DIR")
        .env_remove("INOX_THEME")
        .output()
        .expect("the child test process runs");

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "the offscreen preview self-check failed: {text}"
    );
    assert!(
        text.contains(SELF_CHECK_MARKER),
        "the child did not report a successful self-check: {text}"
    );
    // Qt reports QML errors on stderr even when the process survives.
    for marker in [
        "is not a type",
        "Cannot assign",
        "TypeError",
        "ReferenceError",
        "Unable to assign",
    ] {
        assert!(!text.contains(marker), "QML reported an error: {text}");
    }
}
