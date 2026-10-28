//! What each surface projects: colours, sizes, sentences and clicks (Q02, D01, D05).
//!
//! These are the assertions the render tests used to make on pixels, made on the
//! scene instead: a colour has to be the token's conversion, a size has to be the
//! one the geometry derives, a sentence has to be the one Fluent returns, and a
//! click has to be the message the application acts on. The one thing they cannot
//! say is "the renderer really drew it" — that is [`super::qml`]'s job, with one
//! real frame through the offscreen host.

use inox_config::{
    DockConfig, MenubarConfig, PanelConfig, PanelPosition, PanelSurface, Rgba, ShellConfig,
    Theme as Tokens,
};

use crate::app::Message;
use crate::dock::DockGeometry;
use crate::layer::{Button, Point, Pointer};
use crate::menubar::MenubarGeometry;
use crate::tasks::{WindowMode, WorkspaceId};
use crate::view::{EntryAction, PanelGeometry};
use crate::view_model::PanelStatus;

use super::support::{
    Panel, add_tray_item, color, empty_surface, has_text, items, key, number, pane_at, panes,
    populated_surface, populated_with, rect_at, rects, sentences, string, text_item, texts, token,
    unavailable_surface, window,
};

// ---------------------------------------------------------------- the taskbar

#[test]
fn the_strip_is_projected_at_the_configured_size() {
    let panel = Panel::new();
    let scene = panel.scene();
    let (width, height) = panel.size();
    let root = rect_at(&scene, 0.0, 0.0).expect("the strip paints its own background");

    assert_eq!(scene.surface, PanelSurface::Taskbar);
    assert!((number(root, "w") - width).abs() < f32::EPSILON);
    assert!((number(root, "h") - height).abs() < f32::EPSILON);
    assert_eq!(number(root, "h"), f32::from(PanelConfig::default().height));
}

#[test]
fn the_strip_wears_the_theme_surface_token() {
    let panel = Panel::new();
    let scene = panel.scene();
    let root = rect_at(&scene, 0.0, 0.0).expect("the strip paints its own background");

    assert_eq!(
        color(root),
        crate::theme::panel_fill(panel.tokens()),
        "the strip is the `surface` token and nothing else"
    );
    assert_eq!(
        scene.json["background"],
        token(Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        }),
        "the surface's own root is structurally transparent; the strip paints over it"
    );
}

#[test]
fn the_light_and_dark_themes_produce_different_strips() {
    let light = Panel::with(PanelConfig::default(), Tokens::builtin_light()).scene();
    let dark = Panel::with(PanelConfig::default(), Tokens::builtin_dark()).scene();

    assert_ne!(color(&items(&light)[0]), color(&items(&dark)[0]));
}

#[test]
fn an_empty_session_projects_its_own_state() {
    let panel = empty_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let scene = panel.scene();
    let notice = panel
        .state()
        .model()
        .notice
        .expect("an empty session says so");

    assert!(
        has_text(&scene, &notice),
        "the panel says what it is doing (D05)"
    );
    assert_eq!(
        color(text_item(&scene, &notice).expect("the notice is drawn")),
        crate::theme::muted_text(panel.tokens()),
        "an ordinary state is secondary information"
    );
    assert!(
        panel.state().model().retry.is_none(),
        "nothing to retry yet"
    );
}

#[test]
fn a_service_that_is_down_projects_its_message_and_its_retry() {
    let panel = unavailable_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let scene = panel.scene();
    let model = panel.state().model();

    assert_eq!(model.status, PanelStatus::Unavailable);
    let notice = model.notice.expect("a service that is down says so");
    let retry = model.retry.expect("and offers a way out");

    assert!(has_text(&scene, &notice));
    assert_eq!(
        color(text_item(&scene, &notice).expect("the notice is drawn")),
        crate::theme::warning_text(panel.tokens()),
        "the state that needs action is a warning, not secondary information"
    );
    assert!(has_text(&scene, &retry));
    assert!(
        scene
            .entries
            .iter()
            .any(|entry| entry.action == EntryAction::Retry),
        "the retry is a click target, not a sentence"
    );
}

