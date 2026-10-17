//! The user task of acceptance D04, end to end and on real files.
//!
//! Every test here runs the whole path a user runs: the window reads the
//! configuration, the draft holds the edit, the update loop queues the writes,
//! the runtime performs them with [`inox_settings::persist`], and the outcome
//! comes back as a message. Nothing is stubbed, and the files are real files in
//! a real directory — which is what makes the promise ADR 0014 asks for
//! checkable rather than asserted:
//!
//! - the saved state and the in-effect state are different sentences (D04);
//! - cancelling writes nothing at all;
//! - an error names the field, and the old running state survives;
//! - after a restart the file is what was saved;
//! - a key the user never touched is never written back.
//!
//! The one property test is the algorithmic core: **whatever** a user does to
//! the window, applying the draft and reading the files back must give exactly
//! the values the window was showing. A draft the validator refuses must reach
//! no file at all.

mod support;

use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_settings::app::Message;
use inox_settings::apply::ApplyState;
use inox_settings::draft::{Field, Settings, Value};
use inox_settings::pages::PageId;
use inox_settings::persist::SaveOutcome;
use inox_settings::validate::validate;
use inox_settings::view_model::StatusKind;
use proptest::prelude::*;
use support::Window;

/// A shell file with a comment, a key the window edits and one it does not.
const SHELL: &str = "\
# the strip along the bottom edge
[panel]
height = 32
# the panel's own cap, which this window never touches
max_task_buttons = 12
autohide = false
";

/// A window over a directory holding `files`.
fn window(files: &[(&str, &str)]) -> Window {
    Window::with_files(files)
}

/// Whether `inox-config` accepts a `shell.toml` holding that panel height.
fn accepts_height(height: i64) -> bool {
    let directory = tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        directory.path().join("shell.toml"),
        format!("[panel]\nheight = {height}\n"),
    )
    .expect("the file is written");

    InoxConfig::load(&ConfigPaths::new(directory.path()), Appearance::System).is_ok()
}

/// Whether `inox-config` accepts a `notifications.toml` holding that timeout.
fn accepts_timeout(milliseconds: i64) -> bool {
    let directory = tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        directory.path().join("notifications.toml"),
        format!("[timeouts]\nnormal_ms = {milliseconds}\n"),
    )
    .expect("the file is written");

    InoxConfig::load(&ConfigPaths::new(directory.path()), Appearance::System).is_ok()
}

#[test]
fn cancelling_writes_nothing_and_leaves_the_file_byte_for_byte() {
    let mut window = window(&[("shell.toml", SHELL)]);
    let before = window.file("shell.toml");

    window.update(Message::PageSelected(PageId::Panel));
    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(64),
    });
    window.update(Message::FieldSet {
        field: Field::PanelAutohide,
        value: Value::Flag(true),
    });

    assert!(
        window.state().draft().is_modified(),
        "the window holds a change"
    );

    window.update(Message::Cancel);

    assert!(
        window.state_mut().take_outgoing().is_empty(),
        "Cancel queues nothing at all"
    );
    assert!(!window.state().draft().is_modified());
    assert_eq!(
        window.file("shell.toml"),
        before,
        "the file is byte for byte what it was"
    );
}

#[test]
fn applying_writes_only_the_keys_the_user_changed() {
    let mut window = window(&[("shell.toml", SHELL)]);

    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(48),
    });

    assert_eq!(window.apply_and_drain(), 1);

    let written = String::from_utf8(window.file("shell.toml")).expect("the file is text");

    assert!(written.contains("height = 48"), "written:\n{written}");
    assert!(
        written.contains("# the strip along the bottom edge"),
        "a comment this window never touched survives:\n{written}"
    );
    assert!(
        written.contains("max_task_buttons = 12"),
        "a key this window never touched is not written back:\n{written}"
    );
    assert!(
        written.contains("autohide = false"),
        "a key the user did not change keeps the value the file had:\n{written}"
    );
}

#[test]
fn a_save_over_a_hand_written_file_keeps_it_readable() {
    // The shapes a hand-written file can spell the same key in. A save has to
    // leave the file holding the values the window shows **and** able to be read
    // back: a key written a second time is a configuration that refuses to parse,
    // which is a session that cannot start at all.
    let shapes = [
        ("a dotted key at the top level", "panel.height = 12\n"),
        (
            "a dotted key with a table below it",
            "panel.height = 12\n\n[launcher]\nmax_results = 4\n",
        ),
        ("a quoted key", "[panel]\n\"height\" = 12\n"),
        ("an inline table", "panel = { height = 12 }\n"),
        ("an empty inline table", "panel = { }\n"),
        ("a quoted table header", "[\"panel\"]\nheight = 12\n"),
        (
            "a comment on the line",
            "[panel]\nheight = 12 # hand-written\n",
        ),
    ];

    for (shape, file) in shapes {
        let mut window = window(&[("shell.toml", file)]);

        window.update(Message::FieldSet {
            field: Field::PanelHeight,
            value: Value::Number(48),
        });
        window.update(Message::FieldSet {
            field: Field::PanelShowClock,
            value: Value::Flag(false),
        });

        assert_eq!(window.apply_and_drain(), 1, "{shape}");

        let written = String::from_utf8(window.file("shell.toml")).expect("the file is text");
        let reloaded = match InoxConfig::load(window.paths(), Appearance::System) {
            Ok(config) => config,
            Err(error) => panic!("{shape}: the saved file no longer parses: {error}\n{written}"),
        };
        let settings = Settings::from_config(&reloaded);

        assert_eq!(reloaded.shell.panel.height, 48, "{shape}:\n{written}");
        assert!(!reloaded.shell.panel.show_clock, "{shape}:\n{written}");
        assert_eq!(settings.panel_height, 48, "{shape}:\n{written}");
        assert!(!settings.panel_show_clock, "{shape}:\n{written}");
    }
}

