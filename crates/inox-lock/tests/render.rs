//! What the lock screen draws, asserted on pixels (acceptance Q02, D05).
//!
//! Every test here renders the real widget tree through the tiny-skia renderer
//! the binary uses, at the size acceptance Q02 snapshots are taken at. There is no
//! compositor and no GPU involved, so these are E1 evidence: they prove the screen
//! draws what the view model says it draws, not that it appears on a display, and
//! not that it covers one.
//!
//! The scenarios are the ones acceptance Q02 asks for — light, dark, and the error
//! and empty states — plus the ones ADR 0015 turns into rules: a lock with no UI, a
//! session that cannot be locked at all, and a rate limit that must not look like
//! a prompt.

mod support;

use support::{Lock, count_of, distinct_colours, pixel, snapshot};

use iced::Point;
use inox_common::i18n::Locale;
use inox_config::Theme as Tokens;
use inox_lock::app::Message;
use inox_lock::contract::{AuthFailure, AuthOutcome, ProtocolAbsence, ServiceFailure};
use inox_lock::state::{LockReason, OutputId, Phase};
use inox_lock::view_model::Face;

const OUTPUT: &str = "eDP-1";

fn output() -> OutputId {
    OutputId::new(OUTPUT)
}

/// A lock screen whose compositor offers the protocol and whose session is
/// locked, with its one output covered.
fn locked(tokens: Tokens) -> Lock {
    let mut lock = Lock::with(tokens);

    lock.update(Message::ProtocolPresent);
    lock.update(Message::OutputAdded(output()));
    lock.update(Message::LockRequested(LockReason::User));
    lock.update(Message::OutputCovered(output()));
    lock.state_mut().take_outgoing();

    assert_eq!(lock.state().machine().phase(), Phase::Locked);
    assert!(lock.state().machine().reports_locked());
    lock
}

/// The answer the boundary gives to the attempt in flight.
fn answer(lock: &mut Lock, outcome: AuthOutcome) {
    let generation = lock.state().boundary().generation().expect("an attempt");
    lock.update(Message::AuthAnswered(generation, outcome));
}

/// A locked screen in `locale` whose boundary failed with `failure`.
///
/// The same lock as [`locked`], in a language and a text scale the two can differ
/// in: a translated line has a different length, and `zh-CN` draws glyphs the
/// Latin fallback does not have.
fn locked_in(locale: &Locale, tokens: Tokens, failure: ServiceFailure) -> Lock {
    let mut lock = Lock::in_locale(tokens, locale);

    lock.update(Message::ProtocolPresent);
    lock.update(Message::OutputAdded(output()));
    lock.update(Message::LockRequested(LockReason::User));
    lock.update(Message::OutputCovered(output()));
    lock.update(Message::AuthUnavailable(failure));

    assert_eq!(lock.state().machine().phase(), Phase::Locked);
    lock
}

/// Fails an attempt: types, submits, and has the boundary refuse it.
fn fail_an_attempt(lock: &mut Lock) {
    lock.type_text("wrong");
    lock.update(Message::Submitted);
    answer(lock, AuthOutcome::Failure(AuthFailure::Rejected));
}

#[test]
fn the_locked_prompt_fills_the_screen_with_the_background_token() {
    let lock = locked(Tokens::builtin_dark());
    let pixels = lock.render();
    let size = lock.size();
    let tokens = lock.tokens().clone();

    assert_eq!(
        pixels.len(),
        (size.width as usize) * (size.height as usize) * 4
    );
    assert_eq!(
        pixel(&pixels, size.width as u32, 0, 0),
        support::token_pixel(tokens.colors.background),
        "the whole screen is painted, or the desktop shows through it"
    );
    assert_eq!(
        pixel(
            &pixels,
            size.width as u32,
            size.width as u32 - 1,
            size.height as u32 - 1
        ),
        support::token_pixel(tokens.colors.background),
        "including the corners"
    );
    assert!(
        count_of(&pixels, tokens.colors.surface) > 0,
        "the card is drawn on the surface token"
    );
    assert!(distinct_colours(&pixels) > 2, "there is something to read");
    assert!(
        matches!(lock.state().model().face, Face::Prompt),
        "{:?}",
        lock.state().model().face
    );

    snapshot("dark", &pixels, size);
}

#[test]
fn the_light_theme_draws_a_different_screen() {
    let light = locked(Tokens::builtin_light());
    let dark = locked(Tokens::builtin_dark());

    let light_pixels = light.render();
    let dark_pixels = dark.render();
    let size = light.size();

    assert_ne!(
        pixel(&light_pixels, size.width as u32, 0, 0),
        pixel(&dark_pixels, size.width as u32, 0, 0)
    );
    assert!(distinct_colours(&light_pixels) > 2);

    snapshot("light", &light_pixels, size);
}

