//! The panel's protocol leg: a real `wlr-layer-shell` surface, drawn from shared
//! memory and taking pointer input.
//!
//! Until this module existed the panel ran as an ordinary `xdg-shell` window on
//! iced's own backend: [`crate::placement`] computed anchors, an exclusive zone
//! and a keyboard-interactivity policy, and **nothing applied them** — the binary
//! logged exactly one warning saying so (ADR 0018). That is the one thing a panel
//! may not do. A panel that claims to reserve a strip while it is really a
//! floating window is a lie about the desktop's geometry, and the compositor
//! cannot keep windows out from under a strip it was never asked to keep free.
//!
//! So this leg is hand-written rather than delegated to `iced_layershell` (which
//! this crate's `AGENTS.md` forbids, and which does not compile against iced
//! 0.14's winit generation anyway). It is the same shape as `inox-lock`'s
//! `session.rs`, because that is this repository's proven protocol-leg pattern:
//! `Connection::connect_to_env`, one `WlRegistry` handler collecting the globals,
//! a `calloop` event loop with a `Generic` source over the connection, `wl_shm`
//! frames from `rustix::fs::memfd_create` with a bounded number in flight, and
//! `wl_buffer.release` bookkeeping. What it adds is what a panel needs and a lock
//! screen does not: the layer role, pointer input, and the frosted-glass request.
//!
//! # The placement is applied, not described
//!
//! One `zwlr_layer_surface_v1` is created for the surface `[panel] surface`
//! names, and [`crate::placement`] is the single source of truth for everything
//! it asks for: the anchors, the layer, the exclusive zone, the margin, the
//! keyboard interactivity (`None` — ordinary display must not take focus, D01)
//! and the size. Nothing here computes anchors; this module only turns that data
//! into protocol requests (`anchor_bits`, `layer_of`, `keyboard_of`).
//!
//! A surface anchored to both horizontal edges is *given* its width by the
//! compositor, so the panel asks for no width of its own and only names its
//! thickness. A surface anchored to one horizontal edge — the dock — has nothing
//! for the compositor to size it by, so the panel asks for the width of the row
//! it draws: `placement` derives it from the dock's geometry and the projection
//! this iteration is about to show — its tiles, the tray labels it will write and
//! the sentence about its own state. That width changes when any of them does,
//! and the request is re-sent then; the compositor answers with a new configure.
//!
//! # Frames
//!
//! A frame is [`crate::app::view`]'s widget tree, laid out at the configured size
//! and drawn by the same `tiny-skia` renderer the render tests use — there is no
//! second implementation of the interface here. The bytes go into an anonymous
//! shared-memory file and are attached with `wl_shm`'s **`Argb8888`**, written
//! premultiplied `B, G, R, A`.
//!
//! Both halves of that matter:
//!
//! - `Argb8888`, not `Xrgb8888`, because the menu bar and the dock are
//!   *translucent*: the pixels the view does not paint must stay transparent, or
//!   the compositor would blend an opaque square over the wallpaper. The frame is
//!   screenshotted over [`Color::TRANSPARENT`] for the same reason.
//! - Premultiplied, because that is what `iced_tiny_skia`'s `Headless::screenshot`
//!   returns: the tiny-skia pixmap is premultiplied RGBA, and `Argb8888` is the
//!   premultiplied-alpha format. Un-premultiplying (or writing the channels in
//!   screenshot order) would put a halo around every rounded corner and every
//!   glyph, which is exactly the artefact a glass pane shows most.
//!
//! A frame is drawn only when something has changed — a state change, a
//! configure, a pointer-driven interaction, or a glass parameter that needs the
//! next commit to take effect. There is no frame clock: the clock is the
//! [`app::Message::Tick`] the iced runtime used to deliver, driven by a
//! `calloop::timer::Timer` on [`crate::app::TICK`].
//!
//! # Pointer input
//!
//! `wl_seat` is bound, and when it advertises a pointer a `wl_pointer` is bound
//! and its enter / leave / motion / button events become iced events at the same
//! cursor position the render tests use. They are fed through the same
//! [`UserInterface::update`] the tests drive, the messages the widget tree
//! produces are applied with [`crate::app::update`], and the surface is redrawn
//! when iced asked for a redraw or the state moved. A click on a task tile
//! therefore produces the *same* [`crate::app::Message::TaskActivated`] the
//! render tests assert on; what the panel then does with it (queue it for a
//! transport that does not exist yet) is [`crate::app`]'s business, not this
//! module's.
//!
//! Coordinates arrive in surface-local coordinates, which are physical pixels of
//! the buffer; iced lays the interface out in logical pixels. The two are related
//! by the surface's scale factor, which the panel does not scale (it draws at
//! [`SCALE_FACTOR`]), so [`logical`] is the one place that division happens.
//!
//! # Frosted glass
//!
//! The glass object is created **after** the layer role exists, because the
//! protocol answers `max_radius = 0` for a surface with no layer role and a
//! client that asks earlier is told a maximum the compositor would not honour.
//! The decision itself (whether to ask at all, how the radius is clamped, what
//! the region is) lives in [`crate::frost`] as pure data with unit tests; this
//! module only carries it to the wire and degrades honestly when the answer is
//! "no": plain translucency, said once.
//!
//! # Lifecycle
//!
//! `SIGINT`/`SIGTERM` stop the loop and tear the objects down in order (glass,
//! role, surface, buffers). `SIGHUP` is deliberately **not** taken here: the
//! configuration reload host ([`crate::reload`]) owns it and hands its requests
//! over a channel, which the loop drains and answers. A connection error, a lost
//! global or a `closed` event stops the leg with a readable report; nothing here
//! exits the process mid-frame.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write as _};
use std::os::fd::AsFd;
use std::rc::Rc;
use std::time::Instant;