#[test]
fn the_window_keeps_claiming_nothing_until_the_readback_comes() {
    let mut window = window(&[("shell.toml", SHELL)]);
    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(48),
    });

    window.update(Message::Apply);
    assert_eq!(
        window.state().model().status.kind,
        StatusKind::Saving,
        "the file is not written yet"
    );

    window.drain();

    assert!(window.state().application().state().is_saved());
    assert!(
        !window.state().application().state().is_applied(),
        "nothing has confirmed it, so the window must not say it took effect"
    );
    assert_eq!(
        window.state().model().status.kind,
        StatusKind::SavedOnly,
        "and it says which of the two it is (ADR 0014, decision 4)"
    );
}

#[test]
fn a_restart_shows_what_was_saved() {
    let mut window = window(&[("shell.toml", SHELL)]);

    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(64),
    });
    window.update(Message::FieldSet {
        field: Field::NotificationsPosition,
        value: Value::Choice("bottom-left".to_owned()),
    });

    assert_eq!(window.apply_and_drain(), 2, "two files were written");

    let reloaded = window.reload_config();

    assert_eq!(reloaded.shell.panel.height, 64);
    assert_eq!(
        reloaded.notifications.display.position,
        inox_config::NotificationPosition::BottomLeft
    );
    assert_eq!(
        Settings::from_config(&reloaded).panel_height,
        64,
        "a window opened over the same directory shows what was saved"
    );
    assert!(
        !Settings::from_config(&reloaded).panel_autohide,
        "a key the user never touched keeps the value the file had"
    );
}

#[test]
fn an_external_edit_is_not_overwritten_and_is_reported() {
    let mut window = window(&[("shell.toml", SHELL)]);
    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(48),
    });
    window.update(Message::Apply);

    // Somebody else edits the file between the click and the write.
    let manual = "# edited by hand while the window was open\n[panel]\nheight = 12\n";
    std::fs::write(window.path("shell.toml"), manual).expect("the external edit");

    window.drain();

    assert_eq!(
        String::from_utf8(window.file("shell.toml")).expect("text"),
        manual,
        "the manual edit survives untouched"
    );
    match window.state().application().state() {
        ApplyState::Rejected { reason, .. } => {
            assert!(
                matches!(reason, inox_settings::apply::Rejection::Conflict { .. }),
                "expected a conflict, got {reason:?}"
            );
        }
        other => panic!("expected a rejection, got {other:?}"),
    }
    assert!(
        window.state().draft().is_modified(),
        "the user's change is still theirs"
    );
}

#[test]
fn a_conflict_offers_a_reload_which_takes_the_file_back() {
    let mut window = window(&[("shell.toml", SHELL)]);
    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(48),
    });
    window.update(Message::Apply);
    std::fs::write(window.path("shell.toml"), "[panel]\nheight = 12\n").expect("the edit");
    window.drain();

    assert!(
        window
            .state()
            .model()
            .actions
            .iter()
            .any(|action| action.action == inox_settings::view_model::Action::Reload),
        "the way out is offered"
    );

    window.update(Message::Reload);
    window.drain();

    assert_eq!(
        window.state().draft().base().panel_height,
        12,
        "the file the user edited by hand is the new baseline"
    );
    assert_eq!(
        window.state().draft().settings().panel_height,
        48,
        "and the change they made in this window is still theirs"
    );
    assert!(window.state().draft().is_modified());
    assert_eq!(window.state().application().state(), &ApplyState::Draft);
}

#[test]
fn a_save_that_cannot_happen_names_the_file_and_keeps_the_old_bytes() {
    let mut window = window(&[("shell.toml", SHELL)]);
    let before = window.file("shell.toml");
    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(48),
    });
    window.update(Message::Apply);

    let generation = match window.state_mut().take_outgoing().first() {
        Some(inox_settings::app::OutgoingCommand::Save(request)) => request.generation,
        other => panic!("expected a write, got {other:?}"),
    };

    window.update(Message::SaveSettled {
        generation,
        file: inox_settings::document::SettingFile::Shell,
        outcome: SaveOutcome::Failed {
            reason: "read-only file system".to_owned(),
        },
    });

    assert_eq!(window.file("shell.toml"), before, "the old bytes survive");
    assert_eq!(
        window.state().model().status.kind,
        StatusKind::Rejected,
        "and the window says so rather than staying silent"
    );
    assert!(window.state().draft().is_modified());
}

