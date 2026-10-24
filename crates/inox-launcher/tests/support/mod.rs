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
//! The focus operation below is the one `main` runs before the first frame, so a
//! screenshot shows the launcher as it opens — caret in the search field, focus
//! ring and all.

use std::path::Path;

use iced_core::renderer::{Headless, Style};
use iced_core::widget::Id;
use iced_core::widget::operation::focusable;
use iced_core::{Color, Font, Pixels, Point, Size, mouse};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;
use inox_config::Theme as Tokens;

use inox_launcher::app::{Message, SEARCH_INPUT_ID};
use inox_launcher::theme::{self, Metrics, TextRole};

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

/// Puts the caret in the search field, through the operation the binary runs at
/// startup.
pub fn focus_search(
    ui: &mut UserInterface<'_, Message, iced::Theme, iced_tiny_skia::Renderer>,
    renderer: &mut iced_tiny_skia::Renderer,
) {
    let mut operation = focusable::focus(Id::new(SEARCH_INPUT_ID));
    ui.operate(renderer, &mut operation);
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

/// Renders a view as the launcher opens: focus in the search field.
pub fn render(view: iced::Element<'_, Message>, theme: &iced::Theme, name: &str) -> Vec<u8> {
    render_with_focus(view, theme, name, true)
}

/// Renders a view with nothing focused.
///
/// The negative of [`render`]: it is how a test states that a colour on the
/// screen comes from the focus ring rather than from something else.
pub fn render_unfocused(
    view: iced::Element<'_, Message>,
    theme: &iced::Theme,
    name: &str,
) -> Vec<u8> {
    render_with_focus(view, theme, name, false)
}

/// Remembers the status a widget will be drawn with.
///
/// `TextInput` decides between `Active`, `Hovered` and `Focused` while handling
/// events and keeps the result until the next redraw request; a real window gets
/// one because focusing the field asks for it. The headless harness has no event
/// loop, so it delivers that redraw itself — without it every screenshot would
/// draw the field as `Disabled`.
fn request_redraw(
    ui: &mut UserInterface<'_, Message, iced::Theme, iced_tiny_skia::Renderer>,
    renderer: &mut iced_tiny_skia::Renderer,
) {
    let mut messages = Vec::new();
    let mut clipboard = iced_core::clipboard::Null;
    let event = iced_core::Event::Window(iced_core::window::Event::RedrawRequested(
        std::time::Instant::now(),
    ));

    let _ = ui.update(
        &[event],
        mouse::Cursor::Available(Point::ORIGIN),
        renderer,
        &mut clipboard,
        &mut messages,
    );
}

/// Renders a view, optionally with the caret in the search field, and writes the
/// screenshot when the environment asks for it.
fn render_with_focus(
    view: iced::Element<'_, Message>,
    theme: &iced::Theme,
    name: &str,
    focus: bool,
) -> Vec<u8> {
    let mut ui = build(view, SIZE);
    let mut renderer = renderer();
    if focus {
        focus_search(&mut ui, &mut renderer);
    }
    request_redraw(&mut ui, &mut renderer);
    let pixels = shot(&mut ui, &mut renderer, theme, SIZE);
    assert_eq!(
        pixels.len(),
        (SIZE.width as usize) * (SIZE.height as usize) * 4,
        "the screenshot is RGBA8 at the requested size",
    );
    write_png(name, &pixels);
    pixels
}

/// Whether a pixel carries `colour`, within a renderer's tolerance.
///
/// Colours are compared with a small tolerance: the renderer blends antialiased
/// edges, so an exact match is only guaranteed where a solid area is drawn.
fn matches(pixel: &[u8], wanted: [i32; 3]) -> bool {
    (0..3).all(|channel| (i32::from(pixel[channel]) - wanted[channel]).abs() <= 2)
}

/// The channels of a colour, as the renderer writes them.
fn channels(colour: Color) -> [i32; 3] {
    [
        (colour.r * 255.0).round() as i32,
        (colour.g * 255.0).round() as i32,
        (colour.b * 255.0).round() as i32,
    ]
}

/// Whether any pixel has the given colour.
pub fn contains(pixels: &[u8], colour: Color) -> bool {
    let wanted = channels(colour);
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| matches(pixel, wanted))
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

/// The longest horizontal run of `colour` on scanline `y`.
pub fn longest_run(pixels: &[u8], y: usize, colour: Color) -> usize {
    let wanted = channels(colour);
    let mut longest = 0;
    let mut current = 0;

    for x in 0..SIZE.width as usize {
        if matches(&pixel(pixels, x, y), wanted) {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }

    longest
}

/// The bounding box of the pixels of `colour`, as `(left, top, right, bottom)`.
///
/// `None` when the colour is not on the screen at all.
pub fn bounds(pixels: &[u8], colour: Color) -> Option<(usize, usize, usize, usize)> {
    let wanted = channels(colour);
    let mut bounds: Option<(usize, usize, usize, usize)> = None;

    for y in 0..SIZE.height as usize {
        for x in 0..SIZE.width as usize {
            if matches(&pixel(pixels, x, y), wanted) {
                bounds = Some(match bounds {
                    Some((left, top, right, bottom)) => {
                        (left.min(x), top.min(y), right.max(x), bottom.max(y))
                    }
                    None => (x, y, x, y),
                });
            }
        }
    }

    bounds
}

/// Whether the image holds a solid rectangle of `colour` at least `width` by
/// `height` pixels.
///
/// A run on one scanline is not a fill: the focus ring is a long thin outline
/// and an icon tile is a wide-as-it-is-tall square. Only a run that stays wide
/// over a whole band of scanlines counts, which is what makes a *selected row*
/// distinguishable from both.
pub fn contains_block(pixels: &[u8], colour: Color, width: usize, height: usize) -> bool {
    if width == 0 || height == 0 {
        return false;
    }

    let runs: Vec<usize> = (0..SIZE.height as usize)
        .map(|y| longest_run(pixels, y, colour))
        .collect();

    (0..runs.len()).any(|top| {
        top + height <= runs.len() && runs[top..top + height].iter().all(|run| *run >= width)
    })
}

/// How many scanlines carry a run of `colour` that is `length` pixels wide, give
/// or take a pixel of antialiasing.
///
/// A square icon tile of `icon_size` produces `icon_size` such scanlines; an icon
/// column that has been stretched across the row produces none.
pub fn scanlines_with_run(pixels: &[u8], colour: Color, length: usize) -> usize {
    let tolerance = 1;
    (0..SIZE.height as usize)
        .filter(|y| {
            let run = longest_run(pixels, *y, colour);
            run + tolerance >= length && run <= length + tolerance
        })
        .count()
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
///
/// The field is the first group of the launcher: the window's own padding above
/// it, then its control padding, then a title-sized line. The position is
/// derived from the same tokens and the same [`Metrics`] the view uses, so a
/// change to the vertical rhythm moves the click along with the widget.
pub fn over_search_field(tokens: &Tokens) -> mouse::Cursor {
    let metrics = Metrics::from_tokens(tokens);
    let line = theme::line_height(tokens, TextRole::Title);
    let y = metrics.space_xl + metrics.space_sm + line / 2.0;

    mouse::Cursor::Available(Point::new(SIZE.width / 2.0, y))
}
