//! Headless evidence for the migrated surface (D06, Q03, and the window leg of M03).
//!
//! Everything lives inside the binary. cxx-qt 0.10 embeds the bridge's generated
//! initializers in the target that declares the bridge and links its generated
//! C++ into every target of the package, so an integration test in `tests/` — a
//! separate target — fails to link (`undefined reference to 'cxx_qt_init_*'`,
//! KDAB/cxx-qt #770, docs/qml-migration-contract.md §2.1). The binary is
//! therefore the only place this evidence can live.
//!
//! Three kinds of evidence:
//!
//!   * **the catalogues** (`en-US` against `zh-CN`, and the code against both),
//!     which is acceptance Q03 and is what makes the projection's strings
//!     trustworthy in either language;
//!   * **the projection through the bridge**, including the Chinese one, because
//!     the QML surface renders whatever `modelJson` says and nothing else;
//!   * **the surface actually loading on the offscreen platform**, plus the
//!     executable's command line and exit codes. Both need a process whose
//!     environment is set *before* Qt starts, so the tests spawn this binary as
//!     that child: mutating this process's environment would be `unsafe` in
//!     edition 2024, which Inox code does not write.
//!
//! Run with:
//!
//! ```sh
//! QT_QPA_PLATFORM=offscreen cargo test -p inox-notifications
//! ```

use std::process::Command;
use std::rc::Rc;

use inox_common::i18n::{DEFAULT_LOCALE, Locale, Localizer, compare_catalogs, message_ids};
use inox_config::{NotificationsConfig, Theme};

use crate::app::{Inbox, State};
use crate::cxxqt_object::NotificationsRust;
use crate::i18n::{EN_US, MESSAGES, ZH_CN};
use crate::runtime::Runtime;

/// The tag of the second locale the daemon ships.
const TRANSLATED: &str = "zh-CN";

/// Environment variable the parent test sets on the QML child it spawns.
const QML_CHILD_MARKER: &str = "INOX_NOTIFICATIONS_QML_CHILD";

/// Line the QML child prints once the surface has loaded and reported ready.
const SELF_CHECK_MARKER: &str = "INOX_NOTIFICATIONS_QML_SELF_CHECK_OK";

/// Environment variable that turns the test binary into a command-line child.
const CLI_CHILD_MARKER: &str = "INOX_NOTIFICATIONS_CLI_CHILD";

/// Environment variable carrying the child's arguments, separated by `\u{1f}`.
const CLI_CHILD_ARGS: &str = "INOX_NOTIFICATIONS_CLI_ARGS";

fn locale(tag: &str) -> Locale {
    Locale::parse(tag).expect("a well-formed language tag")
}

fn localizer(tag: &str) -> Localizer {
    crate::i18n::load(&locale(tag)).expect("the catalogs parse")
}

fn daemon(tag: &str) -> State {
    State::new(
        NotificationsConfig::default(),
        Theme::builtin_dark(),
        Rc::new(localizer(tag)),
    )
}

// --- the catalogues (Q03) ---------------------------------------------------

#[test]
fn both_locales_define_the_same_messages_with_the_same_placeholders() {
    let difference = compare_catalogs(EN_US, ZH_CN).expect("both catalogs parse");

    assert!(
        difference.is_clean(),
        "`{TRANSLATED}` is out of step with `{DEFAULT_LOCALE}`: {difference:?}"
    );
}

#[test]
fn every_message_is_translated_rather_than_falling_back() {
    let ids = message_ids(EN_US).expect("the default catalog parses");
    assert!(!ids.is_empty(), "the daemon ships messages");

    let chinese = localizer(TRANSLATED);

    for id in ids {
        assert!(
            chinese.is_translated(&id),
            "`{id}` is only defined for `{DEFAULT_LOCALE}`"
        );
    }
}

#[test]
fn the_code_and_the_catalogs_name_the_same_messages() {
    let defined = message_ids(EN_US).expect("the default catalog parses");
    let named: std::collections::BTreeSet<String> =
        MESSAGES.iter().map(|id| (*id).to_owned()).collect();

    assert_eq!(
        defined, named,
        "a message the code names and the catalogs do not, or the other way \
         round, is a message id on screen or a string nobody can reach"
    );
}

