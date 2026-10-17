//! What the daemon draws, asserted on pixels (acceptance Q02, D05, D06).
//!
//! Every test here renders the real widget tree through the tiny-skia renderer
//! the binary uses, at the size acceptance Q02 snapshots are taken at. There is
//! no compositor and no GPU involved, so these are E1 evidence: they prove the
//! stack draws what the view model says it draws, not that it appears on a
//! screen — for that the compositor has to advertise `zwlr_layer_shell_v1`
//! (ADR 0018).
//!
//! The assertions avoid font metrics on purpose: they are about token colours,
//! where the stack is, how much of the surface it covers, and which messages a
//! click produces — none of which depend on which glyphs the CI machine's fonts
//! happen to provide.

mod support;

use support::{
    Daemon, HEIGHT, WIDTH, count_of, distinct_colours, has_ink, ink_span, lowest_ink, pixel,
    snapshot, token_pixel,
};

use inox_config::{NotificationPosition, NotificationsConfig, Theme as Tokens};
use inox_notifications::actions::ActionId;
use inox_notifications::app::Message;
use inox_notifications::contract::{NotifyRequest, Outgoing, Signal};
use inox_notifications::policy::SessionState;
use inox_notifications::protocol::{CloseReason, HINT_URGENCY, HintValue};
use inox_notifications::view_model::StackStatus;

/// The daemon of a scenario: the default configuration and the dark tokens.
fn daemon(tokens: Tokens) -> Daemon {
    Daemon::with(NotificationsConfig::default(), tokens)
}

/// A daemon holding three notifications, one of which offers an action.
fn populated(tokens: Tokens) -> Daemon {
    let mut daemon = daemon(tokens);

    daemon.notify("firefox", "Inox — Documentation", "A page finished loading");
    daemon.notify(
        "terminal",
        "Build finished",
        "with <b>markup</b> and a link <a href=\"https://example.com\">here</a>",
    );
    daemon.notify_with(NotifyRequest {
        app_name: "chat".to_owned(),
        summary: "A message arrived".to_owned(),
        actions: vec![
            "default".to_owned(),
            String::new(),
            "open".to_owned(),
            "Open".to_owned(),
        ],
        hints: vec![(HINT_URGENCY.to_owned(), HintValue::Byte(2))],
        ..NotifyRequest::default()
    });

    daemon
}

#[test]
fn the_stack_is_drawn_at_the_acceptance_size() {
    let daemon = populated(Tokens::builtin_light());
    let pixels = daemon.render();
    let size = daemon.size();

    snapshot("light", &pixels, size);

    assert_eq!(
        pixels.len(),
        (size.width as usize) * (size.height as usize) * 4,
        "one RGBA pixel per logical pixel"
    );
    assert_eq!((size.width, size.height), (WIDTH, HEIGHT));
}

#[test]
fn the_surface_wears_the_background_token_and_the_popups_the_surface_token() {
    let tokens = Tokens::builtin_dark();
    let daemon = populated(tokens.clone());
    let pixels = daemon.render();

    assert_eq!(
        pixel(&pixels, WIDTH as u32, 0, 0),
        token_pixel(tokens.colors.background),
        "away from the stack the surface is the theme's background"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.surface)) > 0,
        "the popups are drawn on the surface token"
    );
    assert!(count_of(&pixels, token_pixel(tokens.colors.accent)) > 0);
    assert!(
        distinct_colours(&pixels) > 8,
        "a populated stack is not a flat rectangle: {} colours",
        distinct_colours(&pixels)
    );
}

#[test]
fn the_light_and_dark_themes_produce_different_stacks() {
    let light = populated(Tokens::builtin_light());
    let dark = populated(Tokens::builtin_dark());

    let light_pixels = light.render();
    let dark_pixels = dark.render();

    snapshot("light-theme", &light_pixels, light.size());
    snapshot("dark-theme", &dark_pixels, dark.size());

    assert_ne!(light_pixels, dark_pixels);
    assert_eq!(
        pixel(&light_pixels, WIDTH as u32, 0, 0),
        token_pixel(Tokens::builtin_light().colors.background)
    );
    assert_eq!(
        pixel(&dark_pixels, WIDTH as u32, 0, 0),
        token_pixel(Tokens::builtin_dark().colors.background)
    );
}

