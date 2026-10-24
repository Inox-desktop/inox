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
//! this component's own rules turn into facts about pixels: an authenticator that
//! is not there takes no input, a control that cannot be used draws no ring, the
//! account a login would use is drawn as a medallion and a name, and the accounts
//! it could use instead are a list the keyboard can walk.

mod support;

use support::{
    Login, accounts, bounds_of, count_of, find_disc, ink_inside, is_opaque, pixel, snapshot,
    token_pixel,
};

use iced::Point;
use inox_common::i18n::Locale;
use inox_config::Theme as Tokens;
use inox_greet::accounts::Account;
use inox_greet::state::{AttemptId, AuthReply, Phase, SESSION_STARTED_DWELL};
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
/// a ring is an outline. At 1280×720 the log-in control's fill is about 13,600
/// pixels — two thirds of the half-width card — while the ring around the
/// password field is about 2,000, so this floor sits between them with a factor of
/// three either way. The count is measured from the pixels rather than guessed at
/// from a coordinate, so the test cannot drift away from the layout.
const FILLED_ACCENT_FLOOR: usize = 4_000;

/// Whether a control is drawn *filled* in the accent colour.
fn draws_a_filled_accent_control(login: &Login) -> bool {
    count_of(&login.render(), login.tokens().colors.accent) > FILLED_ACCENT_FLOOR
}

/// Submits what is in the form and returns the attempt the model has in flight.
///
/// The harness drives the state machine rather than the transport, so the attempt is
/// read from the model: `app::update` drains the command queue into the tasks that
/// would speak `auth-v1`, and `tests/boundary.rs` is where that path is exercised.
fn submit(login: &mut Login) -> AttemptId {
    login.focus(FocusTarget::Submit);
    login.update(Message::Submitted);

    login
        .state()
        .greeter()
        .attempt_in_flight()
        .expect("an authentication request")
}

/// Fails an attempt: types a password, submits, and is refused.
fn fail_an_attempt(login: &mut Login) {
    login.type_credential("hunter2");
    let id = submit(login);
    login.update(Message::AuthAnswered(id, AuthReply::Rejected));
}

/// The area one field fills, as the floor for "a field is drawn here".
///
/// Derived from the theme rather than guessed at: the card takes half the window,
/// a control two thirds of the card, and a field is a label, the value's line and
/// the control's own padding tall.
fn one_field(tokens: &Tokens, size: iced::Size<f32>) -> usize {
    let width = size.width / 3.0;
    let height = inox_greet::theme::caption_size(tokens)
        + f32::from(tokens.spacing.sm)
        + inox_greet::theme::body_size(tokens)
        + 2.0 * inox_greet::theme::control_padding(tokens).top;

    (width * height) as usize
}

#[test]
fn a_screen_that_cannot_log_anyone_in_draws_no_field_to_type_into() {
    // `--preview` is this build's honest state (ADR 0021): there is no
    // authentication boundary, so the password field cannot be typed into, and a
    // framed box under a screen that says so is the same invitation as a prompt.
    // The field is drawn as a patch of the `background` token inside the card's
    // `surface`, so it is that patch this counts.
    let disabled = Login::new();
    let editable = ready();

    let size = disabled.size();
    let tokens = disabled.tokens().clone();
    let inset = u32::from(tokens.radii.md) + 2;

    let fill_in = |login: &Login| {
        let pixels = login.render();
        let card = bounds_of(&pixels, size, tokens.colors.surface)
            .expect("the card is drawn on the surface token");

        ink_inside(&pixels, size, card, tokens.colors.background, inset)
    };

    let drawn = fill_in(&editable);
    let absent = fill_in(&disabled);

    assert!(editable.state().model().credential.editable);
    assert!(!disabled.state().model().credential.editable);
    assert!(
        drawn >= absent + one_field(&tokens, size) / 2,
        "a screen that can log someone in draws a framed password field, and a \
         screen that cannot draws none of it: {drawn} pixels of the field's fill \
         against {absent}"
    );
}

