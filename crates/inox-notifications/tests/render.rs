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
    Daemon, HEIGHT, WIDTH, count_of, distinct_colours, has_ink, ink_span, lowest_drawn_row,
    lowest_ink, pixel, runs_in_column, snapshot, token_pixel,
};

use inox_config::{NotificationPosition, NotificationsConfig, Theme as Tokens, Typography};
use inox_notifications::actions::ActionId;
use inox_notifications::app::Message;
use inox_notifications::contract::{NotifyRequest, Outgoing, Signal};
use inox_notifications::policy::SessionState;
use inox_notifications::protocol::{CloseReason, HINT_URGENCY, HintValue};
use inox_notifications::view::StackGeometry;
use inox_notifications::view_model::{ActionEmphasis, StackStatus};

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

/// Where a card's left edge is at the acceptance size, with the default display
/// configuration.
fn card_left() -> f32 {
    WIDTH
        - f32::from(NotificationsConfig::default().display.margin)
        - f32::from(NotificationsConfig::default().display.width)
}

/// The column inside a card's own padding: pure surface for as tall as the card
/// is, which is what makes the gaps between the runs the gaps between the cards.
fn padding_column() -> u32 {
    (card_left() as u32) + 1
}

/// The `(from, to)` rectangle to the left of the popup's own column, which no
/// scenario may draw in (ui-guidelines §7: no horizontal overflow).
fn left_of_the_column() -> ((u32, u32), (u32, u32)) {
    ((0, 0), (card_left() as u32 - 4, HEIGHT as u32))
}

/// A notification that says how urgent it is.
fn with_urgency(app_name: &str, summary: &str, urgency: u8) -> NotifyRequest {
    NotifyRequest {
        app_name: app_name.to_owned(),
        summary: summary.to_owned(),
        hints: vec![(HINT_URGENCY.to_owned(), HintValue::Byte(urgency))],
        ..NotifyRequest::default()
    }
}

#[test]
fn a_card_is_the_surface_token_inset_by_the_large_step_and_gapped_by_the_large_step() {
    let tokens = Tokens::builtin_dark();
    let daemon = populated(tokens.clone());
    let pixels = daemon.render();
    let size = daemon.size();
    let geometry = StackGeometry::new(&tokens, &NotificationsConfig::default().display);

    snapshot("cards", &pixels, size);

    assert!(
        count_of(&pixels, token_pixel(tokens.colors.border)) > 0,
        "every card has a hairline edge (ui-guidelines §4)"
    );

    let runs = runs_in_column(
        &pixels,
        size,
        padding_column(),
        token_pixel(tokens.colors.surface),
    );

    assert_eq!(
        runs.len(),
        3,
        "one run of surface per card, so no two cards touch: {runs:?}"
    );

    for pair in runs.windows(2) {
        let gap = pair[1].0 - pair[0].1 - 1;

        assert!(
            f32::from(u16::try_from(gap).unwrap_or(u16::MAX)) >= geometry.card_gap,
            "two cards are {gap} px apart, closer than the {} px the tokens ask \
             for (ui-guidelines §3: blocks are separated by `lg`)",
            geometry.card_gap
        );
    }
}

#[test]
fn a_critical_notification_is_edged_and_says_so_rather_than_only_being_red() {
    let tokens = Tokens::builtin_dark();
    let mut critical = daemon(tokens.clone());
    critical.notify_with(with_urgency("accounts", "Disk is full", 2));

    let critical_pixels = critical.render();
    snapshot("urgency-critical", &critical_pixels, critical.size());

    assert_eq!(
        critical.state().model().popups[0].urgency_note.as_deref(),
        Some("Critical"),
        "ui-guidelines §5: urgency is a colour *and* a sentence"
    );
    assert!(
        count_of(&critical_pixels, token_pixel(tokens.colors.danger)) > 0,
        "a critical card is edged in the danger colour"
    );
    assert!(
        count_of(&critical_pixels, token_pixel(tokens.colors.text)) > 0,
        "and its title is still ordinary text, not a second urgency colour"
    );

    let mut ordinary = daemon(tokens.clone());
    ordinary.notify("accounts", "Disk is full", "body");

    let ordinary_pixels = ordinary.render();

    assert_eq!(ordinary.state().model().popups[0].urgency_note, None);
    assert_eq!(
        count_of(&ordinary_pixels, token_pixel(tokens.colors.danger)),
        0,
        "an ordinary notification claims no urgency it was not sent"
    );
}

