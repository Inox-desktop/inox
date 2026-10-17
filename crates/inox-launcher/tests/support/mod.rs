//! The headless render harness.
//!
//! It builds the real widget tree, lays it out at a fixed size and draws it
//! through the same tiny-skia renderer the binary uses — with no compositor, no
//! Wayland connection and no GPU. That is what lets acceptance Q02's screenshot
//! evidence exist on a CI host that cannot run a session.
//!
//! The same `UserInterface` is what the interaction test drives: an event goes
//! in through `UserInterface::update` (the path a real window would take) and
//! the messages it produces are fed back through the component's own `update`.

use std::path::Path;

use iced_core::renderer::{Headless, Style};
use iced_core::{Color, Font, Pixels, Point, Size, mouse};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;

use inox_launcher::app::Message;

/// The size every scenario renders at (acceptance Q02).
pub const SIZE: Size<f32> = Size::new(1280.0, 720.0);

/// The renderer the binary and the tests share.
pub fn renderer() -> iced_tiny_skia::Renderer {
    iced_tiny_skia::Renderer::new(Font::default(), Pixels(16.0))
}

/// Lays a view out at `size`.
pub fn build(
    view: iced::Element<'_, Message>,
    size: Size<f32>,
) -> UserInterface<'_, Message, iced::Theme, iced_tiny_skia::Renderer> {
    let mut renderer = renderer();
    UserInterface::build(view, size, Cache::default(), &mut renderer)
}

/// Draws a laid-out interface and reads the pixels back.
pub fn shot(
    ui: &mut UserInterface<'_, Message, iced::Theme, iced_tiny_skia::Renderer>,
    renderer: &mut iced_tiny_skia::Renderer,
    theme: &iced::Theme,
    size: Size<f32>,
) -> Vec<u8> {
    ui.draw(
        renderer,
        theme,
        &Style::default(),
        mouse::Cursor::Available(Point::ORIGIN),
    );
    Headless::screenshot(
        renderer,
        Size::new(size.width as u32, size.height as u32),
        1.0,
        Color::WHITE,
    )
}

/// Renders a view in one step, and writes the screenshot when the environment
/// asks for it.
pub fn render(view: iced::Element<'_, Message>, theme: &iced::Theme, name: &str) -> Vec<u8> {
    let mut ui = build(view, SIZE);
    let mut renderer = renderer();
    let pixels = shot(&mut ui, &mut renderer, theme, SIZE);
    assert_eq!(
        pixels.len(),
        (SIZE.width as usize) * (SIZE.height as usize) * 4,
        "the screenshot is RGBA8 at the requested size",
    );
    write_png(name, &pixels);
    pixels
}

/// Whether any pixel has the given colour.
///
/// Colours are compared with a small tolerance: the renderer blends antialiased
/// edges, so an exact match is only guaranteed where a solid area is drawn.
pub fn contains(pixels: &[u8], colour: Color) -> bool {
    let wanted = [
        (colour.r * 255.0).round() as i32,
        (colour.g * 255.0).round() as i32,
        (colour.b * 255.0).round() as i32,
    ];
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| (0..3).all(|channel| (i32::from(pixel[channel]) - wanted[channel]).abs() <= 2))
}

/// The RGBA value of one pixel, at `(x, y)`.
pub fn pixel(pixels: &[u8], x: usize, y: usize) -> [u8; 4] {
    let index = (y * (SIZE.width as usize) + x) * 4;
    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

/// How many distinct colours the image has.
pub fn distinct_colours(pixels: &[u8]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for pixel in pixels.as_chunks::<4>().0 {
        seen.insert(*pixel);
    }
    seen.len()
}

/// Writes a PNG when `INOX_SCREENSHOT_DIR` is set, so CI can collect it as an
/// artifact (acceptance Q02); otherwise does nothing.
pub fn write_png(name: &str, pixels: &[u8]) {
    let Ok(directory) = std::env::var("INOX_SCREENSHOT_DIR") else {
        return;
    };
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory).expect("the screenshot directory is writable");
    let image = image::RgbaImage::from_raw(SIZE.width as u32, SIZE.height as u32, pixels.to_vec())
        .expect("the buffer is RGBA8 at the right size");
    let path = directory.join(format!("inox-launcher-{name}.png"));
    image.save(&path).expect("the screenshot is written");
}

/// A pointer position inside the search field.
pub fn over_search_field(gutter: f32) -> mouse::Cursor {
    mouse::Cursor::Available(Point::new(SIZE.width / 2.0, gutter + 8.0))
}