#[test]
fn a_disabled_entry_projects_differently_from_an_enabled_one() {
    let tokens = Tokens::builtin_dark();
    let enabled = populated_surface(PanelSurface::Taskbar, tokens.clone());
    let mut disabled = populated_surface(PanelSurface::Taskbar, tokens.clone());
    // The service goes away: the panel keeps the tiles it last knew and lays them
    // out disabled with a reason (D05), which is the state a disabled entry is
    // drawn in.
    disabled.update(Message::ServiceUnavailable {
        reason: crate::contract::ServiceFailure::Disconnected,
    });

    let enabled_scene = enabled.scene();
    let disabled_scene = disabled.scene();
    // One application's button, in both scenes: the notice shifts the layout, so
    // the position cannot be the identity.
    let action = EntryAction::Activate(key(1));
    let enabled_entry = enabled_scene
        .entries
        .iter()
        .find(|entry| entry.action == action)
        .expect("the application has a button to click");
    let disabled_entry = disabled_scene
        .entries
        .iter()
        .find(|entry| entry.action == action)
        .expect("the button is still laid out when the service is away");

    assert!(enabled_entry.enabled);
    assert!(
        !disabled_entry.enabled,
        "a service that is down disables the buttons that would talk to it"
    );

    let enabled_style = rect_at(
        &enabled_scene,
        enabled_entry.x + 1.0,
        enabled_entry.y + enabled_entry.height / 2.0,
    )
    .expect("an enabled entry is drawn");
    let disabled_style = rect_at(
        &disabled_scene,
        disabled_entry.x + 1.0,
        disabled_entry.y + disabled_entry.height / 2.0,
    )
    .expect("a disabled entry is still laid out");

    assert_eq!(
        color(disabled_style),
        crate::theme::TRANSPARENT,
        "a disabled entry has no fill"
    );
    assert_eq!(number(disabled_style, "border"), 0.0, "and no outline");
    assert_eq!(
        number(enabled_style, "border"),
        crate::theme::hairline(&tokens),
        "an entry that can be clicked carries the hairline outline"
    );
    assert_ne!(
        disabled_style, enabled_style,
        "a disabled entry has to look different from an enabled one"
    );
}

#[test]
fn long_titles_do_not_push_the_clock_off_the_strip() {
    let tokens = Tokens::builtin_dark();
    let mut panel = populated_surface(PanelSurface::Taskbar, tokens.clone());
    panel.update(Message::Tick(chrono::Local::now()));

    let long = "A window title that would fill the whole panel if it were allowed";
    let windows: Vec<_> = (10..14)
        .map(|window_id| (key(window_id), window(window_id.to_string().as_str(), long)))
        .collect();
    panel.update(Message::Snapshot {
        generation: super::support::generation(),
        windows,
    });

    let scene = panel.scene();
    let geometry = PanelGeometry::new(&tokens, panel.state().config());
    let (width, _) = panel.size();
    let clock = panel.state().model().clock.expect("the clock is shown");
    let item = text_item(&scene, &clock.text).expect("the clock is drawn");

    assert!(
        number(item, "x") + number(item, "w") <= width - geometry.padding + 0.01,
        "the clock keeps its space whatever an application calls itself (D05)"
    );
}

#[test]
fn a_configured_cap_bounds_the_buttons_that_are_projected() {
    let mut panel = Panel::with(
        PanelConfig {
            max_task_buttons: 2,
            ..PanelConfig::default()
        },
        Tokens::builtin_dark(),
    );

    panel.update(Message::Snapshot {
        generation: super::support::generation(),
        windows: super::support::three_tasks(),
    });

    assert_eq!(panel.scene().entries.len(), 2, "the cap is the surface's");
}

#[test]
fn the_entry_navigation_reached_is_visibly_different() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let resting = panel.scene();
    let entry = resting.entries[0];
    let resting_style = rect_at(&resting, entry.x + 1.0, entry.y + entry.height / 2.0)
        .map(|item| color(item).to_owned())
        .expect("the entry is drawn");

    panel.update(Message::FocusNext);

    let focused = panel.scene();
    let focused_style = rect_at(&focused, entry.x + 1.0, entry.y + entry.height / 2.0)
        .map(|item| color(item).to_owned())
        .expect("the entry is drawn");

    assert_ne!(
        resting_style, focused_style,
        "the entry explicit navigation reached has to be visible (D01)"
    );
    assert_eq!(
        focused_style,
        token(panel.tokens().colors.surface_variant),
        "the focus takes over the background"
    );
}

