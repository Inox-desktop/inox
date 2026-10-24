//! What the settings window draws, asserted on pixels (acceptance Q02, D04, D05).
//!
//! Every test here renders the real widget tree through the tiny-skia renderer
//! the binary uses, at the size acceptance Q02 snapshots are taken at. There is
//! no compositor and no GPU involved, so these are E1 evidence: they prove the
//! window draws what the view model says it draws, not that it appears on a
//! screen.
//!
//! The scenarios are the ones acceptance Q02 asks for — light, dark, and an
//! error state — plus the three the D04 and D05 scenarios name: a control that
//! is disabled has to look different from one that is not, the keyboard's focus
//! has to be visible, and an error has to be drawn in the colour the tokens give
//! for it.
//!
//! Two interactions are driven through the real widget tree rather than through
//! the update loop: a click on the navigation list, and a click on Apply. The
//! second one asks the layout where the button is instead of writing a
//! coordinate down, because the row moves with the number of fields the page
//! holds.

mod support;

use iced::Point;
use inox_config::Theme as Tokens;
use inox_settings::app::Message;
use inox_settings::draft::{Field, Value};
use inox_settings::pages::PageId;
use inox_settings::view::SettingsGeometry;
use inox_settings::view_model::{ControlView, StatusKind};
use support::{
    HEIGHT, WIDTH, Window, count_near, distinct_colours, has_ink, pixel, snapshot, token_pixel,
    widest_block,
};

/// The theme the dark scenarios draw with.
fn dark() -> Tokens {
    Tokens::builtin_dark()
}

/// The theme the light scenarios draw with.
fn light() -> Tokens {
    Tokens::builtin_light()
}

/// A window showing a configuration with values worth drawing.
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
        inox_common::i18n::Locale::default_locale(),
        tokens,
    );

    window.update(Message::PageSelected(PageId::Panel));
    window.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(48),
    });

    window
}

#[test]
fn the_window_is_drawn_at_the_size_the_snapshots_use() {
    let window = populated(light());
    let pixels = window.render();

    snapshot("light", &pixels);

    assert_eq!(
        pixels.len(),
        (WIDTH as usize) * (HEIGHT as usize) * 4,
        "one RGBA pixel per logical pixel"
    );
}

#[test]
fn the_window_wears_the_theme_background_token() {
    let tokens = dark();
    let window = populated(tokens.clone());
    let pixels = window.render();

    // The body's own background, away from the navigation list and from every
    // control, is the background token.
    assert_eq!(
        pixel(&pixels, WIDTH as u32 - 1, 4),
        token_pixel(tokens.colors.background),
        "the window's own background is the background token"
    );
    assert!(
        distinct_colours(&pixels) > 4,
        "the window draws more than its background"
    );

    // The navigation list down the left-hand side is its own surface.
    assert_eq!(pixel(&pixels, 12, 12), token_pixel(tokens.colors.surface));
}

#[test]
fn the_light_and_dark_themes_produce_different_windows() {
    let light_pixels = populated(light()).render();
    let dark_pixels = populated(dark()).render();

    snapshot("light-theme", &light_pixels);
    snapshot("dark-theme", &dark_pixels);

    assert_ne!(light_pixels, dark_pixels);
    assert_eq!(
        pixel(&light_pixels, WIDTH as u32 - 1, 4),
        token_pixel(light().colors.background)
    );
    assert_eq!(
        pixel(&dark_pixels, WIDTH as u32 - 1, 4),
        token_pixel(dark().colors.background)
    );
}

