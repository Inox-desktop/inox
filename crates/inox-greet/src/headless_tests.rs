//! Headless tests of the preview (ADR 0036 M03 evidence).
//!
//! Everything lives inside the binary because cxx-qt 0.10 links the bridge's
//! generated C++ archive into every target of the package: an integration test
//! in `tests/` is a separate target and fails to link (`undefined reference to
//! 'cxxbridge1$…GreetPreview…'`), for the same reason a library target would
//! (KDAB/cxx-qt #770). The binary is therefore the only place that can hold
//! evidence.
//!
//! Three kinds of evidence:
//!
//!   * the Rust projection QML reads (properties, colours, tokens, strings);
//!   * the process-level refusal to log in (ADR 0015), through the same
//!     argument handling `main` uses;
//!   * the QML module actually loading on the offscreen platform, which needs a
//!     process whose environment sets `QT_QPA_PLATFORM` *before* Qt starts. The
//!     test binary spawns itself as that child: mutating this process's
//!     environment would be `unsafe` in edition 2024, which Inox code may not
//!     write.
//!
//! Run with:
//!
//! ```sh
//! cargo test -p inox-greet --bin inox-greet
//! ```

use std::process::Command;

use inox_common::i18n::Locale;
use inox_config::Theme;

use crate::app::{Action, Notice};
use crate::cxxqt_object::GreetPreviewRust;
use crate::view::View;

/// Environment variable the parent test sets on the child it spawns.
const CHILD_MARKER: &str = "INOX_GREET_QML_CHILD";

/// Line the child prints once the QML tree has loaded and reported ready.
const SELF_CHECK_MARKER: &str = "INOX_GREET_QML_SELF_CHECK_OK";

/// Environment variable that turns the test binary into a command-line child.
const CLI_CHILD_MARKER: &str = "INOX_GREET_CLI_CHILD";

/// Environment variable carrying the child's arguments, separated by `\u{1f}`.
const CLI_CHILD_ARGS: &str = "INOX_GREET_CLI_ARGS";

fn view(tag: &str) -> View {
    let locale = Locale::parse(tag).expect("locale");
    let localizer = crate::i18n::load(locale).expect("catalog");
    View::new(Theme::builtin_dark(), &localizer)
}

#[test]
fn the_backing_struct_projects_the_theme_and_the_catalog() {
    let view = view("en-US");
    let preview = GreetPreviewRust::from_view(view.clone());

    assert_eq!(String::from(&preview.title), view.strings.title);
    assert_eq!(String::from(&preview.preview_notice), view.strings.preview);
    assert_eq!(String::from(&preview.user_label), view.strings.user);
    assert_eq!(String::from(&preview.login_label), view.strings.login);
    assert_eq!(String::from(&preview.clear_label), view.strings.clear);
    assert_eq!(String::from(&preview.notice), view.strings.blocked);

    // The theme tokens arrive as values QML can bind to, not as literals.
    let colors = view.theme.colors;
    assert_eq!(preview.background.red(), i32::from(colors.background.r));
    assert_eq!(preview.background.green(), i32::from(colors.background.g));
    assert_eq!(preview.background.blue(), i32::from(colors.background.b));
    assert_eq!(preview.background.alpha(), i32::from(colors.background.a));
    assert_eq!(preview.accent.red(), i32::from(colors.accent.r));
    assert_eq!(preview.accent_text.red(), i32::from(colors.on_accent.r));
    assert_eq!(preview.warning.red(), i32::from(colors.warning.r));
    assert_eq!(preview.danger.red(), i32::from(colors.danger.r));

    assert_eq!(preview.font_size, view.font_px());
    assert_eq!(preview.spacing_sm, view.spacing_sm());
    assert_eq!(preview.spacing_md, view.spacing_md());
    assert_eq!(preview.spacing_lg, view.spacing_lg());
    assert_eq!(preview.radius_md, view.radius_md());
    assert_eq!(preview.window_width, crate::view::WINDOW_WIDTH);
    assert_eq!(preview.window_height, crate::view::WINDOW_HEIGHT);
}