#[test]
fn a_hovered_entry_projects_the_hover_state() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let entry = panel.scene().entries[0];
    let point = Point::new(entry.x + 1.0, entry.y + entry.height / 2.0);

    panel.hover(point);

    let scene = panel.scene();
    let style = rect_at(&scene, point.x, point.y).expect("the entry is drawn");

    assert_eq!(
        color(style),
        token(panel.tokens().colors.surface_variant),
        "a hover is visible while it lasts (ui-guidelines §5)"
    );
    assert!(number(style, "border") > 0.0, "and it keeps its outline");
}

#[test]
fn a_pressed_entry_fills_with_the_accent_colour() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let entry = panel.scene().entries[0];
    let point = Point::new(entry.x + 1.0, entry.y + entry.height / 2.0);

    panel.press(point);

    let scene = panel.scene();
    let style = rect_at(&scene, point.x, point.y).expect("the entry is drawn");

    assert_eq!(
        color(style),
        token(panel.tokens().colors.accent),
        "a press is a state of its own, not a hover"
    );
}

#[test]
fn text_scaled_to_two_hundred_percent_stays_inside_the_strip() {
    let mut tokens = Tokens::builtin_dark();
    tokens.typography.scale = 2.0;
    let panel = populated_surface(PanelSurface::Taskbar, tokens);

    let scene = panel.scene();
    let (_, height) = panel.size();
    let label = text_item(&scene, "Inox — Documentation")
        .or_else(|| {
            texts(&scene)
                .into_iter()
                .find(|item| number(item, "size") > 0.0)
        })
        .expect("a task label is drawn");

    assert!(
        number(label, "size") <= height,
        "scaled text never grows past the strip"
    );
}

#[test]
fn clicking_a_task_button_produces_the_activation_message() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let entry = panel.scene().entries[0];
    let point = Point::new(entry.x + entry.width / 2.0, entry.y + entry.height / 2.0);

    let messages = panel.click(point);

    assert!(
        matches!(messages.as_slice(), [Message::TaskActivated { key: activated }] if *activated == key(1)),
        "the click became the task activation: {messages:?}"
    );
}

#[test]
fn clicking_where_there_is_nothing_leaves_the_panel_alone() {
    let mut panel = empty_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let (width, height) = panel.size();

    let messages = panel.click(Point::new(width / 2.0, height / 2.0));

    assert!(
        messages.is_empty(),
        "a click on nothing means nothing (D01)"
    );
}

#[test]
fn the_tray_group_sits_between_the_tasks_and_the_clock() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    add_tray_item(&mut panel, "Mail");
    panel.update(Message::Tick(chrono::Local::now()));

    let scene = panel.scene();
    let tray = text_item(&scene, "Mail").expect("the tray item is drawn");
    let clock = panel.state().model().clock.expect("the clock is shown");
    let clock = text_item(&scene, &clock.text).expect("the clock is drawn");

    assert!(
        number(tray, "x") + number(tray, "w") <= number(clock, "x"),
        "the tray stays left of the clock"
    );
    assert!(
        rects(&scene)
            .iter()
            .any(|item| number(item, "w") == crate::theme::hairline(panel.tokens())),
        "a hairline separates what the user acts on from what the panel reports"
    );
}

#[test]
fn the_workspace_indicator_sits_at_the_left_of_the_task_area() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    panel.update(Message::WorkspacesChanged {
        active: WorkspaceId::new(1),
        count: 3,
    });

    let scene = panel.scene();
    let indicator = panel
        .state()
        .model()
        .workspace
        .expect("three workspaces have an indicator");
    let label = text_item(&scene, &indicator.label).expect("the indicator is drawn");
    let first_task = text_item(&scene, "Inox — Documentation").expect("the first task is drawn");

    assert!(
        number(label, "x") < number(first_task, "x"),
        "the indicator opens the task area: it says which workspace the buttons belong to \
         (ADR 0045 §4)"
    );
    assert_eq!(label["text"], "2/3");

    let entry = scene
        .entries
        .iter()
        .find(|entry| entry.action == EntryAction::SwitchWorkspace(WorkspaceId::new(2)))
        .expect("the indicator is a click target, not a sentence");
    assert!(
        (entry.x - number(label, "x")).abs() <= f32::from(panel.tokens().spacing.md),
        "the click target is the box the label is drawn in"
    );
}