#[test]
fn a_field_the_schema_refuses_is_drawn_in_the_danger_colour() {
    let tokens = dark();
    let clean = populated(tokens.clone()).render();

    let mut refusing = populated(tokens.clone());
    // No click can produce this; a draft can hold it, and what the window has to
    // do is show it (acceptance D04: 错误定位到字段).
    refusing.update(Message::FieldSet {
        field: Field::PanelHeight,
        value: Value::Number(9_000),
    });
    let pixels = refusing.render();

    snapshot("field-error", &pixels);

    let field = refusing
        .state()
        .model()
        .field(Field::PanelHeight)
        .cloned()
        .expect("the height is on the panel page");
    let error = field.error.expect("the value is out of range");
    assert!(error.contains("panel.height"), "sentence `{error}`");

    assert!(
        count_near(&pixels, token_pixel(tokens.colors.danger), 24) > 0,
        "the error is not drawn in the danger colour"
    );
    assert_ne!(pixels, clean, "an error changes what the window looks like");
}

#[test]
fn a_disabled_control_looks_different_from_an_enabled_one() {
    let enabled = populated(dark());
    let enabled_pixels = enabled.render();

    let mut busy = populated(dark());
    // A save in flight disables every control until it lands (D05).
    busy.update(Message::Apply);
    let busy_pixels = busy.render();

    snapshot("disabled", &busy_pixels);

    assert_eq!(busy.state().model().status.kind, StatusKind::Saving);
    assert!(
        busy.state()
            .model()
            .fields
            .iter()
            .all(|field| !field.enabled),
        "every control is disabled while a save is in flight"
    );
    assert_ne!(
        enabled_pixels, busy_pixels,
        "a disabled control must not look like a usable one"
    );
}

#[test]
fn the_keyboards_focus_is_drawn() {
    let mut window = populated(dark());
    let unfocused = window.render();

    window.update(Message::FocusNext);
    window.update(Message::FocusNext);
    window.update(Message::FocusNext);
    window.update(Message::FocusNext);
    window.update(Message::FocusNext);

    let focused = window.render();

    snapshot("focused", &focused);

    assert!(
        window.state().focused().is_some(),
        "the keyboard landed somewhere"
    );
    assert_ne!(
        unfocused, focused,
        "the focus has to be visible, and it is drawn as a colour (Q01)"
    );
}

#[test]
fn the_focus_is_drawn_on_every_entry_the_walk_holds() {
    // The walk is the keyboard's whole map of the window, and the focus is a
    // colour drawn on the control it is on: an entry the keyboard can land on
    // that draws nothing is a keyboard on nothing (Q01). The two states where
    // the walk and the drawing could disagree are the ones a reviewer named — a
    // window nobody has touched, whose Apply and Cancel have nothing to do, and
    // a window whose save is in flight, where the page's controls are disabled.
    fn untouched() -> Window {
        Window::new()
    }

    fn in_flight() -> Window {
        let mut window = populated(dark());
        // A save in flight disables every control until it lands (D05).
        window.update(Message::Apply);

        window
    }

    for (state, build) in [
        ("a window nobody has touched", untouched as fn() -> Window),
        ("a window whose save is in flight", in_flight),
    ] {
        let walk = build().state().model().focus_order;
        assert!(!walk.is_empty(), "{state}: the walk is empty");

        for (index, target) in walk.iter().enumerate() {
            let mut window = build();
            let unfocused = window.render();

            for _ in 0..=index {
                window.update(Message::FocusNext);
            }

            assert_eq!(
                window.state().model().focused,
                Some(*target),
                "{state}: step {index} did not land where the walk says"
            );
            assert_ne!(
                unfocused,
                window.render(),
                "{state}: {target:?} is an entry of the walk and draws no focus"
            );
        }
    }
}

