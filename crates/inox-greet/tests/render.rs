//! What the login screen draws, asserted on pixels (acceptance Q02, D05, Q01).
//!
//! Every test here renders the real widget tree through the tiny-skia renderer
//! the binary uses. There is no compositor and no GPU involved, so these are E1
//! evidence: they prove the screen draws what the view model says it draws, not
//! that it appears on a display, and — ADR 0021 is explicit about this — **not**
//! that any of it is a secure login surface. The binary refuses a real login
//! whatever these pixels look like.
//!
//! The scenarios are the ones acceptance Q02 asks for — light, dark, 200 % text,
//! both locales, the error state and the empty/unavailable state — plus the ones
//! ADR 0015 turns into rules for this component: an authenticator that is not
//! there takes no input, and a control that cannot be used draws no ring.

mod support;

use support::{Login, bounds_of, count_of, is_opaque, pixel, snapshot, token_pixel};

use iced::Point;
use inox_common::i18n::Locale;
use inox_config::Theme as Tokens;
use inox_greet::state::{AttemptId, AuthReply, Command, Phase};
use inox_greet::view_model::FocusTarget;
use inox_greet::{Message, app};

/// A login screen with an authenticator, which is the configuration the states
/// the shipped binary cannot reach are rendered in.
fn ready() -> Login {
    Login::with(Tokens::builtin_dark(), true)
}

/// The count of accent pixels above which they are an area rather than an outline.
///
/// The accent is both the log-in control's fill and every control's focus ring,
/// so "is the log-in control drawn as operable?" cannot be asked as "is there any
/// accent on screen?" — a screen where nothing can be submitted still has the
/// ring. It can be asked as "is there an *area* of it?": a fill is a rectangle and
/// a ring is an outline. At 1280×720 the ring around the identity field is about
/// 1,300 pixels and around one button about 170, while the log-in control's fill
/// is about 12,600; this floor sits between them with a factor of three either
/// way. The count is measured from the pixels rather than guessed at from a
/// coordinate, so the test cannot drift away from the layout.
const FILLED_ACCENT_FLOOR: usize = 4_000;

/// Whether a control is drawn *filled* in the accent colour.
fn draws_a_filled_accent_control(login: &Login) -> bool {
    count_of(&login.render(), login.tokens().colors.accent) > FILLED_ACCENT_FLOOR
}

/// Submits what is in the field and returns the attempt the model asked for.
fn submit(login: &mut Login) -> AttemptId {
    login.update(Message::Submitted);

    login
        .state_mut()
        .take_outgoing()
        .into_iter()
        .find_map(|command| match command {
            Command::Authenticate { id, .. } => Some(id),
            _ => None,
        })
        .expect("an authentication request")
}

/// Fails an attempt: types a name, submits it, and has the authenticator refuse.
fn fail_an_attempt(login: &mut Login) {
    login.type_text("alice");
    let id = submit(login);
    login.update(Message::AuthAnswered(id, AuthReply::Rejected));
}

#[test]
fn the_screen_is_drawn_under_both_themes_locales_and_double_font_scale() {
    for tag in ["en-US", "zh-CN"] {
        let locale = Locale::parse(tag).expect("a well-formed tag");

        for mut tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
            for scale in [1.0, 2.0] {
                tokens.typography.scale = scale;
                let name = format!("{}-{tag}-{scale}", tokens.name);

                // The shipped configuration: no authenticator, so the screen can
                // collect nothing and says so.
                let unavailable = Login::in_locale(tokens.clone(), false, &locale);
                let size = unavailable.size();
                let background = token_pixel(unavailable.tokens().colors.background);
                let pixels = unavailable.render();

                assert_eq!(
                    pixel(&pixels, size.width as u32, 0, 0),
                    background,
                    "the screen covers what is behind it ({name})"
                );
                assert_eq!(
                    pixel(
                        &pixels,
                        size.width as u32,
                        size.width as u32 - 1,
                        size.height as u32 - 1
                    ),
                    background,
                    "and covers it all the way to the far corner ({name})"
                );
                assert!(
                    is_opaque(&pixels),
                    "a login window that is not opaque shows what is behind it ({name})"
                );
                assert!(
                    count_of(&pixels, unavailable.tokens().colors.background) < pixels.len() / 4,
                    "something was drawn on the screen ({name})"
                );
                snapshot(&format!("unavailable-{name}"), &pixels, size);

                // And with an authenticator, where the log-in control is the one
                // thing drawn in the accent colour.
                let mut usable = Login::in_locale(tokens.clone(), true, &locale);
                usable.type_text("alice");
                usable.update(Message::FocusNext);
                let size = usable.size();
                let accent = usable.tokens().colors.accent;
                let pixels = usable.render();

                let button = bounds_of(&pixels, size, accent)
                    .unwrap_or_else(|| panic!("an enabled log-in button ({name})"));
                assert!(
                    button.y + button.height <= size.height,
                    "the log-in control is inside the screen at this scale ({name})"
                );
                assert!(
                    button.x >= 0.0 && button.x + button.width <= size.width,
                    "and inside it horizontally ({name})"
                );
                assert!(is_opaque(&pixels), "the screen is opaque ({name})");
                snapshot(&format!("ready-{name}"), &pixels, size);
            }
        }
    }
}

