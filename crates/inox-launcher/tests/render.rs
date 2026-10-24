//! Render and interaction tests (acceptance Q02, D02, D05).
//!
//! Each scenario builds the real widget tree from a state with a fixed list of
//! applications, draws it through the tiny-skia renderer at 1280x720 and reads
//! the pixels back. Nothing here needs a compositor, which is what makes the
//! evidence collectable on the CI host that has none.
//!
//! The scenarios render the launcher as it opens — with the caret in the search
//! field, which is what `main` does before the first frame — and every colour a
//! scenario asserts on is read back from the theme tokens rather than written
//! into the test.

mod support;

use std::rc::Rc;

use inox_common::i18n::Locale;
use inox_config::Theme as Tokens;
use inox_config::shell::LauncherConfig;
use inox_launcher::app::{self, Message, Prepared, State};
use inox_launcher::contract::{Failure, FailureKind, ServiceState};
use inox_launcher::index::{AppIndex, IndexedEntry};
use inox_launcher::placement::{LogicalSize, Placement};
use inox_launcher::theme::{self, TextRole};
use inox_launcher::view;
use inox_launcher::view_model::BannerKind;

use support::SIZE;

/// An entry with the fields a row draws from.
fn entry(id: &str, name: &str, generic: Option<&str>, exec: &str) -> IndexedEntry {
    IndexedEntry {
        id: id.to_owned(),
        path: std::path::PathBuf::from(format!("/usr/share/applications/{id}")),
        name: name.to_owned(),
        generic_name: generic.map(str::to_owned),
        comment: None,
        icon: Some(format!("{id}-icon")),
        exec: exec.to_owned(),
        terminal: false,
        working_dir: None,
        startup_wm_class: None,
        categories: Vec::new(),
        actions: Vec::new(),
        source: std::path::PathBuf::from("/usr/share/applications"),
    }
}

/// The list every scenario starts from: two ordinary applications and one that
/// asks for a terminal nobody configured, so a disabled row is on screen too.
fn applications() -> Vec<IndexedEntry> {
    let mut terminal = entry("htop.desktop", "htop", Some("Process Viewer"), "htop");
    terminal.terminal = true;
    vec![
        entry(
            "firefox.desktop",
            "Firefox",
            Some("Web Browser"),
            "firefox %u",
        ),
        entry("files.desktop", "Files", Some("File Manager"), "nautilus"),
        terminal,
    ]
}

fn prepared(tokens: Tokens) -> Prepared {
    prepared_on(tokens, SIZE.width as u32, SIZE.height as u32)
}

/// The same, for an output of a given size: the placement policy decides how
/// many characters a row's label has room for, and a test that wants a narrow
/// budget asks for a small output rather than shortening the name.
fn prepared_on(tokens: Tokens, width: u32, height: u32) -> Prepared {
    let config = LauncherConfig::default();
    let placement = Placement::overlay(&config, &tokens, LogicalSize::new(width, height), None);
    Prepared {
        tokens,
        localizer: Rc::new(
            inox_launcher::i18n::localizer(&Locale::parse("en-US").expect("a tag"))
                .expect("the catalogs load"),
        ),
        config,
        placement,
    }
}

/// A state over [`applications`], optionally with a query typed in and a
/// service state other than "ready".
fn state(tokens: Tokens, query: &str, service: Option<ServiceState>) -> State {
    let prepared = prepared(tokens);
    let mut state = State::with_index(&prepared, AppIndex::from_entries(applications(), 1), 1);
    if let Some(service) = service {
        state.service = service;
        // The projection runs after every message; setting the service state
        // directly is only what a test does.
        state.refresh_model();
    }
    if !query.is_empty() {
        send(&mut state, Message::QueryChanged(query.to_owned()));
    }
    state
}

fn dark() -> Tokens {
    Tokens::builtin_dark()
}

fn light() -> Tokens {
    Tokens::builtin_light()
}