use calloop::{
    EventLoop, Interest, Mode, PostAction,
    generic::Generic,
    ping::make_ping,
    signals::{Signal, Signals},
    timer::{TimeoutAction, Timer},
};
use chrono::Local;
use iced_core::renderer::{Headless, Style};
use iced_core::{Color, Event, Font, Pixels, Point, Size, clipboard, mouse, window};
use iced_runtime::UserInterface;
use iced_runtime::user_interface::Cache;
use inox_common::i18n::Localizer;
use inox_config::{ConfigPaths, PanelSurface, ShellConfig, Theme as Tokens};
use inox_protocols::{
    glass::inox_glass::client::{
        inox_glass_manager_v1::InoxGlassManagerV1, inox_glass_v1 as glass_object,
    },
    standard::{
        core::client::{
            wl_buffer::{self, WlBuffer},
            wl_compositor::WlCompositor,
            wl_pointer::{self, WlPointer},
            wl_registry::{self, WlRegistry},
            wl_seat::{self, WlSeat},
            wl_shm::{self, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
        wlr::layer_shell::client::{
            zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
            zwlr_layer_surface_v1::{self as role, ZwlrLayerSurfaceV1},
        },
    },
};
use tracing::{debug, error, info, warn};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

use crate::app::{self, Message, OutgoingCommand, State};
use crate::dock::{DockGeometry, DockView};
use crate::placement::{self, Anchors, PanelPlacement};
use crate::view_model::PanelViewModel;
use crate::{frost, reload, theme};

/// The namespace the panel's surface claims.
///
/// The compositor uses it to tell shell surfaces apart (a screenshot tool or a
/// window rule can name it), and it is part of what makes the surface
/// recognisably *the panel* rather than an anonymous client.
pub const NAMESPACE: &str = "inox-panel";

/// The scale factor the panel draws its buffer at.
///
/// One, because the panel does not scale its buffer: the size comes from the
/// compositor's configure and the sizes inside the interface come from the
/// design tokens' `typography.scale`. A second scale factor here would be this
/// component inventing a third size. The surface-local coordinates `wl_pointer`
/// reports are therefore already logical, and [`logical`] divides by this value
/// so that the one place a future buffer scale would apply is written down.
pub const SCALE_FACTOR: f32 = 1.0;

/// How many frames may be in flight before the panel stops committing new ones.
///
/// A compositor releases a buffer when it replaces it on the surface, so the
/// steady state is one buffer being displayed and one being handed over — two.
/// This leaves room for two more so a release still in flight cannot stall a
/// redraw. Past this many the frame already on the surface is left there: it is
/// the panel one state behind, not a failure. The bound exists so a compositor
/// that never releases anything cannot have this process allocate a frame per
/// change for ever.
pub const MAX_FRAMES_IN_FLIGHT: usize = 4;

/// The largest frame this component will draw, in bytes.
///
/// A 8K frame is about 132 MiB, so this leaves room for the widest display Inox
/// claims to support while refusing a size nothing could have meant. It is a
/// statement about supported outputs, not the bound on this process's memory:
/// drawing a frame costs several times these bytes while an earlier frame is
/// still held.
pub const MAX_FRAME_BYTES: usize = 256 * 1024 * 1024;

/// What running the panel's protocol leg ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The process was asked to stop and the surface was torn down cleanly.
    Stopped,
    /// The compositor connection failed, a global the panel needs went away, or
    /// the compositor closed the layer surface.
    ConnectionLost,
}

/// Why the panel's layer surface could not be run at all.
#[derive(Debug, thiserror::Error)]
pub enum LegError {
    /// The display could not be reached.
    #[error("no Wayland display to place the panel on: {0}")]
    Connect(String),
    /// A protocol the panel's surface needs is not advertised.
    ///
    /// This is deliberately fatal. A panel that cannot be a layer surface has no
    /// placed surface to draw on, and running as an ordinary window would place
    /// it somewhere it never asked for — the lie ADR 0018 refuses.
    #[error(
        "the compositor does not offer {0}: the panel is a wlr-layer-shell surface and will \
         not run as an ordinary window whose placement is not applied (ADR 0018)"
    )]
    MissingGlobal(&'static str),
    /// The event loop could not be built or failed while running.
    #[error("the panel's event loop failed: {0}")]
    EventLoop(String),
    /// A shared-memory frame could not be built.
    #[error("failed to prepare a panel frame: {0}")]
    Frame(#[from] io::Error),
}

/// One pointer event, in surface-local logical coordinates.
///
/// [`Interface::pointer`] speaks this rather than `wl_pointer`'s events so that
/// the interface half is testable without a compositor, and so that the
/// translation from the wire is one small function ([`logical`],
/// [`pointer_button`]) with its own tests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pointer {
    /// The pointer entered the surface at this point.
    Enter(Point),
    /// The pointer moved to this point.
    Motion(Point),
    /// The pointer left the surface.
    Leave,
    /// A button was pressed, at the last position the pointer had.
    Press(mouse::Button),
    /// A button was released, at the last position the pointer had.
    Release(mouse::Button),
}

/// What feeding one pointer event produced.
#[derive(Debug, Clone)]
pub struct Produced {
    /// The messages the widget tree produced. [`Interface::pointer`] has already
    /// applied them with [`crate::app::update`]; they are returned so a caller
    /// (and a test) can see what the interaction meant.
    pub messages: Vec<Message>,
    /// Whether the interface asked for a redraw — a hover or a press that has to
    /// be visible while it is held (`docs/quality/ui-guidelines.md` §5).
    pub repaint: bool,
}