#[test]
fn an_absent_authenticator_says_so_instead_of_prompting() {
    let login = Login::new();
    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert!(
        !draws_a_filled_accent_control(&login),
        "nothing on a screen that cannot log anyone in is drawn as an operable \
         control"
    );
    assert!(
        count_of(&pixels, tokens.colors.danger) > 0 || count_of(&pixels, tokens.colors.warning) > 0,
        "the screen says why it cannot run, rather than showing a bare form"
    );
    assert!(
        matches!(
            login.state().model().face,
            inox_greet::view_model::Face::Unavailable { .. }
        ),
        "an absent authenticator is the unavailable face"
    );

    snapshot("absent-authenticator", &pixels, size);
}

#[test]
fn the_one_control_that_can_be_used_carries_the_focus_ring() {
    let login = Login::new();
    let accent = login.tokens().colors.accent;

    let ring = login
        .find(accent)
        .expect("the clear control can still be used, so it carries the focus ring");

    assert!(
        ring.width > 1.0 && ring.height > 1.0,
        "the ring is a ring and not a single pixel: {ring:?}"
    );

    // The one operable thing on the screen is where the focus is.
    assert_eq!(login.state().focus(), Some(FocusTarget::Cancel));
}

#[test]
fn a_started_session_draws_no_control_at_all() {
    let mut login = ready();
    login.type_text("alice");
    let id = submit(&mut login);
    login.update(Message::AuthAnswered(id, AuthReply::Accepted));
    login.update(Message::SessionAnswered(id, true));

    assert_eq!(login.state().greeter().phase(), Phase::SessionStarted);
    assert!(login.state().focus().is_none());
    assert!(
        login.find(login.tokens().colors.accent).is_none(),
        "a screen with nothing left to operate draws no ring and no filled button"
    );
}

#[test]
fn clicking_log_in_produces_exactly_one_submission() {
    let mut login = ready();
    login.type_text("alice");
    // Off the field, so the accent on screen is the log-in button and nothing
    // else — which is what makes the button findable from the pixels.
    login.update(Message::FocusNext);

    let accent = login.tokens().colors.accent;
    let bounds = login.find(accent).expect("an enabled log-in button");
    let messages = login.click(Point::new(bounds.center_x(), bounds.center_y()));

    assert_eq!(messages, vec![Message::Submitted], "one click, one request");

    login.update_all(messages);

    assert_eq!(login.state().greeter().phase(), Phase::Authenticating);
}

#[test]
fn clicking_where_there_is_nothing_does_nothing() {
    let mut login = ready();
    login.type_text("alice");

    let messages = login.click(Point::new(1.0, 1.0));

    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(login.state().greeter().phase(), Phase::Ready);
}

#[test]
fn a_refused_login_is_shown_as_a_refusal_and_stays_retryable() {
    let mut login = ready();
    fail_an_attempt(&mut login);

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert_eq!(login.state().greeter().phase(), Phase::Failed);
    assert!(
        count_of(&pixels, tokens.colors.danger) > 0,
        "a refusal is drawn in the refusal colour"
    );
    assert!(
        bounds_of(&pixels, size, tokens.colors.accent).is_some(),
        "a refusal must leave the person able to try again"
    );

    snapshot("failed", &pixels, size);
}