/// The horizontal band the launcher's cards occupy, as `(left, right)`.
///
/// The view caps the groups at the surface the host will map and the window's
/// own padding (`view::view`), and centres them; a test that wants to say "this
/// text stayed inside the card" has to know that the same way the view does.
fn card_band(state: &State) -> (f32, f32) {
    let gutter = f32::from(state.tokens.spacing.xl);
    let content = state.placement.size.width as f32 - gutter * 2.0;
    let left = (SIZE.width - content) / 2.0;
    (left, left + content)
}

#[test]
fn the_dark_scenario_draws_the_overlay_in_the_theme_colours() {
    let tokens = dark();
    let state = state(tokens.clone(), "", None);
    let pixels = support::render(view::view(&state), &theme::iced_theme(&tokens), "dark");

    assert!(support::contains(
        &pixels,
        theme::colour(tokens.colors.surface)
    ));
    assert!(
        support::contains(&pixels, theme::colour(tokens.colors.accent)),
        "the selected row is drawn in the accent colour",
    );
    assert!(
        support::contains(&pixels, theme::colour(tokens.colors.warning)),
        "the reason a row cannot be launched is on screen",
    );
    assert!(
        support::distinct_colours(&pixels) > 16,
        "a drawn interface is more than a flat fill",
    );
}

#[test]
fn the_light_scenario_uses_the_light_tokens() {
    let tokens = light();
    let state = state(tokens.clone(), "", None);
    let pixels = support::render(view::view(&state), &theme::iced_theme(&tokens), "light");

    assert!(support::contains(
        &pixels,
        theme::colour(tokens.colors.surface)
    ));
    assert!(support::contains(
        &pixels,
        theme::colour(tokens.colors.accent)
    ));
}

#[test]
fn the_two_looks_are_actually_different() {
    let dark = dark();
    let light = light();

    let dark_pixels = support::render(
        view::view(&state(dark.clone(), "", None)),
        &theme::iced_theme(&dark),
        "dark",
    );
    let light_pixels = support::render(
        view::view(&state(light.clone(), "", None)),
        &theme::iced_theme(&light),
        "light",
    );

    assert_ne!(
        dark_pixels, light_pixels,
        "a light theme that renders the same as the dark one is not a theme",
    );
    // The top-left corner is outside the groups, so it is the window's own
    // background token and nothing else; the cards on top of it are `surface`,
    // which is a token of its own and therefore a different colour.
    assert_eq!(
        support::pixel(&dark_pixels, 8, 8)[..3],
        [
            dark.colors.background.r,
            dark.colors.background.g,
            dark.colors.background.b
        ],
    );
    assert_eq!(
        support::pixel(&light_pixels, 8, 8)[..3],
        [
            light.colors.background.r,
            light.colors.background.g,
            light.colors.background.b
        ],
    );
    assert_ne!(
        dark.colors.background, dark.colors.surface,
        "the window and the cards have to be told apart",
    );
}

#[test]
fn the_icon_column_is_a_square_tile_and_not_the_whole_row() {
    let tokens = dark();
    let state = state(tokens.clone(), "", None);
    let icon_size = state.config.icon_size as usize;
    let pixels = support::render(view::view(&state), &theme::iced_theme(&tokens), "dark");

    // Two rows name an icon, so two accent tiles of exactly the configured size
    // are on the screen. A stretched icon column — the shape this guards against
    // — would leave no scanline with a run this short.
    let tile_lines =
        support::scanlines_with_run(&pixels, theme::colour(tokens.colors.accent), icon_size);
    assert!(
        tile_lines >= icon_size,
        "the icon column is not a square tile: {tile_lines} scanlines carry a run of {icon_size} px",
    );
}

#[test]
fn the_selection_is_a_fill_one_row_tall() {
    let tokens = dark();
    let state = state(tokens.clone(), "", None);
    let icon_size = state.config.icon_size as usize;
    let accent = theme::colour(tokens.colors.accent);
    let pixels = support::render(view::view(&state), &theme::iced_theme(&tokens), "dark");

    assert!(
        support::contains_block(&pixels, accent, icon_size * 3, icon_size),
        "the selected row is a fill as tall as a row and as wide as the list",
    );
    assert!(
        !support::contains_block(
            &pixels,
            theme::colour(tokens.colors.surface_variant),
            icon_size * 3,
            icon_size,
        ),
        "no row is drawn as hovered when the pointer is not over one",
    );
}

