//! What the component actually draws, as pixels (acceptance Q02, D05).
//!
//! Every scenario runs the real pipeline — a real PNG written to a temporary
//! directory, the real decoder, the real state transition, the real widget tree
//! and the renderer the binary uses — and then asserts on the bytes that came
//! out. Nothing here reads the source of a view function or trusts a comment.
//!
//! The assertions are chosen so that they do not depend on font metrics: what is
//! asserted about a control is its background, not its glyphs, so a machine with
//! no fonts installed still tests the same decisions.

mod support;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use iced_core::mouse;
use iced_core::{Event, Point};

use inox_common::i18n::{Locale, Localizer};
use inox_config::{Theme, WallpaperMode};
use inox_wallpaper::app::{Message, PREVIEW_SIZE, Preview, State, update};
use inox_wallpaper::model::{OutputName, Selection, Size as OutputSize, Unusable};
use inox_wallpaper::view_model;
use inox_wallpaper::{apply, i18n, theme, view};

use support::{Pixel, SIZE};

/// The colour of every fixture image.
///
/// Deliberately nothing any built-in theme uses, so that "this pixel is the
/// wallpaper" and "this pixel is a token" can never be confused.
const IMAGE: [u8; 4] = [1, 2, 3, 255];

/// The output the preview stands in for.
const OUTPUT: &str = "HDMI-A-1";

fn tokens(appearance: inox_config::Appearance) -> Arc<Theme> {
    Arc::new(match appearance {
        inox_config::Appearance::Light => Theme::builtin_light(),
        _ => Theme::builtin_dark(),
    })
}

fn tokens_light() -> Arc<Theme> {
    tokens(inox_config::Appearance::Light)
}

fn tokens_dark() -> Arc<Theme> {
    tokens(inox_config::Appearance::Dark)
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
    // The footer is a caption drawn straight onto the wallpaper with no
    // background of its own, so the font-independent way to say "there is
    // something to read it against" is the drawing underneath it: the fill
    // covers the whole output, overlays and all. Asserting the glyphs themselves
    // would depend on font rasterization, which crate AGENTS.md forbids — a
    // machine with no fonts installed has to pass this test too. That the footer
    // line is produced at all is asserted in `view_model`, where it is a string.
    assert_eq!(
        support::most_common(&pixels),
        Pixel {
            red: IMAGE[0],
            green: IMAGE[1],
            blue: IMAGE[2],
            alpha: 255,
        },
        "a fill covers the output with the image, with the footer drawn over it"
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
fn an_empty_scenario_draws_the_theme_background_and_no_panel() {
    let state = boot(
        tokens_light(),
        Selection::ThemeBackground,
        WallpaperMode::Fill,
        None,
    );
    let pixels = draw(&state, "empty");

    for (x, y) in [(4, 4), (640, 360), (1270, 710)] {
        assert_eq!(
            support::pixel(&pixels, SIZE, x, y),
            colour(tokens_light().colors.background),
            "nothing configured means the theme background, everywhere"
        );
    }
    assert!(
        !support::contains(&pixels, colour(tokens_light().colors.surface)),
        "there is nothing to report, so there is no panel"
    );
    assert!(
        !support::contains(&pixels, colour(tokens_light().colors.accent)),
        "and nothing to press"
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
        "the diagnosis is drawn in a panel"
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
        if name == "loaded" {
            assert!(model.wallpaper.is_some(), "{name}");
        }
    }
}
