//! What the window draws, asserted as values instead of pixels (Q02, D04, D05, Q01).
//!
//! The iced version of this evidence rendered the real widget tree with tiny-skia
//! and read pixels back. Qt draws the window now, so the same facts are asserted
//! where they are decided: in the JSON projection the QML tree renders
//! ([`crate::view::project`]). Every assertion below is the value the pixel test
//! used to recognise — a colour that is a token, a control that says why it is
//! disabled, a focus identifier that lands on a control the tree draws, the
//! warning colour around a change that is not settled yet.
//!
//! The one thing a projection test cannot prove is that Qt draws the tree at all.
//! [`crate::headless_tests::qml`] is the evidence for that half: it loads the real
//! QML module on the offscreen platform and fails unless the tree reports the
//! title, the number of pages and the accessibility items it built.

use inox_common::i18n::Locale;
use inox_config::Theme as Tokens;
use serde_json::Value;

use crate::app::Message;
use crate::draft::{Field, Value as DraftValue};
use crate::headless_tests::support::Window;
use crate::pages::PageId;
use crate::theme;
use crate::view::{
    self, Control, Projection, Width, field_name, focus_id, page_name, status_name, width_of,
};
use crate::view_model::{StatusKind, TrialChoice};

/// The theme the dark scenarios are projected with.
fn dark() -> Tokens {
    Tokens::builtin_dark()
}

/// The theme the light scenarios are projected with.
fn light() -> Tokens {
    Tokens::builtin_light()
}

/// The projection of a window, which is what the QML tree renders.
fn projection(window: &Window) -> Projection {
    view::project(window.state().model(), window.state().tokens())
}

/// A window showing a configuration with values worth drawing.
///
/// It is the same fixture the rendered-pixel tests built: the panel page with its
/// height edited, so the projection has a modified field, a status line that says
/// so, and an Apply that can be taken.
fn populated(tokens: Tokens) -> Window {
    let mut window = Window::with_locale(
        &[
            (
                "shell.toml",
                "[panel]\n# the strip\nheight = 40\nshow_clock = true\nautohide = false\n",
            ),
            (
                "notifications.toml",
                "[display]\nposition = \"top-right\"\n\n[timeouts]\nlow_ms = 5000\nnormal_ms = 8000\n",
            ),
        ],
        Locale::default_locale(),
        tokens,
    );

    window.update(Message::PageSelected(PageId::Panel));
    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: DraftValue::Number(48),
    });

    window
}

/// A window with a display change the compositor has taken on trial.
///
/// The trial is the state ADR 0028 exists for: the values are in effect and the
/// window is asking whether to keep them.
fn on_trial(tokens: Tokens) -> Window {
    let mut window = Window::with_locale(&[], Locale::default_locale(), tokens).with_compositor();
    window.update(Message::PageSelected(PageId::Displays));
    window.update(Message::RowAdded(Field::Outputs));
    window.update(Message::CellEdited {
        field: Field::Outputs,
        row: 0,
        column: 0,
        text: "DP-1".to_owned(),
    });
    window.apply_and_drain();

    window
}