#[test]
fn the_account_a_login_would_use_is_drawn_as_a_medallion_beside_its_name() {
    // The identity is a picture of an account rather than a text box: the medallion
    // is the only circle on the screen, and it is as wide as it is tall.
    let login = ready();
    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    let medallion = find_disc(&pixels, size, tokens.colors.surface_variant)
        .expect("the medallion is drawn on the alternate surface");
    let diameter = inox_greet::theme::medallion_size(&tokens);

    assert!(
        (medallion.width - medallion.height).abs() <= 2.0,
        "the medallion is a circle, not an ellipse: {medallion:?}"
    );
    assert!(
        (medallion.width - diameter).abs() <= 6.0,
        "the medallion is {} px across, and the theme says {diameter}: the fill \
         measured here is the diameter less the outline and its anti-aliasing",
        medallion.width
    );

    // The name is the other thing drawn in the text colour: the letter on the
    // medallion is one glyph wide, and the name is wider than the disc it sits
    // under and reaches below it.
    let name = bounds_of(&pixels, size, tokens.colors.text).expect("the account's name");

    assert!(
        name.width > medallion.width * 2.0,
        "the account's name is drawn, not just the initial: {name:?}"
    );
    assert!(
        name.y + name.height > medallion.y + medallion.height,
        "the name is drawn under the medallion, not through it"
    );

    snapshot("account", &pixels, size);
}

#[test]
fn the_accounts_the_screen_offers_are_a_list_the_keyboard_can_walk() {
    let mut login = ready();
    login.open_account_list();

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert_eq!(
        login.state().model().focus,
        Some(FocusTarget::account(0)),
        "the list opens on a row the keyboard is showing the focus on"
    );
    assert!(
        draws_a_filled_accent_control(&login),
        "the account a login would use is the filled row"
    );
    assert!(
        count_of(&pixels, tokens.colors.border) > 0,
        "the rows that are not chosen have their own outline"
    );

    snapshot("accounts", &pixels, size);
}

#[test]
fn a_screen_with_no_account_at_all_says_so_instead_of_prompting() {
    let mut login = Login::with_accounts(
        Tokens::builtin_dark(),
        true,
        &Locale::default_locale(),
        Vec::new(),
    );

    assert!(login.state().model().accounts.is_empty());
    assert!(!login.state().model().credential.editable);
    assert!(login.state().model().switch_user.is_none());
    assert!(
        !draws_a_filled_accent_control(&login),
        "there is nobody to log in as, so nothing is drawn as operable"
    );
    assert!(
        login.state().model().accounts_reason.is_some(),
        "the screen has to say why it has nobody to offer"
    );

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert_eq!(
        find_disc(&pixels, size, tokens.colors.surface_variant),
        None,
        "no medallion is drawn for an account that does not exist"
    );
    assert!(
        count_of(&pixels, tokens.colors.danger) > 0 || count_of(&pixels, tokens.colors.warning) > 0,
        "the reason is drawn in a colour that reads as a problem"
    );

    // And it stays usable as a screen: the keyboard has somewhere to be.
    login.focus(FocusTarget::Cancel);

    snapshot("no-accounts", &pixels, size);
}

#[test]
fn an_account_whose_name_the_boundary_would_refuse_is_drawn_as_a_refusal() {
    let login = Login::with_accounts(
        Tokens::builtin_dark(),
        true,
        &Locale::default_locale(),
        vec![Account::new("a b", 1000, None)],
    );

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert!(
        !login.state().model().submit.enabled,
        "a name the contract would not carry cannot be submitted"
    );
    assert!(
        !draws_a_filled_accent_control(&login),
        "and nothing is drawn as if it could be"
    );
    assert!(
        count_of(&pixels, tokens.colors.warning) > 0,
        "the reason is on screen rather than only in the model"
    );

    snapshot("unusable-account", &pixels, size);
}