/// A window with a display change the compositor has taken on trial.
///
/// The trial is the state ADR 0028 exists for: the values are in effect, and the
/// window is asking whether to keep them.
fn on_trial(tokens: Tokens) -> Window {
    let mut window = Window::with_locale(&[], inox_common::i18n::Locale::default_locale(), tokens)
        .with_compositor();
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

#[test]
fn a_display_trial_is_drawn_as_a_dialog_over_the_window() {
    let tokens = dark();
    let window = on_trial(tokens.clone());
    assert!(
        window.state().model().trial.is_some(),
        "the compositor took the change, so the window asks"
    );

    let asking = window.render();
    snapshot("display-trial", &asking);

    // The same window with the question answered: the dialog is the difference.
    let mut answered = on_trial(tokens.clone());
    answered.update(Message::TrialConfirm);
    answered.drain();
    let settled = answered.render();
    snapshot("display-trial-confirmed", &settled);

    assert_ne!(
        asking, settled,
        "an unanswered display change has to look different from a settled one"
    );
    assert!(
        window
            .state()
            .model()
            .status
            .message
            .contains("waiting for confirmation"),
        "the status line says the display is waiting: {}",
        window.state().model().status.message
    );
    assert!(
        count_near(&asking, token_pixel(tokens.colors.warning), 24) > 0,
        "the dialog and the status line are drawn in the warning colour"
    );
    assert!(
        has_ink(
            &asking,
            400,
            880,
            280,
            460,
            token_pixel(tokens.colors.background)
        ),
        "the question is drawn in the middle of the window"
    );
}

#[test]
fn clicking_the_confirm_answer_keeps_the_display_settings() {
    let mut window = on_trial(dark());

    // Where the answers are depends on the layout, so the test walks the middle
    // of the window and stops at the first click the dialog answers.
    let mut pressed = None;

    'outer: for step in 0..60 {
        let y = 300.0 + step as f32 * 3.0;

        for column in 0..80 {
            let x = 400.0 + column as f32 * 5.0;

            if window
                .click(Point::new(x, y))
                .iter()
                .any(|message| matches!(message, Message::TrialConfirm))
            {
                pressed = Some(Point::new(x, y));
                break 'outer;
            }
        }
    }

    let point = pressed.expect("the dialog draws a confirm answer");
    window.click_through(point);
    window.drain();

    assert!(
        window.state().model().trial.is_none(),
        "the click answered the question"
    );
    assert_eq!(
        window.reload_config().compositor.outputs.len(),
        1,
        "the confirmed display settings are still there"
    );
}

#[test]
fn the_display_trial_is_answerable_by_keyboard_alone() {
    let mut window = on_trial(dark());

    // The question puts the keyboard on the answer that keeps the change, so one
    // Enter answers it (Q01).
    assert_eq!(
        window
            .state()
            .model()
            .focused
            .map(|target| format!("{target:?}")),
        Some("Trial(Confirm)".to_owned())
    );

    window.update(Message::Activate);
    window.drain();

    assert!(window.state().model().trial.is_none());
    assert_eq!(window.reload_config().compositor.outputs.len(), 1);
}

#[test]
fn a_change_that_is_saved_but_not_in_effect_says_so() {
    let mut window = populated(dark());
    let written = window.apply_and_drain();

    let pixels = window.render();
    snapshot("saved-only", &pixels);

    assert_eq!(written, 1, "the shell file is written");
    assert_eq!(
        window.state().model().status.kind,
        StatusKind::SavedOnly,
        "nothing could confirm it, so the window must not claim it took effect"
    );

    let mut applied = populated(dark());
    applied.update(Message::Apply);
    let saving = applied.render();

    assert_ne!(pixels, saving, "the status line says which state this is");
}

#[test]
fn an_empty_configuration_still_draws_a_whole_window() {
    let tokens = dark();
    let mut window = Window::new();
    window.update(Message::PageSelected(PageId::Notifications));

    let pixels = window.render();

    snapshot("empty-configuration", &pixels);

    assert_eq!(
        window.state().model().fields.len(),
        PageId::Notifications.fields().len(),
        "every field of the page is drawn even when the file says nothing"
    );
    assert!(has_ink(
        &pixels,
        176,
        640,
        16,
        48,
        token_pixel(tokens.colors.background)
    ));
}