#[test]
fn the_default_locale_is_english() {
    assert_eq!(Locale::default_locale().tag(), DEFAULT_LOCALE);
    assert_eq!(DEFAULT_LOCALE, "en-US");
}

#[test]
fn ids_follow_the_component_convention() {
    let ids = message_ids(EN_US).expect("the default catalog parses");

    for id in &ids {
        assert!(
            id.starts_with("notifications-"),
            "`{id}` is not named after the component"
        );
        assert!(
            !id.contains('.'),
            "`{id}` carries a dot, which only an attribute may use"
        );
        assert!(
            id.chars().all(|character| character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'),
            "`{id}` uses characters Fluent identifiers do not allow"
        );
    }
}

#[test]
fn a_locale_the_daemon_does_not_ship_still_gets_a_daemon() {
    let french = localizer("fr-FR");

    assert_eq!(french.locale().tag(), DEFAULT_LOCALE);
    assert_eq!(
        french.message("notifications-empty"),
        "No notifications",
        "a missing translation falls back to the default language (Q03)"
    );
}

#[test]
fn no_message_is_left_empty() {
    for tag in [DEFAULT_LOCALE, TRANSLATED] {
        let localizer = localizer(tag);
        let sources = if tag == DEFAULT_LOCALE { EN_US } else { ZH_CN };
        let ids = message_ids(sources).expect("the catalog parses");

        for id in ids {
            assert!(
                !localizer.message(&id).trim().is_empty(),
                "`{id}` renders nothing in `{tag}`"
            );
        }
    }
}

#[test]
fn the_overflow_sentence_carries_the_number_in_both_locales() {
    for tag in [DEFAULT_LOCALE, TRANSLATED] {
        let localizer = localizer(tag);
        let sentence = localizer.message_with("notifications-overflow", &[("count", 7i32.into())]);

        assert!(
            sentence.contains('7'),
            "the count is in the `{tag}` sentence: {sentence}"
        );
        assert!(
            !sentence.contains('$'),
            "the placeholder was substituted, not left standing: {sentence}"
        );
    }
}

#[test]
fn the_two_locales_say_the_muted_line_differently() {
    let english = localizer(DEFAULT_LOCALE).message("notifications-suppressed");
    let chinese = localizer(TRANSLATED).message("notifications-suppressed");

    assert_ne!(english, chinese, "the second locale is a translation");
    assert!(!english.is_empty());
    assert!(!chinese.is_empty());
}

// --- the projection through the bridge --------------------------------------

/// The JSON a fresh bridge object hands to QML.
fn bridge_json(state: State) -> serde_json::Value {
    let inbox = Inbox::new();
    let runtime = Runtime::new(state, &inbox);

    assert!(
        crate::cxxqt_object::install(runtime),
        "the daemon is installed once per process"
    );

    let object = NotificationsRust::projected();
    let raw = String::from(&object.model_json);

    serde_json::from_str(&raw).expect("the bridge property is valid JSON")
}

#[test]
fn the_bridge_starts_from_the_projection_the_daemon_computed() {
    let value = bridge_json(daemon(DEFAULT_LOCALE));

    assert_eq!(value["window"]["title"], "Inox Notifications");
    assert_eq!(value["stack"]["status"], "empty");
    assert!(value["palette"]["background"].is_string());
    assert!(value["metrics"]["width"].as_f64().is_some_and(|w| w > 0.0));
}

#[test]
fn the_chinese_projection_reaches_the_bridge() {
    let value = bridge_json(daemon(TRANSLATED));

    assert_eq!(value["window"]["title"], "Inox 通知");
    assert_eq!(value["stack"]["notice"]["text"], "没有通知");
}

// --- the executable's command line and exit codes ---------------------------

/// Spawn this test binary as a process that behaves like `main`.
///
/// The binary target itself is not built by `cargo test`, and an integration test
/// cannot link the cxx-qt bridge, so the child re-runs [`crate::run`] and
/// translates its result into the exit status `main` would produce. That keeps the
/// process-level contract — zero for a good command line, non-zero with a readable
/// message for a bad one — observable from inside the package.
fn run_as_process(arguments: &[&str]) -> std::process::Output {
    let home = tempfile::tempdir().expect("a temporary home");

    Command::new(std::env::current_exe().expect("the test binary path"))
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "headless_tests::cli_child",
        ])
        .env(CLI_CHILD_MARKER, "1")
        .env(CLI_CHILD_ARGS, arguments.join("\u{1f}"))
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
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
fn help_succeeds_without_a_display() {
    let help = run_as_process(&["--help"]);
    assert!(
        help.status.success(),
        "--help must succeed without a display: {}",
        String::from_utf8_lossy(&help.stderr)
    );
}