#[test]
fn a_disabled_control_is_drawn_differently_from_an_enabled_one() {
    // The same screen and the same typed credential, with the one difference the
    // guidelines name (acceptance D05): whether the control can be used. The
    // enabled one is the accent; the disabled one keeps only the muted text, and
    // the reason it cannot be used is printed beside it.
    let mut enabled = locked(Tokens::builtin_dark());
    enabled.type_text("hunter2");

    let mut disabled = locked(Tokens::builtin_dark());
    disabled.type_text("hunter2");
    disabled.update(Message::AuthUnavailable(ServiceFailure::NotProvided));

    let size = enabled.size();
    let tokens = enabled.tokens().clone();
    let enabled_pixels = enabled.render();
    let disabled_pixels = disabled.render();

    assert!(enabled.state().model().submit.enabled);
    assert!(!disabled.state().model().submit.enabled);
    assert!(
        support::bounds_of(&enabled_pixels, size, tokens.colors.accent).is_some(),
        "a control that can be used is drawn as one"
    );
    assert!(
        support::bounds_of(&disabled_pixels, size, tokens.colors.accent).is_none(),
        "and a control that cannot is not drawn as if it could"
    );
    assert_ne!(
        enabled_pixels, disabled_pixels,
        "a disabled control that looks like an enabled one is not disabled"
    );
    assert!(
        disabled
            .state()
            .model()
            .disabled_reason
            .as_ref()
            .is_some_and(|reason| !reason.is_empty()),
        "a disabled control without a readable reason leaves the user guessing"
    );
}

/// The area one field fills, as the floor for "a field is drawn here".
///
/// Derived from the theme rather than guessed at: the card takes half the surface,
/// a control two thirds of the card, and a field is one body line plus the
/// control's own padding tall.
fn one_field(tokens: &Tokens, size: iced::Size<f32>) -> usize {
    let width = size.width / 3.0;
    let height =
        inox_lock::theme::body_size(tokens) + 2.0 * inox_lock::theme::control_padding(tokens).top;

    (width * height) as usize
}

#[test]
fn a_screen_that_cannot_check_a_password_draws_no_field_to_type_into() {
    // The shipped `--take-lock` state: the screen says it cannot check anything,
    // and a screen that says that must not also hold out a password box (ADR 0025
    // decision 5). The field is drawn on the `background` token inside the card's
    // `surface`, so it is an *area of the background token inside the card* — which
    // is what this counts, rather than looking for a colour that antialiasing can
    // put anywhere.
    let prompt = {
        let mut lock = locked(Tokens::builtin_dark());
        lock.type_text("hunter2");
        lock
    };
    let missing = {
        let mut lock = locked(Tokens::builtin_dark());
        lock.update(Message::AuthUnavailable(ServiceFailure::NotProvided));
        lock
    };

    let size = prompt.size();
    let tokens = prompt.tokens().clone();
    let inset = u32::from(tokens.radii.md) + 2;

    let fill_in = |lock: &Lock| {
        let pixels = lock.render();
        let card = support::bounds_of(&pixels, size, tokens.colors.surface)
            .expect("the card is drawn on the surface token");

        support::ink_inside(&pixels, size, card, tokens.colors.background, inset)
    };

    let drawn = fill_in(&prompt);
    let absent = fill_in(&missing);

    assert!(prompt.state().model().prompts_for_credential);
    assert!(!missing.state().model().prompts_for_credential);
    assert!(
        drawn > one_field(&tokens, size),
        "a screen that asks for a password draws the field to type it into: \
         {drawn} pixels of the field's fill"
    );
    assert!(
        absent < drawn / 10,
        "and a screen that cannot check one draws no field at all: {absent} pixels \
         of the field's fill against {drawn}"
    );
}

#[test]
fn a_typed_credential_is_masked_and_the_unlock_button_is_drawn() {
    let mut lock = locked(Tokens::builtin_dark());
    lock.type_text("hunter2");
    let size = lock.size();
    let tokens = lock.tokens().clone();
    let pixels = lock.render();

    let model = lock.state().model();
    assert_eq!(model.mask, "•••••••");
    assert_eq!(
        model.mask.chars().count(),
        7,
        "one glyph per character, and nothing about the characters"
    );
    assert!(
        !format!("{model:?}").contains("hunter2"),
        "the credential is nowhere near the view model"
    );

    let button = support::bounds_of(&pixels, size, tokens.colors.accent)
        .expect("an enabled unlock button is drawn in the accent colour");
    assert!(
        button.y > size.height / 2.0,
        "the button is below the middle of the screen, not floating over the prompt"
    );
    assert!(button.width > 0.0 && button.height > 0.0);

    snapshot("typed", &pixels, size);
}

