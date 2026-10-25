//! The lock screen's frames: QML, drawn offscreen into a lock surface's buffer.
//!
//! the session-lock boundary puts the lock state in the compositor and makes this crate the
//! trusted client of `ext-session-lock-v1`. [`crate::session`] is the client half
//! of that; this module is what puts an *interface* on the frames it commits.
//!
//! # Why there is no Qt window here
//!
//! A lock surface is not a window. Qt has no `ext-session-lock-v1` client, and
//! `LayerShellQt` is not an option for exactly the reason the session-lock boundary gives: a Qt
//! window would be a *second* surface, not a lock surface — the layer-shell-shaped
//! mistake the ADR refuses. So the crate keeps the protocol leg and makes QML a
//! pure renderer: [`crate::cxxqt_object`] holds a C++ `QGuiApplication` on the
//! **offscreen** platform with Qt Quick's **software** scene graph, renders
//! [`crate::view`]'s projection into a `QQuickWindow`, and grabs the frame. That
//! frame is what `wl_shm` wants.
//!
//! Before the migration this module drew the widget tree itself through
//! `iced_core`, `iced_runtime` and `iced_tiny_skia`. Those dependencies are gone
//! (migration contract §5): the pixels now come from Qt Quick, and the only part
//! of the old renderer left is the byte translation into the `Argb8888` frame
//! `wl_shm` takes.
//!
//! One [`Interface`] per lock surface. Each holds its own
//! `QQmlApplicationEngine` + `QQuickWindow`, so several outputs can be rendered at
//! their own sizes in the same process — a session with a laptop panel and a 4K
//! monitor gets two different frames, and neither has to be scaled to the other.
//! The process-wide `QGuiApplication` is created once, on the first `Interface`;
//! that is the only piece of Qt state the surfaces share.
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
//!   the size is set from the configure and nothing else, and a grab that comes
//!   back at another size is refused rather than written out.
//! - **It must not draw an unbounded amount.** A size a compositor asks for is
//!   data from outside this process; a frame larger than any display Inox
//!   supports is refused rather than allocated.

use std::time::Instant;

use tracing::{debug, warn};

use crate::app::State;
use crate::cxxqt_object::qobject::{self, Offscreen};
use crate::view;

/// The resource URL of the crate's QML module window.
///
/// cxx-qt registers `qml/Main.qml` under the module's import path and keeps the
/// file's own relative path as its resource alias, so the module URI
/// `com.inox.lock.qml` (path segments `com/inox/lock/qml`) plus the alias
/// `qml/Main.qml` give the doubled `qml` below (contract §2.2; the same shape
/// `inox-greet` and the offscreen probe use).
pub const QML_URL: &str = "qrc:/qt/qml/com/inox/lock/qml/qml/Main.qml";

/// The scale factor a lock frame is drawn at.
///
/// One, and the reason is what the number *means*: the frame is grabbed at the
/// surface's own pixels, so anything but one would draw the interface at an
/// arbitrary size of this component's own choosing. The size to draw at comes from
/// the compositor's configure, and how large the text should be comes from the
/// design tokens' `typography.scale` — this component does not get a third say.
/// The offscreen platform reports a device pixel ratio of one, which is what makes
/// the grab's size equal the configure's; a grab that disagrees is refused below.
pub const SCALE_FACTOR: f32 = 1.0;

/// The largest frame this component will draw, in bytes.
///
/// An 8K frame is about 132 MiB, so this leaves room for the widest display Inox
/// claims to support while still refusing a size nothing could have meant. It is a
/// statement about supported outputs, and **not** the bound on this process's
/// memory: drawing one frame costs several times these bytes, because Qt's
/// offscreen path allocates the window's backing store and the grabbed image while
/// the previous frame is still held, and a caller may be holding a few frames the
/// compositor has not released. What it refuses is a *single* frame larger than any
/// display this project claims, which is the case where "draw it anyway" would
/// allocate whatever two integers asked for.
pub const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

/// A lock surface's interface: an offscreen QML host, its size, and its last frame.
///
/// One per output, because each output has its own configured size and its own
/// scene to draw.
pub struct Interface {
    /// The C++ host: the QML engine and window this surface renders through.
    ///
    /// Not `Send`: Qt objects belong to the thread that made them, and the whole
    /// crate is single-threaded around its calloop loop.
    host: cxx::UniquePtr<Offscreen>,
    /// The surface's size in pixels, kept as the integers the configure delivered.
    ///
    /// Not as `f32`: `f32` cannot represent every `u32` exactly, so a dimension
    /// past 2^24 would round on the way in and the frame would be drawn with
    /// different dimensions from the ones the buffer is created for — a declared
    /// size larger than the memory that was written. The compositor's configure is
    /// the one number in this component that comes from outside, and it is kept
    /// exactly.
    width: u32,
    height: u32,
    /// The last frame drawn, in RGBA order.
    pixels: Vec<u8>,
}

