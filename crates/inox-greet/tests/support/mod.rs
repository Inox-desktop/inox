//! The harness the login screen's render tests draw through.
//!
//! It builds the crate's real widget tree, lays it out at a fixed size and draws
//! it with the same `tiny-skia` renderer the binary uses — with no compositor and
//! no GPU. Interaction is driven the way iced itself drives it: the same
//! `UserInterface` receives real `Event`s at a real cursor position, and the
//! messages it produces are the ones the application's `update` would receive.
//!
//! It is the same harness `inox-lock/tests/support/mod.rs` uses, and for the same
//! reason: ADR 0018 accepts a copy per component rather than a shared same-layer
//! crate, and acceptance Q02 asks for pixels either way.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use iced_core::clipboard;
use iced_core::mouse;
use iced_core::renderer::{Headless, Style};
use iced_core::{Color, Event, Font, Pixels, Point, Rectangle, Size, window};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;

use inox_common::i18n::Locale;
use inox_config::{Rgba, Theme as Tokens};
use inox_greet::app::{self, Message, State};
use inox_greet::theme;

/// The size acceptance Q02 snapshots are taken at.
///
/// 1280×720, the size `docs/quality/acceptance.md` §9 Q02 fixes and the size the
/// L3 components snapshot at. A login screen is shown before there is a session to
/// ask about the output's size, so the fixed size is the only size the evidence
/// has; it is deliberately not the window's.
pub const WIDTH: f32 = 1280.0;

/// The size acceptance Q02 snapshots are taken at.
pub const HEIGHT: f32 = 720.0;

/// The renderer's default text size, in logical pixels.
const TEXT_SIZE: f32 = 16.0;

/// The login screen, its state, and the size it is drawn at.
pub struct Login {
    state: State,
    size: Size<f32>,
}

impl Login {
    /// A login screen with no authentication transport, as the binary runs it.
    pub fn new() -> Self {
        Self::with(Tokens::builtin_dark(), false)
    }

    /// A login screen with `tokens`, and an authenticator only if `available`.
    pub fn with(tokens: Tokens, available: bool) -> Self {
        Self::in_locale(tokens, available, &Locale::default_locale())
    }

    /// A login screen in a particular language.
    pub fn in_locale(tokens: Tokens, available: bool, locale: &Locale) -> Self {
        let localizer = inox_greet::i18n::load(locale).expect("the login screen's catalogs parse");

        Self {
            state: State::with_boundary(Rc::new(localizer), tokens, available),
            size: Size::new(WIDTH, HEIGHT),
        }
    }

    /// The login screen's state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The login screen's state, for a test that has to take what it queued.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// The design tokens.
    pub fn tokens(&self) -> &Tokens {
        self.state.tokens()
    }

    /// The size the login screen is drawn at.
    pub const fn size(&self) -> Size<f32> {
        self.size
    }

    /// Feeds a message to the update loop, the way the runtime would.
    ///
    /// The `Task` iced would run is dropped: this harness asserts on state and
    /// pixels, and the only task the login screen returns is a focus operation,
    /// which the offscreen tree has no runtime to carry out.
    pub fn update(&mut self, message: Message) {
        let _ = app::update(&mut self.state, message);
    }

    /// Feeds several messages.
    pub fn update_all(&mut self, messages: impl IntoIterator<Item = Message>) {
        for message in messages {
            self.update(message);
        }
    }

    /// Types `text` into the identity field, one message at a time.
    pub fn type_text(&mut self, text: &str) {
        for prefix in 1..=text.len() {
            if text.is_char_boundary(prefix) {
                self.update(Message::Identity(text[..prefix].to_owned()));
            }
        }
    }

    /// The iced theme the login screen draws under.
    pub fn theme(&self) -> iced::Theme {
        theme::from_tokens(self.state.tokens())
    }

    /// Draws the login screen and returns its RGBA8 pixels.
    pub fn render(&self) -> Vec<u8> {
        let mut renderer = renderer();
        let mut ui = self.interface(&mut renderer);

        frame(&mut ui, &mut renderer);

        ui.draw(
            &mut renderer,
            &self.theme(),
            &Style::default(),
            mouse::Cursor::Available(Point::ORIGIN),
        );

        Headless::screenshot(
            &mut renderer,
            Size::new(self.size.width as u32, self.size.height as u32),
            1.0,
            Color::WHITE,
        )
    }