#[test]
fn a_low_priority_notification_is_said_in_a_sentence_and_not_in_the_danger_colour() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    daemon.notify_with(with_urgency("backup", "A backup finished", 0));

    let pixels = daemon.render();
    snapshot("urgency-low", &pixels, daemon.size());

    assert_eq!(
        daemon.state().model().popups[0].urgency_note.as_deref(),
        Some("Low priority")
    );
    assert_eq!(
        count_of(&pixels, token_pixel(tokens.colors.danger)),
        0,
        "low urgency is quiet, not alarming"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.border)) > 0,
        "its card keeps the ordinary hairline edge"
    );
}

#[test]
fn three_action_tiers_are_drawn_in_three_colours() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    daemon.notify_with(NotifyRequest {
        app_name: "files".to_owned(),
        summary: "Delete this file?".to_owned(),
        body: "It will not go to the trash".to_owned(),
        actions: vec![
            "open".to_owned(),
            "Open".to_owned(),
            "snooze".to_owned(),
            "Snooze".to_owned(),
            "delete".to_owned(),
            "Delete".to_owned(),
        ],
        ..NotifyRequest::default()
    });

    let pixels = daemon.render();
    snapshot("action-tiers", &pixels, daemon.size());

    let tiers: Vec<ActionEmphasis> = daemon.state().model().popups[0]
        .actions
        .iter()
        .map(|action| action.emphasis)
        .collect();

    assert_eq!(
        tiers,
        [
            ActionEmphasis::Primary,
            ActionEmphasis::Secondary,
            ActionEmphasis::Danger
        ]
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.accent)) > 0,
        "the primary action is accent-coloured"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.danger)) > 0,
        "the dangerous one is danger-coloured"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.text)) > 0,
        "the ordinary one is written in ordinary text"
    );
}

#[test]
fn many_long_actions_wrap_inside_the_card_instead_of_running_off_it() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    let actions: Vec<String> = (0..8)
        .flat_map(|index| {
            [
                format!("action-{index}"),
                format!("Do something fairly long, number {index}"),
            ]
        })
        .collect();

    daemon.notify_with(NotifyRequest {
        app_name: "app".to_owned(),
        summary: "Eight actions".to_owned(),
        actions,
        ..NotifyRequest::default()
    });

    let pixels = daemon.render();
    snapshot("many-actions", &pixels, daemon.size());

    assert_eq!(daemon.state().model().popups[0].actions.len(), 8);
    assert!(
        !has_ink(
            &pixels,
            WIDTH as u32,
            left_of_the_column().0,
            left_of_the_column().1,
            token_pixel(tokens.colors.background)
        ),
        "ui-guidelines §7: a row of actions may wrap, never overflow"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.accent)) > 0,
        "the first action is still drawn"
    );
}