#[test]
fn the_longest_state_fits_the_screen_at_two_hundred_percent_text() {
    // The unavailable state at 200 % is the tallest thing this surface ever draws:
    // the medallion and the name, the banner, the field, the lines of state, two
    // rows of controls and the session line. Acceptance Q02 asks for no clipping at
    // 200 %, so every line — the last one included — has to end inside the
    // 1280×720 window rather than at its edge.
    let zh = Locale::parse("zh-CN").expect("a well-formed tag");
    let english = Locale::default_locale();

    for locale in [&english, &zh] {
        for mut tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
            tokens.typography.scale = 2.0;
            let login = Login::in_locale(tokens, false, locale);

            let size = login.size();
            let pixels = login.render();

            for (what, colour) in [
                ("the card's outline", login.tokens().colors.border),
                ("the last line of text", login.tokens().colors.text_muted),
            ] {
                let ink = bounds_of(&pixels, size, colour)
                    .unwrap_or_else(|| panic!("{what} is drawn at 200 % ({locale:?})"));

                assert!(
                    ink.y + ink.height < size.height,
                    "{what} ends at {} in a window {} tall, so the layout is cut off \
                     instead of fitting ({locale:?})",
                    ink.y + ink.height,
                    size.height
                );
            }
        }
    }
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
                // Off the field, so the accent on screen is the log-in control and
                // nothing else.
                usable.focus(FocusTarget::Submit);
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
fn the_password_field_carries_the_focus_ring_until_the_keyboard_leaves_it() {
    // The field is not an iced widget that can hold focus, so the ring is the
    // projection's answer — and it is the answer the person has to be able to see
    // (acceptance Q01).
    let mut login = ready();

    assert_eq!(login.state().focus(), Some(FocusTarget::Credential));
    let on_the_field = login.find(login.tokens().colors.accent).expect("a ring");

    login.focus(FocusTarget::Submit);
    let on_the_button = login.find(login.tokens().colors.accent).expect("a ring");

    assert!(
        on_the_field.width > on_the_button.width,
        "a ring drawn around a button is not a ring drawn around the field: \
         {on_the_field:?} against {on_the_button:?}"
    );
}

#[test]
fn a_started_session_draws_no_control_at_all() {
    let mut login = ready();
    login.type_credential("hunter2");
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
fn a_started_session_stays_on_screen_and_the_login_comes_back() {
    // ADR 0037 §决策 4: the session starting is not the end of this window. The
    // screen shows the started face, and three seconds later it is a login screen
    // again — same window, same account, no password. What happens in between is
    // that the VT is paused (ADR 0037 §决策 2/3), so the frame the person comes back
    // to has to be a login rather than nothing at all.
    //
    // The dwell ends on the tick this component already runs for the reply
    // deadline: nothing here sleeps, and the exact instant the session started is
    // the model's, so the tick is simply past it.
    for tag in ["en-US", "zh-CN"] {
        let locale = Locale::parse(tag).expect("a well-formed tag");

        for mut tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
            for scale in [1.0, 2.0] {
                tokens.typography.scale = scale;
                let name = format!("{}-{tag}-{scale}", tokens.name);

                let mut login = Login::in_locale(tokens.clone(), true, &locale);
                login.type_credential("hunter2");
                let id = submit(&mut login);
                login.update(Message::AuthAnswered(id, AuthReply::Accepted));
                login.update(Message::SessionAnswered(id, true));

                let size = login.size();
                let started_tokens = login.tokens().clone();
                let started_pixels = login.render();

                assert_eq!(
                    login.state().greeter().phase(),
                    Phase::SessionStarted,
                    "the screen is still here when the session starts ({name})"
                );
                assert!(
                    matches!(
                        login.state().model().face,
                        inox_greet::view_model::Face::Started { .. }
                    ),
                    "and it says what happened ({name})"
                );
                assert!(
                    !draws_a_filled_accent_control(&login),
                    "a screen nothing can be done on draws nothing that invites it ({name})"
                );
                assert!(
                    count_of(&started_pixels, started_tokens.colors.success) > 0,
                    "the started face is drawn in the colour that means it worked ({name})"
                );
                assert!(is_opaque(&started_pixels), "the screen is opaque ({name})");
                snapshot(&format!("started-{name}"), &started_pixels, size);

                login.update(Message::Ticked(
                    std::time::Instant::now()
                        + SESSION_STARTED_DWELL
                        + std::time::Duration::from_secs(1),
                ));

                let resumed = login.render();

                assert_eq!(
                    login.state().greeter().phase(),
                    Phase::Ready,
                    "the login screen is a login screen again ({name})"
                );
                assert_eq!(
                    login.state().greeter().username(),
                    "alice",
                    "with the account the session was started for ({name})"
                );
                assert!(
                    login.state().greeter().credential_is_empty(),
                    "and none of the password ({name})"
                );
                assert_eq!(
                    login.state().model().credential.mask,
                    "",
                    "nothing of it is drawn either ({name})"
                );
                assert_eq!(
                    login.state().focus(),
                    Some(FocusTarget::Credential),
                    "the keyboard is back where the typing starts ({name})"
                );
                assert!(
                    login.state().model().submit.enabled,
                    "and a second login can be submitted ({name})"
                );
                assert!(
                    draws_a_filled_accent_control(&login),
                    "the log-in control is drawn as operable again ({name})"
                );
                assert_ne!(
                    started_pixels, resumed,
                    "the frame the person comes back to is not the one they left ({name})"
                );
                snapshot(&format!("resumed-{name}"), &resumed, size);
            }
        }
    }
}