#[test]
fn a_long_language_tag_does_not_push_the_control_off_the_row() {
    let tokens = dark();
    let mut window = Window::new();
    window.update(Message::PageSelected(PageId::Locale));
    window.update(Message::FieldSet {
        field: Field::LocaleLanguage,
        value: Value::Choice("zh-Hant-HK-very-long-variant".to_owned()),
    });

    let pixels = window.render();
    snapshot("long-value", &pixels);

    // The three options Inox ships and the file's own tag, all on the row, with
    // the labels cloned into the widgets so nothing the file holds can make the
    // window wider than its size.
    let Some(inox_settings::view_model::ControlView::Choices(options)) = window
        .state()
        .model()
        .field(Field::LocaleLanguage)
        .map(|field| field.control.clone())
    else {
        panic!("the language is a choice");
    };

    assert_eq!(options.len(), 4);
    assert!(
        has_ink(
            &pixels,
            176,
            1279,
            16,
            48,
            token_pixel(tokens.colors.background)
        ),
        "the page heading is still drawn"
    );
}

#[test]
fn clicking_the_navigation_list_opens_a_page() {
    let mut window = Window::new();

    // The navigation column's entries are at the top-left; the test asks the
    // layout where they are rather than writing a coordinate down, by walking
    // down the strip until each page has answered once.
    let mut panel = None;
    let mut opened = Vec::new();

    for step in 0..80 {
        let y = 4.0 + step as f32 * 4.0;
        for message in window.click(Point::new(40.0, y)) {
            if let Message::PageSelected(page) = message {
                opened.push((page, y));
                if page == PageId::Panel && panel.is_none() {
                    panel = Some(y);
                }
            }
        }
    }

    assert!(
        opened
            .iter()
            .any(|(page, _)| *page == PageId::Notifications),
        "the navigation list answered with some page: {opened:?}"
    );

    let y = panel.expect("the panel's entry is in the navigation list");
    window.click_through(Point::new(40.0, y));

    assert_eq!(window.state().page(), PageId::Panel);
    assert_eq!(window.state().model().page_title, "Panel");
}

#[test]
fn clicking_nothing_leaves_the_window_alone() {
    let mut window = populated(dark());

    // The middle of the page body is the window's ordinary display: a click
    // there changes nothing and writes nothing (D01's rule, on this window).
    let messages = window.click(Point::new(900.0, 300.0));

    assert!(messages.is_empty(), "{messages:?}");
    assert!(window.state_mut().take_outgoing().is_empty());
}

#[test]
fn clicking_apply_writes_the_change() {
    let mut window = populated(dark());

    // The action row is at the foot of the window; where exactly depends on the
    // height iced gives the widgets, so the test walks down from the bottom edge
    // and stops at the first click that produces a message.
    let mut pressed = None;

    'outer: for step in 0..24 {
        let y = HEIGHT - 4.0 - step as f32 * 2.0;

        for column in 0..32 {
            let x = 176.0 + column as f32 * 6.0;

            if window
                .click(Point::new(x, y))
                .iter()
                .any(|message| matches!(message, Message::Apply))
            {
                pressed = Some(Point::new(x, y));
                break 'outer;
            }
        }
    }

    let point = pressed.expect("the Apply action is drawn at the foot of the window");
    window.click_through(point);

    assert!(
        matches!(
            window.state().application().state(),
            inox_settings::apply::ApplyState::Saving { .. }
        ),
        "the click became an Apply, and the window is writing"
    );
    assert_eq!(window.drain(), 1, "exactly one file was written");
}