#[test]
fn a_one_workspace_desktop_projects_no_indicator() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    panel.update(Message::WorkspacesChanged {
        active: WorkspaceId::new(0),
        count: 1,
    });

    let scene = panel.scene();

    assert_eq!(panel.state().model().workspace, None);
    assert!(
        !scene
            .entries
            .iter()
            .any(|entry| matches!(entry.action, EntryAction::SwitchWorkspace(_))),
        "a control that cannot change anything is not drawn"
    );
}

#[test]
fn clicking_the_workspace_indicator_asks_for_the_next_workspace() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    panel.update(Message::WorkspacesChanged {
        active: WorkspaceId::new(0),
        count: 3,
    });

    let entry = panel
        .scene()
        .entries
        .into_iter()
        .find(|entry| matches!(entry.action, EntryAction::SwitchWorkspace(_)))
        .expect("the indicator is clickable");
    let point = Point::new(entry.x + entry.width / 2.0, entry.y + entry.height / 2.0);

    let messages = panel.click(point);

    assert!(
        matches!(
            messages.as_slice(),
            [Message::WorkspaceSwitched { workspace }] if *workspace == WorkspaceId::new(1)
        ),
        "the click asks for the next workspace: {messages:?}"
    );
}

#[test]
fn a_task_entry_answers_close_and_mode_cycle_on_the_other_buttons() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    // The first group is drawn from `firefox`'s first window, which is tiled.
    let entry = panel.scene().entries[0];
    let point = Point::new(entry.x + entry.width / 2.0, entry.y + entry.height / 2.0);

    panel.pointer(Pointer::Enter(point));
    panel.pointer(Pointer::Press(Button::Middle));
    let closed = panel.pointer(Pointer::Release(Button::Middle)).messages;

    assert!(
        matches!(closed.as_slice(), [Message::TaskClosed { key: closed }] if *closed == key(1)),
        "a middle click asks the application to close: {closed:?}"
    );

    panel.pointer(Pointer::Press(Button::Right));
    let cycled = panel.pointer(Pointer::Release(Button::Right)).messages;

    assert!(
        matches!(
            cycled.as_slice(),
            [Message::TaskModeCycled { key: cycled, mode }]
                if *cycled == key(1) && *mode == WindowMode::Floating
        ),
        "a right click asks for the next mode in the cycle: {cycled:?}"
    );
}

#[test]
fn a_left_click_keeps_the_semantics_the_taskbar_always_had() {
    // The middle and right buttons were added, the left one was not changed:
    // an unfocused window is activated and a focused one is minimized (D03).
    let panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let activate = panel.scene().entries[0];
    let toggle = panel
        .scene()
        .entries
        .into_iter()
        .find(|entry| matches!(entry.action, EntryAction::Toggle(_)))
        .expect("the focused application's button inverts the click");

    assert!(matches!(activate.action, EntryAction::Activate(_)));
    assert!(matches!(toggle.action, EntryAction::Toggle(_)));
    assert_eq!(
        activate
            .middle
            .map(|action| matches!(action, EntryAction::Close(_))),
        Some(true)
    );
    assert_eq!(
        toggle
            .middle
            .map(|action| matches!(action, EntryAction::Close(_))),
        Some(true),
        "closing is a property of the window, not of whether it has focus"
    );
}

#[test]
fn a_side_button_on_a_task_entry_means_nothing() {
    let mut panel = populated_surface(PanelSurface::Taskbar, Tokens::builtin_dark());
    let entry = panel.scene().entries[0];
    let point = Point::new(entry.x + entry.width / 2.0, entry.y + entry.height / 2.0);

    panel.pointer(Pointer::Enter(point));
    panel.pointer(Pointer::Press(Button::Back));
    let released = panel.pointer(Pointer::Release(Button::Back));

    assert!(
        released.messages.is_empty(),
        "the panel has no meaning for a side button, so it invents none: {:?}",
        released.messages
    );
}

// --------------------------------------------------------------- the menu bar