#[test]
fn the_stack_is_drawn_in_the_configured_corner() {
    let tokens = Tokens::builtin_dark();
    let background = token_pixel(tokens.colors.background);

    let top_right = populated(tokens.clone());
    let pixels = top_right.render();
    snapshot("top-right", &pixels, top_right.size());
    assert!(
        has_ink(
            &pixels,
            WIDTH as u32,
            (900, 0),
            (WIDTH as u32, 400),
            background
        ),
        "the configuration puts the stack in the upper right"
    );
    assert!(
        !has_ink(
            &pixels,
            WIDTH as u32,
            (0, 400),
            (400, HEIGHT as u32),
            background
        ),
        "and draws nothing in the lower left"
    );

    let mut config = NotificationsConfig::default();
    config.display.position = NotificationPosition::BottomLeft;
    let mut bottom_left = Daemon::with(config, tokens);
    bottom_left.notify("firefox", "Inox — Documentation", "A page finished loading");
    let pixels = bottom_left.render();
    snapshot("bottom-left", &pixels, bottom_left.size());

    assert!(
        has_ink(
            &pixels,
            WIDTH as u32,
            (0, 400),
            (400, HEIGHT as u32),
            background
        ),
        "a bottom-left configuration draws in the lower left"
    );
    assert!(
        !has_ink(
            &pixels,
            WIDTH as u32,
            (900, 0),
            (WIDTH as u32, 300),
            background
        ),
        "and nothing in the upper right"
    );
}

#[test]
fn an_empty_daemon_says_so_instead_of_drawing_nothing() {
    let tokens = Tokens::builtin_dark();
    let mut config = NotificationsConfig::default();
    config.display.position = NotificationPosition::BottomLeft;
    let daemon = Daemon::with(config, tokens.clone());
    let pixels = daemon.render();
    let size = daemon.size();

    snapshot("empty", &pixels, size);

    assert_eq!(daemon.state().model().status, StackStatus::Empty);
    assert_eq!(
        daemon.state().model().notice.as_deref(),
        Some("No notifications")
    );
    assert!(
        has_ink(
            &pixels,
            WIDTH as u32,
            (0, 400),
            (400, HEIGHT as u32),
            token_pixel(tokens.colors.background)
        ),
        "the daemon's own line is drawn where the popups would be"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.surface_variant)) > 0,
        "the notice wears the alternate surface token"
    );
}

#[test]
fn a_locked_session_draws_less_because_the_body_is_withheld() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = populated(tokens.clone());

    let unlocked = daemon.render();
    let unlocked_background = count_of(&unlocked, token_pixel(tokens.colors.background));

    daemon.update(Message::SessionLockChanged(SessionState::Locked));

    let locked = daemon.render();
    let locked_background = count_of(&locked, token_pixel(tokens.colors.background));

    snapshot("locked", &locked, daemon.size());

    let model = daemon.state().model();
    assert!(model.popups.iter().all(|popup| popup.content.is_withheld()));
    assert!(
        !format!("{model:?}").contains("A page finished loading"),
        "the body is not in the model at all"
    );
    assert!(
        locked_background > unlocked_background,
        "a popup with no body and no application name covers less of the surface \
         ({locked_background} against {unlocked_background})"
    );

    daemon.update(Message::SessionLockChanged(SessionState::Unlocked));
    assert_eq!(daemon.render(), unlocked, "unlocking restores the stack");
}

#[test]
fn more_waiting_than_can_be_shown_is_drawn_as_a_sentence() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());

    for index in 0..3 {
        daemon.notify("app", &format!("notification {index}"), "body");
    }
    let three = daemon.render();
    let three_background = count_of(&three, token_pixel(tokens.colors.background));

    for index in 3..6 {
        daemon.notify("app", &format!("notification {index}"), "body");
    }
    let six = daemon.render();
    let six_background = count_of(&six, token_pixel(tokens.colors.background));

    snapshot("overflow", &six, daemon.size());

    assert_eq!(daemon.state().model().status, StackStatus::Showing);
    assert_eq!(
        daemon.state().model().hidden(),
        3,
        "the sentence counts what the stack cannot show"
    );
    assert_eq!(
        daemon
            .state()
            .model()
            .overflow
            .map(|overflow| overflow.message),
        Some("3 more waiting".to_owned())
    );
    assert_eq!(daemon.state().queue().len(), 6);
    assert!(
        six_background < three_background,
        "the overflow line is drawn rather than the extra three being hidden \
         ({six_background} against {three_background} background pixels)"
    );
}

