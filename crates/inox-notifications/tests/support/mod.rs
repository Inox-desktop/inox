//! The harness the daemon's render tests draw through.
//!
//! It builds the real widget tree, lays it out at a fixed size and draws it with
//! the same `tiny_skia` renderer the binary uses — with no compositor and no GPU,
//! which is what makes acceptance Q02 testable in CI at all.
//!
//! Interaction is driven the same way iced itself drives it: the same
//! `UserInterface` receives real `Event`s at a real cursor position, and the
//! messages it produces are the ones the application's `update` would receive.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use iced_core::clipboard;
use iced_core::mouse;
use iced_core::renderer::{Headless, Style};
use iced_core::{Color, Event, Font, Pixels, Point, Size};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;

use inox_config::{NotificationsConfig, Rgba, Theme as Tokens};
use inox_notifications::app::{self, Message, State};
use inox_notifications::contract::{CallId, NotifyRequest, Request, RequestBody};
use inox_notifications::queue::NotificationId;
use inox_notifications::{i18n, theme};

/// The size the acceptance snapshots are taken at (Q02).
pub const WIDTH: f32 = 1280.0;

/// The height the acceptance snapshots are taken at (Q02).
pub const HEIGHT: f32 = 720.0;

/// The renderer's default text size, in logical pixels.
const TEXT_SIZE: f32 = 16.0;

/// The daemon, its state, and the size it is drawn at.
pub struct Daemon {
    state: State,
    size: Size<f32>,
    calls: u64,
}

impl Daemon {
    /// A daemon with the default configuration and the dark tokens.
    pub fn new() -> Self {
        Self::with(NotificationsConfig::default(), Tokens::builtin_dark())
    }

    /// A daemon with `config` and `tokens`.
    pub fn with(config: NotificationsConfig, tokens: Tokens) -> Self {
        let locale = inox_common::i18n::Locale::default_locale();
        let localizer = i18n::load(&locale).expect("the daemon's catalogs parse");

        Self {
            state: State::new(config, tokens, Rc::new(localizer)),
            size: Size::new(WIDTH, HEIGHT),
            calls: 0,
        }
    }

    /// The daemon's state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The daemon's state, for a test that has to take what it queued.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// The size the daemon is drawn at.
    pub fn size(&self) -> Size<f32> {
        self.size
    }

    /// Feeds a message to the update loop, the way the runtime would.
    ///
    /// The `Task` iced would run is dropped: this harness asserts on state and
    /// pixels, and the only task the daemon ever returns is `Task::none()`.
    pub fn update(&mut self, message: Message) {
        let _ = app::update(&mut self.state, message);
    }

    /// Notifies, the way a client's `Notify` call would, and returns the
    /// identifier the daemon answered with.
    pub fn notify(&mut self, app_name: &str, summary: &str, body: &str) -> NotificationId {
        self.notify_with(NotifyRequest {
            app_name: app_name.to_owned(),
            summary: summary.to_owned(),
            body: body.to_owned(),
            ..NotifyRequest::default()
        })
    }

    /// The same, for a call that needs hints or actions.
    pub fn notify_with(&mut self, request: NotifyRequest) -> NotificationId {
        self.calls += 1;
        let call = CallId::new(self.calls);

        self.update(Message::Request(Request::new(
            call,
            RequestBody::Notify(request),
        )));

        self.state_mut()
            .take_outgoing()
            .into_iter()
            .find_map(|outgoing| match outgoing {
                inox_notifications::contract::Outgoing::Reply(
                    inox_notifications::contract::Reply {
                        body: inox_notifications::contract::ReplyBody::Notified { id },
                        ..
                    },
                ) => Some(id),
                _ => None,
            })
            .expect("a Notify call is answered with an identifier")
    }

    /// Notifies and then lets the configured lifetime run out.
    pub fn expire_everything(&mut self) {
        self.update(Message::Tick(Instant::now() + Duration::from_secs(86_400)));
        let _ = self.state_mut().take_outgoing();
    }

    /// The iced theme the daemon draws under.
    pub fn theme(&self) -> iced::Theme {
        theme::iced_theme(self.state.tokens())
    }

    /// Draws the popup stack and returns its RGBA8 pixels.
    ///
    /// The user interface is updated with one redraw request before it is
    /// drawn, which is what gives every widget its interaction status: iced
    /// resolves a widget's status inside `update`, and only a redraw request
    /// makes a button keep it. A widget that has never been through one draws
    /// itself disabled, and a test about an enabled button would be comparing
    /// two disabled ones.
    pub fn render(&self) -> Vec<u8> {
        let mut renderer = renderer();
        let mut ui = self.interface(&mut renderer);
        let mut messages = Vec::new();

        ui.update(
            &[Event::Window(iced_core::window::Event::RedrawRequested(
                std::time::Instant::now(),
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

impl Default for Daemon {
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

/// How many pixels of a rendered buffer carry `colour`.
pub fn count_of(pixels: &[u8], colour: [u8; 4]) -> usize {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|chunk| **chunk == colour)
        .count()
}

/// Whether anything was drawn over `background` in the rectangle
/// `(from_x, from_y)..(to_x, to_y)`.
///
/// This is how a test asks "is there something in the top-right corner?" without
/// depending on which glyphs the platform's fonts happen to provide.
pub fn has_ink(
    pixels: &[u8],
    width: u32,
    from: (u32, u32),
    to: (u32, u32),
    background: [u8; 4],
) -> bool {
    (from.1..to.1).any(|y| (from.0..to.0).any(|x| pixel(pixels, width, x, y) != background))
}

/// The middle of the highest run of `colour` inside a rectangle.
///
/// Used to find a control that sits above others of the same colour — the close
/// button, which shares the muted colour with the application name beside it —
/// by looking only where it can be: the caller passes the half of the popup the
/// control is in.
pub fn ink_span(
    pixels: &[u8],
    size: Size<f32>,
    colour: [u8; 4],
    from: (u32, u32),
    to: (u32, u32),
) -> Option<Point> {
    let width = size.width as u32;

    for y in from.1..to.1.min(size.height as u32) {
        let run: Vec<u32> = (from.0..to.0.min(width))
            .filter(|x| pixel(pixels, width, *x, y) == colour)
            .collect();

        if run.is_empty() {
            continue;
        }

        let first = run.first().copied().unwrap_or(from.0);
        let last = run.last().copied().unwrap_or(first);

        return Some(Point::new((first + last) as f32 / 2.0, y as f32 + 0.5));
    }

    None
}

/// The lowest pixel of `colour`, and the middle of the run it is in.
///
/// The daemon's action buttons are the only accent-coloured thing in the stack,
/// so a test that needs to click one finds it by its ink instead of by guessing
/// where the layout put it — which would depend on the platform's font metrics.
pub fn lowest_ink(pixels: &[u8], size: Size<f32>, colour: [u8; 4]) -> Option<Point> {
    let width = size.width as u32;
    let height = size.height as u32;

    for y in (0..height).rev() {
        let run: Vec<u32> = (0..width)
            .filter(|x| pixel(pixels, width, *x, y) == colour)
            .collect();

        if let Some(middle) = run.get(run.len() / 2) {
            return Some(Point::new(*middle as f32 + 0.5, y as f32 + 0.5));
        }
    }

    None
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

    let path: PathBuf = Path::new(&directory).join(format!("inox-notifications-{scenario}.png"));
    let image = image::RgbaImage::from_raw(size.width as u32, size.height as u32, pixels.to_vec())
        .expect("the rendered buffer matches the surface's size");
    image.save(&path).expect("the snapshot is written");
}