#[test]
fn starting_without_a_display_is_refused_with_a_readable_message() {
    // The daemon is started by the session, so no arguments means "run". The L3 component architecture
    // requires that case to fail with a message a user can act on rather than
    // reaching Qt and dying there.
    let start = run_as_process(&[]);

    assert!(
        !start.status.success(),
        "no display is a reason not to start"
    );

    let text = String::from_utf8_lossy(&start.stdout).to_string()
        + &String::from_utf8_lossy(&start.stderr);

    assert!(
        text.contains("no display to draw notifications on"),
        "the refusal says what is missing: {text}"
    );
}

#[test]
fn an_unknown_argument_exits_non_zero_with_a_readable_message() {
    let bogus = run_as_process(&["--bogus"]);

    assert!(!bogus.status.success(), "an unknown argument must fail");

    let text = String::from_utf8_lossy(&bogus.stdout).to_string()
        + &String::from_utf8_lossy(&bogus.stderr);

    assert!(
        text.contains("--bogus"),
        "the refusal names what it refused: {text}"
    );
}

// --- the QML surface --------------------------------------------------------

/// The child half of [`the_qml_surface_loads_offscreen`].
///
/// Ignored so a normal `cargo test` does not run it in-process; the parent spawns
/// the test binary with `QT_QPA_PLATFORM=offscreen` already set, which is the only
/// way to select a Qt platform without mutating the environment.
#[test]
#[ignore = "spawned as a child process by the_qml_surface_loads_offscreen"]
fn qml_child_self_check() {
    assert_eq!(
        std::env::var(QML_CHILD_MARKER).as_deref(),
        Ok("1"),
        "this test only runs as the spawned child"
    );

    // The real startup path, minus the bus: `--self-check` loads the configuration
    // (a temporary home, so the defaults are in force), installs the daemon and
    // then loads the QML module, waiting for the tree to confirm the projection.
    crate::run(["--self-check".to_owned()].into_iter())
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
fn the_qml_surface_loads_offscreen() {
    let home = tempfile::tempdir().expect("a temporary home");
    let binary = std::env::current_exe().expect("the test binary path");

    let output = Command::new(binary)
        .args([
            "--ignored",
            "--exact",
            "--nocapture",
            "headless_tests::qml_child_self_check",
        ])
        .env(QML_CHILD_MARKER, "1")
        .env("QT_QPA_PLATFORM", "offscreen")
        // The offscreen platform has no GPU, so the scene graph draws in software.
        .env("QT_QUICK_BACKEND", "software")
        // Qt suppresses its own messages when stderr is a pipe (which it is here),
        // and a QML type error is the one thing that explains a failed load; force
        // them out so the assertion below carries the reason.
        .env("QT_ASSUME_STDERR_HAS_CONSOLE", "1")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("XDG_CACHE_HOME", home.path().join("cache"))
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .output()
        .expect("the child test process runs");

    let text = String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "the offscreen surface self-check failed: {text}"
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
        "Unable to assign",
        "ReferenceError",
    ] {
        assert!(!text.contains(marker), "QML reported an error: {text}");
    }
}