#[test]
fn clicking_log_in_produces_exactly_one_submission() {
    let mut login = ready();
    // Off the field, so the accent on screen is the log-in button and nothing
    // else — which is what makes the button findable from the pixels.
    login.focus(FocusTarget::Submit);

    let accent = login.tokens().colors.accent;
    let bounds = login.find(accent).expect("an enabled log-in button");
    let messages = login.click(Point::new(bounds.center_x(), bounds.center_y()));

    assert_eq!(messages, vec![Message::Submitted], "one click, one request");

    login.update_all(messages);

    assert_eq!(login.state().greeter().phase(), Phase::Authenticating);
}

#[test]
fn clicking_a_row_of_the_account_list_chooses_that_account() {
    let mut login = ready();
    login.open_account_list();

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();
    let card = bounds_of(&pixels, size, tokens.colors.surface).expect("the card");

    // The chosen row is the one filled with the accent, so the row *below* it is one
    // row height and one `sm` further down — measured from the pixels rather than
    // guessed at, so the click cannot drift away from the layout.
    let chosen = bounds_of(&pixels, size, tokens.colors.accent).expect("the chosen row");
    let next_row = chosen.y + chosen.height + f32::from(tokens.spacing.sm) + chosen.height / 2.0;
    let point = Point::new(card.center_x(), next_row);

    let messages = login.click(point);

    assert!(
        messages.iter().any(|message| matches!(
            message,
            Message::AccountChosen(name) if name == "bob"
        )),
        "clicking a row chooses the account that row is: {messages:?}"
    );

    login.update_all(messages);
    assert_eq!(login.state().greeter().username(), "bob");
    assert!(
        !login.state().model().choosing,
        "the list closed on the answer"
    );
}