/// Every string in a JSON value, in document order.
fn collect_strings(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::String(text) => into.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_strings(item, into);
            }
        }
        Value::Object(fields) => {
            for item in fields.values() {
                collect_strings(item, into);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The projection as the JSON string the bridge hands to QML.
fn as_json(window: &Projection) -> Value {
    let serialised = view::to_json(window).expect("the projection serialises");
    serde_json::from_str(&serialised).expect("the projection is valid JSON")
}

/// Every colour the projection of `window` can draw with.
///
/// They are the theme's own colours plus the status line's border, which is
/// chosen by how far a change has got ([`crate::theme::status_border`]).
fn colours_of(projection: &Projection) -> Vec<String> {
    let theme = serde_json::to_value(&projection.theme).expect("the theme serialises");
    let mut strings = Vec::new();
    collect_strings(&theme, &mut strings);

    let mut colours: Vec<String> = strings
        .into_iter()
        .filter(|value| value.len() == 9 && value.starts_with('#'))
        .collect();
    colours.push(projection.status.border.clone());
    colours.sort();
    colours.dedup();

    colours
}

/// Every focus identifier the projection draws a control with.
///
/// The keyboard's walk is only honest if each step lands on something the tree
/// actually renders: this is that set, gathered from the projection itself.
fn drawn_focus_ids(projection: &Projection) -> Vec<String> {
    let mut ids: Vec<String> = projection
        .pages
        .iter()
        .map(|page| page.focus_id.clone())
        .collect();

    for group in &projection.groups {
        for field in &group.fields {
            ids.push(field.focus_id.clone());
            ids.push(field.reset_focus_id.clone());

            match &field.control {
                Control::Collection {
                    rows, add_focus_id, ..
                } => {
                    ids.push(add_focus_id.clone());
                    for row in rows {
                        ids.push(row.remove_focus_id.clone());
                        ids.extend(row.cells.iter().map(|cell| cell.focus_id.clone()));
                    }
                }
                Control::Choices { options } => {
                    ids.extend(options.iter().map(|option| option.focus_id.clone()));
                }
                Control::Text { .. } | Control::Flag { .. } | Control::Number { .. } => {}
            }
        }
    }

    ids.extend(
        projection
            .actions
            .iter()
            .map(|entry| entry.focus_id.clone()),
    );

    if let Some(trial) = &projection.trial {
        ids.extend(trial.choices.iter().map(|choice| choice.focus_id.clone()));
    }

    ids
}

/// Every sentence a user reads, gathered from the projection.
fn sentences_of(projection: &Projection) -> Vec<String> {
    let mut sentences = vec![projection.title.clone(), projection.page.title.clone()];
    sentences.push(projection.page.description.clone());
    sentences.extend(projection.page.notices.iter().cloned());

    for page in &projection.pages {
        sentences.push(page.label.clone());
        sentences.push(page.description.clone());
    }

    for group in &projection.groups {
        sentences.push(group.title.clone());
        for field in &group.fields {
            sentences.push(field.label.clone());
            sentences.extend(field.reset.clone());
            sentences.extend(field.error.clone());
            sentences.extend(field.disabled_reason.clone());

            match &field.control {
                Control::Collection {
                    columns,
                    rows,
                    add,
                    help,
                    empty,
                    ..
                } => {
                    sentences.extend(columns.iter().cloned());
                    sentences.extend(
                        rows.iter()
                            .flat_map(|row| row.cells.iter().map(|cell| cell.text.clone())),
                    );
                    sentences.extend(rows.iter().map(|row| row.remove.clone()));
                    sentences.push(add.clone());
                    sentences.push(help.clone());
                    sentences.push(empty.clone());
                }
                Control::Text { value } => sentences.push(value.clone()),
                Control::Choices { options } => {
                    sentences.extend(options.iter().map(|option| option.label.clone()));
                }
                Control::Flag { label, .. } => sentences.push(label.clone()),
                Control::Number {
                    unit,
                    input,
                    display,
                    decrease,
                    increase,
                    ..
                } => {
                    sentences.push(unit.clone());
                    sentences.push(input.clone());
                    sentences.push(display.clone());
                    sentences.push(decrease.clone());
                    sentences.push(increase.clone());
                }
            }
        }
    }

    sentences.extend(projection.actions.iter().map(|action| action.label.clone()));
    sentences.extend(
        projection
            .actions
            .iter()
            .filter_map(|action| action.disabled_reason.clone()),
    );
    sentences.push(projection.status.message.clone());
    sentences.extend(projection.status.details.iter().cloned());

    if let Some(trial) = &projection.trial {
        sentences.push(trial.title.clone());
        sentences.push(trial.message.clone());
        sentences.extend(trial.choices.iter().map(|choice| choice.label.clone()));
        sentences.extend(trial.error.clone());
    }

    sentences
}

#[test]
fn the_window_projects_to_one_json_value_the_tree_can_parse() {
    let window = projection(&populated(dark()));
    let json = as_json(&window);

    assert!(json["title"].is_string());
    assert_eq!(
        json["pages"].as_array().map(Vec::len),
        Some(PageId::ALL.len())
    );
    assert!(json["theme"]["colors"]["window"].is_string());
    assert!(json["theme"]["geometry"]["bodySize"].is_number());
    assert!(!json["groups"].as_array().expect("groups").is_empty());
    assert!(json["trial"].is_null());
    assert!(json["status"]["border"].is_string());

    // The QML window binds its own size to the projection rather than choosing
    // one: accepted snapshots were taken at this size (Q02).
    assert_eq!(json["width"], 1280.0);
    assert_eq!(json["height"], 720.0);
}

#[test]
fn every_colour_the_tree_draws_comes_from_the_tokens() {
    let tokens = dark();
    let window = populated(tokens.clone());
    let projected = projection(&window);
    let drawn = as_json(&projected);

    let mut strings = Vec::new();
    collect_strings(&drawn, &mut strings);

    let allowed = colours_of(&projected);
    let mut colours = 0;

    for value in strings.iter().filter(|value| value.starts_with('#')) {
        assert_eq!(value.len(), 9, "`{value}` is not Qt's #AARRGGBB");
        assert!(
            value[1..]
                .chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character)),
            "`{value}` is not lower-case #AARRGGBB"
        );
        assert!(
            allowed.contains(value),
            "`{value}` is drawn but is not a colour of the theme projection"
        );
        colours += 1;
    }

    assert!(colours > 10, "the window draws more than its background");
    assert_eq!(
        projected.theme.colors.window,
        theme::qml_color(tokens.colors.background),
        "the window's own background is the background token"
    );
    assert_eq!(
        projected.theme.colors.surface,
        theme::qml_color(tokens.colors.surface),
        "the navigation list and the cards are the surface token"
    );

    // The light and the dark theme project differently, which is what the two
    // snapshot scenarios used to prove by comparing pixels.
    let other = projection(&populated(light()));
    assert_ne!(projected.theme.colors.window, other.theme.colors.window);
    assert_ne!(projected.theme.colors.text, other.theme.colors.text);
}