impl Interface {
    /// An interface for a surface of `width` × `height`.
    ///
    /// `None` when the size is one this component will not draw (a zero-sized
    /// surface, or a frame past [`MAX_FRAME_BYTES`]), and — for the same reason —
    /// when the QML module cannot be loaded. Both leave the output on the
    /// compositor's own locked blank, which is a covering frame: a
    /// surface that cannot draw is not a hole in the lock.
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let (width, height) = frame_size(width, height)?;

        let mut host = qobject::new_offscreen();
        if host.is_null() {
            warn!(
                "the offscreen Qt host could not be created; this output keeps the compositor's blank"
            );
            return None;
        }

        let url = cxx_qt_lib::QString::from(QML_URL);
        let loaded = host
            .as_mut()
            .map(|host| host.load(&url, dimension(width), dimension(height)))
            .unwrap_or(false);

        if !loaded {
            warn!(
                "the lock screen's QML module did not load; this output keeps the compositor's blank"
            );
            return None;
        }

        Some(Self {
            host,
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

    /// Projects the state, hands the projection to QML, and grabs the frame.
    fn draw(&mut self, state: &State) {
        // The whole document is rebuilt every frame (contract §3). It is a handful
        // of strings and numbers, and the correctness of "the frame shows the state"
        // is worth more than the diffing a hundred fields would need.
        let document = view::screen_json(state.model(), state.tokens(), self.width, self.height);
        let json = cxx_qt_lib::QString::from(document.as_str());

        if let Some(host) = self.host.as_mut() {
            host.set_model(&json);
        } else {
            warn!("the offscreen host is gone; the frame on the surface is left alone");
            return;
        }

        let started = Instant::now();
        let frame = self
            .host
            .as_mut()
            .map(|host| host.render(dimension(self.width), dimension(self.height)))
            .unwrap_or_default();
        let bytes = frame.as_slice();
        let expected = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(4);

        if bytes.len() != expected {
            // A frame of the wrong length is a protocol error waiting to happen
            // (`wl_shm` would map the declared size over these bytes), so it is
            // discarded and the previous covering frame stays on the surface.
            warn!(
                expected,
                drawn = bytes.len(),
                width = self.width,
                height = self.height,
                "the offscreen QML host did not return a frame of the configured size; \
                 not committing it"
            );
            return;
        }

        self.pixels.clear();
        self.pixels.extend_from_slice(bytes);
        debug!(
            width = self.width,
            height = self.height,
            elapsed = ?started.elapsed(),
            "drew the lock screen through the offscreen QML host"
        );
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

/// A dimension as the `int` Qt takes, or `None` when it cannot be represented.
///
/// [`frame_size`] bounds the area, so every dimension that gets here is far below
/// `i32::MAX`; the conversion is still checked rather than cast, because a
/// dimension is data from outside this process.
fn dimension(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// The bytes of a `wl_shm` `Argb8888` frame, from an RGBA screenshot.
///
/// `wl_shm` names the *words* rather than the bytes, and on little-endian — every
/// architecture Inox supports — a word of `Argb8888` is `B, G, R, A` in memory.
/// `Argb8888` is also the *premultiplied* format, which is why the pixels are
/// grabbed as `QImage::Format_RGBA8888_Premultiplied` and copied without an
/// un-premultiply step. The compositor's own read-back tests assert on that order,
/// so this is the same translation `src/session.rs` used to do for its flat colour.
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

    #[test]
    fn a_size_nothing_could_have_meant_is_refused_rather_than_allocated() {
        assert!(frame_size(0, 240).is_none(), "a surface with no width");
        assert!(frame_size(320, 0).is_none(), "a surface with no height");
        assert!(
            frame_size(65_536, 65_536).is_none(),
            "16 GiB of frame is not a display"
        );
        assert!(
            frame_size(3840, 2160).is_some(),
            "and a real 4K output is not refused"
        );
        assert!(
            frame_size(7680, 4320).is_some(),
            "an 8K output is inside the bound this constant is about"
        );
    }

    #[test]
    fn a_dimension_f32_cannot_hold_exactly_is_kept_exactly() {
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

        let (kept, height) = frame_size(width, 1).expect("within the frame bound");

        assert_eq!(kept, width, "the configure is kept exactly, not rounded");
        assert_eq!(height, 1);
        assert_eq!(
            kept as usize * height as usize * 4,
            width as usize * 4,
            "and the frame is that many pixels, which is what the buffer declares"
        );
    }

    #[test]
    fn a_dimension_is_handed_to_qt_as_the_integer_it_is() {
        assert_eq!(dimension(1), 1);
        assert_eq!(dimension(3840), 3840);
        assert_eq!(
            dimension(u32::MAX),
            i32::MAX,
            "a dimension Qt cannot hold is clamped rather than wrapped"
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
        let rgba = [0x11u8, 0x22, 0x33, 0xff].repeat(64);
        let mut out = Vec::new();

        write_argb(&rgba, &mut out);

        assert_eq!(out.len(), rgba.len());
    }
}