#[test]
fn the_menu_bar_is_projected_at_its_configured_size() {
    let tokens = Tokens::builtin_dark();
    let config = MenubarConfig::default();
    let geometry = MenubarGeometry::new(&tokens, &config);
    let panel = populated_surface(PanelSurface::Menubar, tokens);
    let scene = panel.scene();
    let (width, height) = panel.size();

    assert_eq!(scene.surface, PanelSurface::Menubar);
    assert!(
        (height - (geometry.height + geometry.margin)).abs() < f32::EPSILON,
        "the surface is the bar plus the margin above it"
    );

    let pane = pane_at(&scene, width / 2.0, geometry.margin + geometry.height / 2.0)
        .expect("the bar paints its pane");

    assert!(
        (number(pane, "w") - width).abs() < f32::EPSILON,
        "the bar spans the output"
    );
    assert!((number(pane, "y") - geometry.margin).abs() < f32::EPSILON);
}

#[test]
fn the_light_and_dark_menu_bars_differ() {
    let light = populated_surface(PanelSurface::Menubar, Tokens::builtin_light()).scene();
    let dark = populated_surface(PanelSurface::Menubar, Tokens::builtin_dark()).scene();

    assert_ne!(color(panes(&light)[0]), color(panes(&dark)[0]));
}

#[test]
fn an_empty_menu_bar_projects_its_own_state_and_no_invented_menu() {
    let panel = empty_surface(PanelSurface::Menubar, Tokens::builtin_dark());
    let scene = panel.scene();
    let notice = panel
        .state()
        .model()
        .notice
        .expect("an empty session says so");

    assert!(has_text(&scene, &notice));

    for invented in ["File", "Edit", "View", "Help", "Window"] {
        assert!(
            !has_text(&scene, invented),
            "there is no application-menu protocol, so `{invented}` cannot be drawn"
        );
    }
}

#[test]
fn the_menu_bar_names_the_focused_application() {
    let panel = populated_surface(PanelSurface::Menubar, Tokens::builtin_dark());
    let scene = panel.scene();
    let name = text_item(&scene, "Inkscape")
        .or_else(|| {
            sentences(&scene)
                .into_iter()
                .find(|sentence| sentence.contains("Inkscape"))
                .and_then(|sentence| text_item(&scene, &sentence))
        })
        .expect("the bar shows the focused application's name");

    assert_eq!(
        name["bold"].as_bool(),
        Some(true),
        "the name is emphasized by weight (ui-guidelines §2)"
    );
}

#[test]
fn the_menu_bars_own_switches_hide_the_clock_and_the_tray() {
    // The same bar twice: once with the `[menubar]` switches on, once with both
    // off. The projection is the only difference, so the pair isolates what the
    // switches do.
    let bar = |show_clock: bool, show_tray: bool| {
        let mut shell = ShellConfig::default();
        shell.panel.surface = PanelSurface::Menubar;
        shell.menubar.show_clock = show_clock;
        shell.menubar.show_tray = show_tray;

        let mut panel = Panel::with_shell(shell, Tokens::builtin_dark());
        add_tray_item(&mut panel, "Mail");
        panel.update(Message::Tick(chrono::Local::now()));

        let clock = panel.state().clock().map(|clock| clock.text.clone());

        (panel.scene(), clock)
    };

    let (shown, clock) = bar(true, true);
    let clock = clock.expect("the bar reads the clock when its switch is on");

    assert!(has_text(&shown, &clock), "the clock is drawn when it is on");
    assert!(has_text(&shown, "Mail"), "and so is the tray");

    let (hidden, hidden_clock) = bar(false, false);

    assert!(
        hidden_clock.is_none(),
        "a switched-off clock is not read at all, so there is nothing to draw"
    );
    assert!(
        !has_text(&hidden, &clock),
        "no clock is drawn when `show_clock` is off"
    );
    assert!(
        !has_text(&hidden, "Mail"),
        "and the tray group is gone when `show_tray` is off"
    );
}

#[test]
fn a_menu_bar_service_that_is_down_projects_its_notice_and_retry() {
    let mut panel = unavailable_surface(PanelSurface::Menubar, Tokens::builtin_dark());
    let scene = panel.scene();
    let model = panel.state().model();
    let retry = model.retry.expect("the notice offers a way out");
    let entry = scene
        .entries
        .iter()
        .find(|entry| entry.action == EntryAction::Retry)
        .copied()
        .expect("the retry is a click target");
    let point = Point::new(entry.x + 1.0, entry.y + entry.height / 2.0);

    assert!(has_text(&scene, &retry));

    let messages = panel.click(point);

    assert!(
        matches!(messages.as_slice(), [Message::Retry]),
        "clicking the retry asks the service again: {messages:?}"
    );
}