#[test]
fn a_field_the_schema_refuses_names_the_field_and_the_danger_colour() {
    let tokens = dark();
    let clean = projection(&populated(tokens.clone()));

    let mut refusing = populated(tokens.clone());
    // No click can produce this; a draft can hold it, and what the window has to
    // do is show it (acceptance D04: the error is located at the field).
    refusing.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: DraftValue::Number(9_000),
    });
    let projected = projection(&refusing);

    let field = projected
        .groups
        .iter()
        .flat_map(|group| group.fields.iter())
        .find(|field| field.field == field_name(Field::PanelHeight))
        .expect("the height is on the panel page");

    let error = field.error.clone().expect("the value is out of range");
    assert!(error.contains("panel.height"), "sentence `{error}`");
    assert!(field.enabled, "a refused value is still editable");
    assert_eq!(
        projected.theme.input.border_refused,
        theme::qml_color(tokens.colors.danger),
        "the refused field is outlined in the danger colour"
    );

    let clean_field = clean
        .groups
        .iter()
        .flat_map(|group| group.fields.iter())
        .find(|field| field.field == field_name(Field::PanelHeight))
        .expect("the height is on the panel page");
    assert!(clean_field.error.is_none());
    // A refused value still leaves the window in the state a modification does:
    // the error is the field's, and the status line says nothing has been written.
    assert_eq!(projected.status.kind, status_name(StatusKind::Modified));
}

#[test]
fn a_save_in_flight_disables_every_control_with_the_reason_it_cannot_be_used() {
    let mut window = populated(dark());
    // A save in flight disables every control until it lands (D05).
    window.update(Message::Apply);

    let projected = projection(&window);

    assert_eq!(projected.status.kind, status_name(StatusKind::Saving));
    assert!(
        !projected.page.notices.is_empty(),
        "the page says why, once"
    );
    assert!(
        !projected.groups.is_empty(),
        "the page's fields are still drawn"
    );

    for group in &projected.groups {
        for field in &group.fields {
            assert!(!field.enabled, "{} is usable during a save", field.field);
            assert!(
                field.disabled_reason.is_some(),
                "{} is disabled without saying why",
                field.field
            );
            assert!(
                !projected.focus.order.contains(&field.focus_id),
                "{} is disabled and still in the keyboard's walk",
                field.field
            );
        }
    }

    for action in &projected.actions {
        assert!(!action.enabled, "{} is usable during a save", action.id);
        assert!(
            action.disabled_reason.is_some(),
            "{} is disabled without saying why",
            action.id
        );
    }
}

