//! What the component actually draws, as pixels (acceptance Q02, D05).
//!
//! Every scenario runs the real pipeline — a real PNG written to a temporary
//! directory, the real decoder, the real state transition, the real widget tree
//! and the renderer the binary uses — and then asserts on the bytes that came
//! out. Nothing here reads the source of a view function or trusts a comment.
//!
//! The assertions are chosen so that they do not depend on font metrics: what is
//! asserted about a control is its background, not its glyphs, and what is
//! asserted about a card is the rectangle its token colour covers, not the text
//! inside it. That is also what makes the 200% font-scale scenario meaningful —
//! the card either grows and stays inside the window, or the scenario fails, on a
//! machine with no fonts installed as much as on one with them.

mod support;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use iced_core::mouse;
use iced_core::{Event, Point};

use inox_common::i18n::{Locale, Localizer};
use inox_config::{Appearance, Theme, WallpaperMode};
use inox_wallpaper::app::{Message, PREVIEW_SIZE, Preview, State, update};
use inox_wallpaper::model::{OutputName, Selection, Size as OutputSize, Unusable};
use inox_wallpaper::view_model;
use inox_wallpaper::{apply, i18n, theme, view};

use support::{Bounds, Pixel, SIZE};

/// The colour of every fixture image.
///
/// Deliberately nothing any built-in theme uses, so that "this pixel is the
/// wallpaper" and "this pixel is a token" can never be confused.
const IMAGE: [u8; 4] = [1, 2, 3, 255];

/// The output the preview stands in for.
const OUTPUT: &str = "HDMI-A-1";

fn tokens(appearance: Appearance) -> Arc<Theme> {
    Arc::new(match appearance {
        Appearance::Light => Theme::builtin_light(),
        _ => Theme::builtin_dark(),
    })
}

fn tokens_light() -> Arc<Theme> {
    tokens(Appearance::Light)
}

fn tokens_dark() -> Arc<Theme> {
    tokens(Appearance::Dark)
}

/// The same tokens at a font-scale preference, which is what 200% means here.
fn tokens_at_scale(appearance: Appearance, scale: f32) -> Arc<Theme> {
    let mut tokens = match appearance {
        Appearance::Light => Theme::builtin_light(),
        _ => Theme::builtin_dark(),
    };
    tokens.typography.scale = scale;

    Arc::new(tokens)
}

fn localizer() -> Rc<Localizer> {
    Rc::new(i18n::localizer(Locale::default_locale()).expect("the shipped catalogs"))
}

/// A PNG of `width` × `height` in the fixture colour.
fn fixture(dir: &Path, width: u32, height: u32) -> PathBuf {
    let path = dir.join("wall.png");
    image::RgbaImage::from_pixel(width, height, image::Rgba(IMAGE))
        .save(&path)
        .expect("the fixture is written");

    path
}

/// A state that has loaded nothing yet.
fn boot(
    tokens: Arc<Theme>,
    selection: Selection,
    mode: WallpaperMode,
    reason: Option<Unusable>,
) -> State {
    State::new(
        tokens,
        localizer(),
        Preview {
            output: OutputName::from(OUTPUT),
            size: OutputSize::new(PREVIEW_SIZE.width, PREVIEW_SIZE.height),
            mode,
            selection,
            wallpaper_dirs: Arc::from(Vec::<PathBuf>::new()),
        },
        reason,
    )
}

/// A state that went through the request the runtime would have made.
fn requested(tokens: Arc<Theme>, selection: Selection, mode: WallpaperMode) -> State {
    let mut state = boot(tokens, selection, mode, None);
    let request = state
        .take_request()
        .expect("a request is queued for a configured image");
    let preparation = apply::prepare(
        request.output,
        &request.selection,
        mode,
        OutputSize::new(PREVIEW_SIZE.width, PREVIEW_SIZE.height),
        &[],
    );

    let _ = update(
        &mut state,
        Message::FramePrepared {
            generation: request.generation,
            preparation,
        },
    );

    state
}