#[test]
fn clicking_unlock_produces_exactly_one_submission() {
    let mut lock = locked(Tokens::builtin_dark());
    lock.type_text("hunter2");
    let size = lock.size();
    let tokens = lock.tokens().clone();

    let button = support::bounds_of(&lock.render(), size, tokens.colors.accent)
        .expect("an enabled unlock button");
    let centre = Point::new(
        button.x + button.width / 2.0,
        button.y + button.height / 2.0,
    );

    let messages = lock.click_through(centre);

    assert_eq!(
        messages,
        vec![Message::Submitted],
        "the click became exactly one submission"
    );
    assert!(
        lock.state().boundary().in_flight(),
        "and the attempt is with the boundary"
    );
    assert_eq!(
        lock.state().credential_len(),
        0,
        "the credential left the state with the attempt"
    );
}

#[test]
fn clicking_where_there_is_nothing_does_nothing() {
    let lock = locked(Tokens::builtin_dark());
    let size = lock.size();

    let messages = lock.click(Point::new(2.0, size.height - 2.0));

    assert!(messages.is_empty(), "{messages:?}");
}

#[test]
fn a_refused_attempt_is_shown_in_the_danger_colour() {
    let mut lock = locked(Tokens::builtin_dark());
    fail_an_attempt(&mut lock);
    let size = lock.size();
    let tokens = lock.tokens().clone();
    let pixels = lock.render();

    assert!(
        count_of(&pixels, tokens.colors.danger) > 0,
        "a refusal that is not visible is a refusal a user will retype"
    );

    let model = lock.state().model();
    let Face::Failed { line, reason } = &model.face else {
        panic!("a refused attempt is the failure face: {:?}", model.face);
    };
    assert!(!line.starts_with("lock-") && !reason.starts_with("lock-"));
    assert!(
        model.accepts_input,
        "the user can try again after a refusal"
    );

    snapshot("failed", &pixels, size);
}

#[test]
fn a_spent_allowance_looks_like_a_wait_and_not_like_a_prompt() {
    let mut lock = locked(Tokens::builtin_dark());
    let allowance = inox_lock::attempts::AttemptPolicy::standard().allowance;

    for _ in 0..allowance {
        fail_an_attempt(&mut lock);
    }

    let size = lock.size();
    let tokens = lock.tokens().clone();
    let pixels = lock.render();

    assert!(matches!(lock.state().model().face, Face::Waiting { .. }));
    assert!(
        count_of(&pixels, tokens.colors.warning) > 0,
        "the wait is visible"
    );
    assert!(
        support::bounds_of(&pixels, size, tokens.colors.accent).is_none(),
        "a rate-limited screen must not draw an enabled unlock button"
    );
    assert!(!lock.state().model().accepts_input);

    snapshot("waiting", &pixels, size);
}

#[test]
fn a_lock_with_no_ui_says_so_and_still_covers_the_screen() {
    let mut lock = locked(Tokens::builtin_dark());
    lock.update(Message::UiExited);

    let size = lock.size();
    let tokens = lock.tokens().clone();
    let pixels = lock.render();

    assert_eq!(lock.state().machine().phase(), Phase::LockedWithoutUi);
    assert!(lock.state().machine().reports_locked());
    assert_eq!(
        pixel(&pixels, size.width as u32, 0, 0),
        support::token_pixel(tokens.colors.background)
    );
    assert!(
        support::bounds_of(&pixels, size, tokens.colors.accent).is_none(),
        "nothing can be typed with no UI, so nothing is drawn as pressable"
    );

    let model = lock.state().model();
    let Face::HeldWithoutUi { detail } = &model.face else {
        panic!("a lock with no UI says so: {:?}", model.face);
    };
    assert!(!detail.is_empty());
    assert_eq!(
        model.headline, "The session is locked",
        "the headline says what the screen is, because it is not a prompt"
    );
    assert_eq!(model.hint, None);

    snapshot("held-without-ui", &pixels, size);
}

