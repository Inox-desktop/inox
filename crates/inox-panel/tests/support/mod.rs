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

use inox_config::{PanelConfig, Rgba, ShellConfig, Theme as Tokens};
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

    /// A taskbar panel with `config` and `tokens`.
    pub fn with(config: PanelConfig, tokens: Tokens) -> Self {
        Self::with_shell(
            ShellConfig {
                panel: config,
                ..ShellConfig::default()
            },
            tokens,
        )
    }

    /// A panel of the surface `shell.panel.surface` names, drawn at the size its
    /// ordinary fallback window asks for.
    pub fn with_shell(shell: ShellConfig, tokens: Tokens) -> Self {
        let size = Size::new(WIDTH, app::window_height(&shell, &tokens));

        Self::with_shell_at(shell, tokens, size)
    }

    /// A panel of the surface `shell.panel.surface` names, drawn at `size`.
    pub fn with_shell_at(shell: ShellConfig, tokens: Tokens, size: Size<f32>) -> Self {
        let locale = inox_common::i18n::Locale::default_locale();
        let localizer = std::rc::Rc::new(i18n::load(&locale).expect("the panel's catalogs parse"));

        Self {
            state: State::with_shell(shell, tokens, localizer),
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
        self.render_over(Color::WHITE)
    }

    /// Draws the panel over `backdrop`.
    ///
    /// The menu bar and the dock are translucent: what a pixel inside the pane
    /// comes out as depends on what is behind the surface, and the compositor is
    /// what puts the wallpaper there. Rendering over a chosen colour is how a
    /// test asks whether the glass really is composited rather than assumed
    /// (ui-guidelines §12).
    pub fn render_over(&self, backdrop: Color) -> Vec<u8> {
        self.render_with_cursor_over(Point::ORIGIN, backdrop)
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
        self.render_with_cursor_over(cursor, Color::WHITE)
    }

    /// Draws the panel with the pointer at `cursor` over `backdrop`.
    pub fn render_with_cursor_over(&self, cursor: Point, backdrop: Color) -> Vec<u8> {
        self.draw(cursor, backdrop, false)
    }

    /// Draws the panel with the pointer at `cursor` and the left button held.
    ///
    /// A press is a state of its own (ui-guidelines §5): it has to be visible
    /// while it is held, not only once the button is released, so the harness
    /// needs a way to draw the frame between the two.
    pub fn render_while_pressed(&self, cursor: Point) -> Vec<u8> {
        self.render_while_pressed_over(cursor, Color::WHITE)
    }

    /// Draws the panel with the left button held over `backdrop`.
    pub fn render_while_pressed_over(&self, cursor: Point, backdrop: Color) -> Vec<u8> {
        self.draw(cursor, backdrop, true)
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

    /// One frame, over `backdrop`, with or without the left button held.
    fn draw(&self, cursor: Point, backdrop: Color, pressed: bool) -> Vec<u8> {
        let mut renderer = renderer();
        let mut ui = self.interface(&mut renderer);
        let pointer = mouse::Cursor::Available(cursor);
        let mut events = Vec::new();

        if pressed {
            events.push(Event::Mouse(mouse::Event::CursorMoved { position: cursor }));
            events.push(Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            )));
        }
        // The redraw request is what makes the interaction status iced remembers
        // visible in the frame that is drawn next (see `render_with_cursor`).
        events.push(Event::Window(
            window::Event::RedrawRequested(Instant::now()),
        ));

        ui.update(
            &events,
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
            backdrop,
        )
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

/// Whether a rendered buffer contains a pixel of exactly this colour.
///
/// This is how a test asks "did this state fill with the accent colour?"
/// without depending on where the glyphs of the platform's fonts happen to
/// land.
pub fn contains_colour(pixels: &[u8], colour: [u8; 4]) -> bool {
    pixels.as_chunks::<4>().0.contains(&colour)
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

/// The first and last column that holds anything other than `background`.
///
/// This is how a test asks *how wide* what was drawn is: the pane of a glass
/// surface is content-sized, so its own extent is the only honest measure of
/// whether what it holds fitted inside the surface it was drawn on.
pub fn ink_columns(
    pixels: &[u8],
    width: u32,
    height: u32,
    background: [u8; 4],
) -> Option<(u32, u32)> {
    let painted = |x: u32| (0..height).any(|y| pixel(pixels, width, x, y) != background);

    let first = (0..width).find(|x| painted(*x))?;
    let last = (first..width).rev().find(|x| painted(*x))?;

    Some((first, last))
}

/// The first and last column where two renders of the same surface disagree.
///
/// This is how a test isolates what one *sentence* adds to a pane that is
/// otherwise the same pane: the columns that differ are the columns the
/// sentence's own glyphs changed, which is a measurement of the text rather than
/// of the pane it sits in.
pub fn differing_columns(
    first: &[u8],
    second: &[u8],
    width: u32,
    height: u32,
) -> Option<(u32, u32)> {
    let differs = |x: u32| {
        (0..height).any(|y| {
            let at = ((y * width + x) * 4) as usize;

            first[at..at + 4] != second[at..at + 4]
        })
    };

    let first_column = (0..width).find(|x| differs(*x))?;
    let last_column = (first_column..width).rev().find(|x| differs(*x))?;

    Some((first_column, last_column))
}

/// The RGBA bytes an iced colour has when it is opaque, or its channels.
pub fn iced_pixel(colour: Color) -> [u8; 4] {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;

    [
        channel(colour.r),
        channel(colour.g),
        channel(colour.b),
        channel(colour.a),
    ]
}

/// Whether `value` lies strictly between the two sides on every colour channel.
///
/// This is the arithmetic behind "the glass is translucent": a pane pixel is
/// neither the backdrop it is drawn over nor the tint it is drawn with, but a
/// blend of the two on all three channels.
pub fn strictly_between(value: [u8; 4], first: [u8; 4], second: [u8; 4]) -> bool {
    (0..3).all(|channel| {
        let low = first[channel].min(second[channel]);
        let high = first[channel].max(second[channel]);

        value[channel] > low && value[channel] < high
    })
}

/// The first pixel that is strictly between `backdrop` and `tint` on every
/// channel, if the buffer has one.
pub fn between_pixel(pixels: &[u8], backdrop: [u8; 4], tint: [u8; 4]) -> Option<[u8; 4]> {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .find(|pixel| strictly_between(*pixel, backdrop, tint))
}

/// The pixel a straight-alpha `source` becomes when it is composited over an
/// opaque `backdrop`.
///
/// This is the arithmetic a frosted pane implies, written down once so a test
/// can ask whether the renderer really blended the pane rather than assuming it
/// did. The renderer works in premultiplied eight-bit channels, so a test
/// compares against this with a tolerance of a channel unit or two.
pub fn composite(source: [u8; 4], backdrop: [u8; 4]) -> [u8; 4] {
    let alpha = f32::from(source[3]) / 255.0;

    std::array::from_fn(|channel| {
        if channel == 3 {
            0xff
        } else {
            (f32::from(source[channel]) * alpha + f32::from(backdrop[channel]) * (1.0 - alpha))
                .round() as u8
        }
    })
}

/// The sum of a pixel's colour channels, for comparing two pixels' brightness
/// without depending on the exact rounding of one channel.
pub fn brightness(pixel: [u8; 4]) -> u32 {
    u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2])
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