    /// The bounding box of everything drawn in `colour`, in logical pixels.
    ///
    /// `None` when nothing is drawn in it, which is itself an assertion a test can
    /// make: a screen where no control can be used has no focus ring anywhere.
    pub fn find(&self, colour: Rgba) -> Option<Rectangle> {
        bounds_of(&self.render(), self.size, colour)
    }

    /// Clicks at `point` and returns the messages the click produced.
    pub fn click(&self, point: Point) -> Vec<Message> {
        let mut renderer = renderer();
        let mut ui = self.interface(&mut renderer);
        let mut messages = Vec::new();

        frame(&mut ui, &mut renderer);

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

impl Default for Login {
    fn default() -> Self {
        Self::new()
    }
}

/// Delivers the redraw a real runtime delivers once per frame.
///
/// iced's widgets work out their interaction status while handling
/// `RedrawRequested` and store it for the next draw — a button that has never
/// seen one is drawn as disabled (`iced_widget::button`, `Widget::update`). A
/// harness that skipped this would render every control in its disabled state.
fn frame(
    ui: &mut UserInterface<'_, Message, iced::Theme, iced_tiny_skia::Renderer>,
    renderer: &mut iced_tiny_skia::Renderer,
) {
    let mut messages = Vec::new();

    ui.update(
        &[Event::Window(
            window::Event::RedrawRequested(Instant::now()),
        )],
        mouse::Cursor::Available(Point::ORIGIN),
        renderer,
        &mut clipboard::Null,
        &mut messages,
    );

    assert!(
        messages.is_empty(),
        "a redraw produced messages: {messages:?}"
    );
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

/// The RGBA bytes a token colour renders as.
pub fn token_pixel(colour: Rgba) -> [u8; 4] {
    [colour.r, colour.g, colour.b, colour.a]
}

/// The bounding box of every pixel of `pixels` that is exactly `colour`.
pub fn bounds_of(pixels: &[u8], size: Size<f32>, colour: Rgba) -> Option<Rectangle> {
    let want = token_pixel(colour);
    let width = size.width as u32;
    let height = size.height as u32;

    let (mut min_x, mut min_y) = (u32::MAX, u32::MAX);
    let (mut max_x, mut max_y) = (0u32, 0u32);

    for y in 0..height {
        for x in 0..width {
            if pixel(pixels, width, x, y) == want {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if min_x == u32::MAX {
        return None;
    }

    Some(Rectangle {
        x: min_x as f32,
        y: min_y as f32,
        width: (max_x - min_x + 1) as f32,
        height: (max_y - min_y + 1) as f32,
    })
}

/// How many pixels of `pixels` are exactly `colour`.
pub fn count_of(pixels: &[u8], colour: Rgba) -> usize {
    let want = token_pixel(colour);

    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|px| **px == want)
        .count()
}

/// Whether every pixel of `pixels` is opaque.
pub fn is_opaque(pixels: &[u8]) -> bool {
    pixels.as_chunks::<4>().0.iter().all(|px| px[3] == 0xff)
}

/// Writes a snapshot when `INOX_SCREENSHOT_DIR` is set (acceptance Q02).
///
/// CI collects the directory as an artifact; without the variable the test only
/// asserts on the pixels, and no file is written.
pub fn snapshot(scenario: &str, pixels: &[u8], size: Size<f32>) {
    let Ok(directory) = std::env::var("INOX_SCREENSHOT_DIR") else {
        return;
    };

    let directory: PathBuf = Path::new(&directory).join("greet");
    std::fs::create_dir_all(&directory).expect("the snapshot directory is created");

    let path = directory.join(format!("inox-greet-{scenario}.png"));
    let image = image::RgbaImage::from_raw(size.width as u32, size.height as u32, pixels.to_vec())
        .expect("the rendered buffer matches the login screen's size");
    image.save(&path).expect("the snapshot is written");
}
