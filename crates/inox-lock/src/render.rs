//! The lock screen's own renderer: this crate's widget tree, drawn offscreen.
//!
//! [ADR 0015] puts the lock state in the compositor and makes this crate the
//! trusted client of `ext-session-lock-v1`. [`crate::session`] is the client half
//! of that; this module is what puts an *interface* on the frames it commits.
//! Until it existed the protocol leg could cover an output with a colour and
//! nothing else, because the interface was only ever drawn by the test harness.
//!
//! # Why there is no window here
//!
//! A lock surface is not a window. `iced`'s own Wayland support creates
//! `xdg-shell` surfaces, and there is no way to ask it for a session-lock
//! surface — an interface drawn through `iced`'s runtime would be a *second*
//! surface, not a lock surface, which is the layer-shell-shaped mistake ADR 0015
//! refuses. What `iced` does expose is the pair the harness in `tests/render.rs`
//! already uses: [`iced_runtime::UserInterface`] lays a widget tree out and
//! routes events to it, and the `tiny-skia` renderer draws it into pixels. Those
//! pixels are what `wl_shm` wants, so the lock surface carries the same widget
//! tree the tests assert on — not a reimplementation of it.
//!
//! The consequence worth stating: this module draws, and does nothing else. It
//! does not read the keyboard, it does not decide anything, and it hands no
//! message anywhere. What it draws is the projection of [`crate::app::State`],
//! which is the same state the protocol leg drives.
//!
//! # The two things it must not do
//!
//! - **It must not draw a frame of the wrong size.** The protocol makes the
//!   committed buffer's size an exact requirement of the configure it answers, so
//!   the size is set from the configure and nothing else.
//! - **It must not draw an unbounded amount.** A size a compositor asks for is
//!   data from outside this process; a frame larger than any display Inox
//!   supports is refused rather than allocated.
//!
//! [ADR 0015]: https://github.com/Inox-desktop/inox/blob/main/docs/adr/0015-session-lock-and-failure-boundaries.md

use std::time::Instant;

use iced_core::renderer::{Headless, Style};
use iced_core::{Color, Event, Font, Pixels, Size, clipboard, mouse, window};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;
use tracing::debug;

use crate::app::{self, State};
use crate::theme;

/// The scale factor a lock frame is drawn at.
///
/// One, and the reason is what the number *means*: iced multiplies every logical
/// coordinate by it to get a pixel, so anything but one would draw the interface at
/// an arbitrary size of this component's own choosing. The size to draw at comes
/// from the compositor's configure, and how large the text should be comes from the
/// design tokens' `typography.scale` — this component does not get a third say.
pub const SCALE_FACTOR: f32 = 1.0;

/// The largest frame this component will draw, in bytes.
///
/// An 8K frame is about 132 MiB, so this leaves room for the widest display Inox
/// claims to support while still refusing a size nothing could have meant. It is a
/// statement about supported outputs, and **not** the bound on this process's
/// memory: drawing one frame costs several times these bytes, because iced's
/// offscreen path allocates the frame again (an offscreen buffer, a clip mask and
/// the returned bytes) while the previous frame is still held, and a caller may be
/// holding a few frames the compositor has not released. What it refuses is a
/// *single* frame larger than any display this project claims, which is the case
/// where "draw it anyway" would allocate whatever two integers asked for.
pub const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

/// The text size the renderer starts from, matching [`crate::theme::text_size`].
fn default_text_size() -> Pixels {
    iced::Settings::default().default_text_size
}

/// A lock surface's interface: a renderer, the widget tree's cache, and its size.
///
/// One per output, because each output has its own configured size and its own
/// widget state to lay out.
pub struct Interface {
    renderer: iced_tiny_skia::Renderer,
    /// The widget tree carried between frames. Rebuilt only in identity: keeping
    /// it is what lets `iced` match the new tree against the old one instead of
    /// treating every frame as a brand new screen.
    cache: Option<Cache>,
    /// The surface's size in pixels, kept as the integers the configure delivered.
    ///
    /// Not as `Size<f32>`: `f32` cannot represent every `u32` exactly, so a
    /// dimension past 2^24 would round on the way in and the frame would be drawn
    /// with different dimensions from the ones the buffer is created for — a
    /// declared size larger than the memory that was written. The compositor's
    /// configure is the one number in this component that comes from outside, and
    /// it is kept exactly.
    width: u32,
    height: u32,
    /// The last frame drawn, in RGBA order.
    pixels: Vec<u8>,
}

