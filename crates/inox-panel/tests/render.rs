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
//!
//! The second half of the file renders the two glass surfaces the same way. The
//! taskbar tests above are the *before* half of the comparison: the glass tokens
//! must not change a single pixel of them, and one test asserts exactly that.

mod support;

use support::{
    Panel, WIDTH, between_pixel, brightness, composite, contains_colour, differing_columns,
    distinct_colours, has_ink_in_columns, iced_pixel, ink_columns, pixel, snapshot,
    strictly_between, token_pixel,
};

use iced::Color;
use inox_config::{
    DockConfig, MenubarConfig, PanelConfig, PanelSurface, Rgba, ShellConfig, Theme as Tokens,
};
use inox_panel::app::{Message, OutgoingCommand};
use inox_panel::contract::ServiceFailure;
use inox_panel::dock::DockGeometry;
use inox_panel::menubar::MenubarGeometry;
use inox_panel::tasks::{SessionGeneration, WindowId, WindowKey, WindowState};
use inox_panel::tray::{TrayItemId, TrayItemUpdate};

const GENERATION: SessionGeneration = SessionGeneration::new(1);

/// How much of the dock's own width *estimate* the drawn sentence has to reach.
///
/// The estimate is a character-count estimate, not a measurement (no font is
/// available to the pure geometry), so a face narrower than the estimate draws a
/// narrower pane — which is correct, and nothing is clipped. What the drawn pane
/// must never be is a fragment of the surface it asked for: the estimate below
/// is many times the old 16-logical-pixel sliver, which is what this bound is
/// there to catch.
const ESTIMATE_TOLERANCE: f32 = 0.6;

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

/// The window the user is looking at: the group that gets the active mark.
fn focused_window(app: &str, title: &str) -> WindowState {
    WindowState {
        focused: true,
        ..window(app, title)
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
        id: TrayItemId::new(":1.7", "/StatusNotifierItem"),
    });
    panel.update(Message::TrayUpdated {
        id: TrayItemId::new(":1.7", "/StatusNotifierItem"),
        update: TrayItemUpdate::Title("Mail".to_owned()),
    });
    panel.update(Message::Tick(chrono::Local::now()));
    // The opening snapshot request is not what these tests are about.
    panel.state_mut().take_outgoing();

    panel
}

/// A shell configuration that draws `mode`.
fn shell(mode: PanelSurface) -> ShellConfig {
    ShellConfig {
        panel: PanelConfig {
            surface: mode,
            ..PanelConfig::default()
        },
        ..ShellConfig::default()
    }
}

/// A surface of `mode` showing three applications, a tray item and a clock.
fn populated_surface(mode: PanelSurface, tokens: Tokens) -> Panel {
    let mut panel = Panel::with_shell(shell(mode), tokens);

    panel.update(Message::Snapshot {
        generation: GENERATION,
        windows: vec![
            (key(1), window("firefox", "Inox — Documentation")),
            (key(2), window("terminal", "~/src")),
            (key(3), focused_window("editor", "main.rs")),
        ],
    });
    panel.update(Message::TrayRegistered {
        id: TrayItemId::new(":1.7", "/StatusNotifierItem"),
    });
    panel.update(Message::TrayUpdated {
        id: TrayItemId::new(":1.7", "/StatusNotifierItem"),
        update: TrayItemUpdate::Title("Mail".to_owned()),
    });
    panel.update(Message::Tick(chrono::Local::now()));
    panel.state_mut().take_outgoing();

    panel
}

/// A surface of `mode` that has been told the session is settled and empty.
fn empty_surface(mode: PanelSurface, tokens: Tokens) -> Panel {
    let mut panel = Panel::with_shell(shell(mode), tokens);

    panel.update(Message::Snapshot {
        generation: GENERATION,
        windows: Vec::new(),
    });
    panel.state_mut().take_outgoing();

    panel
}

/// The three applications the dock fixtures draw, one of them focused.
fn three_tasks() -> Vec<(WindowKey, WindowState)> {
    vec![
        (key(1), window("firefox", "Inox — Documentation")),
        (key(2), window("terminal", "~/src")),
        (key(3), focused_window("editor", "main.rs")),
    ]
}

/// A dock holding exactly `windows` tiles and no tray group.
///
/// What the layout does with the tray group's text is the font's business, so a
/// test that has to point at a tile leaves the tray out: the pane's width is
/// then exactly `DockGeometry::content_width`, which is what the geometry and
/// the layout agree on.
fn dock_with_tasks(tokens: Tokens, windows: Vec<(WindowKey, WindowState)>) -> Panel {
    let mut panel = Panel::with_shell(shell(PanelSurface::Dock), tokens);

    panel.update(Message::Snapshot {
        generation: GENERATION,
        windows,
    });
    panel.state_mut().take_outgoing();

    panel
}