#[test]
fn the_search_field_wears_the_focus_ring_and_the_rest_of_the_screen_does_not() {
    let tokens = dark();
    let state = state(tokens.clone(), "zzzz", None);
    let accent = theme::colour(tokens.colors.accent);
    let icon_size = state.config.icon_size as usize;

    let focused = support::render(
        view::view(&state),
        &theme::iced_theme(&tokens),
        "no-matches",
    );
    let resting = support::render_unfocused(
        view::view(&state),
        &theme::iced_theme(&tokens),
        "no-matches-unfocused",
    );

    assert_ne!(focused, resting, "focus is visible, so it changes pixels");
    assert!(
        support::contains(&focused, accent),
        "the focused search field wears the accent ring",
    );
    assert!(
        !support::contains(&resting, accent),
        "nothing else on a screen with no results is accent-coloured",
    );
    assert!(
        !support::contains_block(&focused, accent, icon_size * 3, icon_size),
        "the ring is an outline, not a fill",
    );
}

#[test]
fn the_no_matches_scenario_renders_the_empty_state() {
    let tokens = dark();
    let state = state(tokens.clone(), "zzzz", None);
    let icon_size = state.config.icon_size as usize;
    let pixels = support::render(
        view::view(&state),
        &theme::iced_theme(&tokens),
        "no-matches",
    );

    assert!(state.model.rows.is_empty());
    assert!(state.model.banner.is_some());
    assert_eq!(
        state.model.banner.as_ref().map(|banner| banner.kind),
        Some(BannerKind::NoMatch),
    );
    assert!(support::contains(
        &pixels,
        theme::colour(tokens.colors.surface)
    ));
    assert!(
        !support::contains_block(
            &pixels,
            theme::colour(tokens.colors.accent),
            icon_size * 3,
            icon_size,
        ),
        "there is no selected row to highlight",
    );
}

#[test]
fn the_resting_state_and_the_no_match_state_are_two_different_screens() {
    let tokens = dark();
    let resting_state = state(tokens.clone(), "", None);
    let no_match_state = state(tokens.clone(), "zzzz", None);

    assert!(
        resting_state.model.prompt.is_some(),
        "an empty query says what typing would do",
    );
    assert!(resting_state.model.banner.is_none());
    assert!(no_match_state.model.prompt.is_none());
    assert!(no_match_state.model.banner.is_some());

    let resting = support::render(
        view::view(&resting_state),
        &theme::iced_theme(&tokens),
        "resting",
    );
    let no_match = support::render(
        view::view(&no_match_state),
        &theme::iced_theme(&tokens),
        "no-matches",
    );

    assert_ne!(
        resting, no_match,
        "the prompt and the no-match state cannot be the same screen",
    );
}

#[test]
fn the_loading_scenario_says_what_it_is_waiting_for() {
    let tokens = dark();
    let state = state(
        tokens.clone(),
        "",
        Some(ServiceState::Loading { generation: 2 }),
    );
    let pixels = support::render(view::view(&state), &theme::iced_theme(&tokens), "loading");
    let icon_size = state.config.icon_size as usize;
    let accent = theme::colour(tokens.colors.accent);

    assert_eq!(
        state.model.banner.as_ref().map(|banner| banner.kind),
        Some(BannerKind::Loading),
    );
    assert!(
        !state.model.can_retry_index(),
        "there is nothing to act on while the index is being read",
    );
    assert!(
        state.model.prompt.is_none(),
        "a list that is not there may not be described as one",
    );
    assert!(support::contains(
        &pixels,
        theme::colour(tokens.colors.surface)
    ));
    assert!(
        support::scanlines_with_run(&pixels, accent, icon_size) == 0,
        "the rows give way to the state that replaces them",
    );
}