#[test]
fn an_empty_field_is_a_prompt_and_not_an_error() {
    let login = ready();

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert_eq!(
        login.state().model().identity.error,
        None,
        "a screen that shouts at someone for not having typed yet shouts at \
         everyone the moment it opens"
    );
    assert_eq!(
        count_of(&pixels, tokens.colors.danger),
        0,
        "and nothing is drawn in the refusal colour"
    );
    assert!(
        !draws_a_filled_accent_control(&login),
        "an empty name cannot be submitted, so the log-in control is not drawn \
         as operable"
    );

    snapshot("empty", &pixels, size);
}

#[test]
fn an_invalid_name_is_marked_in_the_field_and_the_button_loses_its_fill() {
    let mut login = ready();
    login.type_text("0alice");

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert_eq!(
        login.state().model().identity.error.as_deref(),
        Some("Enter a valid username.")
    );
    assert!(
        count_of(&pixels, tokens.colors.danger) > 0,
        "the field is marked wrong in the colour that means wrong"
    );
    assert!(
        !draws_a_filled_accent_control(&login),
        "a name the model would refuse is not drawn as submittable"
    );

    snapshot("invalid-name", &pixels, size);
}

#[test]
fn a_spent_allowance_looks_like_a_wait_and_not_like_a_prompt() {
    let mut login = ready();

    for _ in 0..3 {
        login.type_text("alice");
        let id = submit(&mut login);
        login.update(Message::AuthAnswered(id, AuthReply::Rejected));
    }

    login.type_text("alice");

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert!(
        !draws_a_filled_accent_control(&login),
        "there is no attempt to be made, so nothing invites one"
    );
    // The banner is drawn in the warning colour on every screen, so "there is
    // warning ink" proves nothing. What the wait adds is the notice line under the
    // form, so the comparison is against the same screen without it.
    let without_notice = {
        let mut fresh = ready();
        fresh.type_text("alice");
        fresh
    };
    let waiting_ink = count_of(&pixels, tokens.colors.warning);
    let plain_ink = count_of(&without_notice.render(), tokens.colors.warning);

    assert!(
        waiting_ink > plain_ink,
        "the person is told to wait rather than left guessing why the button does \
         nothing: {waiting_ink} warning pixels against {plain_ink} without the \
         notice"
    );
    assert!(
        matches!(
            login.state().model().face,
            inox_greet::view_model::Face::Failed { .. }
        ),
        "the allowance is spent after a refusal, which is the failure state"
    );

    snapshot("waiting", &pixels, size);
}

#[test]
fn an_attempt_in_flight_offers_the_way_out_and_nothing_else() {
    let mut login = ready();
    login.type_text("alice");
    let _ = submit(&mut login);

    let size = login.size();
    let pixels = login.render();

    assert_eq!(login.state().greeter().phase(), Phase::Authenticating);
    assert_eq!(
        login.state().focus(),
        Some(FocusTarget::Cancel),
        "the field and the button are gone, so the focus moved to what is left"
    );
    assert!(
        login.state().model().cancel.enabled,
        "a login that cannot be abandoned is a login that traps the person"
    );

    snapshot("checking", &pixels, size);
}

#[test]
fn the_shared_keyboard_is_the_whole_login_path() {
    // ADR 0021 asks for Tab/Shift+Tab, Enter and Escape; ADR 0015 forbids a
    // shortcut that eats a character someone may type into their own name.
    let mut login = ready();

    assert_eq!(
        app::key_binding(
            &iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
            iced::keyboard::Modifiers::default()
        ),
        Some(Message::FocusNext)
    );
    assert_eq!(
        app::key_binding(
            &iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            iced::keyboard::Modifiers::default()
        ),
        Some(Message::Cancelled)
    );

    // And the path really is walkable: type, focus, operate, clear.
    login.type_text("alice");
    login.update(Message::FocusNext);
    login.update(Message::Activate);
    assert_eq!(login.state().greeter().phase(), Phase::Authenticating);

    let _ = login.state_mut().take_outgoing();
    login.update(Message::FocusNext);
    login.update(Message::Activate);
    assert_eq!(login.state().greeter().phase(), Phase::Ready);
    assert_eq!(login.state().greeter().username(), "");
}
