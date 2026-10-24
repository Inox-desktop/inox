//! End-to-end regressions for the expanded settings editors and save boundaries.

mod support;

use inox_config::{Appearance, InoxConfig};
use inox_settings::app::{Message, OutgoingCommand};
use inox_settings::apply::{ApplyState, Generation};
use inox_settings::document::{Edit, SettingFile};
use inox_settings::draft::{Field, Settings, Value, ValueKind};
use inox_settings::pages::PageId;
use inox_settings::persist::{self, SaveOutcome};
use inox_settings::validate;
use inox_settings::view_model::{Action, FocusTarget};
use support::Window;

fn set(window: &mut Window, field: Field, value: Value) {
    window.update(Message::FieldSet { field, value });
}

#[test]
fn every_numeric_limit_agrees_with_the_authoritative_parser() {
    for field in Field::ALL {
        let Some(range) = field.control().range else {
            continue;
        };
        for number in [range.min - 1, range.min, range.max, range.max + 1] {
            let mut settings = Settings::defaults();
            let value = Value::Number(number);
            field
                .set(&mut settings, value.clone())
                .expect("numeric field");
            let edit = Edit::set(
                field,
                field.toml_value(&value).expect("number").expect("value"),
            );
            let source = inox_settings::document::patch("", std::slice::from_ref(&edit));
            let schema = validate::document(field.file(), &source, &[edit]);
            assert_eq!(
                validate::validate_field(field, &settings).is_none(),
                schema.is_ok(),
                "{}:{} = {number}: {schema:?}",
                field.file().name(),
                field.path()
            );
        }
    }
}

#[test]
fn all_scalar_controls_save_and_reload_the_value_the_window_shows() {
    for field in Field::ALL {
        let mut window = Window::new();
        let value = match field.control().kind {
            ValueKind::Number => Value::Number(field.control().range.expect("range").max),
            ValueKind::Choice => Value::Choice(field.control().choices[0].id.to_owned()),
            ValueKind::Flag => Value::Flag(!matches!(
                window.state().draft().field(field),
                Value::Flag(true)
            )),
            ValueKind::Text => Value::Text(if field == Field::PanelClockFormat {
                "%H:%M:%S".to_owned()
            } else {
                "a path with 空格 and \"quotes\"".to_owned()
            }),
            ValueKind::Collection => continue,
        };
        set(&mut window, field, value.clone());
        window.apply_and_drain();
        let loaded = Settings::from_config(&window.reload_config());
        assert_eq!(
            field.value(&loaded),
            value,
            "{}:{}",
            field.file().name(),
            field.path()
        );
    }
}

fn output(name: &str) -> Vec<String> {
    [name, "2560", "1440", "144", "1.5", "-1707", "0"]
        .map(str::to_owned)
        .to_vec()
}

#[test]
fn output_arrays_and_nested_positions_are_replaced_without_changing_sibling_bytes() {
    let sibling = "# keyboard remains byte-for-byte\n[input]\nkeyboard_layout = 'us'  # keep\n";
    for original in [
        format!(
            "[[outputs]]\nname = 'HDMI-A-1'\nwidth = 1920\n[outputs.position]\nx = 0\ny = 0\n{sibling}"
        ),
        format!("[[\"outputs\"]]\nname = 'HDMI-A-1'\nposition = {{ x = 0, y = 0 }}\n{sibling}"),
        format!("outputs = [\n  {{ name = 'HDMI-A-1', width = 1920 }},\n]\n{sibling}"),
    ] {
        let mut window = Window::with_files(&[("compositor.toml", &original)]);
        set(
            &mut window,
            Field::Outputs,
            Value::Collection(vec![output("DP-1"), output("DP-2")]),
        );
        assert_eq!(window.apply_and_drain(), 1, "{original}");
        let bytes = window.file("compositor.toml");
        assert!(
            String::from_utf8(bytes).expect("UTF-8").contains(sibling),
            "sibling changed: {original}"
        );
        let config = window.reload_config();
        assert_eq!(config.compositor.outputs.len(), 2);
        assert_eq!(config.compositor.outputs[0].name.as_deref(), Some("DP-1"));
        assert_eq!(
            config.compositor.outputs[0].position.expect("position").x,
            -1707
        );
        assert_eq!(config.compositor.outputs[0].scale, 1.5);
        window.update(Message::AllReset);
        window.apply_and_drain();
        assert!(window.reload_config().compositor.outputs.is_empty());
    }
}