/// The panel's interface on its layer surface: the widget tree, drawn offscreen.
///
/// One per surface, because the surface has one configured size and one widget
/// state. The size is kept as the integers the configure delivered, exactly: an
/// `f32` cannot represent every `u32`, and a frame drawn at the rounded size
/// would be handed to a buffer created for the real one.
pub struct Interface {
    renderer: iced_tiny_skia::Renderer,
    /// The widget tree carried between frames, so iced matches the new tree
    /// against the old one instead of treating every frame as a new screen.
    cache: Option<Cache>,
    width: u32,
    height: u32,
    /// The last frame drawn, in RGBA order (the screenshot's order, not the
    /// buffer's).
    pixels: Vec<u8>,
    /// Where the pointer is, as iced remembers it. `Press` and `Release` carry
    /// no position of their own, so the last motion is what they happen at.
    cursor: mouse::Cursor,
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
            cursor: mouse::Cursor::Unavailable,
        })
    }

    /// The size this interface draws at, in pixels.
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
    /// draw leaves the interface as it was.
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

    /// Draws the panel into `out` as the byte order `wl_shm` wants.
    pub fn render_argb(&mut self, state: &State, out: &mut Vec<u8>) {
        self.draw(state);
        write_argb(&self.pixels, out);
    }

    /// The last frame drawn, in RGBA order, and empty before the first draw.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Feeds one pointer event to the interface and applies what it produced.
    ///
    /// Returns the messages the widget tree produced and whether the interface
    /// asked for a redraw. The messages are applied with [`crate::app::update`]
    /// here, so the caller has exactly one way to learn what an interaction
    /// meant, and the `Task` the update returns is dropped — the panel returns
    /// `Task::none()` (see [`crate::app::update`]) and the leg owns the clock.
    pub fn pointer(&mut self, state: &mut State, event: Pointer) -> Produced {
        let (events, cursor) = match event {
            Pointer::Enter(point) => (
                vec![Event::Mouse(mouse::Event::CursorMoved { position: point })],
                mouse::Cursor::Available(point),
            ),
            Pointer::Motion(point) => (
                vec![Event::Mouse(mouse::Event::CursorMoved { position: point })],
                mouse::Cursor::Available(point),
            ),
            Pointer::Leave => (
                vec![Event::Mouse(mouse::Event::CursorLeft)],
                mouse::Cursor::Unavailable,
            ),
            Pointer::Press(button) => (
                vec![Event::Mouse(mouse::Event::ButtonPressed(button))],
                self.cursor,
            ),
            Pointer::Release(button) => (
                vec![Event::Mouse(mouse::Event::ButtonReleased(button))],
                self.cursor,
            ),
        };

        self.cursor = cursor;
        let (messages, repaint) = self.produce(state, &events, cursor);

        for message in &messages {
            let _ = app::update(state, message.clone());
        }

        Produced { messages, repaint }
    }

    /// Lays the widget tree out, handles `events` and draws the panel.
    fn draw(&mut self, state: &State) {
        let theme = theme::from_tokens(state.tokens());
        let redraw = Event::Window(window::Event::RedrawRequested(Instant::now()));
        let (produced, _) = self.produce(state, &[redraw], self.cursor);

        // A redraw produces no messages in this crate — nothing here reacts to
        // being drawn — so this is a diagnostic rather than a queue something
        // drains. It is logged rather than dropped silently because a widget that
        // started producing one would be a widget whose state this module is
        // quietly discarding.
        if !produced.is_empty() {
            debug!(
                messages = produced.len(),
                "drawing the panel produced messages, which the redraw path does not consume"
            );
        }

        let cache = self.cache.take().unwrap_or_default();
        let mut ui = UserInterface::build(app::view(state), self.size(), cache, &mut self.renderer);
        ui.draw(&mut self.renderer, &theme, &Style::default(), self.cursor);
        self.cache = Some(ui.into_cache());

        // Composited over nothing: the pane is translucent, so a screenshot over
        // an opaque colour would make every unpainted pixel a colour the
        // compositor then blends as if the panel had painted it.
        let mut frame = Headless::screenshot(
            &mut self.renderer,
            Size::new(self.width, self.height),
            SCALE_FACTOR,
            Color::TRANSPARENT,
        );

        std::mem::swap(&mut self.pixels, &mut frame);
    }

    /// Feeds `events` to a freshly built interface and returns what it produced.
    ///
    /// The interface is rebuilt from the kept cache on every call, which is what
    /// makes the widget tree see one continuous interaction rather than a new
    /// screen per event.
    fn produce(
        &mut self,
        state: &State,
        events: &[Event],
        cursor: mouse::Cursor,
    ) -> (Vec<Message>, bool) {
        let cache = self.cache.take().unwrap_or_default();
        let mut messages = Vec::new();
        let redraw = {
            let mut ui =
                UserInterface::build(app::view(state), self.size(), cache, &mut self.renderer);
            let (status, _) = ui.update(
                events,
                cursor,
                &mut self.renderer,
                &mut clipboard::Null,
                &mut messages,
            );
            let requested = match status {
                iced_runtime::user_interface::State::Updated { redraw_request, .. } => {
                    !matches!(redraw_request, window::RedrawRequest::Wait)
                }
                // The tree decided its layout is out of date; the next draw is
                // the one that will rebuild it, so it has to happen.
                iced_runtime::user_interface::State::Outdated => true,
            };

            self.cache = Some(ui.into_cache());

            requested
        };

        (messages, redraw)
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

/// The bytes of a `wl_shm` `Argb8888` frame, from a premultiplied RGBA screenshot.
///
/// `wl_shm` names the *words*: on little-endian — every architecture Inox
/// supports — a word of `Argb8888` is `B, G, R, A` in memory, and its alpha is
/// premultiplied, which is exactly what `iced_tiny_skia`'s screenshot already
/// holds. Swapping the channels without un-premultiplying them is what makes a
/// translucent pane show a halo, which is why the frame is translated here rather
/// than written straight out.
fn write_argb(rgba: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(rgba.len());

    for pixel in rgba.as_chunks::<4>().0 {
        out.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
}

/// The text size the renderer starts from, before any widget asks for its own.
fn default_text_size() -> Pixels {
    iced::Settings::default().default_text_size
}

/// A surface-local logical coordinate for a protocol coordinate.
///
/// `wl_pointer` reports coordinates in surface-local physical pixels and iced
/// lays the interface out in logical pixels; the surface's scale factor is what
/// relates them. The panel does not scale its buffer ([`SCALE_FACTOR`] is one),
/// but dividing here is what keeps a future buffer scale a one-line change
/// instead of a hunt.
pub fn logical(pixels: f64, scale: f32) -> f32 {
    if scale > 0.0 {
        (pixels / f64::from(scale)) as f32
    } else {
        pixels as f32
    }
}

/// The iced button for a Linux input event code, when the panel acts on it.
///
/// The codes are the `BTN_*` values `wl_pointer.button` carries
/// (`linux/input-event-codes.h`); a button the panel has no meaning for is not an
/// error and not a panicked `unreachable` — it is simply not a button the panel
/// reacts to.
pub const fn pointer_button(code: u32) -> Option<mouse::Button> {
    match code {
        0x110 => Some(mouse::Button::Left),
        0x111 => Some(mouse::Button::Right),
        0x112 => Some(mouse::Button::Middle),
        0x113 => Some(mouse::Button::Back),
        0x114 => Some(mouse::Button::Forward),
        _ => None,
    }
}

/// The protocol anchors for a placement's anchor set.
fn anchor_bits(anchors: &Anchors) -> role::Anchor {
    let mut bits = role::Anchor::empty();

    for anchor in anchors.iter() {
        bits |= match anchor {
            placement::Anchor::Top => role::Anchor::Top,
            placement::Anchor::Bottom => role::Anchor::Bottom,
            placement::Anchor::Left => role::Anchor::Left,
            placement::Anchor::Right => role::Anchor::Right,
        };
    }

    bits
}

/// The protocol layer for a placement's layer.
const fn layer_of(layer: placement::Layer) -> zwlr_layer_shell_v1::Layer {
    match layer {
        placement::Layer::Background => zwlr_layer_shell_v1::Layer::Background,
        placement::Layer::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
        placement::Layer::Top => zwlr_layer_shell_v1::Layer::Top,
        placement::Layer::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
    }
}

/// The protocol keyboard interactivity for a placement's policy.
const fn keyboard_of(
    interactivity: placement::KeyboardInteractivity,
) -> role::KeyboardInteractivity {
    match interactivity {
        placement::KeyboardInteractivity::None => role::KeyboardInteractivity::None,
        placement::KeyboardInteractivity::OnDemand => role::KeyboardInteractivity::OnDemand,
        placement::KeyboardInteractivity::Exclusive => role::KeyboardInteractivity::Exclusive,
    }
}

/// One frame committed to the surface, and whether the compositor is done with it.
struct Frame {
    /// The buffer attached to the surface.
    buffer: WlBuffer,
    /// Whether `wl_buffer.release` has arrived for it. A buffer the compositor
    /// may still be reading must not be freed; one it has released may be.
    released: bool,
}

/// An anonymous shared-memory file for one frame.
fn frame_memory(name: &str) -> io::Result<std::os::fd::OwnedFd> {
    rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::CLOEXEC).map_err(io::Error::from)
}

/// The globals this leg binds, in the order they are required.
const REQUIRED_GLOBALS: [&str; 3] = ["wl_compositor", "wl_shm", "zwlr_layer_shell_v1"];

/// Everything one connection to the compositor holds.
struct Client {
    /// Every global the registry reported, by interface name.
    globals: HashMap<String, (u32, u32)>,
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    seat: Option<WlSeat>,
    pointer: Option<WlPointer>,
    layer_shell: Option<ZwlrLayerShellV1>,
    glass_manager: Option<InoxGlassManagerV1>,
    surface: Option<WlSurface>,
    role: Option<ZwlrLayerSurfaceV1>,
    glass: Option<glass_object::InoxGlassV1>,
    /// The largest radius the compositor reported for this surface, once the
    /// `capabilities` event arrived.
    max_radius: Option<u32>,
    /// The glass request the surface is currently carrying; `None` is frosting
    /// off.
    glass_request: Option<frost::Request>,
    /// Whether the "the compositor will not blur" degradation has been said.
    degraded_reported: bool,
    /// Whether the "no inox-glass-v1" degradation has been said.
    no_glass_reported: bool,
    /// Whether the "no pointer on this seat" degradation has been said.
    no_pointer_reported: bool,
    /// Whether the "the panel's requests have nowhere to go" note has been said.
    no_transport_reported: bool,
    /// The interface drawn on the surface, once it has been configured.
    interface: Option<Interface>,
    /// The size the compositor last configured this surface to.
    configured: Option<(u32, u32)>,
    /// Whether a placement change has been committed and its configure is
    /// outstanding. Frames are not attached until it arrives: the compositor has
    /// not yet said what size it wants.
    awaiting_configure: bool,
    /// The placement the surface is carrying, so a reload or a growing task list
    /// is noticed only when it really changes something.
    applied_placement: Option<PanelPlacement>,
    /// The frames committed to this surface the compositor has not released.
    frames: Vec<Frame>,
    /// The panel itself: the state every frame is drawn from and the queue of
    /// requests the state wants carried to a service.
    state: State,
    /// The configuration reload host: `SIGHUP` and `org.inox.Panel` hand it
    /// requests, and this loop is what answers them.
    reload: reload::Host,
    /// The projection that is on the surface right now, so an unchanged state
    /// does not commit a frame.
    drawn: Option<crate::view_model::PanelViewModel>,
    /// Whether something has happened that the surface still has to show.
    pending_repaint: bool,
    stopped: bool,
    connection_lost: bool,
    /// Frames committed, which is the observable proof that the panel drew.
    committed: u64,
}

impl Dispatch<WlRegistry, ()> for Client {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                state.globals.insert(interface.clone(), (name, version));

                match interface.as_str() {
                    "wl_compositor" => {
                        state.compositor = Some(registry.bind(name, version.min(5), qh, ()));
                    }
                    "wl_shm" => {
                        state.shm = Some(registry.bind(name, version.min(1), qh, ()));
                    }
                    "wl_seat" => {
                        state.seat = Some(registry.bind(name, version.min(5), qh, ()));
                    }
                    "zwlr_layer_shell_v1" => {
                        state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                    }
                    // Optional by design: without it the panel draws the plain
                    // translucent material it already has (see `crate::frost`).
                    "inox_glass_manager_v1" => {
                        state.glass_manager = Some(registry.bind(name, version.min(1), qh, ()));
                    }
                    _ => {}
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                state.global_removed(name);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for Client {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            let pointer = matches!(
                capabilities,
                wayland_client::WEnum::Value(capabilities) if capabilities.contains(wl_seat::Capability::Pointer)
            );

            if pointer {
                if state.pointer.is_none() {
                    state.pointer = Some(seat.get_pointer(qh, ()));
                    debug!("bound the seat's pointer");
                }
            } else if !state.no_pointer_reported {
                // A seat without a pointer is a real state (a keyboard-only
                // kiosk, a seat whose devices were removed): the panel keeps
                // drawing, it just cannot be clicked.
                state.no_pointer_reported = true;
                info!(
                    "this seat advertises no pointer; the panel keeps drawing and takes no clicks"
                );
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for Client {
    fn event(
        state: &mut Self,
        pointer: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The events are translated first, then fed to the interface, because the
        // two borrow different halves of `Client`.
        let input = match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                // The protocol's own idiom for "use the compositor's default
                // cursor": a null surface. Not hiding the cursor over the panel
                // would leave a user with no pointer at all.
                pointer.set_cursor(serial, None, 0, 0);

                if state.surface.as_ref() != Some(&surface) {
                    return;
                }

                Pointer::Enter(Point::new(
                    logical(surface_x, SCALE_FACTOR),
                    logical(surface_y, SCALE_FACTOR),
                ))
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => Pointer::Motion(Point::new(
                logical(surface_x, SCALE_FACTOR),
                logical(surface_y, SCALE_FACTOR),
            )),
            wl_pointer::Event::Leave { surface, .. } => {
                if state.surface.as_ref() != Some(&surface) {
                    return;
                }

                Pointer::Leave
            }
            wl_pointer::Event::Button {
                button,
                state: button_state,
                ..
            } => {
                let Some(button) = pointer_button(button) else {
                    return;
                };

                match button_state {
                    wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed) => {
                        Pointer::Press(button)
                    }
                    wayland_client::WEnum::Value(wl_pointer::ButtonState::Released) => {
                        Pointer::Release(button)
                    }
                    _ => return,
                }
            }
            _ => return,
        };

        if let Produced { repaint, .. } = state.feed(input)
            && repaint
        {
            state.pending_repaint = true;
        }
    }
}

impl Dispatch<WlBuffer, ()> for Client {
    fn event(
        state: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            // The compositor has finished with this frame. It is not destroyed
            // here — destroying a proxy inside its own event handler is the kind
            // of thing that reads as a use-after-free later — it is marked, and
            // the next commit destroys it.
            for frame in &mut state.frames {
                if &frame.buffer == buffer {
                    frame.released = true;
                }
            }

            // A release is what unblocks a commit that hit the in-flight bound,
            // so it is also a reason to try again.
            state.pending_repaint = true;
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Client {
    fn event(
        state: &mut Self,
        role: &ZwlrLayerSurfaceV1,
        event: role::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            role::Event::Configure {
                serial,
                width,
                height,
            } => {
                // The protocol makes acking the configure a precondition for the
                // commit that answers it, and the committed size an exact
                // requirement.
                role.ack_configure(serial);

                state.configured = Some((width, height));
                state.awaiting_configure = false;
                state.drawn = None;
                state.pending_repaint = true;

                // The size and the interface are set together, because a frame's
                // *pixels* come from the interface and its *dimensions* come from
                // here. Letting them disagree would commit a buffer declaring one
                // size and holding another size's pixels.
                let refused = match state.interface.as_mut() {
                    Some(interface) => {
                        interface.resize(width, height);

                        interface.dimensions() != (width, height)
                    }
                    None => {
                        state.interface = Interface::new(width, height);

                        state.interface.is_none()
                    }
                };

                if refused {
                    state.interface = None;
                    state.configured = None;
                    warn!(
                        width,
                        height,
                        "this is not a size the panel will draw; the surface is left as the \
                         compositor configured it"
                    );
                    return;
                }

                // The layer role exists now, which is the precondition the glass
                // protocol names: a compositor answers `max_radius = 0` for a
                // surface with no layer role, and this client must not ask before
                // the role is there.
                state.create_glass(qh);
                state.sync_glass();

                debug!(
                    width,
                    height, "the compositor configured the panel's surface"
                );
            }
            role::Event::Closed => {
                // The compositor took the surface away: this panel has nowhere
                // left to draw, and staying alive would be a process pretending
                // to be a panel.
                warn!("the compositor closed the panel's layer surface");
                state.connection_lost = true;
                state.stopped = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<glass_object::InoxGlassV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &glass_object::InoxGlassV1,
        event: glass_object::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let glass_object::Event::Capabilities { max_radius } = event {
            state.max_radius = Some(max_radius);
            state.sync_glass();

            if max_radius > 0 {
                debug!(
                    max_radius,
                    "the compositor will frost the panel's backdrop up to this radius"
                );
            }
        }
    }
}

impl Client {
    /// Feeds one pointer event to the interface.
    fn feed(&mut self, input: Pointer) -> Produced {
        let Self {
            interface, state, ..
        } = self;

        let Some(interface) = interface.as_mut() else {
            return Produced {
                messages: Vec::new(),
                repaint: false,
            };
        };

        interface.pointer(state, input)
    }

    /// Notes that a global went away, and stops when it is one the surface needs.
    fn global_removed(&mut self, name: u32) {
        let removed = self
            .globals
            .iter()
            .find(|(_, (global, _))| *global == name)
            .map(|(interface, _)| interface.clone());

        let Some(interface) = removed else {
            return;
        };

        self.globals.remove(&interface);

        match interface.as_str() {
            "wl_compositor" | "wl_shm" | "zwlr_layer_shell_v1" => {
                warn!(
                    interface = %interface,
                    "a protocol the panel's surface needs went away; stopping"
                );
                self.connection_lost = true;
                self.stopped = true;
            }
            "inox_glass_manager_v1" => {
                // The effect was already asked for; the objects it created stay
                // valid (the protocol says so), so what stops is only the ability
                // to ask for a new one. Nothing to do but say so.
                info!("the compositor stopped offering inox-glass-v1; the pane keeps its tint");
            }
            "wl_seat" => {
                info!("the seat went away; the panel keeps drawing and takes no clicks");
                self.seat = None;
                self.pointer = None;
            }
            _ => debug!(interface = %interface, "a global went away"),
        }
    }

    /// The placement the configured surface asks for, as data.
    ///
    /// `model` is the projection this iteration is about to draw, taken by
    /// reference rather than re-projected here so that one loop iteration builds
    /// it once. A dock is sized from it — its tiles, its tray labels and its own
    /// sentence, through [`DockGeometry::content`] — because the surface has to
    /// ask for a width that covers everything it will draw.
    fn placement(&self, model: &PanelViewModel) -> PanelPlacement {
        let shell = self.state.shell();

        match shell.panel.surface {
            PanelSurface::Taskbar => PanelPlacement::derive(&shell.panel),
            PanelSurface::Menubar => PanelPlacement::menubar(&shell.menubar),
            PanelSurface::Dock => {
                let geometry = DockGeometry::new(self.state.tokens(), &shell.dock);
                let view = DockView::project(
                    model,
                    &shell.dock,
                    shell.panel.max_task_buttons,
                    self.state.localizer(),
                );

                PanelPlacement::dock(&shell.dock, self.state.tokens(), geometry.content(&view))
            }
        }
    }

    /// The size the surface asks for, in surface-local logical pixels.
    ///
    /// A surface anchored to both horizontal edges is given the output's width by
    /// the compositor, so it asks for no width of its own and only names its
    /// thickness. A surface anchored to one horizontal edge has nothing for the
    /// compositor to size it by, so it asks for its own content width.
    fn surface_size(&self, placement: &PanelPlacement) -> (u32, u32) {
        let height = u32::from(placement.height);

        match placement.width {
            Some(width) => (u32::from(width), height),
            None => (0, height),
        }
    }

    /// Creates the layer surface and asks for the placement it should carry.
    fn create_surface(&mut self, qh: &QueueHandle<Self>) -> Result<(), LegError> {
        let compositor = self
            .compositor
            .clone()
            .ok_or(LegError::MissingGlobal("wl_compositor"))?;
        let shell = self
            .layer_shell
            .clone()
            .ok_or(LegError::MissingGlobal("zwlr_layer_shell_v1"))?;

        let model = self.state.model();
        let placement = self.placement(&model);
        let surface = compositor.create_surface(qh, ());
        let role = shell.get_layer_surface(
            &surface,
            None,
            layer_of(placement.layer),
            NAMESPACE.to_owned(),
            qh,
            (),
        );

        self.surface = Some(surface);
        self.role = Some(role);

        info!(
            surface = ?self.state.surface(),
            namespace = NAMESPACE,
            anchors = ?placement.anchors.iter().collect::<Vec<_>>(),
            exclusive_zone = placement.exclusive_zone,
            margin = placement.margin,
            layer = ?placement.layer,
            keyboard = ?placement.keyboard_interactivity,
            "created the panel's wlr-layer-shell surface"
        );

        self.apply_placement(&placement);

        Ok(())
    }

    /// Asks the compositor to carry `placement`, and commits it.
    ///
    /// The commit is the one the protocol requires after the role exists and
    /// before the first buffer: it carries no buffer of its own, it applies the
    /// double-buffered surface state, and the compositor answers with a
    /// `configure` this client waits for.
    fn apply_placement(&mut self, placement: &PanelPlacement) {
        let (Some(role), Some(surface)) = (self.role.as_ref(), self.surface.as_ref()) else {
            return;
        };

        let (width, height) = self.surface_size(placement);
        let margin = i32::from(placement.margin);

        role.set_anchor(anchor_bits(&placement.anchors));
        role.set_exclusive_zone(i32::try_from(placement.exclusive_zone).unwrap_or(i32::MAX));
        // The margin applies to the anchored edge; the protocol says setting it
        // for the others has no effect, so all four carry it and this code never
        // has to know which edge is which.
        role.set_margin(margin, margin, margin, margin);
        role.set_keyboard_interactivity(keyboard_of(placement.keyboard_interactivity));
        role.set_size(width, height);
        surface.commit();

        self.applied_placement = Some(placement.clone());
        self.awaiting_configure = true;
    }

    /// Asks for a new placement when a reload or a growing task list made one.
    fn sync_placement(&mut self, model: &PanelViewModel) {
        let placement = self.placement(model);

        if self.applied_placement.as_ref() == Some(&placement) {
            return;
        }

        debug!(
            width = ?placement.width,
            height = placement.height,
            "the panel's placement changed; asking the compositor for it"
        );
        self.apply_placement(&placement);
    }

    /// Creates the glass object, if this surface is a pane that wants one.
    ///
    /// Called once the layer role exists. `glass.blur == 0` creates nothing at
    /// all; a compositor without `inox-glass-v1` is a degradation the panel says
    /// once and draws through.
    fn create_glass(&mut self, qh: &QueueHandle<Self>) {
        if self.glass.is_some() {
            return;
        }

        let blur = theme::glass_blur(self.state.tokens());

        if !frost::asks_for_glass(self.state.surface(), blur) {
            return;
        }

        let Some(manager) = self.glass_manager.clone() else {
            if !self.no_glass_reported {
                self.no_glass_reported = true;
                info!(
                    "the compositor does not offer inox-glass-v1; the pane is drawn as plain \
                     translucency"
                );
            }

            return;
        };

        let Some(surface) = self.surface.clone() else {
            return;
        };

        self.glass = Some(manager.get_glass(&surface, qh, ()));
        debug!("created the panel's inox-glass-v1 object");
    }

    /// Carries the glass decision to the wire, and re-sends it when it changes.
    ///
    /// The decision itself (the clamp, the region, the reasons to draw plain
    /// translucency) is [`crate::frost`]; this is only the transport. Nothing is
    /// sent before the compositor has answered `capabilities`: the maximum radius
    /// is part of the request.
    fn sync_glass(&mut self) {
        let Some(glass) = self.glass.as_ref() else {
            return;
        };

        let Some(max_radius) = self.max_radius else {
            return;
        };

        let blur = theme::glass_blur(self.state.tokens());
        let size = self.configured.unwrap_or((0, 0));

        let wanted = match frost::decide(self.state.surface(), blur, max_radius, size) {
            frost::Material::Frosted(request) => Some(request),
            frost::Material::Plain(reason) => {
                if reason == frost::Plain::CompositorCannotBlur && !self.degraded_reported {
                    self.degraded_reported = true;
                    // The one line the brief of this change asks for: a real
                    // degradation, named once, never a claimed blur.
                    info!(
                        "the compositor will not frost this surface's backdrop \
                         (inox-glass-v1 max_radius = 0); the pane is drawn as plain translucency"
                    );
                }

                None
            }
        };

        if wanted == self.glass_request {
            return;
        }

        match &wanted {
            Some(request) => {
                glass.set_radius(request.radius);
                glass.set_region(
                    request.region.x,
                    request.region.y,
                    request.region.width,
                    request.region.height,
                );
                debug!(
                    radius = request.radius,
                    region = ?request.region,
                    "asked the compositor to frost the panel's pane"
                );
            }
            None => {
                // Turning the effect off: a radius of 0 and a region with no area
                // both mean "nothing to frost", and neither destroys the object,
                // so a later `glass.blur` change does not have to recreate it.
                glass.set_radius(0);
                glass.set_region(0, 0, 0, 0);
                debug!("turned the panel's frosting off");
            }
        }

        self.glass_request = wanted;

        // The protocol applies the parameters "from the next committed frame on",
        // so the change is not visible until one is committed — and the pane's
        // pixels do not change with it, which is exactly why the repaint has to be
        // requested here rather than left to the state comparison.
        self.pending_repaint = true;
    }

    /// Draws and commits a frame when the surface has something new to show.
    ///
    /// `model` is this iteration's projection, built once by the loop: every
    /// reason to draw is a difference between it and the projection the surface
    /// already carries.
    fn repaint(&mut self, qh: &QueueHandle<Self>, model: crate::view_model::PanelViewModel) {
        if self.awaiting_configure || self.configured.is_none() || self.interface.is_none() {
            return;
        }

        if self.drawn.as_ref() == Some(&model) && !self.pending_repaint {
            return;
        }

        match self.commit_frame(qh) {
            Ok(true) => {
                self.drawn = Some(model);
                self.pending_repaint = false;
            }
            Ok(false) => {
                // A frame was refused — the in-flight bound, or a size that does
                // not match the configure. Asking again on the next wake-up is
                // what keeps the surface from staying a state behind for ever.
                self.pending_repaint = true;
            }
            Err(error) => {
                error!(%error, "failed to draw the panel's frame");
                self.pending_repaint = true;
            }
        }
    }

    /// Draws the panel onto the surface and commits it.
    fn commit_frame(&mut self, qh: &QueueHandle<Self>) -> Result<bool, LegError> {
        let Some(shm) = self.shm.clone() else {
            return Err(LegError::MissingGlobal("wl_shm"));
        };

        let Some(surface) = self.surface.clone() else {
            return Ok(false);
        };

        let Some((width, height)) = self.configured else {
            return Ok(false);
        };

        let Some(interface) = self.interface.as_mut() else {
            return Ok(false);
        };

        // The two are set together by the configure handler; if they ever came
        // apart the frame would declare one size and hold another size's pixels.
        if interface.dimensions() != (width, height) {
            warn!(
                configured = ?(width, height),
                drawn = ?interface.dimensions(),
                "the interface is not the size the surface was configured to; not committing a \
                 frame of the wrong size"
            );

            return Ok(false);
        }

        // Buffers the compositor has finished with are the frames it may still be
        // reading; past the bound, the frame already on the surface is left there.
        let in_flight = self.frames.iter().filter(|frame| !frame.released).count();

        if in_flight >= MAX_FRAMES_IN_FLIGHT {
            debug!(
                in_flight,
                "the compositor has not released the frames already committed; leaving the \
                 frame that is there"
            );

            return Ok(false);
        }

        let stride = width.saturating_mul(4);
        let size = stride.saturating_mul(height);
        let (Ok(stride), Ok(size)) = (i32::try_from(stride), i32::try_from(size)) else {
            return Ok(false);
        };

        let mut bytes = Vec::new();
        interface.render_argb(&self.state, &mut bytes);

        let mut file = File::from(frame_memory("inox-panel-frame")?);
        file.write_all(&bytes)?;

        let pool: WlShmPool = shm.create_pool(file.as_fd(), size, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );
        // The buffer holds its own reference to the memory, so the pool has done
        // its job.
        pool.destroy();

        surface.attach(Some(&buffer), 0, 0);

        // `damage_buffer` needs wl_surface 4; older compositors only have the
        // surface-local `damage`, which is the same rectangle here because the
        // panel draws at scale 1 with no transform and no viewport.
        if surface.version() >= 4 {
            surface.damage_buffer(0, 0, width as i32, height as i32);
        } else {
            surface.damage(0, 0, width as i32, height as i32);
        }

        surface.commit();

        // A frame the compositor has released is asked to go, not merely dropped:
        // dropping a `wayland-client` proxy sends nothing, and the compositor
        // keeps a shared-memory mapping plus the pool behind every buffer until
        // `wl_buffer.destroy` or this client's connection ends.
        let (released, kept): (Vec<Frame>, Vec<Frame>) =
            self.frames.drain(..).partition(|frame| frame.released);

        for frame in &released {
            frame.buffer.destroy();
        }

        self.frames = kept;
        self.frames.push(Frame {
            buffer,
            released: false,
        });

        self.committed = self.committed.saturating_add(1);

        Ok(true)
    }

    /// Carries out what the panel asked of the outside world.
    ///
    /// The window service transport does not exist in this milestone, so nothing
    /// can carry a request anywhere. Dropping them is not the same as forgetting
    /// them: the state keeps each request in flight until a service answers, so
    /// the panel does not send a second request for a window it already asked
    /// about. The first dropped request is named once, because an operator
    /// reading the log should not have to guess why a click did nothing.
    fn pump(&mut self) {
        for command in self.state.take_outgoing() {
            if !self.no_transport_reported {
                self.no_transport_reported = true;
                info!(
                    "no window service transport is implemented in this milestone; the panel's \
                     requests are recorded and go nowhere"
                );
            }

            match command {
                OutgoingCommand::Snapshot => {
                    debug!("the panel asked for the window service's snapshot")
                }
                OutgoingCommand::Activate { request, window } => debug!(
                    request = request.get(),
                    %window,
                    "the panel asked to focus a window"
                ),
                OutgoingCommand::Toggle { request, window } => debug!(
                    request = request.get(),
                    %window,
                    "the panel asked to put a window away"
                ),
            }
        }
    }

    /// Takes every configuration reload the bus or `SIGHUP` prepared.
    fn drain_reloads(&mut self) {
        for request in self.reload.take_requests() {
            let result = reload::apply(&mut self.state, &request);
            let applied = result.is_ok();

            request.finish(result);

            if applied {
                info!("applied a configuration reload");
                self.drawn = None;
                self.pending_repaint = true;
            }
        }
    }

    /// Feeds the clock the iced runtime used to feed.
    fn tick(&mut self) {
        let before = self.state.model();
        // The `Task` is dropped: this crate's `update` returns `Task::none()`
        // (there is no asynchronous work behind a tick), and the leg owns the
        // clock — the `calloop` timer is what produces the next one.
        let _ = app::update(&mut self.state, Message::Tick(Local::now()));

        if self.state.model() != before {
            self.pending_repaint = true;
        }
    }

    /// Destroys everything this client holds, in the order the protocol wants.
    ///
    /// The glass object goes first (it describes an effect on the surface), then
    /// the role, then the surface, and finally the buffers the compositor may
    /// still be reading: this client is saying it is finished with all of them.
    fn teardown(&mut self) {
        if let Some(glass) = self.glass.take() {
            glass.destroy();
        }

        if let Some(role) = self.role.take() {
            role.destroy();
        }

        if let Some(surface) = self.surface.take() {
            surface.destroy();
        }

        for frame in self.frames.drain(..) {
            frame.buffer.destroy();
        }
    }

    /// The outcome the loop ended in.
    fn outcome(&self) -> Outcome {
        if self.connection_lost {
            Outcome::ConnectionLost
        } else {
            Outcome::Stopped
        }
    }
}

// The interfaces this client only consumes, or does not consume at all.
wayland_client::delegate_noop!(Client: ignore WlCompositor);
wayland_client::delegate_noop!(Client: ignore WlSurface);
wayland_client::delegate_noop!(Client: ignore WlShm);
wayland_client::delegate_noop!(Client: ignore WlShmPool);
wayland_client::delegate_noop!(Client: ignore ZwlrLayerShellV1);
wayland_client::delegate_noop!(Client: ignore InoxGlassManagerV1);

/// Run the panel as a real layer surface until the process is asked to stop.
///
/// This is the whole run path of the binary: it connects to the session's
/// Wayland display, binds what the surface needs, creates the layer surface from
/// [`crate::placement`]'s data, draws [`crate::app::State`] onto it, routes
/// pointer input into the same state, asks for frosted glass when the tokens want
/// it, and serves configuration reloads.
///
/// # Errors
///
/// Returns [`LegError`] when the display cannot be reached, when one of
/// `wl_compositor`, `wl_shm` or `zwlr_layer_shell_v1` is not advertised, or when
/// the event loop cannot be built. Every one of those is a state the caller must
/// report rather than paper over: a panel that cannot be placed must not pretend
/// to be.
pub fn run(
    shell: ShellConfig,
    tokens: Tokens,
    localizer: Rc<Localizer>,
    paths: ConfigPaths,
) -> Result<Outcome, LegError> {
    let connection =
        Connection::connect_to_env().map_err(|error| LegError::Connect(error.to_string()))?;
    let mut queue = connection.new_event_queue::<Client>();
    let qh = queue.handle();
    // The registry object is kept alive by the connection; every bind goes
    // through it from the dispatch handler.
    let _registry = connection.display().get_registry(&qh, ());

    // Every signal is taken **first**, before anything spawns a thread. A
    // `calloop` signal source blocks the signal for the thread it runs on, and a
    // thread created later inherits that mask — but a thread created earlier does
    // not, and a signal delivered to a thread that does not block it has its
    // default action: `SIGTERM` kills the process before it can tear the surface
    // down, and a `SIGHUP` meant for a reload would do the same instead of
    // reaching the `signalfd`.
    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP])
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    // The reload host is what carries `SIGHUP` and `org.inox.Panel` into this
    // process, and its `Ping` is what wakes this loop when it has something: a
    // `calloop` loop blocks until a source is ready, so a request that only
    // arrived in a channel would otherwise wait for the next pointer or clock
    // event.
    let (wake, wake_source) =
        make_ping().map_err(|error| LegError::EventLoop(error.to_string()))?;
    let reload = reload::Host::start(paths, wake);

    let mut client = Client {
        globals: HashMap::new(),
        compositor: None,
        shm: None,
        seat: None,
        pointer: None,
        layer_shell: None,
        glass_manager: None,
        surface: None,
        role: None,
        glass: None,
        max_radius: None,
        glass_request: None,
        degraded_reported: false,
        no_glass_reported: false,
        no_pointer_reported: false,
        no_transport_reported: false,
        interface: None,
        configured: None,
        awaiting_configure: false,
        applied_placement: None,
        frames: Vec::new(),
        state: State::with_shell(shell, tokens, Rc::clone(&localizer)),
        reload,
        drawn: None,
        pending_repaint: false,
        stopped: false,
        connection_lost: false,
        committed: 0,
    };

    // The registry has to be read before anything can be bound, and the wait is
    // bounded by the roundtrip below rather than by a retry loop.
    connection
        .flush()
        .map_err(|error| LegError::Connect(error.to_string()))?;
    queue
        .roundtrip(&mut client)
        .map_err(|error| LegError::Connect(error.to_string()))?;

    for interface in REQUIRED_GLOBALS {
        if !client.globals.contains_key(interface) {
            return Err(LegError::MissingGlobal(interface));
        }
    }

    if client.seat.is_none() {
        warn!("this compositor advertises no wl_seat; the panel draws but takes no pointer input");
    }

    if client.glass_manager.is_none() {
        info!(
            "the compositor does not offer inox-glass-v1; the panel's panes are drawn as plain \
             translucency"
        );
        client.no_glass_reported = true;
    }

    client.create_surface(&qh)?;
    connection
        .flush()
        .map_err(|error| LegError::Connect(error.to_string()))?;

    info!(
        surface = ?client.state.surface(),
        "the panel is running as a wlr-layer-shell surface"
    );

    let outcome = drive(&connection, &mut queue, &mut client, wake_source, signals);

    // Torn down even when the loop failed: the requests still have to reach the
    // compositor before the connection goes away, and a half-destroyed surface
    // would leave the compositor holding objects for a client that is leaving.
    client.teardown();
    let _ = connection.flush();

    outcome
}

/// Serve the compositor until a signal arrives, the compositor goes away, or the
/// surface is closed.
fn drive(
    connection: &Connection,
    queue: &mut wayland_client::EventQueue<Client>,
    client: &mut Client,
    wake_source: calloop::ping::PingSource,
    signals: Signals,
) -> Result<Outcome, LegError> {
    let mut event_loop: EventLoop<Client> =
        EventLoop::try_new().map_err(|error| LegError::EventLoop(error.to_string()))?;
    let handle = event_loop.handle();

    // Taken before the source below takes ownership of the queue, and used for the
    // whole loop: it is a handle into the same connection, not a second queue.
    let qh = queue.handle();

    let reader = connection.clone();
    handle
        .insert_source(
            Generic::new(reader, Interest::READ, Mode::Level),
            move |_, connection, client| {
                if let Some(guard) = connection.prepare_read() {
                    match guard.read() {
                        Ok(_) => {}
                        Err(wayland_client::backend::WaylandError::Io(error))
                            if error.kind() == io::ErrorKind::WouldBlock => {}
                        Err(error) => {
                            warn!(%error, "the compositor connection failed");
                            client.connection_lost = true;
                            client.stopped = true;
                        }
                    }
                }

                if let Err(error) = queue.dispatch_pending(client) {
                    warn!(%error, "failed to dispatch compositor events");
                    client.connection_lost = true;
                    client.stopped = true;
                }

                Ok(PostAction::Continue)
            },
        )
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    handle
        .insert_source(signals, |event, _, client| {
            match event.signal() {
                // The reload host reads and parses the candidate on a thread of
                // its own and hands the request back over the channel, which the
                // ping then wakes this loop for.
                Signal::SIGHUP => {
                    info!("reloading the panel's configuration, as SIGHUP asked");
                    client.reload.prepare();
                }
                signal => {
                    info!(?signal, "stopping the panel");
                    client.stopped = true;
                }
            }
        })
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    handle
        .insert_source(wake_source, |_, _, client| client.drain_reloads())
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    // The clock is a timer, not a frame clock: it feeds the same
    // `Message::Tick` the iced runtime's subscription used to, at the same
    // `app::TICK` interval, and only a clock that actually changed the projection
    // causes a frame.
    handle
        .insert_source(Timer::from_duration(app::TICK), |_, _, client| {
            client.tick();

            TimeoutAction::ToDuration(app::TICK)
        })
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    while !client.stopped {
        connection
            .flush()
            .map_err(|error| LegError::Connect(error.to_string()))?;
        event_loop
            .dispatch(None, client)
            .map_err(|error| LegError::EventLoop(error.to_string()))?;
        // Protocol events have changed the state — the compositor configured the
        // surface, the pointer moved, the glass capability arrived — and what the
        // state decided about them reaches the wire here.
        client.pump();
        // One projection per iteration: it is what says whether the surface has
        // anything new to show, and (for a dock) how wide the pane should be.
        let model = client.state.model();
        client.sync_placement(&model);
        client.repaint(&qh, model);
    }

    Ok(client.outcome())
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;
    use inox_config::PanelConfig;

    fn state(surface: PanelSurface) -> State {
        let mut shell = ShellConfig::default();
        shell.panel.surface = surface;
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");

        State::with_shell(shell, Tokens::builtin_dark(), Rc::new(localizer))
    }

    #[test]
    fn a_frame_is_the_size_it_was_configured_to() {
        let interface = Interface::new(320, 32).expect("a drawable size");

        assert_eq!(interface.dimensions(), (320, 32));
    }

    #[test]
    fn a_size_nothing_could_have_meant_is_refused_rather_than_allocated() {
        assert!(Interface::new(0, 32).is_none(), "a surface with no width");
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
        let mut interface = Interface::new(320, 32).expect("a drawable size");

        assert!(!interface.resize(320, 32), "the same size");
        assert!(interface.resize(640, 32), "a real resize");
        assert_eq!(interface.dimensions(), (640, 32));
        assert!(
            !interface.resize(u32::MAX, u32::MAX),
            "a size that cannot be drawn leaves the frame that is already there"
        );
        assert_eq!(interface.dimensions(), (640, 32));
    }

    #[test]
    fn a_frame_is_little_endian_premultiplied_argb() {
        // `wl_shm`'s `Argb8888` names the *words*: the byte order on the wire is
        // B, G, R, A. The pixels are premultiplied — what the screenshot already
        // holds — so the alpha is copied, not divided out.
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
            "a frame that left the previous one's tail behind would be a frame of the wrong length"
        );
    }

    #[test]
    fn a_panel_frame_keeps_the_pane_translucent() {
        // The one assertion that catches a screenshot composited over an opaque
        // colour: a glass pane has to leave the pixels it does not paint
        // transparent, or the compositor blends a coloured square over the
        // wallpaper behind it.
        let state = state(PanelSurface::Dock);
        let mut interface = Interface::new(320, 96).expect("a drawable size");
        let mut out = Vec::new();

        interface.render_argb(&state, &mut out);

        assert_eq!(out.len(), 320 * 96 * 4);
        assert!(
            out.as_chunks::<4>().0.iter().any(|pixel| pixel[3] < 0xff),
            "every pixel is opaque, so the pane's transparency was composited away"
        );
        assert!(
            out.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0),
            "every pixel is transparent, so nothing was drawn"
        );
    }

    #[test]
    fn a_click_on_an_empty_panel_produces_nothing() {
        let mut state = state(PanelSurface::Taskbar);
        state.take_outgoing();
        let mut interface = Interface::new(1280, 32).expect("a drawable size");

        let entered = interface.pointer(&mut state, Pointer::Enter(Point::new(640.0, 16.0)));
        let pressed = interface.pointer(&mut state, Pointer::Press(mouse::Button::Left));
        let released = interface.pointer(&mut state, Pointer::Release(mouse::Button::Left));

        assert!(entered.messages.is_empty(), "entering is not a message");
        assert!(pressed.messages.is_empty(), "the press is not a message");
        assert!(
            released.messages.is_empty(),
            "a click where there is no entry means nothing (D01): {:?}",
            released.messages
        );
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn pointer_coordinates_are_logical_pixels_at_the_surface_scale() {
        assert_eq!(logical(160.0, 1.0), 160.0);
        assert_eq!(logical(160.0, 2.0), 80.0);
        assert_eq!(
            logical(160.0, 0.0),
            160.0,
            "a zero scale factor is not a division by zero; the coordinate is left alone"
        );
    }

    #[test]
    fn only_the_buttons_the_panel_acts_on_are_translated() {
        assert_eq!(pointer_button(0x110), Some(mouse::Button::Left));
        assert_eq!(pointer_button(0x111), Some(mouse::Button::Right));
        assert_eq!(pointer_button(0x112), Some(mouse::Button::Middle));
        assert_eq!(pointer_button(0x113), Some(mouse::Button::Back));
        assert_eq!(pointer_button(0x114), Some(mouse::Button::Forward));
        assert_eq!(pointer_button(0x115), None, "BTN_TASK has no meaning here");
        assert_eq!(pointer_button(0), None);
    }

    #[test]
    fn every_placement_anchor_and_policy_reaches_the_protocol() {
        assert_eq!(anchor_bits(&Anchors::new()), role::Anchor::empty());

        // The taskbar's own placement, as the protocol sees it: the strip spans
        // the output from its edge and never takes keyboard focus (D01).
        let taskbar = PanelPlacement::derive(&PanelConfig::default());

        assert_eq!(
            anchor_bits(&taskbar.anchors),
            role::Anchor::Bottom | role::Anchor::Left | role::Anchor::Right
        );
        assert_eq!(
            keyboard_of(taskbar.keyboard_interactivity),
            role::KeyboardInteractivity::None
        );

        let top = PanelPlacement::derive(&PanelConfig {
            position: inox_config::PanelPosition::Top,
            ..PanelConfig::default()
        });
        assert_eq!(
            anchor_bits(&top.anchors),
            role::Anchor::Top | role::Anchor::Left | role::Anchor::Right
        );

        let menubar = PanelPlacement::menubar(&inox_config::MenubarConfig::default());
        assert_eq!(
            anchor_bits(&menubar.anchors),
            role::Anchor::Top | role::Anchor::Left | role::Anchor::Right
        );

        let dock = PanelPlacement::dock(
            &inox_config::DockConfig::default(),
            &Tokens::builtin_dark(),
            crate::dock::DockContent::default(),
        );
        assert_eq!(
            anchor_bits(&dock.anchors),
            role::Anchor::Bottom,
            "the dock is anchored to the bottom only, which is what centres it"
        );

        assert_eq!(
            layer_of(placement::Layer::Background),
            zwlr_layer_shell_v1::Layer::Background
        );
        assert_eq!(
            layer_of(placement::Layer::Bottom),
            zwlr_layer_shell_v1::Layer::Bottom
        );
        assert_eq!(
            layer_of(placement::Layer::Top),
            zwlr_layer_shell_v1::Layer::Top
        );
        assert_eq!(
            layer_of(placement::Layer::Overlay),
            zwlr_layer_shell_v1::Layer::Overlay
        );
        assert_eq!(
            keyboard_of(placement::KeyboardInteractivity::OnDemand),
            role::KeyboardInteractivity::OnDemand
        );
        assert_eq!(
            keyboard_of(placement::KeyboardInteractivity::Exclusive),
            role::KeyboardInteractivity::Exclusive
        );
    }

    #[test]
    fn a_frame_file_holds_exactly_what_was_written() {
        // The compositor maps this fd for the whole pool size, so the frame the
        // client writes has to be exactly that long: a short file would fault the
        // compositor's mapping rather than show a wrong colour.
        use std::io::{Read as _, Seek as _};

        let mut file =
            File::from(frame_memory("inox-panel-test").expect("memfd_create is available"));
        let frame = [0x33u8, 0x22, 0x11, 0xff].repeat(4);
        file.write_all(&frame)
            .expect("the frame memory is writable");
        file.seek(io::SeekFrom::Start(0)).expect("the frame seeks");
        let mut read = vec![0u8; frame.len()];
        file.read_exact(&mut read).expect("the frame reads back");

        assert_eq!(read, frame, "the frame is the bytes that were written");
    }
}