// -------------------------------------------------------------------- the dock

#[test]
fn the_dock_is_projected_above_the_bottom_edge() {
    let tokens = Tokens::builtin_dark();
    let geometry = DockGeometry::new(&tokens, &DockConfig::default());
    let panel = populated_surface(PanelSurface::Dock, tokens);
    let scene = panel.scene();
    let (_, height) = panel.size();
    let pane =
        pane_at(&scene, 1.0, height - geometry.margin - 1.0).expect("the dock paints its pane");

    assert_eq!(scene.surface, PanelSurface::Dock);
    assert!((height - geometry.window_height()).abs() < f32::EPSILON);
    assert!(
        (number(pane, "y") + number(pane, "h") - (height - geometry.margin)).abs() < f32::EPSILON,
        "the pane floats `[dock] margin` above the bottom edge"
    );
    assert!(
        number(pane, "radius") > 0.0,
        "a floating pane takes the group radius"
    );
}

#[test]
fn the_light_and_dark_docks_differ() {
    let light = populated_surface(PanelSurface::Dock, Tokens::builtin_light()).scene();
    let dark = populated_surface(PanelSurface::Dock, Tokens::builtin_dark()).scene();

    assert_ne!(color(panes(&light)[0]), color(panes(&dark)[0]));
}

#[test]
fn an_empty_dock_draws_its_pane_and_its_own_state() {
    let tokens = Tokens::builtin_dark();
    let panel = empty_surface(PanelSurface::Dock, tokens);
    let scene = panel.scene();
    let notice = panel
        .state()
        .model()
        .notice
        .expect("the dock says what it is doing");

    assert!(!panes(&scene).is_empty(), "an empty dock is still a pane");
    assert!(
        has_text(&scene, &notice),
        "and it says why it is empty rather than being a bare pill"
    );
}

#[test]
fn the_dock_marks_the_group_the_user_is_looking_at() {
    let tokens = Tokens::builtin_dark();
    let panel = populated_surface(PanelSurface::Dock, tokens.clone());
    let scene = panel.scene();
    let accent = crate::theme::accent(&tokens);

    // A click on the group the user is already looking at puts it away (D03);
    // one on any other brings it forward.
    let active = scene
        .entries
        .iter()
        .find(|entry| entry.action == EntryAction::Toggle(key(2)))
        .expect("the focused group is the one a click puts away");
    let idle = scene
        .entries
        .iter()
        .find(|entry| entry.action == EntryAction::Activate(key(1)))
        .expect("an application that is not in use is activated");

    let mark_of = |entry: &crate::view::Entry| {
        rect_at(
            &scene,
            entry.x + entry.width / 2.0,
            entry.y + entry.height - 1.0,
        )
        .map(|item| color(item).to_owned())
        .expect("the mark's slot is laid out for every tile")
    };

    assert_eq!(
        mark_of(active),
        accent,
        "the group the compositor reports as focused carries the accent mark"
    );
    assert_eq!(
        mark_of(idle),
        crate::theme::TRANSPARENT,
        "an inactive mark still occupies its slot, drawing nothing"
    );
    assert_eq!(
        color(rect_at(&scene, active.x + 1.0, active.y + 1.0).expect("a tile is a filled square")),
        accent,
        "a dock tile is the accent badge, not a plain label"
    );
}

#[test]
fn a_dock_service_that_is_down_disables_every_tile_with_a_reason() {
    let mut panel = populated_surface(PanelSurface::Dock, Tokens::builtin_dark());

    // The tiles are still there — the panel keeps what it last knew — but the
    // service is gone, so none of them can be acted on (D05).
    panel.update(Message::ServiceUnavailable {
        reason: crate::contract::ServiceFailure::Disconnected,
    });
    let scene = panel.scene();

    // The retry the notice offers is a live control by design; what has to be
    // disabled is every tile, because those are the buttons that would send the
    // service a request nobody will answer (D05).
    let tiles: Vec<&crate::view::Entry> = scene
        .entries
        .iter()
        .filter(|entry| !matches!(entry.action, EntryAction::Retry))
        .collect();

    assert!(!tiles.is_empty(), "the tiles are laid out");
    assert!(
        tiles.iter().all(|tile| !tile.enabled),
        "a service that is down cannot be acted on through its tiles"
    );

    let entry = *tiles[0];
    let messages = panel.click(Point::new(
        entry.x + entry.width / 2.0,
        entry.y + entry.height / 2.0,
    ));

    assert!(
        messages.is_empty(),
        "a disabled entry does nothing at all (D05): {messages:?}"
    );
    assert!(
        panel
            .state()
            .model()
            .tasks
            .iter()
            .all(|task| task.disabled_reason.is_some()),
        "and the projection gives every one of them a readable reason"
    );
}