#[test]
fn invalid_display_values_and_duplicate_wallpapers_never_queue_writes() {
    for (column, text) in [
        (1, "0"),
        (2, "16385"),
        (3, "1001"),
        (4, "nan"),
        (4, "0.1"),
        (5, "-2147483648"),
    ] {
        let mut row = output("DP-1");
        row[column] = text.into();
        let mut window = Window::new();
        set(&mut window, Field::Outputs, Value::Collection(vec![row]));
        window.update(Message::Apply);
        assert!(
            window.state_mut().take_outgoing().is_empty(),
            "outputs column {column}: {text}"
        );
        assert!(
            validate::validate_field(Field::Outputs, window.state().draft().settings()).is_some()
        );
    }
    let mut window = Window::new();
    set(
        &mut window,
        Field::WallpaperPerOutput,
        Value::Collection(vec![
            vec!["DP-1".into(), "a.png".into()],
            vec!["DP-1".into(), "b.png".into()],
        ]),
    );
    assert!(
        !window
            .state()
            .model()
            .action(Action::Apply)
            .expect("apply")
            .enabled
    );
}

#[test]
fn wallpaper_overrides_terminal_and_input_method_arguments_roundtrip_literally() {
    for source in [
        "# shell\n[wallpaper]\nmode = 'fit'\n[wallpaper.per_output]\n'DP-1' = 'old.png'\n",
        "# shell\nwallpaper = { mode = 'fit', per_output = { 'DP-1' = 'old.png' } }\n",
        "# shell\nwallpaper.per_output.'DP-1' = 'old.png'\nwallpaper.mode = 'fit'\n",
    ] {
        let mut window = Window::with_files(&[("shell.toml", source)]);
        set(
            &mut window,
            Field::WallpaperImage,
            Value::Choice("/images/主壁纸.png".into()),
        );
        set(
            &mut window,
            Field::WallpaperPerOutput,
            Value::Collection(vec![
                vec!["DP-1".into(), "/images/one.png".into()],
                vec!["HDMI.A-1".into(), "/images/two with spaces.jpg".into()],
            ]),
        );
        set(
            &mut window,
            Field::LauncherTerminal,
            Value::Collection(vec![
                vec!["terminal".into()],
                vec!["literal $(date)".into()],
                vec!["--title=空格 window".into()],
            ]),
        );
        set(
            &mut window,
            Field::InputMethodCommand,
            Value::Collection(vec![
                vec!["/usr/bin/fcitx5".into()],
                vec!["--replace".into()],
            ]),
        );
        assert_eq!(window.apply_and_drain(), 2, "{source}");
        let config = window.reload_config();
        assert_eq!(
            config.shell.wallpaper.image.expect("image").to_str(),
            Some("/images/主壁纸.png")
        );
        assert_eq!(
            config.shell.wallpaper.per_output["HDMI.A-1"].to_str(),
            Some("/images/two with spaces.jpg")
        );
        assert_eq!(config.shell.launcher.terminal[1], "literal $(date)");
        assert_eq!(
            config.compositor.input.input_method,
            ["/usr/bin/fcitx5", "--replace"]
        );
    }
}

#[test]
fn incomplete_and_overflowing_numbers_stay_visible_block_apply_and_can_be_cancelled() {
    for text in ["", "-", "99999999999999999999999999", "abc"] {
        let mut window = Window::new();
        window.update(Message::PageSelected(PageId::Input));
        window.update(Message::NumberEdited {
            field: Field::KeyboardRepeatRate,
            text: text.into(),
        });
        assert_eq!(
            window
                .state()
                .draft()
                .number_input(Field::KeyboardRepeatRate),
            text
        );
        assert!(window.state().draft().is_modified());
        assert!(
            !window
                .state()
                .model()
                .action(Action::Apply)
                .expect("apply")
                .enabled
        );
        window.update(Message::Apply);
        assert!(window.state_mut().take_outgoing().is_empty());
        window.update(Message::Cancel);
        assert!(!window.state().draft().is_modified());
    }
}

#[test]
fn stale_save_acknowledgements_cannot_replace_a_document_or_consume_its_pending_write() {
    let mut window = Window::new();
    set(&mut window, Field::PanelHeight, Value::Number(64));
    window.update(Message::Apply);
    let commands = window.state_mut().take_outgoing();
    let OutgoingCommand::Save(request) = &commands[0] else {
        panic!("save request");
    };
    let before = window.state().documents().clone();
    window.update(Message::SaveSettled {
        generation: Generation::new(request.generation.get() + 1),
        file: request.file,
        outcome: SaveOutcome::Saved,
    });
    assert_eq!(window.state().documents(), &before);
    assert!(matches!(
        window.state().application().state(),
        ApplyState::Saving { .. }
    ));
    let outcome = persist::save(
        &request.path,
        request.content.as_bytes(),
        Some(&request.guard),
    );
    window.update(Message::SaveSettled {
        generation: request.generation,
        file: request.file,
        outcome,
    });
    assert_eq!(window.state().draft().base().panel_height, 64);
}