/// The element a state renders to, at the size every scenario uses.
fn element(state: &State) -> iced::Element<'_, Message> {
    view::view(state)
}

/// Draws `state` and returns its pixels.
fn draw(state: &State, scenario: &str) -> Vec<u8> {
    let pixels = support::render(element(state), SIZE, &theme::iced_theme(state.theme()));

    support::dump(scenario, pixels, SIZE)
}

fn colour(rgba: inox_config::Rgba) -> Pixel {
    Pixel::from(theme::color(rgba))
}

/// The window inset the tokens ask for, in whole pixels.
fn inset(tokens: &Theme) -> u32 {
    theme::window_padding(tokens).round() as u32
}

/// The card outline the tokens ask for, in whole pixels.
fn outline(tokens: &Theme) -> u32 {
    theme::hairline(tokens).ceil() as u32
}

/// The bands of card surface in a drawing: the top-left column of cards, then the
/// footer strip.
fn cards(pixels: &[u8], tokens: &Theme) -> Vec<Bounds> {
    support::regions(pixels, SIZE, colour(tokens.colors.surface))
}

/// The one card band of a drawing that has exactly one card column.
fn top_card(pixels: &[u8], tokens: &Theme) -> Bounds {
    let regions = cards(pixels, tokens);
    *regions.first().expect("the status card is always drawn")
}

/// Clicks the middle of `bounds` and returns the messages that produced.
fn click(state: &State, bounds: Bounds) -> Vec<Message> {
    let centre = Point::new(
        f32::midpoint(bounds.min_x as f32, bounds.max_x as f32),
        f32::midpoint(bounds.min_y as f32, bounds.max_y as f32),
    );

    support::drive(
        element(state),
        &[
            Event::Mouse(mouse::Event::CursorMoved { position: centre }),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ],
        centre,
        SIZE,
    )
}

#[test]
fn a_light_scenario_draws_the_wallpaper_that_was_configured() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 640, 480);
    let state = requested(tokens_light(), Selection::Image(path), WallpaperMode::Fill);
    let pixels = draw(&state, "light");

    assert_eq!(
        pixels.len(),
        (SIZE.width as usize) * (SIZE.height as usize) * 4,
        "one RGBA quadruple per pixel"
    );
    assert_eq!(
        support::centre(&pixels, SIZE),
        Pixel {
            red: IMAGE[0],
            green: IMAGE[1],
            blue: IMAGE[2],
            alpha: 255,
        },
        "a fill covers the output with the image"
    );
    // The cards are drawn over the wallpaper, so the font-independent way to say
    // "the preview is still the preview" is the drawing underneath them: the fill
    // covers the whole output. Asserting the glyphs themselves would depend on
    // font rasterization, which crate AGENTS.md forbids — a machine with no fonts
    // installed has to pass this test too. That the card lines are produced at all
    // is asserted in `view_model`, where they are strings.
    assert_eq!(
        support::most_common(&pixels),
        Pixel {
            red: IMAGE[0],
            green: IMAGE[1],
            blue: IMAGE[2],
            alpha: 255,
        },
        "a fill covers the output with the image, with the cards drawn over it"
    );
}

#[test]
fn a_dark_scenario_draws_the_same_wallpaper_in_its_own_colours() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 640, 480);
    // Fitted, not filled: the margins are where the theme's own background shows,
    // which is the part of the drawing the appearance decides.
    let light_state = requested(
        tokens_light(),
        Selection::Image(path.clone()),
        WallpaperMode::Fit,
    );
    let dark_state = requested(tokens_dark(), Selection::Image(path), WallpaperMode::Fit);

    let light = draw(&light_state, "light-fit");
    let dark = draw(&dark_state, "dark-fit");

    assert_eq!(support::centre(&light, SIZE).red, IMAGE[0]);
    assert_eq!(support::centre(&dark, SIZE).red, IMAGE[0]);
    assert_eq!(
        support::pixel(&light, SIZE, 4, 4),
        colour(tokens_light().colors.background),
        "the margin is the light theme's background"
    );
    assert_eq!(
        support::pixel(&dark, SIZE, 4, 4),
        colour(tokens_dark().colors.background),
        "the margin is the dark theme's background"
    );
    assert_ne!(
        light, dark,
        "the two appearances must not collapse into one"
    );
}

