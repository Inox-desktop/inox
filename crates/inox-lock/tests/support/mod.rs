//! The harness the lock screen's render tests draw through.
//!
//! It draws through [`inox_lock::render::Interface`], which is the code the binary
//! commits its frames with: what the tests assert on are the pixels the lock
//! surfaces carry, and not a second rendering path that could drift from them.
//! There is no compositor and no GPU, which is what makes acceptance Q02 testable
//! in CI at all.
//!
//! Interaction is the one thing that has no production counterpart to borrow. No
//! lock surface reads input — ADR 0022 decision 6, and this build has nothing on
//! the screen to operate — so `click` builds its own `UserInterface` over the same
//! `app::view` and drives it the way iced itself would. That is a harness for a
//! question the binary cannot be asked yet, and it says so rather than implying
//! the lock screen answers a pointer.
//!
//! Finding a control to click is done from the pixels rather than from a
//! hand-computed coordinate. [`Lock::find`] returns the bounding box of a token
//! colour, and the unlock button is the only thing the theme draws in the accent
//! colour — so a test both *checks* that the control is drawn and gets a place to
//! click that cannot drift away from the layout.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use std::time::Instant;

use iced_core::clipboard;
use iced_core::mouse;
use iced_core::{Event, Font, Pixels, Point, Rectangle, Size, window};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;

use inox_config::{Rgba, Theme as Tokens};
use inox_lock::app::{self, Message, State};
use inox_lock::{i18n, render, theme};

/// The size acceptance Q02 snapshots are taken at.
pub const WIDTH: f32 = 1280.0;

/// The size acceptance Q02 snapshots are taken at.
pub const HEIGHT: f32 = 720.0;

/// The renderer's default text size, in logical pixels.
const TEXT_SIZE: f32 = 16.0;

/// The lock screen, its state, and the size it is drawn at.
pub struct Lock {
    state: State,
    size: Size<f32>,
}

impl Lock {
    /// A lock screen with the dark tokens.
    pub fn new() -> Self {
        Self::with(Tokens::builtin_dark())
    }

    /// A lock screen with `tokens`.
    pub fn with(tokens: Tokens) -> Self {
        Self::in_locale(tokens, &inox_common::i18n::Locale::default_locale())
    }

    /// A lock screen in a particular language.
    ///
    /// The catalogs are not only strings: a translated line has a different
    /// length, and `zh-CN` draws glyphs the Latin fallback does not have, so a
    /// screen that fits in one language may not fit in the other (acceptance
    /// Q02 and Q03).
    pub fn in_locale(tokens: Tokens, locale: &inox_common::i18n::Locale) -> Self {
        let localizer = i18n::load(locale).expect("the lock screen's catalogs parse");

        Self {
            state: State::new(std::rc::Rc::new(localizer), tokens),
            size: Size::new(WIDTH, HEIGHT),
        }
    }

    /// The lock screen's state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The lock screen's state, for a test that has to take what it queued.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// The design tokens.
    pub fn tokens(&self) -> &Tokens {
        self.state.tokens()
    }

    /// The size the lock screen is drawn at.
    pub fn size(&self) -> Size<f32> {
        self.size
    }

    /// Feeds a message to the update loop, the way the runtime would.
    ///
    /// The `Task` iced would run is dropped: this harness asserts on state and
    /// pixels, and the only task the lock screen ever returns is `Task::none()`.
    pub fn update(&mut self, message: Message) {
        let _ = app::update(&mut self.state, message);
    }

    /// Feeds several messages.
    pub fn update_all(&mut self, messages: impl IntoIterator<Item = Message>) {
        for message in messages {
            self.update(message);
        }
    }

    /// Types `text` one character at a time.
    pub fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.update(Message::Character(character));
        }
    }

    /// The iced theme the lock screen draws under.
    pub fn theme(&self) -> iced::Theme {
        theme::from_tokens(self.state.tokens())
    }

    /// Draws the lock screen and returns its RGBA8 pixels.
    ///
    /// Through the component's own renderer, so "the lock frame is opaque" and
    /// "two faces look different" are claims about the frames the binary commits.
    /// The size is the harness's own, so a frame that could not be drawn at it
    /// would be a test bug rather than a case to handle.
    pub fn render(&self) -> Vec<u8> {
        let mut interface = render::Interface::new(self.size.width as u32, self.size.height as u32)
            .expect("the harness draws at a size the component draws at");

        interface.render(&self.state).to_vec()
    }

    /// The bounding box of everything drawn in `colour`, in logical pixels.
    ///
    /// `None` when nothing is drawn in it, which is itself an assertion a test can
    /// make: a control that vanishes when it is disabled has no accent anywhere.
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

impl Default for Lock {
    fn default() -> Self {
        Self::new()
    }
}

/// Delivers the redraw a real runtime delivers once per frame.
///
/// iced's widgets work out their interaction status while handling
/// `RedrawRequested` and store it for the next draw — a button that has never
/// seen one is drawn as disabled (`iced_widget::button`, `Widget::update`). A
/// harness that skipped this would render every control in its disabled state and
/// conclude, wrongly, that the theme had no accent in it.
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

/// Whether anything was drawn over `background` in the columns `from..to`.
///
/// This is how a test asks "is there anything where the mask is?" without
/// depending on which glyphs the platform's fonts happen to provide.
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

    let path: PathBuf = Path::new(&directory).join(format!("inox-lock-{scenario}.png"));
    let image = image::RgbaImage::from_raw(size.width as u32, size.height as u32, pixels.to_vec())
        .expect("the rendered buffer matches the lock screen's size");
    image.save(&path).expect("the snapshot is written");
}