#[test]
fn every_entry_the_keyboard_can_reach_is_drawn_as_a_control() {
    fn untouched() -> Window {
        Window::new()
    }

    fn in_flight() -> Window {
        let mut window = populated(dark());
        window.update(Message::Apply);

        window
    }

    for (state, build) in [
        ("a window nobody has touched", untouched as fn() -> Window),
        ("a window whose save is in flight", in_flight),
    ] {
        let probe = build();
        let projected = projection(&probe);
        let walk = projected.focus.order.clone();

        assert!(!walk.is_empty(), "{state}: the walk is empty");

        let drawn = drawn_focus_ids(&projected);
        for id in &walk {
            assert!(
                drawn.contains(id),
                "{state}: `{id}` is an entry of the walk and no control is drawn with it"
            );
        }

        for (index, expected) in walk.iter().enumerate() {
            let mut stepped = build();
            for _ in 0..=index {
                stepped.update(Message::FocusNext);
            }

            let focus = projection(&stepped).focus;
            assert_eq!(
                focus.id.as_deref(),
                Some(expected.as_str()),
                "{state}: step {index} did not land where the walk says"
            );
            assert!(
                matches!(focus.scroll, None | Some("navigation") | Some("content")),
                "{state}: step {index} names a container the tree does not have: {:?}",
                focus.scroll
            );
        }
    }
}

#[test]
fn the_open_page_is_the_one_the_projection_draws() {
    let mut window = Window::new();

    for page in PageId::ALL {
        window.update(Message::PageSelected(page));
        let projected = projection(&window);

        assert_eq!(projected.page.id, page_name(page));
        assert_eq!(projected.focus.page, page_name(page));

        let selected: Vec<&str> = projected
            .pages
            .iter()
            .filter(|entry| entry.selected)
            .map(|entry| entry.id)
            .collect();
        assert_eq!(selected, vec![page_name(page)], "{page:?}");

        let projected_fields: Vec<&str> = projected
            .groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .map(|field| field.field)
            .collect();
        let expected: Vec<&str> = page
            .fields()
            .iter()
            .map(|field| field_name(*field))
            .collect();
        assert_eq!(projected_fields, expected, "{page:?}");
    }
}

#[test]
fn a_display_trial_projects_its_question_in_the_warning_colour() {
    let tokens = dark();
    let window = on_trial(tokens.clone());
    let projected = projection(&window);

    let trial = projected
        .trial
        .as_ref()
        .expect("the window asks about the display");
    assert!(!trial.title.is_empty());
    assert!(!trial.message.is_empty());
    assert_eq!(trial.remaining_seconds, crate::app::DISPLAY_TRIAL_SECONDS);
    assert_eq!(trial.choices.len(), TrialChoice::ALL.len());
    assert_eq!(trial.choices[0].id, "confirm");
    assert!(trial.choices[0].confirm);
    assert_eq!(trial.choices[1].id, "revert");
    assert!(!trial.choices[1].confirm);

    // The question puts the keyboard on the answer that keeps the change, so one
    // Enter answers it (Q01).
    assert_eq!(
        projected.focus.id.as_deref(),
        Some(trial.choices[0].focus_id.as_str())
    );

    // The dialog is not a settled change, so it is drawn in the warning colour,
    // and the window behind it is dimmed with a token rather than a literal.
    let warning = theme::qml_color(tokens.colors.warning);
    assert_eq!(projected.theme.colors.dialog_border, warning);
    assert_eq!(projected.status.border, warning);
    assert_eq!(projected.status.kind, status_name(StatusKind::Provisional));
    assert_ne!(projected.theme.colors.scrim, projected.theme.colors.window);

    // Answered, the question is gone and the same window carries no dialog.
    let mut answered = on_trial(tokens);
    answered.update(Message::TrialConfirm);
    answered.drain();
    assert!(projection(&answered).trial.is_none());
}