#[test]
fn a_session_that_cannot_be_locked_says_so_instead_of_prompting() {
    let mut lock = Lock::with(Tokens::builtin_dark());
    lock.update(Message::ProtocolAbsent(ProtocolAbsence::GlobalMissing));

    let size = lock.size();
    let tokens = lock.tokens().clone();
    let pixels = lock.render();

    let Face::Unavailable { reason, retry, .. } = &lock.state().model().face else {
        panic!("a missing protocol is the unavailable face");
    };
    assert!(!reason.starts_with("lock-"), "`{reason}` is a message id");
    assert_eq!(retry, &None);
    assert!(support::bounds_of(&pixels, size, tokens.colors.accent).is_none());
    assert_eq!(
        pixel(&pixels, size.width as u32, 0, 0),
        support::token_pixel(tokens.colors.background),
        "the screen still covers what is under it"
    );

    snapshot("unavailable", &pixels, size);
}

#[test]
fn an_unusable_boundary_offers_a_retry_that_is_drawn() {
    let mut lock = locked(Tokens::builtin_dark());
    lock.update(Message::AuthUnavailable(ServiceFailure::Disconnected));

    let size = lock.size();
    let tokens = lock.tokens().clone();
    let pixels = lock.render();

    assert!(matches!(
        lock.state().model().face,
        Face::Unavailable { retry: Some(_), .. }
    ));

    // The retry control is the screen's secondary action: text on a hairline
    // outline and no fill, which is what guidelines §5 asks a control at rest to
    // look like. What says a control is there is therefore its *outline*, and
    // "this colour appears" is not enough to say so: an antialiased card border
    // lands on the odd stray pixel of any token. So the ink is measured against
    // the same screen in the state that draws no retry at all, where the card's
    // own outline is the only one.
    let without = {
        let mut other = locked(Tokens::builtin_dark());
        other.update(Message::AuthUnavailable(ServiceFailure::NotProvided));
        other
    };
    let outline = count_of(&pixels, tokens.colors.border);
    let without_retry = count_of(&without.render(), tokens.colors.border);

    assert!(matches!(
        without.state().model().face,
        Face::Unavailable { retry: None, .. }
    ));
    assert!(
        outline > without_retry,
        "the retry control's outline is a control's worth of ink, and a control \
         that is not drawn leaves none: {outline} pixels of the outline token here \
         against {without_retry} on the same screen without it"
    );
    assert!(
        support::bounds_of(&pixels, size, tokens.colors.border).is_some(),
        "and the outline is bounded, which is what a drawn control looks like"
    );

    snapshot("boundary-unavailable", &pixels, size);
}

#[test]
fn a_boundary_this_build_does_not_have_draws_no_retry() {
    // The shipped `--take-lock` state: the compositor offers the protocol and this
    // build has no authentication boundary, so the screen says it cannot check
    // anything and offers nothing to press (ADR 0025 decision 5). The difference
    // from the case above is the whole point — a service that is down can come
    // back, one that does not exist cannot — so this is asserted by comparing the
    // two screens rather than by looking for a colour: the outline token turns up
    // in a handful of antialiased pixels on any screen, and a test that looked for
    // it would pass for the wrong reason.
    let missing = {
        let mut lock = locked(Tokens::builtin_dark());
        lock.update(Message::AuthUnavailable(ServiceFailure::NotProvided));
        lock
    };
    let retryable = {
        let mut lock = locked(Tokens::builtin_dark());
        lock.update(Message::AuthUnavailable(ServiceFailure::Disconnected));
        lock
    };

    let size = missing.size();
    let tokens = missing.tokens().clone();
    let pixels = missing.render();
    let retryable_pixels = retryable.render();

    let Face::Unavailable { reason, retry, .. } = &missing.state().model().face else {
        panic!("a missing boundary is the unavailable face");
    };
    assert!(!reason.starts_with("lock-"), "`{reason}` is a message id");
    assert_eq!(retry, &None, "there is nothing to try again");
    assert_eq!(
        missing.state().model().disabled_reason.as_deref(),
        Some("This build has no authentication service")
    );

    assert!(
        support::bounds_of(&pixels, size, tokens.colors.accent).is_none(),
        "nothing on the screen is drawn as operable"
    );
    assert_eq!(
        pixel(&pixels, size.width as u32, 0, 0),
        support::token_pixel(tokens.colors.background),
        "the screen still covers what is under it"
    );

    assert!(
        matches!(
            retryable.state().model().face,
            Face::Unavailable { retry: Some(_), .. }
        ),
        "the other screen is the one that offers a retry, so it is the comparison"
    );
    assert!(
        count_of(&pixels, tokens.colors.border) < count_of(&retryable_pixels, tokens.colors.border),
        "a boundary that cannot come back draws no control, and a control that is \
         not drawn leaves no outline behind"
    );
    assert!(
        pixels != retryable_pixels,
        "a screen with no retry control must not look like the same state with one"
    );

    snapshot("boundary-not-provided", &pixels, size);
}