#[test]
fn a_disabled_action_says_why_on_the_card_and_not_only_on_hover() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    daemon.notify_with(NotifyRequest {
        app_name: "chat".to_owned(),
        summary: "A message arrived".to_owned(),
        actions: vec!["open".to_owned(), "Open".to_owned()],
        ..NotifyRequest::default()
    });

    let enabled = daemon.render();
    let enabled_bottom = lowest_drawn_row(
        &enabled,
        daemon.size(),
        token_pixel(tokens.colors.background),
    );

    let mut config = NotificationsConfig::default();
    config.policy.show_actions = false;
    daemon.update(Message::ConfigReloaded(config));

    let disabled = daemon.render();
    snapshot("disabled-with-reason", &disabled, daemon.size());

    let disabled_bottom = lowest_drawn_row(
        &disabled,
        daemon.size(),
        token_pixel(tokens.colors.background),
    );

    assert_eq!(
        daemon.state().model().popups[0].actions[0]
            .disabled_reason
            .as_deref(),
        Some("Actions are turned off in notifications.toml")
    );
    assert_eq!(
        daemon.state().model().popups[0].actions[0].emphasis,
        ActionEmphasis::Primary,
        "an unavailable action keeps the tier it was given"
    );
    assert!(
        disabled_bottom > enabled_bottom,
        "the reason is drawn on the card, so the card is taller \
         ({disabled_bottom:?} against {enabled_bottom:?})"
    );
}

#[test]
fn a_long_body_wraps_inside_the_card_and_leaves_the_actions_reachable() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    daemon.notify_with(NotifyRequest {
        app_name: "app".to_owned(),
        summary: "A long message".to_owned(),
        body: "a long sentence that has to wrap ".repeat(40),
        actions: vec!["open".to_owned(), "Open".to_owned()],
        ..NotifyRequest::default()
    });

    let pixels = daemon.render();
    let size = daemon.size();
    snapshot("long-body", &pixels, size);

    assert!(
        !has_ink(
            &pixels,
            WIDTH as u32,
            left_of_the_column().0,
            left_of_the_column().1,
            token_pixel(tokens.colors.background)
        ),
        "a long body wraps rather than widening the card"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.accent)) > 0,
        "and the actions are still on the card"
    );
    assert!(
        lowest_drawn_row(&pixels, size, token_pixel(tokens.colors.background))
            .is_some_and(|row| row < HEIGHT as u32),
        "the card is drawn inside the surface"
    );
}

#[test]
fn the_overflow_line_is_a_caution_rather_than_a_card() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());

    for index in 0..3 {
        daemon.notify("app", &format!("notification {index}"), "body");
    }

    let three = daemon.render();

    for index in 3..6 {
        daemon.notify("app", &format!("notification {index}"), "body");
    }

    let six = daemon.render();
    snapshot("overflow-caution", &six, daemon.size());

    assert_eq!(
        count_of(&three, token_pixel(tokens.colors.warning)),
        0,
        "nothing to warn about while everything fits"
    );
    assert!(
        count_of(&six, token_pixel(tokens.colors.warning)) > 0,
        "the overflow sentence is drawn in the warning colour: it is about \
         notifications the user cannot see (D05)"
    );
}

#[test]
fn the_empty_state_is_a_bounded_line_that_claims_nothing() {
    let tokens = Tokens::builtin_dark();
    let mut config = NotificationsConfig::default();
    config.display.position = NotificationPosition::BottomLeft;
    let daemon = Daemon::with(config, tokens.clone());

    let pixels = daemon.render();
    snapshot("empty-bounded", &pixels, daemon.size());

    assert!(
        count_of(&pixels, token_pixel(tokens.colors.surface_variant)) > 0,
        "the daemon's own line is drawn"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens.colors.border)) > 0,
        "and it is bounded by a hairline like every other surface"
    );

    for claimed in [
        tokens.colors.accent,
        tokens.colors.danger,
        tokens.colors.warning,
        tokens.colors.success,
    ] {
        assert_eq!(
            count_of(&pixels, token_pixel(claimed)),
            0,
            "an empty stack claims no state it does not have (ui-guidelines §6)"
        );
    }
}