#[test]
fn a_change_that_is_saved_but_not_in_effect_says_so() {
    let tokens = dark();
    let mut window = populated(tokens.clone());
    let written = window.apply_and_drain();
    let projected = projection(&window);

    assert_eq!(written, 1, "the shell file is written");
    assert_eq!(
        projected.status.kind,
        status_name(StatusKind::SavedOnly),
        "nothing could confirm it, so the window must not claim it took effect"
    );
    assert_eq!(
        projected.status.border,
        theme::qml_color(tokens.colors.warning),
        "saved but not in effect is drawn in the warning colour"
    );
    assert!(!projected.status.message.is_empty());

    // The state the window is in while the write is in flight is a different
    // sentence again, so a user can tell the three apart.
    let mut saving = populated(tokens);
    saving.update(Message::Apply);
    let in_flight = projection(&saving);
    assert_eq!(in_flight.status.kind, status_name(StatusKind::Saving));
    assert_ne!(in_flight.status.message, projected.status.message);
    assert_ne!(in_flight.status.border, projected.status.border);
}

#[test]
fn an_empty_configuration_still_projects_the_whole_page() {
    let mut window = Window::new();
    window.update(Message::PageSelected(PageId::Notifications));

    let projected = projection(&window);
    let fields: Vec<&str> = projected
        .groups
        .iter()
        .flat_map(|group| group.fields.iter())
        .map(|field| field.field)
        .collect();

    assert_eq!(
        fields.len(),
        PageId::Notifications.fields().len(),
        "every field of the page is projected even when the file says nothing"
    );
    assert!(!projected.page.title.is_empty());
    assert!(!projected.status.message.is_empty());
    for group in &projected.groups {
        assert!(!group.title.is_empty());
        for field in &group.fields {
            assert!(!field.label.is_empty(), "{}", field.field);
        }
    }
}

#[test]
fn a_list_says_what_an_empty_one_will_do() {
    let mut window = Window::new();
    window.update(Message::PageSelected(PageId::Displays));

    let projected = projection(&window);
    let field = projected
        .groups
        .iter()
        .flat_map(|group| group.fields.iter())
        .find(|field| field.field == field_name(Field::Outputs))
        .expect("the displays are a list");

    let Control::Collection {
        rows,
        empty,
        help,
        is_empty,
        add,
        ..
    } = &field.control
    else {
        panic!("the displays are a list");
    };

    assert!(rows.is_empty(), "the window holds no display preference");
    assert!(*is_empty);
    assert!(
        !empty.trim().is_empty(),
        "an empty list says what the window will do rather than drawing a blank"
    );
    assert!(!help.trim().is_empty());
    assert!(!add.trim().is_empty());

    let mut with_a_row = Window::new();
    with_a_row.update(Message::PageSelected(PageId::Displays));
    with_a_row.update(Message::RowAdded(Field::Outputs));
    let with_a_row = projection(&with_a_row);
    let field = with_a_row
        .groups
        .iter()
        .flat_map(|group| group.fields.iter())
        .find(|field| field.field == field_name(Field::Outputs))
        .expect("the displays are a list");

    let Control::Collection {
        rows,
        columns,
        is_empty,
        ..
    } = &field.control
    else {
        panic!("the displays are a list");
    };
    assert!(!*is_empty, "a list with an entry is not empty");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].cells.len(),
        columns.len(),
        "a row has one cell per column"
    );
}

#[test]
fn a_language_tag_the_file_carries_itself_stays_on_the_row() {
    let mut window = Window::new();
    window.update(Message::PageSelected(PageId::Locale));
    window.update(Message::FieldSet {
        field: Field::LocaleLanguage,
        value: DraftValue::Choice("zh-Hant-HK-very-long-variant".to_owned()),
    });

    let projected = projection(&window);
    let field = projected
        .groups
        .iter()
        .flat_map(|group| group.fields.iter())
        .find(|field| field.field == field_name(Field::LocaleLanguage))
        .expect("the language is on the locale page");

    let Control::Choices { options } = &field.control else {
        panic!("the language is a choice");
    };

    // The three options Inox ships and the file's own tag, all on the row.
    assert_eq!(options.len(), 4);
    assert!(
        options
            .iter()
            .any(|option| option.id == "zh-Hant-HK-very-long-variant"),
        "the tag the file holds is a row of its own"
    );
    assert!(
        options
            .iter()
            .any(|option| option.selected && option.id.starts_with("zh-Hant"))
    );
    for option in options {
        assert!(!option.label.is_empty(), "{}", option.id);
    }

    // The row is drawn at the window's width, which is what keeps a long label
    // from pushing the control out of the card (ui-guidelines §4).
    assert_eq!(field.width, Width::Fill);
}