#[test]
fn the_unavailable_scenario_renders_an_error_with_a_retry() {
    let tokens = dark();
    let state = state(
        tokens.clone(),
        "",
        Some(ServiceState::Unavailable {
            generation: 1,
            failure: Failure::service(FailureKind::IndexUnavailable {
                detail: "no HOME".to_owned(),
            }),
        }),
    );
    let pixels = support::render(
        view::view(&state),
        &theme::iced_theme(&tokens),
        "unavailable",
    );

    assert!(state.model.can_retry_index());
    assert_eq!(
        state.model.banner.as_ref().map(|banner| banner.kind),
        Some(BannerKind::Unavailable),
    );
    assert!(support::contains(
        &pixels,
        theme::colour(tokens.colors.surface)
    ));
    assert!(support::contains(
        &pixels,
        theme::colour(tokens.colors.background)
    ));
}

#[test]
fn the_empty_index_scenario_renders_what_the_launcher_looked_for() {
    let tokens = dark();
    let state = state(
        tokens.clone(),
        "",
        Some(ServiceState::Ready {
            generation: 1,
            entries: 0,
            skipped: 0,
            directories: vec![std::path::PathBuf::from("/usr/share")],
        }),
    );
    let pixels = support::render(
        view::view(&state),
        &theme::iced_theme(&tokens),
        "empty-index",
    );

    assert!(state.model.banner.is_some());
    assert_eq!(
        state.model.banner.as_ref().map(|banner| banner.kind),
        Some(BannerKind::Empty),
    );
    assert!(
        state.model.can_retry_index(),
        "the state a re-read can fix offers the re-read",
    );
    assert!(support::contains(
        &pixels,
        theme::colour(tokens.colors.surface)
    ));
}

#[test]
fn a_long_name_is_truncated_rather_than_breaking_the_row() {
    let tokens = dark();
    // A small output: the placement gives a row's label less room, which is the
    // case a long name has to survive.
    let prepared = prepared_on(tokens.clone(), 520, 400);
    let mut long = entry(
        "long.desktop",
        "An Application With A Name That Would Not Fit In Any Reasonable Row At All",
        None,
        "long",
    );
    long.icon = None;
    let mut state = State::with_index(&prepared, AppIndex::from_entries(vec![long], 1), 1);
    send(&mut state, Message::QueryChanged("application".to_owned()));

    let pixels = support::render(view::view(&state), &theme::iced_theme(&tokens), "long-name");

    assert!(state.model.rows[0].truncated);
    assert!(state.model.rows[0].label.ends_with('…'));
    assert_eq!(
        state.model.rows[0].label.chars().count(),
        state.placement.label_chars,
        "a truncated label is exactly as wide as the row's column",
    );
    assert!(
        state.model.rows[0].icon_placeholder,
        "this entry names no icon, so the placeholder stands in for one",
    );
    assert!(support::distinct_colours(&pixels) > 16);
}

#[test]
fn a_long_reason_wraps_inside_the_card_instead_of_running_out_of_it() {
    let tokens = dark();
    let state = state(tokens.clone(), "", None);
    let pixels = support::render(view::view(&state), &theme::iced_theme(&tokens), "dark");

    let (left, right) = card_band(&state);
    let (text_left, top, text_right, bottom) =
        support::bounds(&pixels, theme::colour(tokens.colors.warning))
            .expect("the reason a row cannot be launched is drawn");

    assert!(
        text_left as f32 >= left,
        "the reason starts before the card does: {text_left} < {left}",
    );
    assert!(
        (text_right as f32) <= right,
        "the reason runs past the card instead of wrapping: {text_right} > {right}",
    );
    assert!(
        (bottom - top) as f32 > theme::line_height(&tokens, TextRole::Caption) * 1.5,
        "a sentence this long has to take more than one line, not be cut off",
    );
}