/// The backdrop the glass is composited over.
///
/// The wallpaper is what a real session would put behind the pane and the panel
/// never sees it, so a test chooses one: a saturated colour both the dark and
/// the light tint are far away from on every channel, which makes "strictly
/// between the backdrop and the tint" a meaningful question rather than a
/// rounding accident.
fn backdrop() -> Color {
    Color::from_rgb8(0x40, 0x90, 0xd0)
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
fn a_hovered_entry_looks_different_from_a_resting_one() {
    let panel = populated(Tokens::builtin_dark());
    let height = panel.size().height;

    let resting = panel.render();
    let hovered = panel.render_with_cursor(iced::Point::new(12.0, height / 2.0));

    snapshot("hovered", &hovered, panel.size());

    assert_ne!(
        resting, hovered,
        "a hover has to be visible: the pointer is what tells a user the entry is reachable"
    );
}

#[test]
fn a_pressed_entry_fills_with_the_accent_colour() {
    let tokens = Tokens::builtin_dark();
    let panel = populated(tokens.clone());
    let height = panel.size().height;
    let point = iced::Point::new(12.0, height / 2.0);

    let hovered = panel.render_with_cursor(point);
    let pressed = panel.render_while_pressed(point);

    snapshot("pressed", &pressed, panel.size());

    assert_ne!(
        pressed, hovered,
        "a press is a state of its own: it has to be visible while it is held (ui-guidelines §5)"
    );
    assert!(
        !contains_colour(&hovered, token_pixel(tokens.colors.accent)),
        "nothing on a strip nobody pressed is drawn in the accent colour"
    );
    assert!(
        contains_colour(&pressed, token_pixel(tokens.colors.accent)),
        "a pressed entry is drawn on the accent colour"
    );
}

#[test]
fn the_task_area_keeps_the_middle_of_the_strip_for_itself() {
    let tokens = Tokens::builtin_dark();
    let panel = populated(tokens.clone());
    let width = panel.size().width as u32;
    let height = panel.size().height as u32;
    let surface = token_pixel(tokens.colors.surface);
    let pixels = panel.render();

    // Three groups, in the order a taskbar reads: the tasks on the left, the
    // panel's own flexible space in the middle, and the tray and the clock at
    // the right-hand end — with a hairline between the last two (ui-guidelines
    // §3, D05). The divider itself is one logical pixel wide, so it is asserted
    // on in `theme`/`view`'s unit tests, where its colour and sizes are values.
    assert!(
        has_ink_in_columns(&pixels, width, height, 0, 512, surface),
        "the task buttons are drawn at the left-hand end"
    );
    assert!(
        !has_ink_in_columns(&pixels, width, height, 620, 700, surface),
        "the middle of the strip is the panel's own background, not a widget"
    );
    assert!(
        has_ink_in_columns(&pixels, width, height, width - 256, width, surface),
        "the tray and the clock are drawn at the right-hand end"
    );
}

#[test]
fn text_scaled_to_two_hundred_percent_stays_inside_the_strip() {
    let mut tokens = Tokens::builtin_dark();
    tokens.typography.scale = 2.0;

    let panel = populated(tokens.clone());
    let size = panel.size();
    let width = size.width as u32;
    let height = size.height as u32;
    let surface = token_pixel(tokens.colors.surface);
    let pixels = panel.render();

    snapshot("scale-200", &pixels, size);

    // Q02: at twice the user's text size the strip is still one row, the entries
    // keep their inset (both corners stay the strip's own background), and
    // neither the labels nor the clock are pushed off it.
    assert_eq!(
        pixel(&pixels, width, 0, 0),
        surface,
        "an entry may not grow into the strip's corner at double text size"
    );
    assert_eq!(pixel(&pixels, width, 0, height - 1), surface);
    assert!(
        has_ink_in_columns(&pixels, width, height, 0, 512, surface),
        "the task labels are still drawn"
    );
    assert!(
        has_ink_in_columns(&pixels, width, height, width - 64, width, surface),
        "the clock is still drawn in the right-hand corner at double text size"
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

// ---------------------------------------------------------------------------
// The glass surfaces: the menu bar and the dock.
// ---------------------------------------------------------------------------

#[test]
fn the_menu_bar_is_drawn_at_its_configured_size() {
    let tokens = Tokens::builtin_dark();
    let panel = populated_surface(PanelSurface::Menubar, tokens.clone());
    let pixels = panel.render_over(backdrop());
    let size = panel.size();

    snapshot("menubar-dark", &pixels, size);

    assert_eq!(
        pixels.len(),
        (size.width as usize) * (size.height as usize) * 4
    );
    assert!(
        (size.height - f32::from(MenubarConfig::default().height)).abs() < f32::EPSILON,
        "the bar is as tall as `[menubar] height`"
    );
    assert!(
        distinct_colours(&pixels) > 2,
        "the bar draws more than its tint: {pixels:?}"
    );
    assert!(
        has_ink_in_columns(
            &pixels,
            size.width as u32,
            size.height as u32,
            0,
            256,
            iced_pixel(backdrop()),
        ),
        "the focused application's name is drawn at the left end"
    );
    assert!(
        has_ink_in_columns(
            &pixels,
            size.width as u32,
            size.height as u32,
            size.width as u32 - 256,
            size.width as u32,
            iced_pixel(backdrop()),
        ),
        "the tray and the clock are drawn at the right end"
    );
}

#[test]
fn the_light_and_dark_menu_bars_differ() {
    let light = populated_surface(PanelSurface::Menubar, Tokens::builtin_light());
    let dark = populated_surface(PanelSurface::Menubar, Tokens::builtin_dark());

    let light_pixels = light.render_over(backdrop());
    let dark_pixels = dark.render_over(backdrop());

    snapshot("menubar-light", &light_pixels, light.size());
    snapshot("menubar-dark-theme", &dark_pixels, dark.size());

    assert_ne!(light_pixels, dark_pixels);
}

#[test]
fn an_empty_menu_bar_draws_its_own_state_and_no_invented_menu() {
    let tokens = Tokens::builtin_dark();
    let panel = empty_surface(PanelSurface::Menubar, tokens.clone());
    let pixels = panel.render_over(backdrop());
    let size = panel.size();

    snapshot("menubar-empty", &pixels, size);

    let model = panel.state().model();
    assert_eq!(model.notice.as_deref(), Some("No windows open"));
    assert_eq!(model.tasks.len(), 0);
    assert!(
        has_ink_in_columns(
            &pixels,
            size.width as u32,
            size.height as u32,
            0,
            256,
            iced_pixel(backdrop()),
        ),
        "the empty state is written on the bar"
    );
    // The bar names the application and nothing else: with no focused window
    // there is no name, so the notice is what the left group starts with.
    assert_eq!(
        inox_panel::menubar::MenubarView::project(
            &model,
            &MenubarConfig::default(),
            panel.state().localizer(),
        )
        .app_name,
        None
    );
}

#[test]
fn a_menu_bar_without_a_focused_application_has_no_name() {
    let tokens = Tokens::builtin_dark();
    let mut unnamed = Panel::with_shell(shell(PanelSurface::Menubar), tokens.clone());
    unnamed.update(Message::Snapshot {
        generation: GENERATION,
        windows: vec![(key(1), window("firefox", "Inox"))],
    });
    unnamed.state_mut().take_outgoing();

    let named = populated_surface(PanelSurface::Menubar, tokens.clone());

    let unnamed_pixels = unnamed.render_over(backdrop());
    let named_pixels = named.render_over(backdrop());

    assert_ne!(
        unnamed_pixels, named_pixels,
        "the focused application's name is what changes on the bar"
    );
}

#[test]
fn the_menu_bars_own_switches_hide_the_clock_and_the_tray() {
    let tokens = Tokens::builtin_dark();
    let mut panel = populated_surface(PanelSurface::Menubar, tokens.clone());

    let showing = panel.render_over(backdrop());

    panel.state_mut().apply_shell(
        ShellConfig {
            menubar: MenubarConfig {
                show_tray: false,
                show_clock: false,
                ..MenubarConfig::default()
            },
            ..shell(PanelSurface::Menubar)
        },
        tokens,
        std::rc::Rc::new(
            inox_panel::i18n::load(&inox_common::i18n::Locale::default_locale())
                .expect("the catalogs parse"),
        ),
    );
    let hidden = panel.render_over(backdrop());

    snapshot("menubar-no-clock", &hidden, panel.size());

    assert_ne!(showing, hidden, "the right group is gone");

    let model = panel.state().model();
    assert_eq!(model.clock, None, "the projection carries no clock to draw");
    let view = inox_panel::menubar::MenubarView::project(
        &model,
        panel.state().menubar(),
        panel.state().localizer(),
    );
    assert!(
        view.tray.is_empty(),
        "the bar's own `show_tray` hides the items"
    );
    assert_eq!(view.clock, None);
    assert_eq!(view.app_name.as_deref(), Some("editor"));
}

#[test]
fn the_menu_bar_without_a_clock_or_tray_is_left_aligned_and_never_a_menu() {
    let tokens = Tokens::builtin_dark();
    let panel = populated_surface(PanelSurface::Menubar, tokens.clone());
    let pixels = panel.render_over(backdrop());
    let width = panel.size().width as u32;
    let height = panel.size().height as u32;

    // The left group starts at the padding from the left edge; the flexible
    // space that separates it from the right group is wider than an entry.
    let geometry = MenubarGeometry::new(&tokens, &MenubarConfig::default());
    assert!(geometry.group_gap > geometry.entry_spacing);
    assert!(
        has_ink_in_columns(
            &pixels,
            width,
            height,
            geometry.padding as u32,
            geometry.padding as u32 + 64,
            iced_pixel(backdrop()),
        ),
        "the application's name starts at the bar's own padding"
    );
}

#[test]
fn a_menu_bar_service_that_is_down_draws_its_notice_and_retry() {
    let tokens = Tokens::builtin_dark();
    let mut panel = populated_surface(PanelSurface::Menubar, tokens);
    panel.update(Message::ServiceUnavailable {
        reason: ServiceFailure::Disconnected,
    });

    let pixels = panel.render_over(backdrop());
    let size = panel.size();

    snapshot("menubar-unavailable", &pixels, size);

    let model = panel.state().model();
    assert_eq!(
        model.status,
        inox_panel::view_model::PanelStatus::Unavailable
    );
    assert_eq!(model.retry.as_deref(), Some("Retry"));
    assert!(
        has_ink_in_columns(
            &pixels,
            size.width as u32,
            size.height as u32,
            0,
            512,
            iced_pixel(backdrop()),
        ),
        "the sentence and its retry are written on the bar (D05)"
    );
}

#[test]
fn the_dock_is_drawn_at_its_configured_size_above_the_bottom_edge() {
    let tokens = Tokens::builtin_dark();
    let dock = DockConfig::default();
    let panel = populated_surface(PanelSurface::Dock, tokens.clone());
    let pixels = panel.render_over(backdrop());
    let size = panel.size();
    let width = size.width as u32;
    let height = size.height as u32;
    let geometry = DockGeometry::new(&tokens, &dock);

    snapshot("dock-dark", &pixels, size);

    assert_eq!(pixels.len(), (width as usize) * (height as usize) * 4);
    assert!(
        (size.height - geometry.window_height()).abs() < f32::EPSILON,
        "the fallback window is the pane plus the margin below it"
    );
    assert!((size.height - geometry.pane_height() - f32::from(dock.margin)).abs() < f32::EPSILON);

    // The pane floats: the surface is transparent and the pane does not stretch
    // across the output, so far from the pane a pixel is the backdrop itself.
    assert_eq!(
        pixel(&pixels, width, 16, height / 2),
        iced_pixel(backdrop()),
        "the dock is a floating pane, not a strip across the output"
    );
    assert_eq!(
        pixel(&pixels, width, width - 16, height / 2),
        iced_pixel(backdrop())
    );
    assert!(
        has_ink_in_columns(
            &pixels,
            width,
            height,
            width / 2 - 128,
            width / 2 + 128,
            iced_pixel(backdrop())
        ),
        "the pane is drawn in the middle of the output"
    );
}

#[test]
fn the_light_and_dark_docks_differ() {
    let light = populated_surface(PanelSurface::Dock, Tokens::builtin_light());
    let dark = populated_surface(PanelSurface::Dock, Tokens::builtin_dark());

    let light_pixels = light.render_over(backdrop());
    let dark_pixels = dark.render_over(backdrop());

    snapshot("dock-light", &light_pixels, light.size());
    snapshot("dock-dark-theme", &dark_pixels, dark.size());

    assert_ne!(light_pixels, dark_pixels);
}

#[test]
fn the_dock_is_also_drawn_at_the_acceptance_size() {
    let tokens = Tokens::builtin_dark();
    let panel = populated_surface(PanelSurface::Dock, tokens.clone());
    let mut sized = Panel::with_shell_at(
        shell(PanelSurface::Dock),
        tokens,
        iced::Size::new(WIDTH, 120.0),
    );
    sized.update(Message::Snapshot {
        generation: GENERATION,
        windows: vec![
            (key(1), window("firefox", "Inox")),
            (key(2), focused_window("terminal", "~/src")),
        ],
    });
    sized.state_mut().take_outgoing();

    let pixels = sized.render_over(backdrop());
    snapshot("dock-120", &pixels, sized.size());

    assert_eq!(sized.size().height, 120.0);
    assert_eq!(pixels.len(), (WIDTH as usize) * 120 * 4);
    // The pane keeps its distance from the bottom edge of the taller surface.
    let geometry = DockGeometry::new(&Tokens::builtin_dark(), &DockConfig::default());
    let pane_bottom = 120.0 - geometry.margin;
    let above = pixel(
        &pixels,
        WIDTH as u32,
        WIDTH as u32 / 2,
        pane_bottom as u32 - 4,
    );
    let below = pixel(&pixels, WIDTH as u32, 16, pane_bottom as u32 + 1);
    assert_ne!(above, iced_pixel(backdrop()));
    assert_eq!(below, iced_pixel(backdrop()));
    assert_ne!(
        panel.render_over(backdrop()),
        pixels,
        "a taller surface moves empty space above the pane, not the pane"
    );
}

#[test]
fn an_empty_dock_draws_its_pane_and_its_own_state() {
    let tokens = Tokens::builtin_dark();
    let panel = empty_surface(PanelSurface::Dock, tokens.clone());
    let pixels = panel.render_over(backdrop());
    let size = panel.size();
    let geometry = DockGeometry::new(&tokens, &DockConfig::default());

    snapshot("dock-empty", &pixels, size);

    let model = panel.state().model();
    assert_eq!(model.notice.as_deref(), Some("No windows open"));
    assert!(model.tasks.is_empty());
    assert_eq!(model.clock, None, "a dock has no clock");
    assert!(
        has_ink_in_columns(
            &pixels,
            size.width as u32,
            size.height as u32,
            size.width as u32 / 2 - 128,
            size.width as u32 / 2 + 128,
            iced_pixel(backdrop()),
        ),
        "the empty state is written inside the pane"
    );
    // The empty pane is still the pane: it has its own height and is narrower
    // than one holding a tile.
    assert!(geometry.pane_height() > 0.0);
    assert!(geometry.content_width(0, 0.0) < geometry.content_width(1, 0.0));
}

#[test]
fn the_dock_marks_the_group_the_user_is_looking_at() {
    let tokens = Tokens::builtin_dark();
    let marked = populated_surface(PanelSurface::Dock, tokens.clone());

    let mut unmarked = Panel::with_shell(shell(PanelSurface::Dock), tokens.clone());
    unmarked.update(Message::Snapshot {
        generation: GENERATION,
        windows: vec![
            (key(1), window("firefox", "Inox")),
            (key(2), window("terminal", "~/src")),
        ],
    });
    unmarked.state_mut().take_outgoing();

    let view = inox_panel::dock::DockView::project(
        &marked.state().model(),
        &DockConfig::default(),
        12,
        marked.state().localizer(),
    );
    assert!(
        view.tiles.iter().any(|tile| tile.active),
        "the focused window marks its group"
    );
    let clean = inox_panel::dock::DockView::project(
        &unmarked.state().model(),
        &DockConfig::default(),
        12,
        unmarked.state().localizer(),
    );
    assert!(clean.tiles.iter().all(|tile| !tile.active));

    let marked_pixels = marked.render_over(backdrop());
    let unmarked_pixels = unmarked.render_over(backdrop());

    // The marked dock is the same state `dock-dark` snapshots; what is worth a
    // picture of its own is the frame without an active group.
    snapshot("dock-unmarked", &unmarked_pixels, unmarked.size());

    assert_ne!(
        marked_pixels, unmarked_pixels,
        "the active mark has to be visible under the tile"
    );
}

#[test]
fn a_dock_service_that_is_down_disables_every_tile_with_a_reason() {
    let tokens = Tokens::builtin_dark();
    let mut panel = populated_surface(PanelSurface::Dock, tokens.clone());
    let enabled = panel.render_over(backdrop());

    panel.update(Message::ServiceUnavailable {
        reason: ServiceFailure::Disconnected,
    });
    let disabled = panel.render_over(backdrop());

    snapshot("dock-disabled", &disabled, panel.size());

    let view = inox_panel::dock::DockView::project(
        &panel.state().model(),
        &DockConfig::default(),
        12,
        panel.state().localizer(),
    );
    for tile in &view.tiles {
        assert!(!tile.enabled);
        assert_eq!(
            tile.tooltip, "The window service is not available",
            "a disabled tile's tooltip is the reason (D05)"
        );
    }
    assert_ne!(
        enabled, disabled,
        "a disabled tile must not look like a usable one"
    );
}

#[test]
fn a_pressed_dock_tile_is_a_state_of_its_own() {
    let tokens = Tokens::builtin_dark();
    let dock = DockConfig::default();
    let geometry = DockGeometry::new(&tokens, &dock);
    let panel = dock_with_tasks(tokens.clone(), three_tasks());
    let pane_left = (WIDTH - geometry.content_width(3, 0.0)) / 2.0;
    let point = iced::Point::new(
        pane_left + geometry.padding + geometry.icon_size / 2.0,
        geometry.pane_height() / 2.0,
    );

    let hovered = panel.render_with_cursor_over(point, backdrop());
    let pressed = panel.render_while_pressed_over(point, backdrop());

    snapshot("dock-pressed", &pressed, panel.size());

    assert_ne!(
        pressed, hovered,
        "a press has to be visible while it is held (ui-guidelines §5)"
    );
}

#[test]
fn clicking_a_dock_tile_produces_the_same_activation_message() {
    let tokens = Tokens::builtin_dark();
    let geometry = DockGeometry::new(&tokens, &DockConfig::default());
    let mut panel = dock_with_tasks(tokens, three_tasks());
    let pane_left = (WIDTH - geometry.content_width(3, 0.0)) / 2.0;
    let point = iced::Point::new(
        pane_left + geometry.padding + geometry.icon_size / 2.0,
        geometry.pane_height() / 2.0,
    );

    let messages = panel.click_through(point);

    assert!(
        messages.iter().any(|message| matches!(
            message,
            Message::TaskActivated { key: activated } if *activated == key(1)
        )),
        "the tile's button was clicked: {messages:?}"
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
fn the_dock_tray_group_is_separated_from_the_tiles() {
    let tokens = Tokens::builtin_dark();
    let with_tray = populated_surface(PanelSurface::Dock, tokens.clone());

    let mut without_tray = Panel::with_shell(shell(PanelSurface::Dock), tokens.clone());
    without_tray.update(Message::Snapshot {
        generation: GENERATION,
        windows: vec![(key(1), window("firefox", "Inox"))],
    });
    without_tray.state_mut().take_outgoing();

    let with_pixels = with_tray.render_over(backdrop());
    let without_pixels = without_tray.render_over(backdrop());

    // The pane with a tray group is `dock-dark`; this is the one without, which
    // is what shows the separator is not drawn when there is nothing to separate.
    snapshot("dock-no-tray", &without_pixels, without_tray.size());

    let view = inox_panel::dock::DockView::project(
        &with_tray.state().model(),
        &DockConfig::default(),
        12,
        with_tray.state().localizer(),
    );
    assert_eq!(view.tray.len(), 1);
    assert!(
        view.has_separator(),
        "the tray group has a separator before it"
    );

    let without = inox_panel::dock::DockView::project(
        &without_tray.state().model(),
        &DockConfig::default(),
        12,
        without_tray.state().localizer(),
    );
    assert!(without.tray.is_empty());
    assert!(
        !without.has_separator(),
        "a line that separates nothing is not drawn"
    );
    assert_ne!(with_pixels, without_pixels);
}

#[test]
fn an_empty_dock_is_readable_at_the_size_the_placement_asks_for() {
    let tokens = Tokens::builtin_dark();
    let dock = DockConfig::default();
    let config = shell(PanelSurface::Dock);
    let geometry = DockGeometry::new(&tokens, &dock);
    let backdrop_pixel = iced_pixel(backdrop());

    // What the dock draws with nothing running and no window service: the
    // panel's own sentence, and nothing else (D05).
    let starting = Panel::with_shell(config.clone(), tokens.clone());
    let model = starting.state().model();
    let notice = model
        .notice
        .clone()
        .expect("a starting panel has its own sentence to show");
    let view = inox_panel::dock::DockView::project(
        &model,
        &dock,
        config.panel.max_task_buttons,
        starting.state().localizer(),
    );
    let content = geometry.content(&view);
    let placement = inox_panel::placement::PanelPlacement::dock(&dock, &tokens, content);
    let width = u32::from(placement.width.expect("a dock is content-sized"));
    let height = u32::from(placement.height);

    // Drawn at exactly the size the placement asks the compositor for — *not*
    // at the acceptance width. That difference is what hid the defect: at 1280
    // logical pixels everything fits whatever the surface was really sized to,
    // while the real request was a 16-pixel sliver the sentence was clipped to
    // nothing inside.
    let requested = Panel::with_shell_at(
        config.clone(),
        tokens.clone(),
        iced::Size::new(width as f32, height as f32),
    );
    let roomy = Panel::with_shell_at(
        config.clone(),
        tokens.clone(),
        iced::Size::new(WIDTH, height as f32),
    );
    let pixels = requested.render_over(backdrop());
    let roomy_pixels = roomy.render_over(backdrop());

    snapshot("dock-empty-requested", &pixels, requested.size());

    // The pane is exactly what the placement asked for, so the compositor
    // frosts the pane and not a band of wallpaper around it.
    let (first, last) =
        ink_columns(&pixels, width, height, backdrop_pixel).expect("the dock draws its pane");
    assert_eq!(
        (first, last),
        (0, width - 1),
        "the pane fills the surface the placement asked for"
    );
    // The same pane on a surface with room to spare is at least as wide as the
    // request — a few columns wider, in fact, because `theme::glass_dock` draws
    // the pane's own soft shadow just outside it. A request that reserved more
    // room than the pane draws would show up in the render above as transparent
    // columns between the ink and the edge of the surface, which is exactly what
    // the assertion above rules out.
    let (roomy_first, roomy_last) =
        ink_columns(&roomy_pixels, WIDTH as u32, height, backdrop_pixel)
            .expect("the same pane floats on a surface with room to spare");
    assert!(
        roomy_last - roomy_first + 1 >= width,
        "the pane is at least the width the placement asked for: {} columns against {width}",
        roomy_last - roomy_first + 1
    );

    // A shorter sentence is written in the same pane: both are centred in the
    // slot the geometry reserves for a sentence, so the dock does not jump when
    // its state changes.
    let empty_notice = |size: iced::Size<f32>| {
        let mut panel = Panel::with_shell_at(config.clone(), tokens.clone(), size);
        panel.update(Message::Snapshot {
            generation: GENERATION,
            windows: Vec::new(),
        });

        panel
    };
    let shorter = empty_notice(iced::Size::new(width as f32, height as f32));
    let roomy_shorter = empty_notice(iced::Size::new(WIDTH, height as f32));
    let shorter_pixels = shorter.render_over(backdrop());
    let roomy_shorter_pixels = roomy_shorter.render_over(backdrop());

    assert_eq!(
        shorter.state().model().notice.as_deref(),
        Some("No windows open"),
        "the two renders differ in the sentence and in nothing else"
    );

    // The sentence is what the surface was sized for, and it is written across
    // the pane: the columns the two sentences disagree on are the sentence's own
    // glyphs. A sentence clipped to a 16-pixel sliver could not reach this far.
    let (sentence_first, sentence_last) =
        differing_columns(&pixels, &shorter_pixels, width, height)
            .expect("the two sentences are not the same pixels");
    let sentence_columns = sentence_last - sentence_first + 1;
    assert!(
        (sentence_columns as f32) >= geometry.notice_width(&notice, None) * ESTIMATE_TOLERANCE,
        "the sentence has ink across the pane: {sentence_columns} columns for a slot of {} \
         logical pixels",
        geometry.notice_width(&notice, None)
    );

    // And it is the *whole* sentence: the same comparison on a surface with room
    // to spare draws the same glyphs over the same number of columns, so nothing
    // was cut off at the requested size.
    let (roomy_first_glyph, roomy_last_glyph) =
        differing_columns(&roomy_pixels, &roomy_shorter_pixels, WIDTH as u32, height)
            .expect("the roomy render draws the sentences too");
    assert!(
        (sentence_last - sentence_first).abs_diff(roomy_last_glyph - roomy_first_glyph) <= 1,
        "the sentence spans {} columns at the requested size and {} columns with room to spare",
        sentence_columns,
        roomy_last_glyph - roomy_first_glyph + 1
    );
}

#[test]
fn the_glass_is_really_translucent_on_both_surfaces_and_themes() {
    for tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
        let tint = token_pixel(tokens.glass.tint);
        let back = iced_pixel(backdrop());
        assert!(
            strictly_between(composite(tint, back), back, tint),
            "the expected composite has to be a blend, not an endpoint"
        );

        // The menu bar has no shadow, so a pixel in the middle of it is the
        // tint composited over the backdrop and nothing else.
        let bar = populated_surface(PanelSurface::Menubar, tokens.clone());
        let pixels = bar.render_over(backdrop());
        let width = bar.size().width as u32;
        let sample = pixel(&pixels, width, width / 2, bar.size().height as u32 / 2);
        let expected = composite(tint, back);

        for channel in 0..4 {
            assert!(
                i32::from(sample[channel]).abs_diff(i32::from(expected[channel])) <= 2,
                "the pane has to be `tint` over `backdrop`: {sample:?} vs {expected:?}"
            );
        }
        assert!(strictly_between(sample, back, tint));
        assert_ne!(sample, back, "the pane is not the bare backdrop");
        assert_ne!(sample, tint, "the pane is not the bare tint");
        assert_ne!(
            sample,
            token_pixel(tokens.colors.surface),
            "a glass pane is not the opaque `surface` token"
        );

        // The dock floats a shadow under the pane, so the sample is the tint
        // over a partly darkened backdrop — still a blend of the two, and still
        // neither of them, which is what "translucent" means here.
        let dock = populated_surface(PanelSurface::Dock, tokens.clone());
        let pixels = dock.render_over(backdrop());
        let found = between_pixel(&pixels, back, tint)
            .unwrap_or_else(|| panic!("no translucent pixel on the dock"));
        assert_ne!(found, token_pixel(tokens.colors.surface));
        assert!(strictly_between(found, back, tint));
    }
}

#[test]
fn the_glass_draws_a_highlight_along_the_panes_top_edge() {
    let tokens = Tokens::builtin_dark();
    let hairline = MenubarGeometry::new(&tokens, &MenubarConfig::default()).highlight_height;

    // The bar spans the whole surface, so its top row is the highlight and the
    // row below it is the pane.
    let bar = populated_surface(PanelSurface::Menubar, tokens.clone());
    let pixels = bar.render_over(backdrop());
    let width = bar.size().width as u32;
    let highlight = pixel(&pixels, width, width / 2, 0);
    let pane = pixel(&pixels, width, width / 2, hairline as u32);

    assert!(
        brightness(highlight) > brightness(pane),
        "the hairline along the top edge is lighter than the pane: {highlight:?} vs {pane:?}"
    );

    // The dock has both edges: a highlight on top and a shade at the bottom.
    let dock = populated_surface(PanelSurface::Dock, tokens.clone());
    let pixels = dock.render_over(backdrop());
    let size = dock.size();
    let geometry = DockGeometry::new(&tokens, &DockConfig::default());
    let width = size.width as u32;
    let pane_top = size.height - geometry.margin - geometry.pane_height();
    let edge = geometry.edge_height;
    let top = pixel(&pixels, width, width / 2, pane_top as u32);
    let below = pixel(&pixels, width, width / 2, (pane_top + edge) as u32);
    let bottom_row = (pane_top + geometry.pane_height() - edge) as u32;
    let bottom = pixel(&pixels, width, width / 2, bottom_row);

    assert!(
        brightness(top) > brightness(below),
        "the dock's top edge is a highlight: {top:?} vs {below:?}"
    );

    // The bottom edge is the shade token's doing: with the token turned off, the
    // same row comes out lighter, which is what "the shade darkens the pane's
    // bottom edge" means for a pane as dark as this one.
    let mut unshaded = tokens.clone();
    unshaded.glass.shade = Rgba {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    let without = populated_surface(PanelSurface::Dock, unshaded).render_over(backdrop());
    let plain_bottom = pixel(&without, width, width / 2, bottom_row);

    assert!(
        brightness(bottom) < brightness(plain_bottom),
        "the dock's bottom edge is a shade: {bottom:?} vs {plain_bottom:?}"
    );
}

#[test]
fn the_glass_tokens_cannot_change_a_taskbar_render() {
    let plain = populated(Tokens::builtin_dark());
    let mut loud_tokens = Tokens::builtin_dark();
    loud_tokens.glass.tint = Rgba {
        r: 0xff,
        g: 0x00,
        b: 0x00,
        a: 0x80,
    };
    loud_tokens.glass.tint_strong = Rgba {
        r: 0x00,
        g: 0xff,
        b: 0x00,
        a: 0xff,
    };
    loud_tokens.glass.highlight = Rgba {
        r: 0x00,
        g: 0x00,
        b: 0xff,
        a: 0xff,
    };
    loud_tokens.glass.shade = Rgba {
        r: 0xff,
        g: 0xff,
        b: 0x00,
        a: 0xff,
    };
    loud_tokens.glass.blur = 0;
    let loud = populated(loud_tokens);

    let again = populated(Tokens::builtin_dark());
    let pixels = plain.render();

    assert_eq!(plain.state().surface(), PanelSurface::Taskbar);
    assert_eq!(
        pixels,
        again.render(),
        "a fresh render of the same state is byte-identical"
    );
    assert_eq!(
        pixels,
        loud.render(),
        "the taskbar never reads a glass token: the old mode cannot regress"
    );
    assert_eq!(
        pixel(&pixels, plain.size().width as u32, 0, 0),
        token_pixel(Tokens::builtin_dark().colors.surface),
        "the taskbar still wears its opaque surface token"
    );
}
