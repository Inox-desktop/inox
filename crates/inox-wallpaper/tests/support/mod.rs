//! The headless render harness.
//!
//! Draws the real widget tree — the same one the binary builds — at a fixed size
//! through the same `tiny-skia` renderer the binary uses, with no compositor and
//! no GPU, and hands back RGBA bytes. It is what makes acceptance Q02 (light,
//! dark, empty and error scenarios at 1280 × 720) an assertion about pixels
//! rather than an assertion about code that is supposed to produce pixels.
//!
//! It can also drive the tree with input events and report the messages they
//! produced, which is how "a wallpaper does not react to a click" is tested
//! without a compositor.

use iced_core::clipboard::Null;
use iced_core::renderer::{Headless, Style};
use iced_core::window;
use iced_core::{Color, Event, Font, Pixels, Point, Size, mouse};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;

use inox_wallpaper::app::Message;

/// The size every scenario is drawn at: acceptance Q02's 1280 × 720.
pub const SIZE: Size<f32> = Size::new(1280.0, 720.0);

/// The RGBA colour of one pixel of a rendered buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pixel {
    /// Red channel.
    pub red: u8,
    /// Green channel.
    pub green: u8,
    /// Blue channel.
    pub blue: u8,
    /// Alpha channel.
    pub alpha: u8,
}

impl From<Color> for Pixel {
    fn from(color: Color) -> Self {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;

        Self {
            red: channel(color.r),
            green: channel(color.g),
            blue: channel(color.b),
            alpha: channel(color.a),
        }
    }
}

/// Lays the view out at `size` and returns the rendered RGBA8 pixels.
#[must_use]
pub fn render(view: iced::Element<'_, Message>, size: Size<f32>, theme: &iced::Theme) -> Vec<u8> {
    let mut renderer = iced_tiny_skia::Renderer::new(Font::default(), Pixels(16.0));
    let mut ui = UserInterface::<Message, iced::Theme, iced_tiny_skia::Renderer>::build(
        view,
        size,
        Cache::default(),
        &mut renderer,
    );
    let mut settled = Vec::new();

    // Widgets decide how to draw themselves while they handle events, not while
    // they draw: a button that has never seen a redraw request still reports
    // itself disabled, and drawing straight after `build` would therefore
    // photograph a state the binary never shows. One redraw pass, as the runtime
    // would send, settles every widget's appearance.
    let _ = ui.update(
        &[Event::Window(window::Event::RedrawRequested(
            std::time::Instant::now(),
        ))],
        mouse::Cursor::Available(Point::ORIGIN),
        &mut renderer,
        &mut Null,
        &mut settled,
    );

    ui.draw(
        &mut renderer,
        theme,
        &Style::default(),
        mouse::Cursor::Available(Point::ORIGIN),
    );

    Headless::screenshot(
        &mut renderer,
        Size::new(size.width as u32, size.height as u32),
        1.0,
        Color::WHITE,
    )
}

/// Feeds `events` to the tree with the cursor at `cursor` and returns the
/// messages the widgets produced.
#[must_use]
pub fn drive(
    view: iced::Element<'_, Message>,
    events: &[Event],
    cursor: Point,
    size: Size<f32>,
) -> Vec<Message> {
    let mut renderer = iced_tiny_skia::Renderer::new(Font::default(), Pixels(16.0));
    let mut ui = UserInterface::<Message, iced::Theme, iced_tiny_skia::Renderer>::build(
        view,
        size,
        Cache::default(),
        &mut renderer,
    );
    let mut messages = Vec::new();

    let _ = ui.update(
        events,
        mouse::Cursor::Available(cursor),
        &mut renderer,
        &mut Null,
        &mut messages,
    );

    messages
}

/// The pixel at (`x`, `y`), counted from the top-left corner.
///
/// # Panics
///
/// When the point is outside the buffer, which in a test means the scenario was
/// drawn at a different size than the assertion expects.
#[must_use]
pub fn pixel(pixels: &[u8], size: Size<f32>, x: u32, y: u32) -> Pixel {
    let width = size.width as usize;
    let index = (y as usize * width + x as usize) * 4;

    Pixel {
        red: pixels[index],
        green: pixels[index + 1],
        blue: pixels[index + 2],
        alpha: pixels[index + 3],
    }
}

/// The pixel in the middle of the buffer.
#[must_use]
pub fn centre(pixels: &[u8], size: Size<f32>) -> Pixel {
    pixel(pixels, size, size.width as u32 / 2, size.height as u32 / 2)
}

/// The colour the buffer is covered in most.
///
/// The useful question about a wallpaper is not how many colours were drawn but
/// which one covers the output: with one it does, with neither it does not.
///
/// # Panics
///
/// When the buffer holds no whole pixel, which in a test means it was drawn at
/// the wrong size.
#[must_use]
pub fn most_common(pixels: &[u8]) -> Pixel {
    let mut counts = std::collections::HashMap::new();

    for pixel in pixels.as_chunks::<4>().0 {
        *counts
            .entry((pixel[0], pixel[1], pixel[2], pixel[3]))
            .or_insert(0usize) += 1;
    }

    let (colour, _) = counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .expect("a buffer with pixels in it");

    Pixel {
        red: colour.0,
        green: colour.1,
        blue: colour.2,
        alpha: colour.3,
    }
}

/// Whether any pixel of the buffer is exactly `colour`.
#[must_use]
pub fn contains(pixels: &[u8], colour: Pixel) -> bool {
    pixels.as_chunks::<4>().0.iter().any(|pixel| {
        pixel[0] == colour.red
            && pixel[1] == colour.green
            && pixel[2] == colour.blue
            && pixel[3] == colour.alpha
    })
}

/// Writes a scenario's pixels as a PNG when `INOX_SCREENSHOT_DIR` is set.
///
/// Acceptance Q02 wants the screenshots kept as CI artifacts; the directory is
/// opt-in so that an ordinary `cargo test` writes nothing.
#[must_use]
pub fn dump(scenario: &str, pixels: Vec<u8>, size: Size<f32>) -> Vec<u8> {
    let Ok(directory) = std::env::var("INOX_SCREENSHOT_DIR") else {
        return pixels;
    };

    let width = size.width as u32;
    let height = size.height as u32;
    let path = std::path::Path::new(&directory).join(format!("inox-wallpaper-{scenario}.png"));

    match image::RgbaImage::from_raw(width, height, pixels.clone()) {
        Some(image) => {
            if let Err(error) = image.save(&path) {
                tracing::warn!(path = %path.display(), %error, "the screenshot was not written");
            }
        }
        None => tracing::warn!(scenario, "the pixel buffer was not the size it claimed"),
    }

    pixels
}