#[test]
fn clicking_a_dock_tile_produces_the_same_activation_message() {
    let mut panel = populated_surface(PanelSurface::Dock, Tokens::builtin_dark());
    let entry = panel.scene().entries[0];
    let point = Point::new(entry.x + entry.width / 2.0, entry.y + entry.height / 2.0);

    let messages = panel.click(point);

    assert!(
        matches!(messages.as_slice(), [Message::TaskActivated { key: activated }] if *activated == key(1)),
        "a dock tile means what a taskbar button means: {messages:?}"
    );
}

#[test]
fn the_dock_tray_group_is_separated_from_the_tiles() {
    let tokens = Tokens::builtin_dark();
    let mut panel = populated_surface(PanelSurface::Dock, tokens.clone());
    add_tray_item(&mut panel, "Mail");

    let scene = panel.scene();
    let geometry = DockGeometry::new(&tokens, &DockConfig::default());
    let divider = rects(&scene)
        .into_iter()
        .find(|item| {
            (number(item, "w") - geometry.separator_width).abs() < f32::EPSILON
                && (number(item, "h") - geometry.separator_height).abs() < f32::EPSILON
        })
        .expect("a hairline separates the tiles from the tray");
    let tray = text_item(&scene, "Mail").expect("the tray item is drawn");

    assert!(
        number(divider, "x") < number(tray, "x"),
        "the separator opens the group it belongs to"
    );
}

// ------------------------------------------------------------- the glass pane

#[test]
fn the_glass_is_really_translucent_on_both_surfaces_and_themes() {
    for tokens in [Tokens::builtin_light(), Tokens::builtin_dark()] {
        for mode in [PanelSurface::Menubar, PanelSurface::Dock] {
            let panel = populated_surface(mode, tokens.clone());
            let scene = panel.scene();
            let tint = crate::theme::glass_tint(&tokens, false);
            let alpha = u8::from_str_radix(&tint[1..3], 16).expect("a colour channel");

            assert!(
                items(&scene).iter().any(|item| color(item) == tint),
                "{mode:?}: the pane is the glass tint, not a colour of its own"
            );
            assert!(
                alpha < 0xff,
                "{mode:?}: the tint is translucent, so the backdrop shows through"
            );
            assert_ne!(
                tint,
                crate::theme::panel_fill(&tokens),
                "{mode:?}: a glass surface is not the opaque `surface` token"
            );
        }
    }
}

#[test]
fn the_glass_draws_a_highlight_along_the_panes_top_edge() {
    let tokens = Tokens::builtin_dark();

    for mode in [PanelSurface::Menubar, PanelSurface::Dock] {
        let panel = populated_surface(mode, tokens.clone());
        let scene = panel.scene();
        // The pane is one `pane` item now, drawn by the shared `GlassPane`
        // component: it carries both edge colours and the hairline width, and the
        // component draws them along the pane's own top and bottom edges. The
        // panel no longer emits a separate band per edge — that was the drawing
        // the component library took over (ADR 0040 §3).
        let pane = items(&scene)
            .iter()
            .find(|item| item["kind"].as_str() == Some("pane"))
            .expect("the glass surface projects its pane");

        assert_eq!(
            string(pane, "highlight"),
            crate::theme::glass_highlight(&tokens),
            "{mode:?}: the pane carries the highlight token along its top edge"
        );
        assert_eq!(
            string(pane, "shade"),
            crate::theme::glass_shade(&tokens),
            "{mode:?}: and the shade token along its bottom edge"
        );
        assert!(
            number(pane, "hairline") > 0.0,
            "{mode:?}: the two edges are drawn one hairline thick"
        );
        assert!(
            (number(pane, "y") - number(pane, "y")).abs() < f32::EPSILON,
            "{mode:?}: the highlight is the pane's own top edge, not a row above it"
        );
    }

    // The menu bar is anchored to the screen edge: it has nothing under its
    // bottom edge to darken against, so it draws no shade at all.
    let menubar = populated_surface(PanelSurface::Menubar, tokens.clone()).scene();
    let pane = items(&menubar)
        .iter()
        .find(|item| item["kind"].as_str() == Some("pane"))
        .expect("the menu bar projects its pane");

    assert!(
        !pane["shaded"].as_bool().unwrap_or(true),
        "a bar that reaches the screen edge draws no bottom shade"
    );

    // The dock floats, so it has both edges and the group radius.
    let dock = populated_surface(PanelSurface::Dock, tokens.clone()).scene();
    let pane = items(&dock)
        .iter()
        .find(|item| item["kind"].as_str() == Some("pane"))
        .expect("the dock projects its pane");

    assert!(
        pane["shaded"].as_bool().unwrap_or(false),
        "a floating pane is shaded along its bottom edge"
    );
    assert!(
        (number(pane, "radius") - crate::theme::radius_md(&tokens)).abs() < f32::EPSILON,
        "the pill's corners are the `radii.md` token"
    );
}