#[test]
fn a_short_value_is_never_drawn_as_wide_as_the_window() {
    let tokens = dark();
    let projected = projection(&populated(tokens.clone()));
    let geometry = &projected.theme.geometry;

    assert!(
        geometry.short_width < geometry.wide_width,
        "a short value is narrower than a long one"
    );
    assert!(geometry.wide_width < view::WINDOW_WIDTH);

    for group in &projected.groups {
        for field in &group.fields {
            assert_eq!(
                field.width,
                width_of(field_name_of(field.field)),
                "{} is drawn at a width the projection did not derive",
                field.field
            );
        }
    }

    assert_eq!(width_of(Field::PanelHeight), Width::Short);
    assert_eq!(width_of(Field::PanelClockFormat), Width::Wide);
    assert_eq!(width_of(Field::Outputs), Width::Fill);
}

/// The field a wire name names.
fn field_name_of(name: &str) -> Field {
    view::field_by_name(name).expect("a projected field has a name the window knows")
}

#[test]
fn a_doubled_font_scale_grows_the_projection_without_losing_a_page() {
    let mut tokens = dark();
    let plain = projection(&populated(tokens.clone())).theme.geometry;

    tokens.typography.scale = 2.0;
    let scaled_window = populated(tokens.clone());
    let scaled = projection(&scaled_window);
    let big = &scaled.theme.geometry;

    assert!(big.body_size > plain.body_size);
    assert!(big.heading_size > plain.heading_size);
    assert!(big.nav_width > plain.nav_width);
    assert!(big.label_width > plain.label_width);
    assert!(big.control_height > plain.control_height);
    assert!(
        !big.list_as_table,
        "a table whose columns are narrower than the words in its headings is \
         drawn one field per row instead"
    );
    assert_eq!(scaled.pages.len(), plain_page_count());
    assert!(!scaled.focus.order.is_empty());

    // The navigation list stays reachable at the enlarged scale: the last entry
    // of the walk's first block is a page.
    let last_page = scaled.pages.last().expect("ten pages").focus_id.clone();
    let mut window = populated(tokens);
    let mut reached = None;
    for _ in 0..scaled.pages.len() {
        window.update(Message::FocusNext);
        reached = projection(&window).focus.id;
    }
    assert_eq!(reached, Some(last_page));
}

/// How many pages the window has, as the projection counts them.
fn plain_page_count() -> usize {
    projection(&Window::new()).pages.len()
}

#[test]
fn no_sentence_in_the_projection_is_a_message_id() {
    let tokens = dark();
    let english = projection(&populated(tokens.clone()));

    for sent in sentences_of(&english) {
        assert!(!sent.trim().is_empty(), "an empty sentence is drawn");
        assert!(
            !sent.starts_with("settings-"),
            "`{sent}` is a message id, not a sentence (Q03)"
        );
    }

    let chinese = projection(&Window::with_locale(
        &[],
        Locale::parse("zh-CN").expect("a well-formed tag"),
        dark(),
    ));
    assert_ne!(
        chinese.page.title,
        projection(&Window::new()).page.title,
        "the window follows the negotiated language"
    );
    assert_eq!(chinese.pages.len(), plain_page_count());
}

#[test]
fn the_keyboards_place_is_named_by_the_projection_the_tree_walks() {
    let mut window = Window::new();
    window.update(Message::FocusNext);

    let projected = projection(&window);
    let Some(id) = projected.focus.id.clone() else {
        panic!("the keyboard landed somewhere");
    };

    // The identifier is the one the focused control carries in the projection,
    // and the tree is told which container has to reveal it.
    assert!(
        drawn_focus_ids(&projected).contains(&id),
        "`{id}` is the focused entry and no control carries it"
    );
    assert!(
        matches!(projected.focus.scroll, Some("navigation") | Some("content")),
        "the first step is a page entry, which the navigation list scrolls: {:?}",
        projected.focus.scroll
    );

    // And it is a page: the walk starts with the navigation list.
    assert_eq!(
        id,
        focus_id(crate::view_model::FocusTarget::Page(PageId::ALL[0]))
    );
}