#[test]
fn an_empty_scenario_draws_the_theme_background_and_an_explicit_empty_card() {
    // This used to assert that nothing at all was drawn, because the projection
    // produced no notice for a configuration that names no image. The UI spec §6
    // requires the empty state to be covered like every other state — "what
    // happened, and what the user can do" — so the theme background is still the
    // whole drawing, and the card that states the empty state is the change.
    let tokens = tokens_light();
    let state = boot(
        Arc::clone(&tokens),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let pixels = draw(&state, "empty");

    for (x, y) in [(4, 4), (640, 360), (1270, 710)] {
        assert_eq!(
            support::pixel(&pixels, SIZE, x, y),
            colour(tokens.colors.background),
            "nothing configured means the theme background"
        );
    }
    assert!(
        support::contains(&pixels, colour(tokens.colors.surface)),
        "the empty state is stated on screen"
    );
    assert!(
        !support::contains(&pixels, colour(tokens.colors.accent)),
        "the empty state offers nothing to press"
    );
    let model = view_model::project(&state);
    assert!(
        model.card.hint.is_some(),
        "and the card says what would change it"
    );
}

#[test]
fn an_error_scenario_degrades_to_the_theme_background_and_says_so() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let missing = dir.path().join("gone.png");
    let state = requested(
        tokens_dark(),
        Selection::Image(missing.clone()),
        WallpaperMode::Fill,
    );
    let pixels = draw(&state, "missing-image");

    assert_eq!(
        support::centre(&pixels, SIZE),
        colour(tokens_dark().colors.background),
        "an image that cannot be used is never a black output"
    );
    assert!(
        support::contains(&pixels, colour(tokens_dark().colors.surface)),
        "the diagnosis is drawn in a card"
    );
    assert!(
        support::contains(&pixels, colour(tokens_dark().colors.accent)),
        "and the control that retries it is enabled"
    );
}

#[test]
fn a_disabled_control_is_drawn_in_the_disabled_token_and_an_enabled_one_is_not() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let light = tokens_light();
    let disabled = boot(
        Arc::clone(&light),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        Some(Unusable::Unreadable {
            detail: "permission denied".to_owned(),
        }),
    );
    let enabled = requested(
        Arc::clone(&light),
        Selection::Image(dir.path().join("gone.png")),
        WallpaperMode::Fill,
    );

    let disabled_pixels = draw(&disabled, "disabled");
    let enabled_pixels = draw(&enabled, "enabled");

    assert!(
        support::contains(&disabled_pixels, colour(light.colors.surface_variant)),
        "a disabled control is drawn in the surface-variant token"
    );
    assert!(
        !support::contains(&disabled_pixels, colour(light.colors.accent)),
        "and not in the accent one"
    );
    assert!(
        support::contains(&enabled_pixels, colour(light.colors.accent)),
        "an enabled control is drawn in the accent token"
    );
    assert!(
        !support::contains(&enabled_pixels, colour(light.colors.surface_variant)),
        "and not in the disabled one"
    );
    assert_ne!(
        disabled_pixels, enabled_pixels,
        "the two states must not be drawn the same way"
    );
}

#[test]
fn a_tiled_wallpaper_is_drawn_as_a_grid_of_copies() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 320, 240);
    let state = requested(tokens_light(), Selection::Image(path), WallpaperMode::Tile);
    let pixels = draw(&state, "tile");

    // Every tile is the fixture colour, and there is more than one of them.
    for (x, y) in [(10, 10), (330, 10), (10, 250), (1010, 710)] {
        assert_eq!(
            support::pixel(&pixels, SIZE, x, y),
            Pixel {
                red: IMAGE[0],
                green: IMAGE[1],
                blue: IMAGE[2],
                alpha: 255,
            },
            "the copy at ({x}, {y})"
        );
    }
    assert_eq!(
        support::most_common(&pixels),
        Pixel {
            red: IMAGE[0],
            green: IMAGE[1],
            blue: IMAGE[2],
            alpha: 255,
        },
        "the grid of copies covers the output"
    );
}