impl Interface {
    /// An interface for a surface of `width` × `height`.
    ///
    /// `None` when the size is one this component will not draw: a zero-sized
    /// surface (nothing to draw on) or a frame past [`MAX_FRAME_BYTES`].
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let (width, height) = frame_size(width, height)?;

        Some(Self {
            renderer: iced_tiny_skia::Renderer::new(Font::DEFAULT, default_text_size()),
            cache: None,
            width,
            height,
            pixels: Vec::new(),
        })
    }

    /// The size this interface draws at, in pixels.
    ///
    /// The integers, not a `Size<f32>`: a caller comparing this against what the
    /// compositor configured has to be able to tell a size that differs, and
    /// rounding is exactly the way to lose that.
    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The size this interface draws at, as layout bounds want it.
    pub fn size(&self) -> Size<f32> {
        Size::new(self.width as f32, self.height as f32)
    }

    /// Resizes the interface for a new configure.
    ///
    /// Returns whether the size changed, so a caller can tell a redundant
    /// configure from one that has to be redrawn. A size this component will not
    /// draw leaves the interface as it was: the previous frame is still a
    /// covering frame, and a covering frame of the wrong size is not better than
    /// the one already on screen.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        let Some((width, height)) = frame_size(width, height) else {
            return false;
        };

        if self.dimensions() == (width, height) {
            return false;
        }

        self.width = width;
        self.height = height;

        true
    }

    /// Draws `state` and returns the frame in RGBA order.
    pub fn render(&mut self, state: &State) -> &[u8] {
        self.draw(state);

        &self.pixels
    }

    /// Draws `state` into `out` as the byte order `wl_shm` wants.
    ///
    /// Written straight out rather than returned, so a caller that is filling a
    /// shared-memory frame does not hold a second copy of the screen.
    pub fn render_argb(&mut self, state: &State, out: &mut Vec<u8>) {
        self.draw(state);
        write_argb(&self.pixels, out);
    }

    /// The last frame drawn, in RGBA order, and empty before the first draw.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Lays the widget tree out and draws it.
    fn draw(&mut self, state: &State) {
        let theme = theme::from_tokens(state.tokens());
        let cache = self.cache.take().unwrap_or_default();
        let mut ui = UserInterface::build(app::view(state), self.size(), cache, &mut self.renderer);
        let mut produced = Vec::new();

        // The redraw a real runtime delivers once per frame. `iced`'s widgets work
        // out their interaction status while handling it and store it for the next
        // draw, and a control that has never seen one is drawn disabled
        // (`iced_widget::button`, `Widget::update`). Without it every lock frame
        // would show a screen whose controls all look unavailable, which is a lie
        // about what the projection said.
        let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));

        ui.update(
            &[redraw],
            mouse::Cursor::Unavailable,
            &mut self.renderer,
            &mut clipboard::Null,
            &mut produced,
        );

        // A redraw produces no messages in this crate — nothing here reacts to
        // being drawn — so this is a diagnostic rather than a queue something
        // drains. It is logged rather than dropped silently because a widget that
        // started producing one would be a widget whose state this module is
        // quietly discarding.
        if !produced.is_empty() {
            debug!(
                messages = produced.len(),
                "drawing the lock screen produced messages, which nothing consumes"
            );
        }

        ui.draw(
            &mut self.renderer,
            &theme,
            &Style::default(),
            mouse::Cursor::Unavailable,
        );

        self.cache = Some(ui.into_cache());

        // Composited over white on the way out: the screen's own background is
        // opaque, so this only decides what a frame would look like if it were
        // not — which is why the tests assert the frame is opaque rather than
        // relying on this.
        let mut frame = Headless::screenshot(
            &mut self.renderer,
            Size::new(self.width, self.height),
            SCALE_FACTOR,
            Color::WHITE,
        );

        std::mem::swap(&mut self.pixels, &mut frame);
    }
}