#[test]
fn a_field_the_schema_refuses_reaches_no_file_and_names_itself() {
    let mut window = window(&[("shell.toml", SHELL)]);
    let before = window.file("shell.toml");

    window.update(Message::PageSelected(PageId::Panel));
    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(9_000),
    });
    window.update(Message::Apply);

    assert!(
        window.state_mut().take_outgoing().is_empty(),
        "an invalid draft is never written"
    );
    assert_eq!(window.file("shell.toml"), before);

    let model = window.state().model();
    let field = model.field(Field::PanelHeight).expect("the height");
    let error = field.error.as_deref().expect("the value is out of range");

    assert!(error.contains("panel.height"), "sentence `{error}`");
    assert!(
        !model
            .action(inox_settings::view_model::Action::Apply)
            .expect("Apply")
            .enabled
    );
}

#[test]
fn the_ranges_the_window_offers_are_the_ones_inox_config_accepts() {
    let height = Field::PanelHeight
        .control()
        .range
        .expect("the height is a number");
    let timeout = Field::NotificationsNormalTimeout
        .control()
        .range
        .expect("a timeout is a number");

    for value in [height.min - 1, height.min, height.max, height.max + 1] {
        assert_eq!(
            accepts_height(value),
            height.contains(value),
            "panel.height = {value} is mirrored wrongly"
        );
    }

    for value in [timeout.min, timeout.max, timeout.max + 1, -1] {
        assert_eq!(
            accepts_timeout(value),
            timeout.contains(value),
            "timeouts.normal_ms = {value} is mirrored wrongly"
        );
    }
}

#[test]
fn a_change_to_the_top_level_file_asks_every_consumer() {
    let mut window = window(&[]);
    window.update(Message::FieldSet {
        field: Field::ThemeAppearance,
        value: Value::Choice("light".to_owned()),
    });

    window.update(Message::Apply);

    assert_eq!(
        window.state().application().files(),
        &std::collections::BTreeSet::from([inox_settings::document::SettingFile::Root]),
        "the top-level file is the one that changes"
    );
    assert!(
        window.state().application().outstanding().is_empty(),
        "the components are asked after the file is written, not before"
    );

    window.drain();

    assert!(
        matches!(
            window.state().application().state(),
            ApplyState::SavedOnly { .. }
        ),
        "the theme is session-wide, so every consumer is asked and none answers"
    );
}

/// One edit a user could make, as a message the window would receive.
///
/// The values deliberately include numbers outside the schema's range, so the
/// property covers the validator's rejection path as well as the happy one.
fn any_edit() -> impl Strategy<Value = (Field, Value)> {
    (
        0usize..Field::ALL.len(),
        0i64..=400,
        any::<bool>(),
        0usize..8,
    )
        .prop_map(|(index, number, flag, choice)| {
            let field = Field::ALL[index];

            let value = match field.control().kind {
                inox_settings::draft::ValueKind::Choice => {
                    let choices = field.control().choices;

                    Value::Choice(choices[choice % choices.len()].id.to_owned())
                }
                inox_settings::draft::ValueKind::Flag => Value::Flag(flag),
                inox_settings::draft::ValueKind::Number => Value::Number(match number % 4 {
                    0 => number,
                    1 => -number,
                    2 => number * 1_000,
                    _ => number % 300,
                }),
            };

            (field, value)
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Whatever a user does to the window, the files end up holding what the
    /// window showed.
    ///
    /// This is the property the whole component exists for, and it covers both
    /// directions: a draft the validator refuses must reach no file at all, and
    /// a draft it accepts must reload to exactly the values that were on screen
    /// — no more (a key the user never touched is never written) and no less (a
    /// change the user made is not dropped).
    #[test]
    fn applying_a_draft_and_reading_the_files_back_gives_the_window_its_values(
        edits in prop::collection::vec(any_edit(), 0..=10),
    ) {
        let mut window = Window::new();

        for (field, value) in edits {
            window.update(Message::FieldSet { field, value });
        }

        let draft = window.state().draft().clone();
        let expected = draft.settings().clone();
        let refused = validate(&expected);

        let written = window.apply_and_drain();

        if !refused.is_empty() {
            prop_assert_eq!(written, 0, "an invalid draft wrote {} file(s)", written);
            prop_assert!(window.state().draft().is_modified());
            prop_assert_eq!(window.state().application().state(), &ApplyState::Draft);
            return Ok(());
        }

        if draft.is_modified() {
            prop_assert!(written > 0, "a valid change wrote nothing");
        } else {
            prop_assert_eq!(written, 0, "nothing changed, so nothing is written");
        }

        let reloaded = window.reload_config();
        let after = Settings::from_config(&reloaded);

        prop_assert_eq!(after, expected);
    }
}
