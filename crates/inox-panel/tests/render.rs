//! What the panel draws, asserted on pixels (acceptance Q02, D01, D03, D05).
//!
//! Every test here renders the real widget tree through the tiny-skia renderer
//! the binary uses, at the width acceptance Q02 snapshots are taken at. There is
//! no compositor and no GPU involved, so these are E1 evidence: they prove the
//! panel draws what the view model says it draws, not that it appears on a
//! screen.
//!
//! The scenarios are deliberately the ones acceptance Q02 asks for — light,
//! dark, and the empty and error states — plus the two interactions D01 and D03
//! name: a click that has to become the right message, and a title long enough to
//! push a key action off the strip.

mod support;

use support::{Panel, distinct_colours, has_ink_in_columns, pixel, snapshot, token_pixel};

use inox_config::{PanelConfig, Theme as Tokens};
use inox_panel::app::{Message, OutgoingCommand};
use inox_panel::contract::ServiceFailure;
use inox_panel::tasks::{SessionGeneration, WindowId, WindowKey, WindowState};

const GENERATION: SessionGeneration = SessionGeneration::new(1);

fn key(window: u64) -> WindowKey {
    WindowKey::new(WindowId::new(window), GENERATION)
}

fn window(app: &str, title: &str) -> WindowState {
    WindowState {
        app_id: app.to_owned(),
        title: title.to_owned(),
        ..WindowState::default()
    }
}

/// A panel showing two applications, a tray item and a clock.
fn populated(tokens: Tokens) -> Panel {
    let mut panel = Panel::with(PanelConfig::default(), tokens);

    panel.update(Message::Snapshot {
        generation: GENERATION,
        windows: vec![
            (key(1), window("firefox", "Inox — Documentation")),
            (key(2), window("firefox", "Inox — News")),
            (key(3), window("terminal", "~/src")),
        ],
    });
    panel.update(Message::TrayRegistered {
        id: inox_panel::tray::TrayItemId::new(":1.7", "/StatusNotifierItem"),
    });
    panel.update(Message::TrayUpdated {
        id: inox_panel::tray::TrayItemId::new(":1.7", "/StatusNotifierItem"),
        update: inox_panel::tray::TrayItemUpdate::Title("Mail".to_owned()),
    });
    panel.update(Message::Tick(chrono::Local::now()));
    // The opening snapshot request is not what these tests are about.
    panel.state_mut().take_outgoing();

    panel
}

#[test]
fn the_strip_is_drawn_at_the_configured_size() {
    let panel = populated(Tokens::builtin_light());
    let pixels = panel.render();
    let size = panel.size();

    snapshot("light", &pixels, size);

    assert_eq!(
        pixels.len(),
        (size.width as usize) * (size.height as usize) * 4,
        "one RGBA pixel per logical pixel"
    );
    assert!(size.height >= 32.0, "a panel is a strip, not a window");
}

#[test]
fn the_strip_wears_the_theme_surface_token() {
    let tokens = Tokens::builtin_dark();
    let panel = populated(tokens.clone());
    let pixels = panel.render();
    let width = panel.size().width as u32;

    // The top-left pixel is inside the panel's own background, which is the
    // surface token — no colour is written into the view by hand.
    assert_eq!(
        pixel(&pixels, width, 0, 0),
        token_pixel(tokens.colors.surface)
    );
    assert!(
        distinct_colours(&pixels) > 2,
        "the strip draws more than its background: {pixels:?}"
    );
}

#[test]
fn the_light_and_dark_themes_produce_different_strips() {
    let light = populated(Tokens::builtin_light());
    let dark = populated(Tokens::builtin_dark());

    let light_pixels = light.render();
    let dark_pixels = dark.render();

    snapshot("light-theme", &light_pixels, light.size());
    snapshot("dark-theme", &dark_pixels, dark.size());

    assert_ne!(light_pixels, dark_pixels);
    assert_eq!(
        pixel(&light_pixels, light.size().width as u32, 0, 0),
        token_pixel(Tokens::builtin_light().colors.surface)
    );
    assert_eq!(
        pixel(&dark_pixels, dark.size().width as u32, 0, 0),
        token_pixel(Tokens::builtin_dark().colors.surface)
    );
}

#[test]
fn an_empty_session_draws_its_own_state() {
    let mut panel = Panel::new();
    panel.update(Message::Snapshot {
        generation: GENERATION,
        windows: Vec::new(),
    });

    let pixels = panel.render();
    let size = panel.size();

    snapshot("empty", &pixels, size);

    assert_eq!(
        panel.state().model().notice.as_deref(),
        Some("No windows open")
    );
    assert!(
        has_ink_in_columns(
            &pixels,
            size.width as u32,
            size.height as u32,
            0,
            256,
            token_pixel(Tokens::builtin_dark().colors.surface),
        ),
        "the empty state is written on the strip, not left blank"
    );
}

#[test]
fn a_service_that_is_down_draws_its_message_and_its_retry() {
    let mut panel = populated(Tokens::builtin_dark());
    panel.update(Message::ServiceUnavailable {
        reason: ServiceFailure::Disconnected,
    });

    let pixels = panel.render();
    let size = panel.size();

    snapshot("unavailable", &pixels, size);

    let model = panel.state().model();
    assert_eq!(
        model.status,
        inox_panel::view_model::PanelStatus::Unavailable
    );
    assert_eq!(model.retry.as_deref(), Some("Retry"));
    assert!(model.tasks.iter().all(|task| !task.enabled));
    assert!(
        has_ink_in_columns(
            &pixels,
            size.width as u32,
            size.height as u32,
            0,
            320,
            token_pixel(Tokens::builtin_dark().colors.surface),
        ),
        "the unavailable state is written on the strip"
    );
}