#[test]
fn clicking_where_there_is_nothing_does_nothing() {
    let login = ready();

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
fn a_screen_nobody_has_typed_into_is_not_an_error() {
    let login = ready();

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert_eq!(
        login.state().model().credential.error,
        None,
        "an account may have no password, so an empty field is not a mistake"
    );
    assert_eq!(
        count_of(&pixels, tokens.colors.danger),
        0,
        "and nothing is drawn in the refusal colour"
    );
    assert!(
        login.state().model().submit.enabled,
        "the account on screen is the account that would be authenticated"
    );
    assert!(
        !login.state().model().focus_order.is_empty(),
        "so there is something for the keyboard to do"
    );

    snapshot("empty", &pixels, size);
}

#[test]
fn a_spent_allowance_looks_like_a_wait_and_not_like_a_prompt() {
    let mut login = ready();

    for _ in 0..3 {
        login.type_credential("hunter2");
        let id = submit(&mut login);
        login.update(Message::AuthAnswered(id, AuthReply::Rejected));
    }

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert!(
        !draws_a_filled_accent_control(&login),
        "there is no attempt to be made, so nothing invites one"
    );
    // The banner is drawn in the warning colour on every screen, so "there is
    // warning ink" proves nothing. What the wait adds is the notice line, so the
    // comparison is against the same screen without it.
    let without_notice = ready();
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
    login.type_credential("hunter2");
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
    assert!(
        login.state().model().switch_user.is_none(),
        "a login being checked is not a moment to offer another account"
    );

    snapshot("checking", &pixels, size);
}

#[test]
fn the_shared_keyboard_is_the_whole_login_path() {
    // ADR 0021 asks for Tab/Shift+Tab, Enter and Escape; ADR 0015 forbids a
    // shortcut that eats a character someone may type into their own password.
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

    // And the path really is walkable with the keyboard alone: type the password,
    // tab to the control and operate it; then choose another account, and clear.
    login.focus(FocusTarget::Credential);
    login.type_credential("hunter2");
    login.focus(FocusTarget::Submit);
    login.update(Message::Activate);
    assert_eq!(login.state().greeter().phase(), Phase::Authenticating);

    login.focus(FocusTarget::Cancel);
    login.update(Message::Activate);
    assert_eq!(login.state().greeter().phase(), Phase::Ready);
    assert!(login.state().greeter().credential_is_empty());

    login.choose_account("bob");
    assert_eq!(login.state().greeter().username(), "bob");

    login.focus(FocusTarget::Submit);
    login.update(Message::Activate);
    assert_eq!(login.state().greeter().phase(), Phase::Authenticating);
}

#[test]
fn the_credential_field_draws_a_count_and_not_a_value() {
    let mut login = ready();
    let before = login.render();

    login.type_credential("hunter2");

    // The model knows the length and nothing else that could be drawn.
    assert_eq!(login.state().greeter().credential_len(), 7);
    assert_eq!(login.state().model().credential.mask.chars().count(), 7);
    assert!(
        login
            .state()
            .model()
            .credential
            .mask
            .chars()
            .all(|glyph| glyph == inox_greet::view_model::MASK_GLYPH),
        "the mask is one glyph per character and none of the characters"
    );

    let after = login.render();
    assert_ne!(
        before, after,
        "the field the person typed into has to look different afterwards"
    );
}

#[test]
fn a_credential_past_the_contracts_bound_is_refused_rather_than_truncated() {
    let mut login = ready();
    login.type_credential(&"x".repeat(inox_auth::wire::MAX_CREDENTIAL_BYTES + 1));

    let size = login.size();
    let tokens = login.tokens().clone();
    let pixels = login.render();

    assert!(login.state().greeter().credential_too_long());
    assert_eq!(
        login.state().model().credential.error.as_deref(),
        Some("That password is too long.")
    );
    assert!(
        !login.state().model().submit.enabled,
        "a truncated password is a different password, so it is refused"
    );
    assert!(
        count_of(&pixels, tokens.colors.danger) > 0,
        "the reason is drawn in the colour that means wrong"
    );

    snapshot("credential-too-long", &pixels, size);
}

#[test]
fn the_account_database_is_read_once_and_handed_to_the_screen() {
    // The roster is the screen's list: what it draws, what it preselects and what a
    // login would authenticate are one list rather than three that could disagree.
    let login = Login::with_accounts(
        Tokens::builtin_dark(),
        true,
        &Locale::default_locale(),
        accounts(),
    );

    let model = login.state().model();

    assert_eq!(model.accounts.len(), 2);
    assert_eq!(
        model.selected.as_ref().map(|account| account.name.as_str()),
        Some("alice"),
        "the first ordinary account the roster offers is the one preselected"
    );
    assert_eq!(
        model
            .selected
            .as_ref()
            .map(|account| account.label.as_str()),
        Some("Alice Doe"),
        "the full name the account database carries is what is drawn"
    );
}