#[test]
fn a_failed_file_keeps_other_inflight_writes_locked_and_commits_their_actual_results() {
    let mut window = Window::new();
    set(
        &mut window,
        Field::ThemeAppearance,
        Value::Choice("light".into()),
    );
    set(&mut window, Field::PanelHeight, Value::Number(64));
    window.update(Message::Apply);
    let commands = window.state_mut().take_outgoing();
    let requests: Vec<_> = commands
        .iter()
        .filter_map(|command| match command {
            OutgoingCommand::Save(request) => Some(request),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 2);
    let root = requests
        .iter()
        .find(|request| request.file == SettingFile::Root)
        .expect("root");
    let shell = requests
        .iter()
        .find(|request| request.file == SettingFile::Shell)
        .expect("shell");
    window.update(Message::SaveSettled {
        generation: root.generation,
        file: root.file,
        outcome: SaveOutcome::Failed {
            reason: "write failed".into(),
        },
    });
    assert!(
        !window
            .state()
            .model()
            .action(Action::Cancel)
            .expect("cancel")
            .enabled
    );
    set(&mut window, Field::PanelHeight, Value::Number(128));
    window.update(Message::Cancel);
    assert_eq!(window.state().draft().settings().panel_height, 64);
    let outcome = persist::save(&shell.path, shell.content.as_bytes(), Some(&shell.guard));
    window.update(Message::SaveSettled {
        generation: shell.generation,
        file: shell.file,
        outcome,
    });
    assert_eq!(window.state().draft().base().panel_height, 64);
    assert_eq!(
        window.state().draft().base().theme_appearance,
        Appearance::System
    );
    assert!(matches!(
        window.state().application().state(),
        ApplyState::Rejected { .. }
    ));
    assert_eq!(
        InoxConfig::load(window.paths(), Appearance::System)
            .expect("saved files")
            .shell
            .panel
            .height,
        64
    );
}

#[test]
fn structured_list_entries_and_remove_actions_are_keyboard_reachable() {
    let mut window = Window::new();
    window.update(Message::PageSelected(PageId::InputMethod));
    window.update(Message::RowAdded(Field::InputMethodCommand));
    let order = window.state().model().focus_order;
    assert!(order.contains(&FocusTarget::Cell(Field::InputMethodCommand, 0, 0)));
    assert!(order.contains(&FocusTarget::RemoveRow(Field::InputMethodCommand, 0)));
    window.update(Message::CellEdited {
        field: Field::InputMethodCommand,
        row: 0,
        column: 0,
        text: "fcitx5".into(),
    });
    window.update(Message::FocusNext);
    assert_eq!(
        window.state().focused(),
        Some(FocusTarget::RemoveRow(Field::InputMethodCommand, 0))
    );
    window.update(Message::Activate);
    assert_eq!(
        window.state().draft().settings().input_method,
        Vec::<String>::new()
    );
}

#[test]
fn schema_valid_blank_wallpaper_preferences_do_not_block_unrelated_edits() {
    let mut window = Window::with_files(&[("shell.toml", "[wallpaper.per_output]\n'' = ''\n")]);
    assert!(validate::is_valid(window.state().draft().settings()));
    set(&mut window, Field::PanelHeight, Value::Number(48));
    assert_eq!(window.apply_and_drain(), 1);
    assert_eq!(window.reload_config().shell.panel.height, 48);
}

#[test]
fn a_single_explicit_output_coordinate_uses_the_other_schema_default() {
    let mut window = Window::new();
    let mut row = output("DP-1");
    row[6] = String::new();
    set(&mut window, Field::Outputs, Value::Collection(vec![row]));
    assert_eq!(window.apply_and_drain(), 1);
    let position = window.reload_config().compositor.outputs[0]
        .position
        .expect("position");
    assert_eq!((position.x, position.y), (-1707, 0));
}

#[test]
fn numeric_keyboard_cycles_do_not_overflow_on_extreme_input() {
    for field in Field::ALL {
        let Some(range) = field.control().range else {
            continue;
        };
        for value in [i64::MIN, i64::MAX] {
            for delta in [i64::MIN, -1, 1, i64::MAX] {
                assert!(
                    range.contains(range.advance(value, delta)),
                    "{field}: {value} {delta}"
                );
            }
        }
    }
}
