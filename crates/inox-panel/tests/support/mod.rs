//! The harness the panel's render tests draw through.
//!
//! It builds the panel's real widget tree, lays it out at a fixed size and draws
//! it with the same `tiny-skia` renderer the binary uses — with no compositor and
//! no GPU, which is what makes acceptance Q02 testable in CI at all.
//!
//! Interaction is driven the same way iced itself drives it: the same
//! `UserInterface` receives real `Event`s at a real cursor position, and the
//! messages it produces are the ones the application's `update` would receive.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use iced_core::clipboard;
use iced_core::mouse;
use iced_core::renderer::{Headless, Style};
use iced_core::time::Instant;
use iced_core::{Color, Event, Font, Pixels, Point, Size, window};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;

use inox_config::{PanelConfig, Rgba, Theme as Tokens};
use inox_panel::app::{self, Message, State};
use inox_panel::{i18n, theme};

/// The width the acceptance snapshots are taken at (Q02).
pub const WIDTH: f32 = 1280.0;

/// The renderer's default text size, in logical pixels.
const TEXT_SIZE: f32 = 16.0;

/// The panel, its state, and the size it is drawn at.
pub struct Panel {
    state: State,
    size: Size<f32>,
}

impl Panel {
    /// A panel with the default configuration and the dark tokens.
    pub fn new() -> Self {
        Self::with(PanelConfig::default(), Tokens::builtin_dark())
    }

    /// A panel with `config` and `tokens`.
    pub fn with(config: PanelConfig, tokens: Tokens) -> Self {
        let size = Size::new(WIDTH, f32::from(config.height));
        let locale = inox_common::i18n::Locale::default_locale();
        let localizer = i18n::load(&locale).expect("the panel's catalogs parse");

        Self {
            state: State::new(config, tokens, std::rc::Rc::new(localizer)),
            size,
        }
    }

    /// The panel's state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The panel's state, for a test that has to take what the panel queued.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// The size the panel is drawn at.
    pub fn size(&self) -> Size<f32> {
        self.size
    }

    /// Feeds a message to the update loop, the way the runtime would.
    ///
    /// The `Task` iced would run is dropped: this harness asserts on state and
    /// pixels, and the only task the panel ever returns is `Task::none()`.
    pub fn update(&mut self, message: Message) {
        let _ = app::update(&mut self.state, message);
    }

    /// Feeds several messages.
    pub fn update_all(&mut self, messages: impl IntoIterator<Item = Message>) {
        for message in messages {
            self.update(message);
        }
    }

    /// The iced theme the panel draws under.
    pub fn theme(&self) -> iced::Theme {
        theme::from_tokens(self.state.tokens())
    }

    /// Draws the panel and returns its RGBA8 pixels.
    ///
    /// The pointer is at the panel's own top-left corner, which is padding: the
    /// ordinary state of a strip nobody is pointing at.
    pub fn render(&self) -> Vec<u8> {
        self.render_with_cursor(Point::ORIGIN)
    }

    /// Draws the panel with the pointer at `cursor`.
    ///
    /// Where the pointer sits is an input to *drawing* as well as to events:
    /// iced remembers a widget's interaction status when it is asked to redraw
    /// and styles it from that memory, so the redraw request is sent first, with
    /// the cursor the frame is drawn at. A harness that only drew would render
    /// every button in its disabled style, and the snapshot would show a strip
    /// with nothing clickable on it (Q02).
    pub fn render_with_cursor(&self, cursor: Point) -> Vec<u8> {
        let mut renderer = renderer();
        let mut ui = self.interface(&mut renderer);
        let pointer = mouse::Cursor::Available(cursor);

        ui.update(
            &[Event::Window(
                window::Event::RedrawRequested(Instant::now()),
            )],
            pointer,
            &mut renderer,
            &mut clipboard::Null,
            &mut Vec::new(),
        );

        ui.draw(&mut renderer, &self.theme(), &Style::default(), pointer);

        Headless::screenshot(
            &mut renderer,
            Size::new(self.size.width as u32, self.size.height as u32),
            1.0,
            Color::WHITE,
        )
    }

    /// Clicks at `point` and returns the messages the click produced.
    pub fn click(&self, point: Point) -> Vec<Message> {
        let mut renderer = renderer();
        let mut ui = self.interface(&mut renderer);
        let mut messages = Vec::new();

        ui.update(
            &[
                Event::Mouse(mouse::Event::CursorMoved { position: point }),
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ],
            mouse::Cursor::Available(point),
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );

        messages
    }

    /// Clicks at `point` and applies whatever the click produced.
    pub fn click_through(&mut self, point: Point) -> Vec<Message> {
        let messages = self.click(point);

        for message in &messages {
            self.update(message.clone());
        }

        messages
    }

    fn interface<'a>(
        &'a self,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'a, Message, iced::Theme, iced_tiny_skia::Renderer> {
        UserInterface::build(
            app::view(&self.state),
            self.size,
            Cache::default(),
            renderer,
        )
    }
}

impl Default for Panel {
    fn default() -> Self {
        Self::new()
    }
}

/// The renderer the binary uses, with no GPU and no compositor.
pub fn renderer() -> iced_tiny_skia::Renderer {
    iced_tiny_skia::Renderer::new(Font::default(), Pixels(TEXT_SIZE))
}

/// The RGBA colour of one pixel of a rendered buffer.
pub fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let index = ((y * width + x) * 4) as usize;

    [
        pixels[index],
        pixels[index + 1],
        pixels[index + 2],
        pixels[index + 3],
    ]
}

/// The RGBA bytes a token colour renders as, over an opaque background.
pub fn token_pixel(colour: Rgba) -> [u8; 4] {
    [colour.r, colour.g, colour.b, colour.a]
}

/// How many different colours a rendered buffer contains.
pub fn distinct_colours(pixels: &[u8]) -> usize {
    let mut seen: Vec<[u8; 4]> = Vec::new();

    for chunk in pixels.as_chunks::<4>().0 {
        if !seen.contains(chunk) {
            seen.push(*chunk);
        }
    }

    seen.len()
}

/// Whether anything was drawn over `background` in the columns `from..to`.
///
/// This is how a test asks "is the clock still there?" without depending on
/// which glyphs the platform's fonts happen to provide.
pub fn has_ink_in_columns(
    pixels: &[u8],
    width: u32,
    height: u32,
    from: u32,
    to: u32,
    background: [u8; 4],
) -> bool {
    (0..height).any(|y| (from..to.min(width)).any(|x| pixel(pixels, width, x, y) != background))
}

/// Writes a snapshot when `INOX_SCREENSHOT_DIR` is set (acceptance Q02).
///
/// CI collects the directory as an artifact; without the variable the test only
/// asserts on the pixels, and no file is written.
pub fn snapshot(scenario: &str, pixels: &[u8], size: Size<f32>) {
    let Ok(directory) = std::env::var("INOX_SCREENSHOT_DIR") else {
        return;
    };

    std::fs::create_dir_all(&directory).expect("the snapshot directory is created");

    let path: PathBuf = Path::new(&directory).join(format!("inox-panel-{scenario}.png"));
    let image = image::RgbaImage::from_raw(size.width as u32, size.height as u32, pixels.to_vec())
        .expect("the rendered buffer matches the panel's size");
    image.save(&path).expect("the snapshot is written");
}