#[test]
fn two_hundred_percent_text_scale_neither_clips_nor_overlaps_nor_overflows() {
    let tokens_100 = Tokens::builtin_dark();
    let tokens_200 = Tokens {
        typography: Typography {
            scale: 2.0,
            ..Tokens::builtin_dark().typography.clone()
        },
        ..Tokens::builtin_dark()
    };

    let scaled = StackGeometry::new(&tokens_200, &NotificationsConfig::default().display);
    let unscaled = StackGeometry::new(&tokens_100, &NotificationsConfig::default().display);

    assert!(
        scaled.body_size > unscaled.body_size,
        "200 % is twice the text, not the same text"
    );

    let mut daemon = daemon(tokens_200.clone());
    daemon.notify("firefox", "Inox — Documentation", "A page finished loading");
    daemon.notify_with(NotifyRequest {
        app_name: "chat".to_owned(),
        summary: "A message arrived".to_owned(),
        actions: vec!["open".to_owned(), "Open".to_owned()],
        ..NotifyRequest::default()
    });
    daemon.notify_with(with_urgency("build", "Build finished", 2));

    // The surface the daemon would ask for is a layer surface the compositor
    // sizes (ADR 0018); while there is no such host, the test gives the stack a
    // surface tall enough for what 200 % asks for rather than the acceptance
    // size, which is the only way "not clipped" can be observed at all.
    let size = iced::Size::new(WIDTH, HEIGHT * 2.0);
    let pixels = daemon.render_at(size);
    snapshot("scale-200", &pixels, size);

    let background = token_pixel(tokens_200.colors.background);

    assert!(
        !has_ink(
            &pixels,
            WIDTH as u32,
            (0, 0),
            (card_left() as u32 - 4, size.height as u32),
            background
        ),
        "200 % pushes nothing out of the popup's own column"
    );
    assert_eq!(
        runs_in_column(
            &pixels,
            size,
            padding_column(),
            token_pixel(tokens_200.colors.surface)
        )
        .len(),
        3,
        "three cards, none of them overlapping the next"
    );
    assert!(
        count_of(&pixels, token_pixel(tokens_200.colors.accent)) > 0,
        "the action button grew with the text instead of being cut off"
    );

    let bottom = lowest_drawn_row(&pixels, size, background).expect("something is drawn");

    assert!(
        bottom + u32::from(NotificationsConfig::default().display.margin) <= size.height as u32,
        "the stack ends inside the surface, margin included: {bottom} of {}",
        size.height
    );
    assert!(
        bottom > HEIGHT as u32,
        "and it is genuinely taller than the 100 % stack, which is what 200 % means"
    );
}

#[test]
fn the_feedback_of_a_control_is_a_hover_before_a_press_and_a_press_at_once() {
    let tokens = Tokens::builtin_dark();
    let mut daemon = daemon(tokens.clone());
    daemon.notify_with(NotifyRequest {
        app_name: "chat".to_owned(),
        summary: "A message arrived".to_owned(),
        actions: vec!["open".to_owned(), "Open".to_owned()],
        ..NotifyRequest::default()
    });

    let button = lowest_ink(
        &daemon.render(),
        daemon.size(),
        token_pixel(tokens.colors.accent),
    )
    .expect("the action button is drawn");

    let resting = daemon.render();
    let hovered = daemon.render_hovered(button);
    let pressed = daemon.render_pressed(button);

    snapshot("action-pressed", &pressed, daemon.size());

    assert_ne!(resting, hovered, "a pointer over a control changes it");
    assert_ne!(
        hovered, pressed,
        "and holding the button down is a third thing"
    );

    let alternate = |pixels: &Vec<u8>| count_of(pixels, token_pixel(tokens.colors.surface_variant));
    let accent = |pixels: &Vec<u8>| count_of(pixels, token_pixel(tokens.colors.accent));

    assert!(
        alternate(&hovered) > alternate(&resting),
        "hovering paints the alternate surface over the one under the pointer"
    );
    assert!(
        accent(&pressed) > accent(&resting) + 1_000,
        "the press fills the control with the accent before anything is released \
         (ui-guidelines §5: the feedback is on the way down, not on the way up): \
         {} against {}",
        accent(&pressed),
        accent(&resting)
    );
}