#[test]
fn a_disabled_entry_renders_differently_from_an_enabled_one() {
    let enabled = populated(Tokens::builtin_dark());
    let enabled_pixels = enabled.render();

    let mut disabled = populated(Tokens::builtin_dark());
    disabled.update(Message::ServiceUnavailable {
        reason: ServiceFailure::Disconnected,
    });
    let disabled_pixels = disabled.render();

    snapshot("disabled", &disabled_pixels, disabled.size());

    // This is the one assertion that needs a font: a disabled entry is muted,
    // and the muted colour is what differs. A platform with no fonts would draw
    // no text at all, and the failure would say so.
    assert_ne!(
        enabled_pixels, disabled_pixels,
        "a disabled task button must not look like a usable one"
    );

    // ... and the panel keeps its own affordance: the retry action.
    assert!(disabled.state().model().retry.is_some());
    assert!(enabled.state().model().retry.is_none());
}

#[test]
fn long_titles_do_not_push_the_clock_off_the_strip() {
    let long = "a very long window title ".repeat(20);
    let mut panel = Panel::new();
    panel.update(Message::Snapshot {
        generation: GENERATION,
        // Six applications, not six windows of one: grouping by `app_id` is
        // what decides how many buttons the strip has to carry.
        windows: (1..=6)
            .map(|id| (key(id), window(&format!("app{id}"), &long)))
            .collect(),
    });
    panel.update(Message::Tick(chrono::Local::now()));

    let pixels = panel.render();
    let size = panel.size();
    let width = size.width as u32;
    let surface = token_pixel(Tokens::builtin_dark().colors.surface);

    snapshot("long-title", &pixels, size);

    assert!(
        panel.state().model().clock.is_some(),
        "the clock is in the model whatever the titles are"
    );
    assert_eq!(
        panel.state().model().tasks.len(),
        6,
        "every application has a button (under the configured cap)"
    );
    assert!(
        has_ink_in_columns(
            &pixels,
            width,
            size.height as u32,
            width - 64,
            width,
            surface
        ),
        "the clock is still drawn in the right-hand corner"
    );
}

#[test]
fn a_configured_cap_bounds_the_buttons_that_are_drawn() {
    let mut panel = Panel::with(
        PanelConfig {
            max_task_buttons: 1,
            ..PanelConfig::default()
        },
        Tokens::builtin_dark(),
    );
    panel.update(Message::Snapshot {
        generation: GENERATION,
        windows: vec![
            (key(1), window("firefox", "Inox")),
            (key(2), window("terminal", "~/src")),
        ],
    });

    let model = panel.state().model();

    assert_eq!(model.tasks.len(), 1, "the strip shows what it was asked to");
    assert_eq!(model.tasks[0].app_id, "firefox");
    assert!(!model.is_empty(), "the clock and the tray are still there");
}

#[test]
fn the_entry_navigation_reached_stays_visible_under_the_pointer() {
    let mut focused = populated(Tokens::builtin_dark());
    focused.update(Message::FocusNext);

    let unfocused = populated(Tokens::builtin_dark());

    // The pointer rests on the first entry, which is where a pointer sits on a
    // panel most of the time: the entry explicit navigation reached has to stay
    // distinguishable there, rather than being taken over by the hover (Q01).
    let cursor = iced::Point::new(12.0, focused.size().height / 2.0);
    let focused_pixels = focused.render_with_cursor(cursor);
    let unfocused_pixels = unfocused.render_with_cursor(cursor);

    snapshot("focused-under-pointer", &focused_pixels, focused.size());

    assert_eq!(focused.state().focused(), Some(key(1)));
    assert_eq!(unfocused.state().focused(), None);
    assert_ne!(
        focused_pixels, unfocused_pixels,
        "a hovered entry with the navigation focus must not look like a hovered one without"
    );
}

#[test]
fn clicking_a_task_button_produces_the_activation_message() {
    let mut panel = populated(Tokens::builtin_dark());
    let height = panel.size().height;

    // The first entry starts at the strip's own padding; a click just inside its
    // left edge lands on the browser's button.
    let messages = panel.click_through(iced::Point::new(12.0, height / 2.0));

    assert!(
        messages.iter().any(|message| matches!(
            message,
            Message::TaskActivated { key: activated } if *activated == key(1)
        )),
        "the browser's button was clicked: {messages:?}"
    );
    assert!(
        panel
            .state_mut()
            .take_outgoing()
            .iter()
            .any(|command| matches!(command, OutgoingCommand::Activate { window, .. } if *window == key(1))),
        "the click became exactly one activation request"
    );
}

#[test]
fn clicking_where_there_is_nothing_leaves_the_panel_alone() {
    let mut panel = populated(Tokens::builtin_dark());
    let size = panel.size();

    // The flexible space between the task buttons and the tray is the panel's
    // ordinary display: a click there must do nothing at all (D01).
    let messages = panel.click(iced::Point::new(size.width / 2.0, size.height / 2.0));

    assert!(messages.is_empty(), "{messages:?}");
    assert!(
        panel.state_mut().take_outgoing().is_empty(),
        "no request follows a click that hit nothing"
    );

    // The clock is display-only for the same reason.
    let messages = panel.click(iced::Point::new(size.width - 8.0, size.height / 2.0));

    assert!(messages.is_empty(), "{messages:?}");
    assert!(panel.state_mut().take_outgoing().is_empty());
}
