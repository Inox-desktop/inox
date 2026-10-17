//! The harness the settings window's tests run through.
//!
//! It builds the window's real state over a real configuration directory —
//! `Documents` reads files, so the tests use a temporary directory rather than
//! an in-memory stand-in — lays the real widget tree out at a fixed size and
//! draws it with the same `tiny-skia` renderer the binary uses, with no
//! compositor and no GPU. That is what makes acceptance Q02, D04 and D05
//! testable in CI at all.
//!
//! Interaction is driven the same way iced itself drives it: the same
//! `UserInterface` receives real `Event`s at a real cursor position, and the
//! messages it produces are the ones the application's `update` receives. The
//! saves the update loop queues are performed here the way `main.rs` performs
//! them, so a test exercises the same path a user does.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;

use iced_core::clipboard;
use iced_core::mouse;
use iced_core::renderer::{Headless, Style};
use iced_core::{Color, Event, Font, Pixels, Point, Size};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;

use inox_common::i18n::Locale;
use inox_config::{Appearance, ConfigPaths, InoxConfig, Rgba, Theme as Tokens};
use inox_settings::app::{self, Message, OutgoingCommand, SaveRequest, State};
use inox_settings::document::Documents;
use inox_settings::{i18n, persist, theme};

/// The size the acceptance snapshots are taken at (Q02).
pub const WIDTH: f32 = 1280.0;

/// The height the acceptance snapshots are taken at (Q02).
pub const HEIGHT: f32 = 720.0;

/// The renderer's default text size, in logical pixels.
const TEXT_SIZE: f32 = 16.0;

/// The settings window, over a real configuration directory.
pub struct Window {
    directory: tempfile::TempDir,
    paths: ConfigPaths,
    state: State,
}

impl Window {
    /// A window over a directory with no configuration files at all.
    pub fn new() -> Self {
        Self::with_files(&[])
    }

    /// A window over a directory holding `files`, as `(name, content)` pairs.
    pub fn with_files(files: &[(&str, &str)]) -> Self {
        Self::with_locale(files, Locale::default_locale(), Tokens::builtin_dark())
    }

    /// A window with a chosen locale and theme.
    pub fn with_locale(files: &[(&str, &str)], locale: Locale, tokens: Tokens) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = ConfigPaths::new(directory.path());

        for (name, content) in files {
            std::fs::write(directory.path().join(name), content).expect("the file is written");
        }

        let config = InoxConfig::load(&paths, Appearance::System).expect("the configuration loads");
        let documents = Documents::open(&config, &paths).expect("the files are read");
        let localizer = Rc::new(i18n::load(&locale).expect("the catalogs parse"));

        let state = State::new(&config, documents, tokens, localizer);