#[test]
fn do_not_disturb_draws_no_notification_and_no_action() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = populated(tokens.clone());

    assert!(
        lowest_ink(
            &daemon.render(),
            daemon.size(),
            token_pixel(tokens.colors.accent)
        )
        .is_some(),
        "with notifications shown there is an action button to see"
    );

    daemon.update(Message::DoNotDisturbToggled);
    let pixels = daemon.render();

    snapshot("do-not-disturb", &pixels, daemon.size());

    let model = daemon.state().model();
    assert_eq!(model.status, StackStatus::Suppressed);
    assert!(model.popups.is_empty());
    assert_eq!(
        model.notice.as_deref(),
        Some("Notifications are muted while do not disturb is on")
    );
    assert_eq!(daemon.state().queue().len(), 3, "nothing was dropped");
    assert_eq!(
        lowest_ink(&pixels, daemon.size(), token_pixel(tokens.colors.accent)),
        None,
        "a muted stack draws no action button and nothing else accent-coloured"
    );
}

#[test]
fn an_action_that_cannot_be_used_is_drawn_as_unavailable() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    daemon.notify_with(NotifyRequest {
        app_name: "chat".to_owned(),
        summary: "A message arrived".to_owned(),
        actions: vec!["open".to_owned(), "Open".to_owned()],
        ..NotifyRequest::default()
    });

    let enabled = daemon.render();
    assert!(lowest_ink(&enabled, daemon.size(), token_pixel(tokens.colors.accent)).is_some());

    let mut config = NotificationsConfig::default();
    config.policy.show_actions = false;
    daemon.update(Message::ConfigReloaded(config));

    let disabled = daemon.render();
    snapshot("disabled-actions", &disabled, daemon.size());

    let model = daemon.state().model();
    assert_eq!(model.popups.len(), 1);
    assert!(!model.popups[0].actions[0].enabled);
    assert_eq!(
        model.popups[0].actions[0].disabled_reason.as_deref(),
        Some("Actions are turned off in notifications.toml")
    );
    assert_ne!(enabled, disabled);
    assert_eq!(
        lowest_ink(&disabled, daemon.size(), token_pixel(tokens.colors.accent)),
        None,
        "a disabled action is muted rather than accent-coloured"
    );
}

#[test]
fn a_long_summary_stays_inside_the_configured_width() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    daemon.notify("app", &"a very long summary ".repeat(40), "and a body");

    let pixels = daemon.render();
    snapshot("long-title", &pixels, daemon.size());

    let popup_left = WIDTH
        - f32::from(NotificationsConfig::default().display.margin)
        - f32::from(NotificationsConfig::default().display.width);

    assert!(
        !has_ink(
            &pixels,
            WIDTH as u32,
            (0, 0),
            (popup_left as u32 - 4, HEIGHT as u32),
            token_pixel(tokens.colors.background)
        ),
        "nothing is drawn to the left of the popup's own column"
    );
    assert_eq!(daemon.state().model().popups.len(), 1);
}

