//! Offscreen evidence of the real preview tree; this does not assert a secure login surface.
use iced_core::{
    Color, Event, Font, Pixels, Point, Size, clipboard, mouse,
    renderer::{Headless, Style},
    window,
};
use iced_runtime::{UserInterface, user_interface::Cache};
use inox_common::i18n::Locale;
use inox_config::Theme as Tokens;
use inox_greet::{app, i18n};
use std::{rc::Rc, time::Instant};

#[test]
fn preview_renders_under_both_themes_locales_and_double_font_scale() {
    for tag in ["en-US", "zh-CN"] {
        for mut tokens in [Tokens::builtin_dark(), Tokens::builtin_light()] {
            for scale in [1.0, 2.0] {
                tokens.typography.scale = scale;
                let state = app::State::new(
                    Rc::new(i18n::load(Locale::parse(tag).expect("locale")).expect("catalog")),
                    tokens.clone(),
                );
                let mut renderer = iced_tiny_skia::Renderer::new(Font::DEFAULT, Pixels(16.0));
                let size = Size::new(800.0, 600.0);
                let mut ui =
                    UserInterface::build(app::view(&state), size, Cache::default(), &mut renderer);
                let mut messages = Vec::new();
                ui.update(
                    &[Event::Window(
                        window::Event::RedrawRequested(Instant::now()),
                    )],
                    mouse::Cursor::Available(Point::ORIGIN),
                    &mut renderer,
                    &mut clipboard::Null,
                    &mut messages,
                );
                ui.draw(
                    &mut renderer,
                    &state.theme(),
                    &Style::default(),
                    mouse::Cursor::Unavailable,
                );
                let pixels =
                    Headless::screenshot(&mut renderer, Size::new(800, 600), 1.0, Color::WHITE);
                let background = tokens.colors.background;
                assert_eq!(
                    &pixels[pixels.len() - 4..],
                    &[background.r, background.g, background.b, background.a]
                );
                assert!(pixels.as_chunks::<4>().0.iter().any(
                    |pixel| *pixel != [background.r, background.g, background.b, background.a]
                ));
                if let Some(directory) = std::env::var_os("INOX_SCREENSHOT_DIR") {
                    let directory = std::path::PathBuf::from(directory).join("greet");
                    std::fs::create_dir_all(&directory).expect("snapshot directory");
                    image::save_buffer(
                        directory.join(format!("{}-{tag}-{scale}.png", tokens.name)),
                        &pixels,
                        800,
                        600,
                        image::ColorType::Rgba8,
                    )
                    .expect("snapshot");
                }
            }
        }
    }
}