#[test]
fn the_language_reaches_the_projected_strings() {
    let zh = GreetPreviewRust::from_view(view("zh-CN"));
    assert_eq!(String::from(&zh.title), "Inox 登录预览");
    assert_eq!(String::from(&zh.clear_label), "清空");
}

#[test]
fn the_live_properties_start_from_the_state_machine() {
    let preview = GreetPreviewRust::from_view(view("en-US"));
    assert!(preview.username.is_empty());
    assert!(
        !preview.can_login,
        "login must be disabled without a transport"
    );
    assert_eq!(String::from(&preview.notice), view("en-US").strings.blocked);
}

#[test]
fn the_default_constructor_is_total_without_an_installed_projection() {
    // The QML engine constructs the element from the module, which can happen
    // before the binary installs a projection; the default must not panic.
    let preview = GreetPreviewRust::default();
    assert!(!preview.can_login);
    assert_eq!(preview.font_size, 16);
    assert!(preview.window_width > 0);
}

#[test]
fn the_notice_follows_the_actions_the_surface_reports() {
    // The bridge invokables apply these exact actions; an invalid name has to
    // resolve to the invalid-user message, not the blocked one.
    let mut preview = crate::app::Preview::new();
    let now = std::time::Instant::now();
    preview.apply(Action::EditUsername("alice".into()), now);
    assert_eq!(preview.notice(), Notice::Blocked);
    preview.apply(Action::EditUsername("alice;id".into()), now);
    assert_eq!(preview.notice(), Notice::InvalidUser);
    assert_eq!(preview.notice().message_id(), "invalid-user");
    assert!(view("en-US").strings.invalid_user.contains("valid"));
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
fn the_process_refuses_a_real_login_and_explains_why() {
    let output = run_as_process(&[]);
    assert!(!output.status.success(), "a real login must exit non-zero");
    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);
    assert!(
        text.contains("ADR 0015"),
        "the refusal must be readable: {text}"
    );

    let help = run_as_process(&["--help"]);
    assert!(
        help.status.success(),
        "--help must succeed without a display"
    );

    let bogus = run_as_process(&["--bogus"]);
    assert!(!bogus.status.success(), "an unknown argument must fail");
}

#[test]
fn real_login_is_refused_and_help_needs_no_display() {
    // `main` maps the `Err` returned here to a non-zero exit status, so this is
    // the process-level refusal: without `--preview` the greeter never reaches
    // Qt, and the message names the security review that is missing.
    let refused = crate::run(std::iter::empty());
    let error = refused.expect_err("the greeter must refuse a real login");
    assert!(
        format!("{error:#}").contains("ADR 0015"),
        "the refusal must be readable: {error:#}"
    );

    // `--help` succeeds and prints the usage line, still without a display.
    crate::run(["--help".to_owned()].into_iter()).expect("help succeeds");

    // An unknown argument is an error, not a silent fallback.
    assert!(crate::run(["--bogus".to_owned()].into_iter()).is_err());

    // `--self-check` cannot be used to reach a login: it is a diagnostic of the
    // preview only.
    assert!(crate::run(["--self-check".to_owned()].into_iter()).is_err());
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
    let view = view("en-US");
    assert!(
        View::install(view),
        "the child installs the projection once"
    );
    crate::run_preview(crate::Options {
        preview: true,
        self_check: true,
    })
    .expect("the QML module loads and the tree reports ready");
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
    let binary = std::env::current_exe().expect("the test binary path");
    let output = Command::new(binary)
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "headless_tests::qml_preview_child_self_check",
        ])
        .env(CHILD_MARKER, "1")
        .env("QT_QPA_PLATFORM", "offscreen")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
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
    for marker in ["is not a type", "Cannot assign", "TypeError"] {
        assert!(!text.contains(marker), "QML reported an error: {text}");
    }
}