#[test]
fn a_two_hundred_percent_text_scale_stays_inside_its_cards() {
    let mut tokens = dark();
    tokens.typography.scale *= 2.0;
    let state = state(tokens.clone(), "", None);
    let pixels = support::render(view::view(&state), &theme::iced_theme(&tokens), "scale-200");

    let (left, right) = card_band(&state);
    for (what, token) in [
        ("the names and headings", tokens.colors.text),
        ("the reason a row cannot be launched", tokens.colors.warning),
    ] {
        let (text_left, top, text_right, bottom) = support::bounds(&pixels, theme::colour(token))
            .unwrap_or_else(|| panic!("{what} are not drawn at 200 % text"));
        assert!(
            text_left as f32 >= left,
            "{what} start before the card: {text_left} < {left}",
        );
        assert!(
            (text_right as f32) <= right,
            "{what} run past the card at 200 % text: {text_right} > {right}",
        );
        assert!(
            (bottom - top) > 0,
            "{what} are drawn with no height at 200 % text",
        );
    }

    // The name column shrinks with the font, so a name is cut to a column that
    // is half as wide rather than overflowing it.
    assert!(
        state.placement.label_chars > 0,
        "a 200 % scale may not leave a row with no room for a name",
    );
}

#[test]
fn typing_into_the_real_search_field_then_entering_launches_exactly_once() {
    let tokens = dark();
    let prepared = prepared(tokens.clone());
    let mut state = State::with_index(&prepared, AppIndex::from_entries(applications(), 1), 1);
    let theme = theme::iced_theme(&tokens);
    let cursor = support::over_search_field(&tokens);

    // The interface borrows the state while it exists, so the events are
    // delivered in one pass and the messages applied once it is gone — which is
    // what a real application loop does across two frames.
    let mut messages = Vec::new();
    {
        let mut ui = support::build(view::view(&state), SIZE);
        let mut renderer = support::renderer();
        let mut clipboard = iced_core::clipboard::Null;

        let events = [
            // Click the search field: the real widget decides the focus.
            iced_core::Event::Mouse(iced_core::mouse::Event::ButtonPressed(
                iced_core::mouse::Button::Left,
            )),
            key_pressed('e'),
            key_pressed('x'),
        ];

        let _ = ui.update(
            &events,
            cursor,
            &mut renderer,
            &mut clipboard,
            &mut messages,
        );
    }

    assert!(
        !messages.is_empty(),
        "the search field is wired to the query through the real widget tree",
    );
    for message in &messages {
        send(&mut state, message.clone());
    }

    assert_eq!(state.query, "ex", "the widget typed through the component");
    assert_eq!(state.model.rows.len(), 1);
    assert_eq!(state.model.rows[0].label, "Firefox");
    assert_eq!(
        theme.palette().background,
        theme::colour(state.tokens.colors.background),
    );

    // Enter goes through the same binding the window's subscription uses.
    let enter = app::key_binding(&iced_core::keyboard::Key::Named(
        iced_core::keyboard::key::Named::Enter,
    ))
    .expect("Enter is bound");
    let selection = state.selection_generation;
    send(&mut state, enter.clone());
    send(&mut state, enter.clone());
    send(&mut state, enter);

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

/// Applies a message, discarding the task a running window would hand to the
/// executor.
fn send(state: &mut State, message: Message) {
    drop(app::update(state, message));
}

/// A key press with the text a keyboard would produce.
fn key_pressed(character: char) -> iced_core::Event {
    iced_core::Event::Keyboard(iced_core::keyboard::Event::KeyPressed {
        key: iced_core::keyboard::Key::Character(character.to_string().into()),
        modified_key: iced_core::keyboard::Key::Character(character.to_string().into()),
        physical_key: iced_core::keyboard::key::Physical::Code(
            iced_core::keyboard::key::Code::KeyE,
        ),
        location: iced_core::keyboard::Location::Standard,
        modifiers: iced_core::keyboard::Modifiers::default(),
        text: Some(character.to_string().into()),
        repeat: false,
    })
}