#[test]
fn a_click_on_the_wallpaper_produces_no_message_at_all() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 640, 480);
    let state = requested(tokens_light(), Selection::Image(path), WallpaperMode::Fill);
    let centre = Point::new(SIZE.width / 2.0, SIZE.height / 2.0);

    let messages = support::drive(
        element(&state),
        &[
            Event::Mouse(mouse::Event::CursorMoved { position: centre }),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ],
        centre,
        SIZE,
    );

    assert!(
        messages.is_empty(),
        "a wallpaper takes no input: got {messages:?}"
    );
}

#[test]
fn a_click_on_the_retry_control_produces_the_reload_message() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let light = tokens_light();
    let state = requested(
        Arc::clone(&light),
        Selection::Image(dir.path().join("gone.png")),
        WallpaperMode::Fill,
    );
    let pixels = draw(&state, "retry-enabled");
    // The accent fill is the primary control's, and it is found rather than
    // assumed, so this does not depend on how tall the text above it rendered.
    let target = support::bounds(&pixels, SIZE, colour(light.colors.accent))
        .expect("the enabled control is drawn in the accent token");

    let messages = click(&state, target);

    assert!(
        matches!(messages.as_slice(), [Message::Reload]),
        "the one action offers the one message: got {messages:?}"
    );
}

#[test]
fn a_click_on_the_disabled_control_produces_nothing() {
    let light = tokens_light();
    let state = boot(
        Arc::clone(&light),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        Some(Unusable::Unreadable {
            detail: "permission denied".to_owned(),
        }),
    );
    let pixels = draw(&state, "retry-disabled");
    let target = support::bounds(&pixels, SIZE, colour(light.colors.surface_variant))
        .expect("the disabled control is drawn in the surface-variant token");

    let messages = click(&state, target);

    assert!(
        messages.is_empty(),
        "a disabled control is not reachable by pointer: got {messages:?}"
    );
}

#[test]
fn the_loading_scenario_draws_something_operable_rather_than_a_blank_surface() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let state = boot(
        tokens_dark(),
        Selection::Image(dir.path().join("wall.png")),
        WallpaperMode::Fill,
        None,
    );
    let pixels = draw(&state, "loading");

    assert_eq!(
        support::centre(&pixels, SIZE),
        colour(tokens_dark().colors.background)
    );
    assert!(
        support::contains(&pixels, colour(tokens_dark().colors.surface)),
        "the load in progress is reported on screen"
    );
}

#[test]
fn the_cards_are_inset_by_the_window_padding_and_stay_inside_the_window() {
    // UI spec §3 and §7: one window inset, and nothing that runs off the edge at
    // 1280 × 720. Bounds rather than glyphs, so this holds without fonts.
    let tokens = tokens_dark();
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 640, 480);
    let state = requested(
        Arc::clone(&tokens),
        Selection::Image(path),
        WallpaperMode::Fill,
    );
    let pixels = draw(&state, "layout");
    let regions = cards(&pixels, &tokens);

    assert!(regions.len() >= 2, "a status card and a footer strip");
    for region in &regions {
        assert!(
            region.min_x >= inset(&tokens) && region.min_x <= inset(&tokens) + outline(&tokens),
            "a card starts at the window inset: {region:?}"
        );
        assert!(
            region.max_x <= SIZE.width as u32 - inset(&tokens),
            "a card ends inside the window: {region:?}"
        );
        assert!(
            region.max_y <= SIZE.height as u32 - inset(&tokens),
            "a card ends inside the window: {region:?}"
        );
    }
    assert_eq!(
        support::pixel(&pixels, SIZE, 4, 4),
        Pixel {
            red: IMAGE[0],
            green: IMAGE[1],
            blue: IMAGE[2],
            alpha: 255,
        },
        "the window inset is the wallpaper, not a card painted over it"
    );
}