/// The size the interface will draw, or `None` when it will not draw that size.
fn frame_size(width: u32, height: u32) -> Option<(u32, u32)> {
    if width == 0 || height == 0 {
        return None;
    }

    let bytes = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;

    (bytes <= MAX_FRAME_BYTES).then_some((width, height))
}

/// The bytes of a `wl_shm` `Argb8888` frame, from an RGBA screenshot.
///
/// `wl_shm` names the *words* rather than the bytes, and on little-endian — every
/// architecture Inox supports — a word of `Argb8888` is `B, G, R, A` in memory.
/// The compositor's own read-back tests assert on that order, so this is the same
/// translation `src/session.rs` used to do for its flat colour.
fn write_argb(rgba: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(rgba.len());

    for pixel in rgba.as_chunks::<4>().0 {
        out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Message;
    use inox_common::i18n::Locale;
    use inox_config::Theme as Tokens;

    fn state() -> State {
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");

        State::new(std::rc::Rc::new(localizer), Tokens::builtin_dark())
    }

    /// A lock screen that is locked, with its one output covered.
    fn locked() -> State {
        let mut state = state();

        for message in [
            Message::ProtocolPresent,
            Message::OutputAdded(crate::state::OutputId::new("eDP-1")),
            Message::LockRequested(crate::state::LockReason::User),
            Message::OutputCovered(crate::state::OutputId::new("eDP-1")),
        ] {
            let _ = app::update(&mut state, message);
        }

        state
    }

    #[test]
    fn a_frame_is_the_size_it_was_configured_to() {
        let interface = Interface::new(320, 240).expect("a drawable size");

        assert_eq!(interface.dimensions(), (320, 240));
    }

    #[test]
    fn a_size_nothing_could_have_meant_is_refused_rather_than_allocated() {
        assert!(Interface::new(0, 240).is_none(), "a surface with no width");
        assert!(Interface::new(320, 0).is_none(), "a surface with no height");
        assert!(
            Interface::new(65_536, 65_536).is_none(),
            "16 GiB of frame is not a display"
        );
        assert!(
            Interface::new(3840, 2160).is_some(),
            "and a real 4K output is not refused"
        );
    }

    #[test]
    fn a_configure_that_does_not_change_the_size_says_so() {
        let mut interface = Interface::new(320, 240).expect("a drawable size");

        assert!(!interface.resize(320, 240), "the same size");
        assert!(interface.resize(640, 480), "a real resize");
        assert_eq!(interface.dimensions(), (640, 480));
        assert!(
            !interface.resize(u32::MAX, u32::MAX),
            "a size that cannot be drawn leaves the frame that is already there"
        );
        assert_eq!(interface.dimensions(), (640, 480));
    }

    #[test]
    fn a_lock_frame_is_the_projection_drawn_and_is_fully_opaque() {
        let state = locked();
        let mut interface = Interface::new(320, 240).expect("a drawable size");

        let pixels = interface.render(&state).to_vec();

        assert_eq!(pixels.len(), 320 * 240 * 4, "one RGBA quad per pixel");
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 0xff),
            "a lock frame that is not opaque would show what it is covering"
        );

        // The interface, and not a flat colour: a covering frame is exactly the
        // thing ADR 0022 decision 6 used to describe, and this is what replaces it.
        let first = &pixels[..4];
        assert!(
            pixels.as_chunks::<4>().0.iter().any(|pixel| pixel != first),
            "the lock frame is a single colour, so nothing was drawn on it"
        );
    }

    #[test]
    fn two_faces_of_the_same_state_draw_two_different_frames() {
        let mut interface = Interface::new(320, 240).expect("a drawable size");
        let prompt_free = interface.render(&locked()).to_vec();

        let mut other = locked();
        let _ = app::update(
            &mut other,
            Message::AuthUnavailable(crate::contract::ServiceFailure::NotProvided),
        );
        let unavailable = interface.render(&other).to_vec();

        assert_ne!(
            prompt_free, unavailable,
            "a screen that says it cannot run must not look like a screen that asks \
             for a password"
        );
    }

    #[test]
    fn the_frame_carries_the_theme_background_in_the_byte_order_wl_shm_wants() {
        // The one assertion that catches a swapped channel. A screenshot gives
        // RGBA and `wl_shm`'s `Argb8888` wants B, G, R, A in memory, so writing the
        // screenshot straight into the frame renders a lock screen whose background
        // is the wrong colour — and nothing else notices: it is still opaque, still
        // the configured size, and still not uniform. Pinning it against the token
        // rather than against a literal is what makes the expectation the theme's.
        let state = locked();
        let mut interface = Interface::new(160, 120).expect("a drawable size");
        let mut argb = Vec::new();

        interface.render_argb(&state, &mut argb);

        let background = state.tokens().colors.background;
        let expected = [background.b, background.g, background.r, background.a];
        let dominant = argb
            .as_chunks::<4>()
            .0
            .iter()
            .fold(([0u8; 4], 0usize), |(best, count), pixel| {
                let same = argb
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|other| *other == pixel)
                    .count();

                if same > count {
                    (*pixel, same)
                } else {
                    (best, count)
                }
            })
            .0;

        assert_ne!(
            expected,
            [background.r, background.g, background.b, background.a],
            "the theme's background has equal red and blue, so this test cannot \
             tell a swapped channel from a correct one; pin it against a token \
             whose channels differ instead of weakening this"
        );
        assert_eq!(
            dominant, expected,
            "the frame's dominant colour is the screen's background token, in the \
             byte order `wl_shm` names"
        );
    }

    #[test]
    fn a_dimension_f32_cannot_hold_exactly_still_frames_the_declared_size() {
        // `f32` has 24 bits of mantissa, so a dimension past 2^24 rounds on the way
        // in. A frame drawn at the rounded size would be handed to a buffer created
        // for the real one — the compositor mapping bytes nobody wrote. The
        // configure's dimensions are therefore kept as the integers they arrived
        // as, and this is the case that tells.
        let width = (1u32 << 24) + 1;

        assert_ne!(
            width as f32 as u32, width,
            "this test is about a dimension `f32` cannot represent exactly"
        );

        let state = locked();
        let mut interface = Interface::new(width, 1).expect("within the frame bound");
        let mut argb = Vec::new();

        interface.render_argb(&state, &mut argb);

        assert_eq!(
            interface.dimensions(),
            (width, 1),
            "the configure is kept exactly, not as the size it would round to"
        );
        assert_eq!(
            argb.len(),
            width as usize * 4,
            "and the frame is that many pixels, which is what the buffer will \
             declare"
        );
    }

    #[test]
    fn a_frame_is_little_endian_argb() {
        // `wl_shm`'s `Argb8888` names the *words*: the byte order on the wire is
        // B, G, R, A, which is what the compositor's own pixel assertions use.
        let mut out = Vec::new();

        write_argb(&[0x11, 0x22, 0x33, 0xff, 0x44, 0x55, 0x66, 0x80], &mut out);

        assert_eq!(out, [0x33, 0x22, 0x11, 0xff, 0x66, 0x55, 0x44, 0x80]);
    }

    #[test]
    fn writing_a_frame_replaces_whatever_was_in_the_buffer() {
        let mut out = vec![0xde, 0xad, 0xbe, 0xef];

        write_argb(&[0x11, 0x22, 0x33, 0xff], &mut out);

        assert_eq!(
            out,
            [0x33, 0x22, 0x11, 0xff],
            "a frame that left the previous one's tail behind would be a frame of \
             the wrong length"
        );
    }

    #[test]
    fn the_argb_frame_is_exactly_as_long_as_the_rgba_one() {
        let state = locked();
        let mut interface = Interface::new(160, 120).expect("a drawable size");
        let rgba = interface.render(&state).len();
        let mut out = Vec::new();

        interface.render_argb(&state, &mut out);

        assert_eq!(out.len(), rgba);
    }
}