#[test]
fn expanded_pages_render_in_both_locales_with_real_controls() {
    for locale in ["en-US", "zh-CN"] {
        for page in [
            PageId::Input,
            PageId::Layout,
            PageId::Launcher,
            PageId::Wallpaper,
            PageId::Displays,
            PageId::InputMethod,
        ] {
            let mut window = Window::with_locale(
                &[],
                inox_common::i18n::Locale::parse(locale).expect("locale"),
                dark(),
            );
            window.update(Message::PageSelected(page));
            if page == PageId::Displays {
                window.update(Message::RowAdded(Field::Outputs));
            }
            if page == PageId::InputMethod {
                window.update(Message::FieldSet {
                    field: Field::InputMethodCommand,
                    value: Value::Collection(vec![vec!["fcitx5".into()], vec!["--replace".into()]]),
                });
            }
            let pixels = window.render();
            assert!(distinct_colours(&pixels) > 4, "{locale} {page:?}");
            snapshot(&format!("expanded-{locale}-{page:?}"), &pixels);
        }
    }
}

#[test]
fn keyboard_focus_reveals_a_display_cell_below_the_viewport() {
    let mut window = Window::new();
    window.update(Message::PageSelected(PageId::Displays));
    window.update(Message::RowAdded(Field::Outputs));
    window.update(Message::RowAdded(Field::Outputs));
    let before = window.render();
    let target = inox_settings::view_model::FocusTarget::Cell(Field::Outputs, 1, 6);
    let limit = window.state().model().focus_order.len();
    for _ in 0..limit {
        window.update(Message::FocusNext);
        if window.state().focused() == Some(target) {
            break;
        }
    }
    assert_eq!(window.state().focused(), Some(target));
    let after = window.render();
    assert_ne!(
        before, after,
        "keyboard navigation must reveal the selected display cell"
    );
    snapshot("display-keyboard-scroll", &after);
}

#[test]
fn enlarged_english_navigation_remains_reachable_by_keyboard() {
    let mut tokens = dark();
    tokens.typography.scale = 2.0;
    let mut window = Window::with_locale(&[], inox_common::i18n::Locale::default_locale(), tokens);
    let initial = window.render();
    for _ in 0..PageId::ALL.len() {
        window.update(Message::FocusNext);
    }
    assert_eq!(
        window.state().focused(),
        Some(inox_settings::view_model::FocusTarget::Page(
            PageId::InputMethod
        ))
    );
    let enlarged = window.render();
    assert_ne!(initial, enlarged);
    snapshot("enlarged-navigation", &enlarged);
}

#[test]
fn a_short_field_is_not_drawn_across_the_window() {
    let tokens = dark();
    let geometry = SettingsGeometry::new(&tokens);
    let field = token_pixel(tokens.colors.surface_variant);

    // The layout page holds numbers and nothing else, so the widest field drawn
    // in a field's own colour on it is a number with its unit: six extra-large
    // steps at the most (ui-guidelines §4).
    let mut layout = Window::new();
    layout.update(Message::PageSelected(PageId::Layout));
    let pixels = layout.render();
    let measured = widest_block(&pixels, field);

    assert!(
        measured as f32 <= geometry.short_width + 2.0,
        "a number is drawn {measured} pixels wide, more than the {} a short value gets",
        geometry.short_width
    );

    // The panel page also holds one format string. That is a long value and no
    // longer: twelve extra-large steps at the most.
    let pixels = populated(tokens).render();
    let measured = widest_block(&pixels, field);

    assert!(
        measured as f32 <= geometry.wide_width + 2.0,
        "a field is drawn {measured} pixels wide, more than the {} a long value gets",
        geometry.wide_width
    );
    assert!(
        (measured as f32) > geometry.short_width,
        "the format string is the wide field this page holds"
    );
}