#[test]
fn the_chinese_catalog_renders_a_screen_that_still_fits() {
    // Acceptance Q03 says the catalogs are complete; this is the other half of it,
    // which a string comparison cannot see: the translated screen has to fit. It
    // is a real risk here rather than a formality — `zh-CN` draws glyphs the Latin
    // fallback does not have, the lines are a different length, and the widest
    // text on this screen is the reason line under the prompt.
    let locale = Locale::parse("zh-CN").expect("a well-formed tag");
    let mut tokens = Tokens::builtin_dark();
    tokens.typography.scale = 2.0;
    let lock = locked_in(&locale, tokens.clone(), ServiceFailure::Disconnected);
    let without_retry = locked_in(&locale, tokens.clone(), ServiceFailure::NotProvided);

    let size = lock.size();
    let tokens = lock.tokens().clone();
    let pixels = lock.render();

    assert_eq!(
        pixel(&pixels, size.width as u32, 0, 0),
        support::token_pixel(tokens.colors.background),
        "the screen still covers the output"
    );
    assert!(
        !lock.state().model().headline.is_empty() && lock.state().model().headline != "lock-prompt",
        "the headline came from the catalog: {:?}",
        lock.state().model().headline
    );

    // The retry control is a hairline outline with no fill, so it is found by the
    // outline it adds rather than by a colour of its own (guidelines §5).
    assert!(
        count_of(&pixels, tokens.colors.border)
            > count_of(&without_retry.render(), tokens.colors.border),
        "the retry control is drawn"
    );

    let drawn = support::bounds_of(&pixels, size, tokens.colors.border)
        .expect("the screen has an outline on it");
    assert!(
        drawn.x >= 0.0 && drawn.x + drawn.width <= size.width,
        "nothing has overflowed the screen horizontally at 200 % text in the \
         translated catalog: {drawn:?}"
    );

    snapshot("chinese-large-text", &pixels, size);
}

#[test]
fn doubled_text_still_leaves_the_unlock_button_on_screen() {
    let mut tokens = Tokens::builtin_dark();
    tokens.typography.scale = 2.0;
    let mut lock = locked(tokens);
    lock.type_text("a rather long credential indeed");

    let size = lock.size();
    let pixel_colour = lock.tokens().colors.accent;
    let pixels = lock.render();

    let button = support::bounds_of(&pixels, size, pixel_colour).expect("an enabled unlock button");
    assert!(
        button.y + button.height <= size.height,
        "the button is inside the screen at 200 % text (acceptance Q02)"
    );
    assert!(
        button.x >= 0.0 && button.x + button.width <= size.width,
        "and inside it horizontally"
    );

    snapshot("large-text", &pixels, size);
}

#[test]
fn the_longest_state_fits_the_screen_at_two_hundred_percent_text() {
    // The rate-limited screen at 200 % is the tallest thing this surface draws: the
    // heading, two lines of state, the field, the reason, the control and the hint.
    // Acceptance Q02 asks for no clipping at 200 %, and this surface has no
    // scrollable to hide behind — it covers an output whose size it does not choose
    // — so the card has to end inside the output rather than at its edge.
    for tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
        let mut tokens = tokens;
        tokens.typography.scale = 2.0;
        let mut lock = locked(tokens);

        for _ in 0..inox_lock::attempts::AttemptPolicy::standard().allowance {
            fail_an_attempt(&mut lock);
        }

        let size = lock.size();
        let pixels = lock.render();

        assert!(matches!(lock.state().model().face, Face::Waiting { .. }));

        let card = support::bounds_of(&pixels, size, lock.tokens().colors.surface)
            .expect("the card is drawn on the surface token");

        assert!(
            card.y >= 0.0 && card.y + card.height <= size.height,
            "the card is inside the output at 200 % text: {card:?}"
        );
        assert!(
            card.x >= 0.0 && card.x + card.width <= size.width,
            "and inside it horizontally: {card:?}"
        );
    }
}

#[test]
fn the_attempt_that_was_sent_carries_the_credential_out_of_the_state() {
    let mut lock = locked(Tokens::builtin_dark());
    lock.type_text("hunter2");
    let _ = lock.state_mut().take_outgoing();
    lock.update(Message::Submitted);

    let queued = format!("{:?}", lock.state_mut().take_outgoing());

    assert!(
        !queued.contains("hunter2"),
        "the queued command prints the credential: {queued}"
    );

    answer(&mut lock, AuthOutcome::Success);
    lock.update(Message::UnlockConfirmed);

    assert_eq!(lock.state().machine().phase(), Phase::Unlocked);
}