        Self {
            directory,
            paths,
            state,
        }
    }

    /// The directory the window's files live in.
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }

    /// The paths the window resolved.
    pub fn paths(&self) -> &ConfigPaths {
        &self.paths
    }

    /// The window's state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The window's state, for a test that drives an edit by hand.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Feeds a message to the update loop, the way the runtime would.
    pub fn update(&mut self, message: Message) {
        let _ = app::update(&mut self.state, message);
    }

    /// Feeds several messages.
    pub fn update_all(&mut self, messages: impl IntoIterator<Item = Message>) {
        for message in messages {
            self.update(message);
        }
    }

    /// The size the window is drawn at.
    pub fn size(&self) -> Size<f32> {
        Size::new(WIDTH, HEIGHT)
    }

    /// The iced theme the window draws under.
    pub fn theme(&self) -> iced::Theme {
        theme::from_tokens(self.state.tokens())
    }

    /// Draws the window and returns its RGBA8 pixels.
    ///
    /// The widgets are given one redraw event first, which is what makes them
    /// work out their own status: iced's `Button` keeps its
    /// hover/pressed/disabled status in itself and only recomputes it when the
    /// runtime asks for a redraw — until then it falls back to `Disabled`.
    /// Drawing without that step would render every control as unusable, and a
    /// test that drew that way would be asserting about a window no user sees.
    pub fn render(&self) -> Vec<u8> {
        let mut renderer = renderer();
        let mut ui = self.interface(&mut renderer);
        let mut messages = Vec::new();

        ui.update(
            &[Event::Window(iced_core::window::Event::RedrawRequested(
                iced_core::time::Instant::now(),
            ))],
            mouse::Cursor::Available(Point::ORIGIN),
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );

        ui.draw(
            &mut renderer,
            &self.theme(),
            &Style::default(),
            mouse::Cursor::Available(Point::ORIGIN),
        );

        Headless::screenshot(
            &mut renderer,
            Size::new(WIDTH as u32, HEIGHT as u32),
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

    /// Asks for the change to be written, and performs the writes.
    ///
    /// This is the loop `main.rs` runs: [`app::update`] queues, the runtime
    /// performs and the outcome comes back as a message. Returns how many files
    /// were written.
    pub fn apply_and_drain(&mut self) -> usize {
        self.update(Message::Apply);

        self.drain()
    }

    /// Performs every command the update loop has queued.
    pub fn drain(&mut self) -> usize {
        let mut written = 0;

        loop {
            let commands = self.state.take_outgoing();

            if commands.is_empty() {
                return written;
            }

            for command in commands {
                match command {
                    OutgoingCommand::Save(request) => {
                        written += 1;
                        let SaveRequest {
                            generation,
                            file,
                            path,
                            content,
                            guard,
                            ..
                        } = *request;

                        let outcome = persist::save(&path, content.as_bytes(), Some(&guard));

                        self.update(Message::SaveSettled {
                            generation,
                            file,
                            outcome,
                        });
                    }
                    OutgoingCommand::Apply {
                        generation,
                        component,
                    } => {
                        // No transport exists in this milestone, which is what
                        // the binary reports as well (ADR 0018).
                        self.update(Message::ComponentUnreachable {
                            generation,
                            component,
                            reason: "no transport is implemented in this milestone".to_owned(),
                        });
                    }
                    OutgoingCommand::Reload => {
                        let config = InoxConfig::load(&self.paths, Appearance::System)
                            .expect("the files load");
                        let documents = Documents::open(&config, &self.paths)
                            .expect("the files are read again");

                        self.update(Message::Reloaded {
                            config: Box::new(config),
                            documents: Box::new(documents),
                        });
                    }
                }
            }
        }
    }

    /// Reads the configuration back from the directory, the way a restart does.
    pub fn reload_config(&self) -> InoxConfig {
        InoxConfig::load(&self.paths, Appearance::System).expect("the files load")
    }

    /// The bytes of one of the window's files.
    pub fn file(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.directory.path().join(name)).unwrap_or_default()
    }

    /// The path of one of the window's files.
    pub fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn interface<'a>(
        &'a self,
        renderer: &mut iced_tiny_skia::Renderer,
    ) -> UserInterface<'a, Message, iced::Theme, iced_tiny_skia::Renderer> {
        UserInterface::build(
            app::view(&self.state),
            self.size(),
            Cache::default(),
            renderer,
        )
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

/// The renderer the binary uses, with no GPU and no compositor.
pub fn renderer() -> iced_tiny_skia::Renderer {
    iced_tiny_skia::Renderer::new(Font::default(), Pixels(TEXT_SIZE))
}

/// The RGBA colour of one pixel of a rendered buffer.
pub fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let index = ((y * WIDTH as u32 + x) * 4) as usize;

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

/// How many pixels are within `tolerance` of `colour`, channel by channel.
///
/// Text is antialiased, so the colour a glyph is drawn in is exact only in the
/// pixels its strokes cover completely; a tolerance is what lets a test ask "was
/// this drawn in that colour?" without depending on the hinting.
pub fn count_near(pixels: &[u8], colour: [u8; 4], tolerance: u8) -> usize {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|chunk| {
            chunk
                .iter()
                .zip(colour.iter())
                .all(|(left, right)| left.abs_diff(*right) <= tolerance)
        })
        .count()
}

/// Whether anything was drawn over `background` in the rectangle given.
///
/// This is how a test asks "is that label still there?" without depending on
/// which glyphs the platform's fonts happen to provide.
pub fn has_ink(
    pixels: &[u8],
    from_x: u32,
    to_x: u32,
    from_y: u32,
    to_y: u32,
    background: [u8; 4],
) -> bool {
    (from_y..to_y.min(HEIGHT as u32))
        .any(|y| (from_x..to_x.min(WIDTH as u32)).any(|x| pixel(pixels, x, y) != background))
}

/// Writes a snapshot when `INOX_SCREENSHOT_DIR` is set (acceptance Q02).
///
/// CI collects the directory as an artifact; without the variable the test only
/// asserts on the pixels, and no file is written.
pub fn snapshot(scenario: &str, pixels: &[u8]) {
    let Ok(directory) = std::env::var("INOX_SCREENSHOT_DIR") else {
        return;
    };

    std::fs::create_dir_all(&directory).expect("the snapshot directory is created");

    let path: PathBuf = Path::new(&directory).join(format!("inox-settings-{scenario}.png"));
    let image = image::RgbaImage::from_raw(WIDTH as u32, HEIGHT as u32, pixels.to_vec())
        .expect("the rendered buffer matches the window's size");
    image.save(&path).expect("the snapshot is written");
}