#[test]
fn the_glass_tokens_cannot_change_a_taskbar_render() {
    // The taskbar is an opaque strip: it never becomes a pane, whatever the
    // glass tokens say.
    let mut tokens = Tokens::builtin_dark();
    let plain = populated_surface(PanelSurface::Taskbar, tokens.clone()).scene();
    let json = plain.json.clone();

    tokens.glass.tint = Rgba {
        r: 1,
        g: 2,
        b: 3,
        a: 0x10,
    };
    tokens.glass.tint_strong = Rgba {
        r: 4,
        g: 5,
        b: 6,
        a: 0x20,
    };
    tokens.glass.highlight = Rgba {
        r: 7,
        g: 8,
        b: 9,
        a: 0x30,
    };
    tokens.glass.shade = Rgba {
        r: 10,
        g: 11,
        b: 12,
        a: 0x40,
    };

    let changed = populated_surface(PanelSurface::Taskbar, tokens).scene();

    assert_eq!(
        json, changed.json,
        "the taskbar's rendering must not change by one pixel with the glass tokens"
    );
}

#[test]
fn every_size_in_a_scene_comes_from_the_tokens() {
    // Nothing in the projection is a written-down pixel: the strip's height, its
    // padding, its radii and its text sizes are all derivable from the tokens and
    // the configuration, and a theme with wider spacing moves them.
    let mut tokens = Tokens::builtin_dark();
    let narrow = populated_surface(PanelSurface::Taskbar, tokens.clone()).scene();
    let narrow_first = narrow.entries[0];

    tokens.spacing.md = 24;
    tokens.spacing.xl = 64;
    let wide = populated_surface(PanelSurface::Taskbar, tokens.clone()).scene();
    let wide_first = wide.entries[0];

    assert!(
        wide_first.x > narrow_first.x,
        "the strip's padding is a spacing token, not a constant"
    );
    assert!(
        wide_first.width > narrow_first.width,
        "and an entry's padding is the same token"
    );
}

#[test]
fn a_top_anchored_panel_still_projects_the_same_data() {
    let tokens = Tokens::builtin_dark();
    let bottom = populated_surface(PanelSurface::Taskbar, tokens.clone()).scene();
    let top = populated_with(
        PanelConfig {
            position: PanelPosition::Top,
            ..PanelConfig::default()
        },
        tokens,
    )
    .scene();

    assert_eq!(
        sentences(&bottom),
        sentences(&top),
        "same data, same sentences"
    );
}

#[test]
fn no_scene_carries_a_raw_colour_or_a_translated_string_of_its_own() {
    // The contract's rule, asserted on every surface: every colour is a
    // `#AARRGGBB` string (the theme's conversion), and every sentence is one the
    // model or a catalog produced.
    for mode in [
        PanelSurface::Taskbar,
        PanelSurface::Menubar,
        PanelSurface::Dock,
    ] {
        let panel = populated_surface(mode, Tokens::builtin_dark());
        let scene = panel.scene();

        for item in items(&scene) {
            let colour = color(item);

            assert!(
                colour.starts_with('#') && colour.len() == 9,
                "{mode:?}: `{colour}` is not a `#AARRGGBB` value from the tokens"
            );
        }
    }
}