#[test]
fn clicking_an_action_produces_exactly_one_invocation_and_closes_the_popup() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    let id = daemon.notify_with(NotifyRequest {
        app_name: "chat".to_owned(),
        summary: "A message arrived".to_owned(),
        actions: vec!["open".to_owned(), "Open".to_owned()],
        ..NotifyRequest::default()
    });

    // The action button is found by its accent-coloured ink rather than by a
    // guess at where the layout put it.
    let button = lowest_ink(
        &daemon.render(),
        daemon.size(),
        token_pixel(tokens.colors.accent),
    )
    .expect("the action button is drawn");

    let messages = daemon.click_through(button);

    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, Message::ActionInvoked { .. }))
            .count(),
        1,
        "one click, one invocation: {messages:?}"
    );
    assert!(matches!(
        &messages[0],
        Message::ActionInvoked { id: invoked, action } if *invoked == id && *action == ActionId::new("open")
    ));

    let outgoing = daemon.state_mut().take_outgoing();
    assert_eq!(
        outgoing
            .iter()
            .filter(|entry| matches!(entry, Outgoing::Signal(Signal::ActionInvoked { .. })))
            .count(),
        1,
        "the client is told once: {outgoing:?}"
    );
    assert!(outgoing.contains(&Outgoing::Signal(Signal::closed(
        id,
        CloseReason::Dismissed
    ))));
    assert!(daemon.state().model().popups.is_empty());

    // A second click where the button used to be finds nothing.
    let messages = daemon.click(button);
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, Message::ActionInvoked { .. })),
        "the notification is gone, so there is nothing left to invoke"
    );
}

#[test]
fn the_close_button_closes_the_popup_it_belongs_to() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    let first = daemon.notify("firefox", "the older one", "body");
    let second = daemon.notify("terminal", "the newer one", "body");

    // The close button is muted-coloured ink at the right-hand end of the
    // popup's header; the application name beside it is muted too, which is why
    // the search is limited to the right-hand half of the popup.
    let popup_left = WIDTH
        - f32::from(NotificationsConfig::default().display.margin)
        - f32::from(NotificationsConfig::default().display.width);
    let pixels = daemon.render();
    let size = daemon.size();
    let close = ink_span(
        &pixels,
        size,
        token_pixel(tokens.colors.text_muted),
        ((popup_left + WIDTH) as u32 / 2, 0),
        (WIDTH as u32, HEIGHT as u32),
    )
    .expect("the close button is drawn in the muted colour");

    let messages = daemon.click_through(close);

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::Dismissed { id } if *id == second)),
        "the newest popup's close button was clicked: {messages:?}"
    );
    assert_eq!(
        daemon.state_mut().take_outgoing(),
        [Outgoing::Signal(Signal::closed(
            second,
            CloseReason::Dismissed
        ))],
        "the close is reported to the client"
    );
    assert!(daemon.state().queue().get(first).is_some());
    assert!(daemon.state().queue().get(second).is_none());
}

#[test]
fn clicking_where_there_is_nothing_leaves_the_daemon_alone() {
    let mut daemon = populated(Tokens::builtin_dark());
    let before = daemon.render();

    let messages = daemon.click(iced::Point::new(20.0, HEIGHT / 2.0));

    assert!(messages.is_empty(), "{messages:?}");
    assert!(daemon.state_mut().take_outgoing().is_empty());
    assert_eq!(
        daemon.state().queue().len(),
        3,
        "a click on nothing changes nothing"
    );
    assert_eq!(daemon.render(), before);
}

#[test]
fn a_stack_whose_notifications_expired_falls_back_to_the_empty_state() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    // Nothing critical here: a critical notification never expires on its own
    // (the queue's own tests cover that), and this test is about the transition
    // back to the empty state.
    daemon.notify("firefox", "one", "body");
    daemon.notify("terminal", "two", "body");
    let populated_pixels = daemon.render();
    let populated_background = count_of(&populated_pixels, token_pixel(tokens.colors.background));

    daemon.expire_everything();
    let pixels = daemon.render();

    snapshot("expired", &pixels, daemon.size());

    assert_eq!(daemon.state().queue().len(), 0);
    assert_eq!(daemon.state().model().status, StackStatus::Empty);
    assert_eq!(
        daemon.state().model().notice.as_deref(),
        Some("No notifications")
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.background)) > populated_background,
        "expiring everything leaves the surface emptier than it was"
    );
}

#[test]
fn every_state_draws_the_background_token_underneath() {
    let tokens = Tokens::builtin_dark();
    let background = token_pixel(tokens.colors.background);
    let corner = ((0, 0), (40, 40));

    for daemon in [daemon(tokens.clone()), populated(tokens.clone())] {
        let pixels = daemon.render();

        assert!(
            !has_ink(&pixels, WIDTH as u32, corner.0, corner.1, background),
            "the corner away from a top-right stack stays background"
        );
    }
}