#[test]
fn a_hovered_entry_looks_different_from_a_resting_one() {
    let tokens = dark();
    let window = Window::new();

    // Where the entries are is the layout's business, so the test asks it for
    // one rather than writing a coordinate down. It looks for an entry that is
    // not the open page: hovering the open page keeps the accent fill, which is
    // its own rule and the next test's subject.
    let mut entry_y = None;

    for step in 0..80 {
        let y = 4.0 + step as f32 * 4.0;

        if window
            .click(Point::new(40.0, y))
            .iter()
            .any(|message| matches!(message, Message::PageSelected(PageId::Panel)))
        {
            entry_y = Some(y);
            break;
        }
    }

    let y = entry_y.expect("the navigation list draws the panel's entry");
    let resting = window.render();
    // Inside the entry and past its label, so the pixel sampled is the entry's
    // own fill rather than a glyph.
    let hovered = window.render_at(Point::new(138.0, y));

    snapshot("hovered", &hovered);

    assert_eq!(
        pixel(&hovered, 138, y as u32),
        token_pixel(tokens.colors.surface_variant),
        "a hovered entry is drawn on the hover surface"
    );
    assert_ne!(resting, hovered, "a hover has to be visible");
}

#[test]
fn hovering_the_open_page_keeps_its_label_readable() {
    let tokens = dark();
    let window = Window::new();
    let mut entry_y = None;

    for step in 0..80 {
        let y = 4.0 + step as f32 * 4.0;

        if window
            .click(Point::new(40.0, y))
            .iter()
            .any(|message| matches!(message, Message::PageSelected(PageId::Appearance)))
        {
            entry_y = Some(y);
            break;
        }
    }

    let y = entry_y.expect("the navigation list draws the appearance entry");
    let hovered = window.render_at(Point::new(138.0, y));

    assert_eq!(
        pixel(&hovered, 138, y as u32),
        token_pixel(tokens.colors.accent),
        "the entry holding the open page keeps the accent fill under the pointer, \
         so the text on it stays readable"
    );
}

#[test]
fn an_empty_list_says_what_it_will_do() {
    let tokens = dark();
    let mut window = Window::new();
    window.update(Message::PageSelected(PageId::Displays));

    let pixels = window.render();
    snapshot("empty-list", &pixels);

    let model = window.state().model();
    let Some(ControlView::Collection { rows, empty, .. }) =
        model.field(Field::Outputs).map(|field| &field.control)
    else {
        panic!("the displays are a list");
    };

    assert!(rows.is_empty(), "the window holds no display preference");
    assert!(
        !empty.trim().is_empty(),
        "an empty list says what the window will do rather than drawing a blank"
    );
    assert_eq!(
        widest_block(&pixels, token_pixel(tokens.colors.surface_variant)),
        0,
        "an empty list draws no fields to type into"
    );

    let mut with_a_row = Window::new();
    with_a_row.update(Message::PageSelected(PageId::Displays));
    with_a_row.update(Message::RowAdded(Field::Outputs));
    assert_ne!(
        pixels,
        with_a_row.render(),
        "a list with an entry looks different from an empty one"
    );
}

#[test]
fn nothing_is_drawn_past_the_windows_own_padding_at_a_doubled_scale() {
    let mut tokens = dark();
    tokens.typography.scale = 2.0;
    let geometry = SettingsGeometry::new(&tokens);
    let edge = (WIDTH - geometry.page_padding.right).max(0.0) as u32;
    let background = token_pixel(tokens.colors.background);

    for page in [PageId::Panel, PageId::Notifications, PageId::Displays] {
        let mut window = Window::with_locale(
            &[],
            inox_common::i18n::Locale::default_locale(),
            tokens.clone(),
        );
        window.update(Message::PageSelected(page));

        if page == PageId::Displays {
            window.update(Message::RowAdded(Field::Outputs));
        }

        let pixels = window.render();
        snapshot(&format!("enlarged-{page:?}"), &pixels);

        assert!(distinct_colours(&pixels) > 4, "{page:?} is not blank");
        assert!(
            !has_ink(
                &pixels,
                edge + 1,
                WIDTH as u32,
                60,
                HEIGHT as u32,
                background
            ),
            "{page:?} draws past the window's own padding at 200%, which is a scroll \
             the user cannot reach"
        );
    }
}