#[test]
fn every_scenario_is_projected_from_a_state_the_application_can_reach() {
    // The scenarios above are not synthetic view models: each one is a `State`
    // built through the same constructors and messages the binary uses, and this
    // asserts that the projection has something to show for each of them.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 640, 480);
    let scenarios = [
        (
            "loaded",
            requested(tokens_light(), Selection::Image(path), WallpaperMode::Fill),
        ),
        (
            "empty",
            boot(
                tokens_light(),
                Selection::ThemeBackground,
                WallpaperMode::Fill,
                None,
            ),
        ),
    ];

    for (name, state) in scenarios {
        let model = view_model::project(&state);

        assert!(!model.title.is_empty(), "{name}");
        assert!(model.footer.is_some(), "{name}");
        assert!(!model.card.heading.is_empty(), "{name}");
        assert!(!model.card.scaling.is_empty(), "{name}");
        assert!(model.card.phase.is_some(), "{name}");
        if name == "loaded" {
            assert!(model.wallpaper.is_some(), "{name}");
        }
    }
}

#[test]
fn the_status_card_and_the_footer_strip_share_the_left_edge() {
    // UI spec §3, §4: the same class of element sits in the same place with the
    // same inset — the card column and the strip line up on one edge.
    let tokens = tokens_dark();
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 640, 480);
    let state = requested(
        Arc::clone(&tokens),
        Selection::Image(path),
        WallpaperMode::Fill,
    );
    let pixels = draw(&state, "edges");
    let regions = cards(&pixels, &tokens);

    let card = regions.first().expect("a status card");
    let strip = regions.last().expect("a footer strip");

    assert_eq!(card.min_x, strip.min_x, "both start on the same edge");
    assert!(
        strip.width() > card.width(),
        "the strip takes the width it is given, the card is bounded"
    );
    let gap = SIZE.width as u32 - (strip.max_x + 1);
    assert!(
        gap >= inset(&tokens) && gap <= inset(&tokens) + outline(&tokens),
        "the strip reaches the window inset: {strip:?} leaves {gap}"
    );
    assert!(
        card.max_y < strip.min_y,
        "the card and the strip do not overlap: {card:?} {strip:?}"
    );
}

#[test]
fn a_two_hundred_percent_font_scale_grows_the_cards_instead_of_clipping_them() {
    // Acceptance Q02 as a pixel assertion: at 200% the card is taller than at
    // 100%, still starts at the window inset, still stays inside the window, and
    // does not overlap the strip above which it would otherwise run.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = fixture(dir.path(), 640, 480);
    let dark = tokens_dark();
    let normal = requested(
        Arc::clone(&dark),
        Selection::Image(path.clone()),
        WallpaperMode::Fill,
    );
    let large = requested(
        tokens_at_scale(Appearance::Dark, 2.0),
        Selection::Image(path),
        WallpaperMode::Fill,
    );

    let normal_pixels = draw(&normal, "scale-100");
    let large_pixels = draw(&large, "scale-200");

    let card_100 = top_card(&normal_pixels, &dark);
    let regions_200 = cards(&large_pixels, &dark);
    let card_200 = *regions_200.first().expect("a status card at 200%");
    let strip_200 = *regions_200.last().expect("a footer strip at 200%");

    assert!(
        card_200.height() > card_100.height(),
        "the card grows with the font: {} then {}",
        card_100.height(),
        card_200.height()
    );
    assert_eq!(
        card_200.min_x, card_100.min_x,
        "and stays where it was horizontally"
    );
    assert!(
        regions_200.len() >= 2,
        "the card and the strip are still two bands, so they do not overlap"
    );
    assert!(
        card_200.max_y < strip_200.min_y,
        "no overlap at 200%: {card_200:?} {strip_200:?}"
    );
    assert!(
        strip_200.max_y <= SIZE.height as u32 - inset(&dark),
        "and nothing runs off the bottom: {strip_200:?}"
    );
}
