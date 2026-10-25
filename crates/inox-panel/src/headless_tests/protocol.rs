//! A real Wayland wire peer plays the compositor and drives the built binary.
//!
//! This is the acceptance evidence for the panel's real protocol leg: the server
//! side below is a hand-written `wlr-layer-shell`, `wl_shm`, `wl_seat` and
//! `inox-glass-v1`, so what is asserted is what the binary actually put on the
//! wire. Three things are only observable here:
//!
//! - **The placement is applied.** The anchors, the layer, the exclusive zone,
//!   the margin and the keyboard interactivity the surface asks for are read off
//!   the wire and compared with [`crate::placement`]'s own data. A panel
//!   that only *computed* a placement — the state this crate was in before ADR
//!   0018's successor landed — could not produce them.
//! - **The frame is the interface.** The shared memory the client hands over is
//!   read back and measured: its declared size is the configure's, and the bytes
//!   are premultiplied `Argb8888` (`B, G, R, A`), which is what makes a
//!   translucent pane translucent instead of holed.
//! - **The glass request follows the layer role.** The order of `get_layer_surface`
//!   and `get_glass` is a fact about the wire, and so is the radius the client
//!   settled on after the compositor reported its maximum.
//!
//! What this file cannot observe is what a click *means*: the panel's activation
//! request goes to a window-service transport that does not exist, so no server
//! event follows it. The click is therefore asserted in process, through the same
//! [`crate::layer::Input`] the binary drives (see
//! `a_pointer_press_on_a_task_tile_produces_the_activation_message`), with a
//! state the render tests show how to populate. The wire test asserts the half
//! that is on the wire: the panel binds the seat's pointer and survives its
//! events.

use std::os::unix::fs::FileExt as _;
use std::os::unix::net::UnixListener;
use std::process::{Child, ExitStatus};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::app::{self, Message, OutgoingCommand, State};
use crate::dock::{DockContent, DockGeometry, DockView};
use crate::i18n;
use crate::layer::{Button, Input, Point, Pointer};
use crate::placement::PanelPlacement;
use crate::tasks::{SessionGeneration, WindowId, WindowKey, WindowState};
use inox_common::i18n::Locale;
use inox_config::{
    DockConfig, MenubarConfig, PanelConfig, PanelSurface, ShellConfig, Theme as Tokens,
};
use inox_protocols::{
    glass::inox_glass::server::{
        inox_glass_manager_v1 as glass_manager, inox_glass_v1 as glass_object,
    },
    standard::{
        core::server::{
            wl_buffer, wl_compositor, wl_pointer, wl_seat, wl_shm, wl_shm_pool, wl_surface,
        },
        wlr::layer_shell::server::{
            zwlr_layer_shell_v1 as layer_shell, zwlr_layer_surface_v1 as layer_surface,
        },
    },
};
use rustix::process::{Pid, Signal, kill_process};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    backend::ClientData,
};

/// The width this compositor gives a surface that asks for none of its own.
const OUTPUT_WIDTH: u32 = 1280;

/// The token a surface's role is created by, and therefore what the placement
/// assertions are about.
const NAMESPACE: &str = "inox-panel";

/// What the server observed on the wire, shared with the test thread.
#[derive(Debug, Default)]
struct Observed {
    /// Requests in the order they arrived, so "the glass object came after the
    /// layer role" is a fact about the wire rather than an assumption.
    trace: Vec<&'static str>,
    /// What the layer surface asked for.
    layer: LayerRequest,
    /// Every configure the server sent, as `(width, height)`.
    configures_sent: Vec<(u32, u32)>,
    /// Every configure the client acknowledged, by serial.
    acks: Vec<u32>,
    /// The size of every buffer attached, in attach order.
    attached: Vec<Option<(i32, i32)>>,
    /// Every declared buffer format, as the protocol's `u32`.
    formats: Vec<u32>,
    /// Every shared-memory frame the client handed over, measured.
    frames: Vec<Frame>,
    /// How many `wl_buffer` objects the client created.
    buffers_created: usize,
    /// How many it asked the compositor to destroy.
    buffers_destroyed: usize,
    commits: usize,
    /// How many `get_layer_surface` requests arrived.
    layer_surfaces: usize,
    /// How many `get_pointer` requests arrived.
    pointers: usize,
    /// How many `inox_glass_v1` objects were created.
    glass_objects: usize,
    /// How many were destroyed.
    glass_destroyed: usize,
    /// Every `set_radius`, in order.
    glass_radius: Vec<u32>,
    /// Every `set_region`, in order.
    glass_regions: Vec<(i32, i32, i32, i32)>,
    /// Every `set_corner_radii`, in order, as `(top_left, top_right, bottom_right,
    /// bottom_left)`.
    glass_corners: Vec<(u32, u32, u32, u32)>,
    /// Every `set_refraction`, in order, as `(height, amount, flags)`.
    glass_refraction: Vec<(u32, u32, u32)>,
    /// The version of every `inox_glass_v1` object the client created.
    glass_versions: Vec<u32>,
    /// How many version 1 `capabilities` events were sent.
    capabilities_events: usize,
    /// How many version 2 `liquid_capabilities` events were sent.
    liquid_capabilities_events: usize,
}

/// What the layer surface asked the compositor to do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LayerRequest {
    namespace: Option<String>,
    /// The protocol's `layer` value: 0 background, 1 bottom, 2 top, 3 overlay.
    layer: Option<u32>,
    /// The protocol's `anchor` bitfield.
    anchor: Option<u32>,
    exclusive_zone: Option<i32>,
    /// `top, right, bottom, left`, as sent.
    margin: Option<(i32, i32, i32, i32)>,
    /// The protocol's keyboard-interactivity value.
    keyboard: Option<u32>,
    /// The size the client asked for; `(0, height)` is "the compositor decides
    /// the width".
    size: Option<(u32, u32)>,
}

/// What one frame the client handed over looks like, measured from its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Frame {
    /// How many bytes the frame holds.
    bytes: usize,
    /// How many distinct pixels it holds.
    distinct: usize,
    /// How many pixels share the most common colour.
    dominant: usize,
    /// Whether every pixel is opaque.
    opaque: bool,
    /// The first pixel, in the buffer's own byte order.
    first: [u8; 4],
}

impl Frame {
    /// Summarise an `Argb8888` frame: the byte order on the wire is `B, G, R, A`.
    fn measure(bytes: &[u8]) -> Self {
        let mut counts: std::collections::HashMap<[u8; 4], usize> =
            std::collections::HashMap::new();

        for pixel in bytes.as_chunks::<4>().0 {
            *counts.entry(*pixel).or_default() += 1;
        }

        Self {
            bytes: bytes.len(),
            distinct: counts.len(),
            dominant: counts.values().copied().max().unwrap_or(0),
            opaque: bytes
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 0xff),
            first: bytes
                .as_chunks::<4>()
                .0
                .first()
                .copied()
                .unwrap_or_default(),
        }
    }
}

/// The mutex around what the server saw, shared with the test thread.
#[derive(Default)]
struct Observations(Mutex<Observed>);

impl Observations {
    fn record(&self, update: impl FnOnce(&mut Observed)) {
        if let Ok(mut guard) = self.0.lock() {
            update(&mut guard);
        }
    }

    fn read(&self) -> MutexGuard<'_, Observed> {
        self.0.lock().expect("the observations are not poisoned")
    }
}

/// What one fixture asks the panel to do, and how the compositor answers.
#[derive(Debug, Clone)]
struct Plan {
    /// Arguments after the program name.
    arguments: Vec<String>,
    /// The `shell.toml` to write, when the surface comes from the file instead of
    /// the command line.
    shell: Option<String>,
    /// The `[glass] blur` the theme asks for.
    blur: u16,
    /// The `[glass] refraction` band the theme asks for; 0 is the explicit "no
    /// lens" of a frosted-only theme.
    refraction: u16,
    /// The `[glass] refraction_amount` displacement the theme asks for.
    refraction_amount: u16,
    /// What the compositor reports as its maximum radius.
    max_radius: u32,
    /// The `inox-glass-v1` version the compositor advertises: 2 is a liquid-glass
    /// compositor, 1 is the frosted-only one a v1 client is answered with.
    manager_version: u32,
    /// The largest refraction band the compositor reports (version 2 only).
    max_refraction_height: u32,
    /// The largest refraction displacement the compositor reports (version 2
    /// only).
    max_refraction_amount: u32,
    /// Whether this compositor advertises `zwlr_layer_shell_v1` at all.
    layer_shell: bool,
    /// A second configure to send after the first frame, if any.
    reconfigure_to: Option<(u32, u32)>,
    /// Whether the seat advertises a pointer.
    pointer: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            arguments: Vec::new(),
            shell: None,
            blur: 40,
            refraction: 24,
            refraction_amount: 12,
            max_radius: 64,
            manager_version: 2,
            max_refraction_height: 128,
            max_refraction_amount: 64,
            layer_shell: true,
            reconfigure_to: None,
            pointer: true,
        }
    }
}

static NEXT_SERIAL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// The next configure serial: unique, which is all the protocol asks.
fn next_serial() -> u32 {
    NEXT_SERIAL.fetch_add(2, std::sync::atomic::Ordering::Relaxed)
}

/// The server side of the fixture.
struct Server {
    observations: Arc<Observations>,
    plan: Plan,
    /// The layer surface, held so a test can configure it again.
    layer_surface: Option<layer_surface::ZwlrLayerSurfaceV1>,
    /// The surface the layer role was created on, so the fixture can send it
    /// pointer events.
    surface: Option<wl_surface::WlSurface>,
    /// The pointer, once the client bound one.
    pointer: Option<wl_pointer::WlPointer>,
    /// Whether the pointer sequence has been sent.
    pointed: bool,
    /// Whether the second configure owed by `plan.reconfigure_to` was sent.
    reconfigured: bool,
    /// The buffer of the last attach, and the one currently on the surface.
    pending_buffer: Option<wl_buffer::WlBuffer>,
    current_buffer: Option<wl_buffer::WlBuffer>,
}

struct ClientState;

impl ClientData for ClientState {}

impl Server {
    /// The size this compositor answers the client's request with.
    ///
    /// A surface that asked for no width is given the output's; one that named
    /// its own size is given exactly that, which is what a layer surface anchored
    /// to a single edge is.
    fn configured_size(&self) -> (u32, u32) {
        let (width, height) = self.observations.read().layer.size.unwrap_or((0, 0));
        let width = if width == 0 { OUTPUT_WIDTH } else { width };

        (width, height)
    }

    /// Answers a commit that applies surface state with the size the client
    /// asked for.
    fn configure_once(&mut self) {
        let (width, height) = self.configured_size();

        if width == 0 || height == 0 {
            return;
        }

        let serial = next_serial();
        self.observations.record(|observed| {
            observed.configures_sent.push((width, height));
        });

        if let Some(role) = self.layer_surface.as_ref() {
            role.configure(serial, width, height);
        }
    }

    /// Sends the pointer events the panel has to survive and act on.
    fn point(&mut self) {
        let (Some(pointer), Some(surface)) = (self.pointer.as_ref(), self.surface.as_ref()) else {
            return;
        };

        let serial = next_serial();
        // A press at the first task tile's column, in surface-local coordinates;
        // with no window service the panel has no tiles, so what this proves is
        // that the events are accepted, routed through iced and do not stop the
        // frames. What a click *means* is asserted in process, where a state with
        // a tile can be built.
        pointer.enter(serial, surface, 12.0, 16.0);
        pointer.frame();
        pointer.motion(0, 12.0, 16.0);
        pointer.frame();
        pointer.button(serial, 0, 0x110, wl_pointer::ButtonState::Pressed);
        pointer.button(serial, 1, 0x110, wl_pointer::ButtonState::Released);
        pointer.frame();
        pointer.leave(serial, surface);
        pointer.frame();

        self.pointed = true;
    }
}

impl GlobalDispatch<wl_compositor::WlCompositor, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_compositor::WlCompositor>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_compositor::WlCompositor,
        request: wl_compositor::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_compositor::Request::CreateSurface { id } = request {
            let surface = init.init(id, ());
            state.observations.record(|observed| {
                observed.trace.push("create_surface");
            });
            state.surface = Some(surface);
        }
    }
}

impl GlobalDispatch<wl_shm::WlShm, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_shm::WlShm>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<wl_shm::WlShm, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_shm::WlShm,
        request: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, fd, size } = request {
            // The client writes the frame before it creates the pool, so
            // everything is in the file by the time this runs. Reading *at* an
            // offset rather than from the current one matters: a descriptor
            // received over the socket shares its file description, and therefore
            // its offset, with the client's.
            let mut frame = vec![0u8; usize::try_from(size).unwrap_or(0)];
            let file = std::fs::File::from(fd);

            if file.read_exact_at(&mut frame, 0).is_ok() {
                state
                    .observations
                    .record(|observed| observed.frames.push(Frame::measure(&frame)));
            }

            init.init(id, ());
        }
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_shm_pool::WlShmPool,
        request: wl_shm_pool::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm_pool::Request::CreateBuffer {
            id,
            width,
            height,
            format,
            ..
        } = request
        {
            state.observations.record(|observed| {
                observed.buffers_created += 1;
                observed.formats.push(format.into());
            });
            // The dimensions travel with the buffer object, so the commit handler
            // can check what was attached without a side table.
            init.init(id, (width, height));
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, (i32, i32)> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_buffer::WlBuffer,
        request: wl_buffer::Request,
        _: &(i32, i32),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // A client that drops its buffers without saying so leaves the compositor
        // holding a mapping and a descriptor for each one.
        if let wl_buffer::Request::Destroy = request {
            state
                .observations
                .record(|observed| observed.buffers_destroyed += 1);
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_surface::WlSurface,
        request: wl_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            wl_surface::Request::Attach { buffer, .. } => {
                let size = buffer
                    .as_ref()
                    .and_then(|buffer| buffer.data::<(i32, i32)>().copied());
                state
                    .observations
                    .record(|observed| observed.attached.push(size));
                state.pending_buffer = buffer;
            }
            wl_surface::Request::Commit => {
                state.observations.record(|observed| observed.commits += 1);

                // A commit with no buffer is the client applying its
                // double-buffered surface state: the first one asks for the
                // initial configure, and a later one asks for a new configure
                // after `set_size` — which is how a reload that changes a
                // surface's thickness reaches the compositor at all.
                if state.pending_buffer.is_none() {
                    state.configure_once();

                    return;
                }

                if let Some(attached) = state.pending_buffer.take() {
                    let replaced = state.current_buffer.replace(attached.clone());

                    match replaced {
                        Some(replaced) if replaced != attached => replaced.release(),
                        _ => {}
                    }

                    // A compositor that changes its mind about the size — an
                    // output change — configures the surface again, and the client
                    // has to answer with a frame of the new size and re-ask for
                    // glass over the new region.
                    if let Some((width, height)) = state.plan.reconfigure_to
                        && !state.reconfigured
                    {
                        state.reconfigured = true;
                        let serial = next_serial();
                        state.observations.record(|observed| {
                            observed.configures_sent.push((width, height));
                        });

                        if let Some(role) = state.layer_surface.as_ref() {
                            role.configure(serial, width, height);
                        }
                    }

                    // Once the frame is on the surface, drive the pointer so the
                    // input path runs against a real seat.
                    if !state.pointed && state.plan.pointer {
                        state.point();
                    }
                }
            }
            _ => {}
        }
    }
}

impl GlobalDispatch<wl_seat::WlSeat, ()> for Server {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_seat::WlSeat>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let seat = init.init(resource, ());

        // A real seat reports its capabilities as soon as it is bound; the panel
        // binds a pointer from that answer.
        seat.capabilities(if state.plan.pointer {
            wl_seat::Capability::Pointer
        } else {
            wl_seat::Capability::Keyboard
        });
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_seat::WlSeat,
        request: wl_seat::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_seat::Request::GetPointer { id } = request {
            state.observations.record(|observed| observed.pointers += 1);
            state.pointer = Some(init.init(id, ()));
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for Server {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_pointer::WlPointer,
        _: wl_pointer::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // `set_cursor` and `release` are the requests a pointer client sends;
        // nothing here acts on them.
    }
}

impl GlobalDispatch<layer_shell::ZwlrLayerShellV1, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<layer_shell::ZwlrLayerShellV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<layer_shell::ZwlrLayerShellV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &layer_shell::ZwlrLayerShellV1,
        request: layer_shell::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let layer_shell::Request::GetLayerSurface {
            id,
            surface,
            layer,
            namespace,
            ..
        } = request
        {
            let role = init.init(id, ());

            state.observations.record(|observed| {
                observed.trace.push("get_layer_surface");
                observed.layer_surfaces += 1;
                observed.layer.namespace = Some(namespace);
                observed.layer.layer = Some(layer.into());
            });
            state.surface = Some(surface);
            state.layer_surface = Some(role);
        }
    }
}

impl Dispatch<layer_surface::ZwlrLayerSurfaceV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &layer_surface::ZwlrLayerSurfaceV1,
        request: layer_surface::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            layer_surface::Request::SetSize { width, height } => {
                state.observations.record(|observed| {
                    observed.trace.push("set_size");
                    observed.layer.size = Some((width, height));
                });
            }
            layer_surface::Request::SetAnchor { anchor } => {
                state.observations.record(|observed| {
                    observed.layer.anchor = Some(into_bits(anchor));
                });
            }
            layer_surface::Request::SetExclusiveZone { zone } => {
                state
                    .observations
                    .record(|observed| observed.layer.exclusive_zone = Some(zone));
            }
            layer_surface::Request::SetMargin {
                top,
                right,
                bottom,
                left,
            } => {
                state
                    .observations
                    .record(|observed| observed.layer.margin = Some((top, right, bottom, left)));
            }
            layer_surface::Request::SetKeyboardInteractivity {
                keyboard_interactivity,
            } => {
                state.observations.record(|observed| {
                    observed.layer.keyboard = Some(into_bits(keyboard_interactivity));
                });
            }
            layer_surface::Request::AckConfigure { serial } => {
                state
                    .observations
                    .record(|observed| observed.acks.push(serial));
            }
            layer_surface::Request::Destroy => {
                state.observations.record(|observed| {
                    observed.trace.push("destroy_role");
                });
            }
            _ => {}
        }
    }
}

impl GlobalDispatch<glass_manager::InoxGlassManagerV1, ()> for Server {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<glass_manager::InoxGlassManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        init.init(resource, ());
    }
}

impl Dispatch<glass_manager::InoxGlassManagerV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &glass_manager::InoxGlassManagerV1,
        request: glass_manager::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let glass_manager::Request::GetGlass { id, .. } = request {
            let glass = init.init(id, ());
            let version = glass.version();
            let max_radius = state.plan.max_radius;

            state.observations.record(|observed| {
                observed.trace.push("get_glass");
                observed.glass_objects += 1;
                observed.glass_versions.push(version);
            });

            // Sent once, right after creation, and never a promise the
            // compositor would not honour for this surface. The version 1 event
            // goes first and always — a client that only listens for it must not
            // think its object is mute — and the version 2 one follows for a v2
            // object, where it is the authoritative answer (ADR 0040 §1).
            state.observations.record(|observed| {
                observed.trace.push("capabilities");
                observed.capabilities_events += 1;
            });
            glass.capabilities(max_radius);

            if version >= 2 {
                state.observations.record(|observed| {
                    observed.trace.push("liquid_capabilities");
                    observed.liquid_capabilities_events += 1;
                });
                glass.liquid_capabilities(
                    max_radius,
                    state.plan.max_refraction_height,
                    state.plan.max_refraction_amount,
                    glass_object::RefractionFlag::Depth | glass_object::RefractionFlag::Dispersion,
                );
            }
        }
    }
}

impl Dispatch<glass_object::InoxGlassV1, ()> for Server {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &glass_object::InoxGlassV1,
        request: glass_object::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            glass_object::Request::SetRadius { radius } => {
                state.observations.record(|observed| {
                    observed.trace.push("set_radius");
                    observed.glass_radius.push(radius);
                });
            }
            glass_object::Request::SetRegion {
                x,
                y,
                width,
                height,
            } => {
                state.observations.record(|observed| {
                    observed.trace.push("set_region");
                    observed.glass_regions.push((x, y, width, height));
                });
            }
            glass_object::Request::SetCornerRadii {
                top_left,
                top_right,
                bottom_right,
                bottom_left,
            } => {
                state.observations.record(|observed| {
                    observed.trace.push("set_corner_radii");
                    observed
                        .glass_corners
                        .push((top_left, top_right, bottom_right, bottom_left));
                });
            }
            glass_object::Request::SetRefraction {
                height,
                amount,
                flags,
            } => {
                let flags = into_bits(flags);

                state.observations.record(|observed| {
                    observed.trace.push("set_refraction");
                    observed.glass_refraction.push((height, amount, flags));
                });
            }
            glass_object::Request::Destroy => {
                state.observations.record(|observed| {
                    observed.trace.push("destroy_glass");
                    observed.glass_destroyed += 1;
                });
            }
            _ => {}
        }
    }
}

/// The protocol word of an enum argument the client sent.
///
/// A request field arrives as a [`WEnum`]: `Value` when the server knows the
/// value and `Unknown` when it does not, which is exactly the distinction a
/// compositor must not lose.
fn into_bits<T: Copy + Into<u32>>(value: WEnum<T>) -> u32 {
    match value {
        WEnum::Value(value) => value.into(),
        WEnum::Unknown(bits) => bits,
    }
}

/// A running server, its socket directory, and the process under test.
struct Fixture {
    _root: tempfile::TempDir,
    /// The process under test. A `RefCell` rather than a `&mut self` accessor so
    /// that a test can hold the observations it is asserting on across the
    /// shutdown, which is exactly when the child is reaped.
    child: std::cell::RefCell<Child>,
    observations: Arc<Observations>,
}

impl Fixture {
    /// Starts the compositor and the binary against it.
    fn start(plan: Plan) -> Self {
        let root = tempfile::tempdir().expect("a private runtime directory");
        let socket_path = root.path().join("wayland-panel");
        let listener = UnixListener::bind(&socket_path).expect("the socket binds");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");

        write_configuration(root.path(), &plan);

        let observations = Arc::new(Observations::default());
        let server_observations = Arc::clone(&observations);
        let server_plan = plan.clone();

        let mut display = Display::<Server>::new().expect("a display");
        let mut handle = display.handle();
        handle.create_global::<Server, wl_compositor::WlCompositor, ()>(5, ());
        handle.create_global::<Server, wl_shm::WlShm, ()>(1, ());
        handle.create_global::<Server, wl_seat::WlSeat, ()>(5, ());
        if plan.layer_shell {
            handle.create_global::<Server, layer_shell::ZwlrLayerShellV1, ()>(4, ());
        }
        // A liquid-glass compositor advertises version 2; `Plan::manager_version`
        // lets a test be a version 1 one instead, which the client must bind
        // without asking for anything `since=2`.
        handle.create_global::<Server, glass_manager::InoxGlassManagerV1, ()>(
            plan.manager_version,
            (),
        );

        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut state = Server {
                observations: server_observations,
                plan: server_plan,
                layer_surface: None,
                surface: None,
                pointer: None,
                pointed: false,
                reconfigured: false,
                pending_buffer: None,
                current_buffer: None,
            };
            let deadline = Instant::now() + Duration::from_secs(30);

            while !server_stop.load(std::sync::atomic::Ordering::Relaxed)
                && Instant::now() < deadline
            {
                if let Ok((stream, _address)) = listener.accept() {
                    stream
                        .set_nonblocking(true)
                        .expect("a non-blocking client connection");
                    handle
                        .insert_client(stream, Arc::new(ClientState))
                        .expect("the client is registered");
                }

                if display.dispatch_clients(&mut state).is_err() {
                    continue;
                }

                if display.flush_clients().is_err() {
                    continue;
                }

                std::thread::sleep(Duration::from_millis(2));
            }
        });

        let log_path = root.path().join("panel.log");
        let log = std::fs::File::create(&log_path).expect("the log file");

        let mut command = super::binary::panel_command();
        command
            .env("INOX_PANEL_ARGUMENTS", plan.arguments.join(" "))
            .env("XDG_RUNTIME_DIR", root.path())
            .env("XDG_CONFIG_HOME", root.path())
            .env("WAYLAND_DISPLAY", "wayland-panel")
            .env("LANG", "en-US")
            .env("LC_ALL", "en-US")
            .env("RUST_LOG", "info")
            .stdout(log.try_clone().expect("a log handle"))
            .stderr(log);

        let child = {
            // `signalfd` only receives a signal that *every* thread blocks, and the
            // panel blocks its stop signals before anything else can spawn a
            // thread. The child is this test binary, so its entry point runs on a
            // test thread rather than on `main`; the mask is handed down across
            // `exec` from the thread that spawns it, which is why these are
            // blocked here for exactly as long as the spawn takes.
            let mask = calloop::signals::Signals::new(&[
                calloop::signals::Signal::SIGINT,
                calloop::signals::Signal::SIGTERM,
                calloop::signals::Signal::SIGHUP,
            ])
            .expect("the stop signals can be blocked");
            let child = command.spawn().expect("the panel binary starts");
            drop(mask);

            child
        };

        Self {
            _root: root,
            child: std::cell::RefCell::new(child),
            observations,
        }
    }

    /// The log the binary wrote.
    fn log(&self) -> String {
        std::fs::read_to_string(self._root.path().join("panel.log")).unwrap_or_default()
    }

    /// Writes `shell.toml` the way a user editing the file would.
    fn write_shell(&self, text: &str) {
        std::fs::write(self._root.path().join("inox").join("shell.toml"), text)
            .expect("shell.toml");
    }

    /// Waits until a frame of `width` × `height` has been committed and its
    /// buffer was declared.
    ///
    /// Asserting on the whole exchange rather than on its first message is what
    /// keeps these tests deterministic: the server thread processes the client's
    /// batch one message at a time, and a test that read the observations after
    /// the pool arrived but before the buffer did would be asserting on a prefix.
    fn wait_for_frame(&self, width: u32, height: u32) {
        self.wait_for("a frame of the configured size", |observed| {
            observed.buffers_created >= 1
                && observed.attached.last().copied().flatten()
                    == Some((width as i32, height as i32))
        });
    }

    /// Waits for `condition`, dispatching nothing: the server thread is running.
    fn wait_for(&self, what: &str, condition: impl Fn(&Observed) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);

        loop {
            if condition(&self.observations.read()) {
                return;
            }

            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn signal(&self, signal: Signal) {
        kill_process(
            Pid::from_raw(self.child.borrow().id() as i32).expect("the child pid"),
            signal,
        )
        .expect("the signal is delivered");
    }

    fn wait_for_exit(&self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut child = self.child.borrow_mut();

        loop {
            if let Some(status) = child.try_wait().expect("the child status") {
                return status;
            }

            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();

                panic!("the panel binary did not exit");
            }

            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn observed(&self) -> MutexGuard<'_, Observed> {
        self.observations.read()
    }
}

/// Writes the configuration the plan asks for.
///
/// The theme is written with the plan's blur: `glass.blur == 0` is the tokens
/// saying "plain translucency", and it has to come from a real file because that
/// is where the binary reads it.
fn write_configuration(root: &std::path::Path, plan: &Plan) {
    let inox = root.join("inox");
    let theme = inox.join("themes").join("panel-test");

    std::fs::create_dir_all(&theme).expect("the theme directory");

    std::fs::write(inox.join("config.toml"), "[theme]\nname = \"panel-test\"\n")
        .expect("config.toml");

    std::fs::write(
        theme.join("theme.toml"),
        format!(
            "[glass]\nblur = {}\nrefraction = {}\nrefraction_amount = {}\n",
            plan.blur, plan.refraction, plan.refraction_amount
        ),
    )
    .expect("theme.toml");

    if let Some(shell) = &plan.shell {
        std::fs::write(inox.join("shell.toml"), shell).expect("shell.toml");
    }
}

/// The taskbar's own placement, as the crate's data computes it.
fn taskbar_placement() -> PanelPlacement {
    PanelPlacement::derive(&inox_config::PanelConfig::default())
}

/// The menu bar's own placement.
fn menubar_placement() -> PanelPlacement {
    PanelPlacement::menubar(&MenubarConfig::default())
}

/// The dock's own geometry and content, as the binary has them when no window
/// service is reachable.
///
/// The content comes from a projection built the same way the process builds
/// it — the same state, the same catalog and the same `DockGeometry::content`
/// the protocol leg calls — so the size asserted below is the size of the
/// interface the panel actually draws rather than a number written here.
fn dock_starting() -> (DockGeometry, DockContent, PanelPlacement) {
    // The fixture starts the binary with `LANG=en-US`, so the sentence the
    // process sizes its pane for is the en-US one.
    let localizer = i18n::load(&Locale::parse("en-US").expect("a well-formed tag"))
        .expect("the catalogs parse");
    let shell = ShellConfig {
        panel: PanelConfig {
            surface: PanelSurface::Dock,
            ..PanelConfig::default()
        },
        ..ShellConfig::default()
    };
    let state = State::with_shell(shell, Tokens::builtin_dark(), Rc::new(localizer));
    let shell = state.shell();
    let geometry = DockGeometry::new(state.tokens(), &shell.dock);
    let view = DockView::project(
        &state.model(),
        &shell.dock,
        shell.panel.max_task_buttons,
        state.localizer(),
    );
    let content = geometry.content(&view);
    let placement = PanelPlacement::dock(&shell.dock, state.tokens(), content);

    (geometry, content, placement)
}

/// The dock's own placement, with no task in it.
fn dock_placement() -> PanelPlacement {
    dock_starting().2
}

/// The `points` of a `u16`-valued dimension, for comparing against protocol
/// integers.
fn points(value: u16) -> u32 {
    u32::from(value)
}

#[test]
fn the_taskbar_is_a_placed_layer_surface_with_an_exclusive_strip() {
    let fixture = Fixture::start(Plan::default());
    let placement = taskbar_placement();
    let height = points(placement.height);

    fixture.wait_for("a layer surface", |observed| observed.layer_surfaces == 1);
    fixture.wait_for("an acknowledged configure", |observed| {
        observed.acks.len() == 1
    });
    fixture.wait_for_frame(OUTPUT_WIDTH, height);

    {
        let observed = fixture.observed();

        // The placement, read off the wire and compared with `placement.rs`'s own
        // data: the anchors are Bottom | Left | Right, the strip is reserved and the
        // panel is on the top layer. This is acceptance D01 finally being applied
        // rather than computed.
        assert_eq!(observed.layer.namespace.as_deref(), Some(NAMESPACE));
        assert_eq!(
            observed.layer.anchor,
            Some(anchor_bits(&placement)),
            "the taskbar spans the output from the bottom edge"
        );
        assert_eq!(
            observed.layer.exclusive_zone,
            Some(i32::try_from(placement.exclusive_zone).unwrap()),
            "the strip the compositor keeps free of windows"
        );
        assert_eq!(
            observed.layer.margin,
            Some((0, 0, placement.margin.into(), 0)),
            "the margin is the placement's"
        );
        assert_eq!(
            observed.layer.layer,
            Some(2),
            "Layer::Top is the protocol's 2"
        );
        assert_eq!(
            observed.layer.keyboard,
            Some(0),
            "ordinary display never takes keyboard focus (D01)"
        );
        assert_eq!(
            observed.layer.size,
            Some((0, height)),
            "a bar asks for no width of its own and names its thickness"
        );
        assert_eq!(
            observed.configures_sent,
            vec![(OUTPUT_WIDTH, height)],
            "the compositor gives the width and the panel asked for the height"
        );
        assert_eq!(observed.acks.len(), 1, "the configure was acknowledged");

        // The frame is the interface, at the configured size, in the byte order
        // `wl_shm` names.
        let frame = observed.frames.first().copied().expect("a frame");
        assert_eq!(
            frame.bytes,
            (OUTPUT_WIDTH * height) as usize * 4,
            "the frame declares the size the compositor configured"
        );
        assert_eq!(
            observed.formats,
            vec![0],
            "Argb8888 is the protocol's 0: a translucent pane cannot be Xrgb8888"
        );
        assert!(frame.distinct > 1, "the frame is not one flat colour");
        assert!(
            frame.opaque,
            "the taskbar draws the opaque `surface` token: every pixel of it is opaque"
        );

        // The first pixel is the theme's own `surface` token, in premultiplied ARGB.
        // Pinning it against the token rather than against a literal is what makes
        // this assertion the theme's, and it is the one thing that catches a swapped
        // channel.
        let surface = Tokens::builtin_dark().colors.surface;
        assert_eq!(
            frame.first,
            [surface.b, surface.g, surface.r, surface.a],
            "the frame's first pixel is the surface token in B, G, R, A order"
        );

        // The taskbar is an opaque strip, so it never asks the compositor to frost
        // anything — not even though the theme asks for a blur.
        assert_eq!(
            observed.glass_objects, 0,
            "the taskbar is not a pane: a blur request for it would describe an effect \
         nothing could see"
        );
    }

    fixture.signal(Signal::TERM);
    let status = fixture.wait_for_exit();
    assert!(status.success(), "a signalled panel stops cleanly");

    fixture.wait_for("the layer role to be destroyed", |observed| {
        observed.trace.contains(&"destroy_role")
    });
}

#[test]
fn the_menu_bar_asks_for_frosted_glass_after_its_layer_role_exists() {
    let plan = Plan {
        arguments: vec!["--surface".to_owned(), "menubar".to_owned()],
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);
    let placement = menubar_placement();
    let height = points(placement.height);

    fixture.wait_for("the glass object's answer", |observed| {
        !observed.glass_regions.is_empty()
    });
    fixture.wait_for_frame(OUTPUT_WIDTH, height);

    {
        let observed = fixture.observed();

        assert_eq!(observed.layer.namespace.as_deref(), Some(NAMESPACE));
        assert_eq!(
            observed.layer.anchor,
            Some(anchor_bits(&placement)),
            "the menu bar spans the top edge"
        );
        assert_eq!(
            observed.layer.exclusive_zone,
            Some(i32::try_from(placement.exclusive_zone).unwrap())
        );
        assert_eq!(observed.layer.layer, Some(2));
        assert_eq!(observed.layer.keyboard, Some(0));
        assert_eq!(observed.layer.size, Some((0, height)));
        assert_eq!(observed.configures_sent, vec![(OUTPUT_WIDTH, height)]);

        // The glass object exists, was created after the layer role (and after
        // the placement was committed: the panel does not ask to frost a surface
        // it has not placed), and carries the clamped radius for the pane's own
        // area.
        assert_eq!(observed.glass_objects, 1);
        let role = observed
            .trace
            .iter()
            .position(|name| *name == "get_layer_surface")
            .expect("the layer role was created");
        let glass = observed
            .trace
            .iter()
            .position(|name| *name == "get_glass")
            .expect("the glass object was created");

        assert!(
            role < glass,
            "the role comes first: a surface without one is answered max_radius = 0: {:?}",
            observed.trace
        );
        assert_eq!(
            observed.glass_radius,
            vec![40],
            "the theme asks for 40 and the compositor accepts up to 64"
        );
        assert_eq!(
            observed.glass_regions,
            vec![(0, 0, OUTPUT_WIDTH as i32, height as i32)],
            "the pane is the whole configured surface"
        );

        // The version 2 wire. The manager is bound at `min(advertised, 2)`, both
        // capability events arrived in the protocol's order, and the request
        // carries the lens the tokens describe: the pane's own corner radius and
        // the reference refraction band, with dispersion because this compositor
        // advertises it.
        assert_eq!(
            observed.glass_versions,
            vec![2],
            "a version 2 compositor gets a version 2 object"
        );
        assert_eq!(
            observed.capabilities_events, 1,
            "the version 1 answer is still sent first, so a v1-only client is not left \
             thinking its object is mute"
        );
        assert_eq!(
            observed.liquid_capabilities_events, 1,
            "and the version 2 answer follows it"
        );

        let capabilities = observed
            .trace
            .iter()
            .position(|name| *name == "capabilities")
            .expect("the v1 capabilities event was sent");
        let liquid = observed
            .trace
            .iter()
            .position(|name| *name == "liquid_capabilities")
            .expect("the v2 capabilities event was sent");

        assert!(
            capabilities < liquid,
            "capabilities comes before liquid_capabilities: {:?}",
            observed.trace
        );
        assert_eq!(
            observed.glass_corners,
            vec![(8, 8, 8, 8)],
            "the refracting shape is the pane's own `radii.md` token, not a second radius"
        );
        assert_eq!(
            observed.glass_refraction,
            vec![(24, 12, inox_glass::REFRACTION_FLAG_DISPERSION)],
            "the reference band (24 px, 12 px displacement), dispersed because the compositor \
             advertised it and not deeper because the tokens do not ask for depth: {:?}",
            observed.trace
        );

        let frame = observed.frames.first().copied().expect("a frame");
        assert_eq!(frame.bytes, (OUTPUT_WIDTH * height) as usize * 4);
        assert!(
            !frame.opaque,
            "a glass pane is translucent: an opaque frame would hide the desktop behind it"
        );
    }

    fixture.signal(Signal::TERM);
    let status = fixture.wait_for_exit();
    assert!(status.success());
    // The requests a client sends on its way out still have to be dispatched by
    // the compositor, so the assertion waits for them rather than reading the
    // moment the process is reaped.
    fixture.wait_for("the glass object to be destroyed", |observed| {
        observed.glass_destroyed == 1
    });
    let observed = fixture.observed();
    assert!(
        observed.trace.contains(&"destroy_role"),
        "the layer role is destroyed too: {:?}",
        observed.trace
    );
}

#[test]
fn the_dock_is_sized_from_its_own_content_and_centred_by_its_anchor() {
    // The dock comes from the file rather than the flag, so both ways of
    // choosing a surface are covered.
    let plan = Plan {
        shell: Some("[panel]\nsurface = \"dock\"\n".to_owned()),
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);
    let (geometry, content, placement) = dock_starting();
    let width = points(placement.width.expect("a dock is content-sized"));
    let height = points(placement.height);

    fixture.wait_for("an acknowledged configure", |observed| {
        observed.acks.len() == 1
    });
    fixture.wait_for_frame(width, height);
    fixture.wait_for("the frosted pane", |observed| {
        !observed.glass_regions.is_empty()
    });

    let observed = fixture.observed();

    assert_eq!(
        observed.layer.anchor,
        Some(anchor_bits(&placement)),
        "the dock is anchored to the bottom edge only, which is what centres it"
    );
    assert_eq!(
        observed.layer.exclusive_zone,
        Some(0),
        "a dock floats over windows by default"
    );
    assert_eq!(
        observed.layer.margin,
        Some((8, 8, 8, 8)),
        "the configured margin is what the placement carries"
    );
    assert_eq!(
        observed.layer.size,
        Some((width, height)),
        "a surface anchored to one edge names its own size, derived from its geometry"
    );
    assert_eq!(
        observed.configures_sent,
        vec![(width, height)],
        "the compositor answers the size the client asked for"
    );
    // The size on the wire is the size of what the projection holds: with no
    // window service the dock has nothing but its own sentence, and the pane has
    // to be wide enough to draw all of it — the old 16-logical-pixel request was
    // a sliver the sentence was clipped to nothing in.
    assert!(
        content.notice_width > 0.0,
        "the starting panel has a sentence to show, so its pane is sized for one"
    );
    assert!(
        f32::from(placement.width.expect("content sized")) >= content.notice_width,
        "the requested width holds the sentence: {:?} for {} logical pixels of text",
        placement.width,
        content.notice_width
    );
    assert!(
        (f32::from(placement.width.expect("content sized")) - geometry.minimum_width).abs()
            < f32::EPSILON,
        "an empty dock asks for exactly the documented minimum width, because its own sentence is \
         the only thing it has to show: {:?}",
        placement.width
    );

    // A tray group is part of the same pane, and the labels it draws are inside
    // the surface: the requested width carries the group's own width, the
    // hairline before it and the gaps around the pair. The tray's transport does
    // not exist, so this is the projection's content rather than the running
    // binary's — which is exactly what the placement is sized from.
    let tray = geometry.tray_width(["Mail", "Music and podcasts"]);
    assert!(tray > 0.0, "the labels have a width to allow for");
    let with_tray = PanelPlacement::dock(
        &DockConfig::default(),
        &Tokens::builtin_dark(),
        DockContent {
            tray_width: tray,
            ..content
        },
    );
    assert!(
        f32::from(with_tray.width.expect("content sized"))
            >= tray + geometry.separator_width + geometry.padding * 2.0 + geometry.spacing * 2.0,
        "the tray group is inside the pane instead of clipped by it: {:?} for {tray} \
         logical pixels of labels",
        with_tray.width
    );

    let frame = observed.frames.first().copied().expect("a frame");
    assert_eq!(
        frame.bytes,
        (width * height) as usize * 4,
        "the committed frame is exactly the size the surface was configured to"
    );
    assert!(!frame.opaque, "the dock is a glass pane");

    // The region the compositor is asked to frost is the pane's own area, which
    // is the whole surface: a surface wider than the pane would frost a band of
    // wallpaper around a floating pill.
    assert_eq!(
        observed.glass_regions,
        vec![(0, 0, width as i32, height as i32)],
        "the frosted region is the pane the dock draws, and only it"
    );

    fixture.signal(Signal::TERM);
    let _ = fixture.wait_for_exit();
}

#[test]
fn a_reconfigure_is_answered_with_a_frame_and_a_region_of_the_new_size() {
    let plan = Plan {
        arguments: vec!["--surface".to_owned(), "menubar".to_owned()],
        reconfigure_to: Some((1024, 40)),
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);
    let height = points(menubar_placement().height);

    fixture.wait_for("the second configure to be answered", |observed| {
        observed.acks.len() == 2
    });
    fixture.wait_for("a frame at the new size", |observed| {
        observed
            .frames
            .iter()
            .any(|frame| frame.bytes == (1024 * 40) as usize * 4)
    });
    fixture.wait_for("the region of the new size", |observed| {
        observed.glass_regions.contains(&(0, 0, 1024, 40))
    });

    let observed = fixture.observed();

    assert_eq!(
        observed.configures_sent,
        vec![(OUTPUT_WIDTH, height), (1024, 40)],
        "the compositor sent both configures"
    );
    assert_eq!(observed.acks.len(), 2, "and both were acknowledged");
    assert!(
        observed.glass_radius.iter().all(|radius| *radius == 40),
        "the radius is the theme's 40 throughout, whatever the size: {:?}",
        observed.glass_radius
    );
    assert_eq!(
        observed.glass_radius.len(),
        2,
        "the whole request is re-sent when its region changes, so the radius goes with it"
    );
    assert_eq!(
        observed.glass_regions,
        vec![(0, 0, OUTPUT_WIDTH as i32, height as i32), (0, 0, 1024, 40)],
        "the region is re-sent for the new pane: the old one would frost the wrong area"
    );
    assert!(
        observed.buffers_created - observed.buffers_destroyed <= crate::layer::MAX_FRAMES_IN_FLIGHT,
        "the client holds at most the frames in flight: created {}, destroyed {}",
        observed.buffers_created,
        observed.buffers_destroyed
    );

    fixture.signal(Signal::TERM);
    let _ = fixture.wait_for_exit();
}

#[test]
fn a_compositor_that_cannot_blur_still_gets_frames_and_is_told_apart() {
    let plan = Plan {
        arguments: vec!["--surface".to_owned(), "dock".to_owned()],
        max_radius: 0,
        // A compositor that can draw nothing reports nothing in *both* capability
        // events: the panel must not fabricate a lens out of the tokens.
        max_refraction_height: 0,
        max_refraction_amount: 0,
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);

    fixture.wait_for("the capabilities answer", |observed| {
        observed.glass_objects == 1
    });
    let placement = dock_placement();
    fixture.wait_for_frame(
        points(placement.width.expect("content sized")),
        points(placement.height),
    );
    fixture.wait_for("the degradation to be logged", |_| {
        fixture.log().contains("will not frost")
    });

    let observed = fixture.observed();

    assert_eq!(
        observed.glass_radius,
        Vec::<u32>::new(),
        "max_radius = 0 means plain translucency: no radius is claimed"
    );
    assert!(
        observed.glass_regions.is_empty(),
        "and no region is frosted"
    );
    assert!(
        observed.glass_corners.is_empty() && observed.glass_refraction.is_empty(),
        "a lens nothing can draw is not described either: the panel asked for neither \
         `set_corner_radii` nor `set_refraction`"
    );
    assert_eq!(
        observed.liquid_capabilities_events, 1,
        "the honest zero answer did arrive; it is the compositor's to give, not the client's \
         to assume"
    );
    assert!(
        observed
            .frames
            .first()
            .is_some_and(|frame| frame.distinct > 1),
        "the panel keeps drawing: the degradation is a material, not a failure"
    );

    fixture.signal(Signal::TERM);
    let status = fixture.wait_for_exit();
    assert!(
        status.success(),
        "a compositor that cannot blur is not an error"
    );
}

#[test]
fn a_theme_that_asks_for_neither_half_creates_no_glass_object_at_all() {
    let plan = Plan {
        arguments: vec!["--surface".to_owned(), "menubar".to_owned()],
        // Both halves off. `blur = 0` *alone* would still be a lens (ADR 0040
        // §5), which is the next test; this is the tokens saying plain
        // translucency.
        blur: 0,
        refraction: 0,
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);

    let placement = menubar_placement();
    fixture.wait_for_frame(OUTPUT_WIDTH, points(placement.height));

    let observed = fixture.observed();

    assert_eq!(
        observed.glass_objects, 0,
        "a material with neither a blur nor a band is plain translucency: no object is created"
    );
    assert!(
        !observed.trace.contains(&"get_glass"),
        "nothing was even asked for: {:?}",
        observed.trace
    );
    assert!(!observed.frames[0].opaque, "the pane is still translucent");

    fixture.signal(Signal::TERM);
    let _ = fixture.wait_for_exit();
}

#[test]
fn a_refraction_band_alone_creates_the_object_and_asks_for_no_blur() {
    // ADR 0040 §5: "create the glass object when blur > 0 *or* refraction > 0".
    // This compositor cannot blur at all; the tokens ask for no blur anyway, so
    // the only effect on the wire is the lens.
    let plan = Plan {
        arguments: vec!["--surface".to_owned(), "dock".to_owned()],
        blur: 0,
        max_radius: 0,
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);
    let placement = dock_placement();
    let width = points(placement.width.expect("content sized"));
    let height = points(placement.height);

    fixture.wait_for("the lens request", |observed| {
        !observed.glass_refraction.is_empty()
    });
    fixture.wait_for_frame(width, height);

    let observed = fixture.observed();

    assert_eq!(
        observed.glass_objects, 1,
        "a band without a blur is still an object: either half creates it"
    );
    assert_eq!(
        observed.glass_radius,
        vec![0],
        "the tokens asked for no blur, and the compositor can draw none"
    );
    assert_eq!(
        observed.glass_regions,
        vec![(0, 0, width as i32, height as i32)],
        "the lens covers the whole pane"
    );
    assert_eq!(
        observed.glass_corners,
        vec![(8, 8, 8, 8)],
        "the refracting shape is the pane's `radii.md` token"
    );
    assert_eq!(
        observed.glass_refraction,
        vec![(24, 12, inox_glass::REFRACTION_FLAG_DISPERSION)],
        "the band and its dispersal, from the tokens"
    );

    fixture.signal(Signal::TERM);
    let status = fixture.wait_for_exit();
    assert!(status.success(), "a lens-only pane is not an error");
}

#[test]
fn a_version_one_compositor_gets_the_blur_only_and_no_version_two_requests() {
    // The protocol's own rule: a manager bound at version 1 creates version 1
    // objects, which have no `set_corner_radii`, no `set_refraction` and never
    // receive `liquid_capabilities`. Sending one of those would be a protocol
    // error, so the panel must simply not have a lens there.
    let plan = Plan {
        arguments: vec!["--surface".to_owned(), "menubar".to_owned()],
        manager_version: 1,
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);
    let height = points(menubar_placement().height);

    fixture.wait_for("the glass request", |observed| {
        !observed.glass_regions.is_empty()
    });
    fixture.wait_for_frame(OUTPUT_WIDTH, height);

    let observed = fixture.observed();

    assert_eq!(
        observed.glass_versions,
        vec![1],
        "the manager is bound at min(advertised, 2)"
    );
    assert_eq!(
        observed.liquid_capabilities_events, 0,
        "a version 1 object never receives the version 2 event"
    );
    assert_eq!(
        observed.capabilities_events, 1,
        "the version 1 answer is the whole answer here"
    );
    assert!(
        !observed.trace.contains(&"set_corner_radii"),
        "a `since=2` request on a version 1 object is a protocol error: {:?}",
        observed.trace
    );
    assert!(
        observed.glass_refraction.is_empty(),
        "a v1 compositor cannot refract, so no band is described: {:?}",
        observed.trace
    );
    assert_eq!(
        observed.glass_radius,
        vec![40],
        "the blur is still asked for, and clamped the same way it always was"
    );
    assert_eq!(
        observed.glass_regions,
        vec![(0, 0, OUTPUT_WIDTH as i32, height as i32)],
        "and the region is still the pane"
    );
    assert!(!observed.frames[0].opaque, "the pane is still translucent");

    fixture.signal(Signal::TERM);
    let status = fixture.wait_for_exit();
    assert!(status.success());
}

#[test]
fn a_sighup_reload_moves_the_surface_to_the_new_height() {
    // The reload path in full: the user edits `shell.toml`, `SIGHUP` reaches the
    // reload host, the host pings the panel's loop, the loop applies the
    // candidate and re-asks for the placement — and the surface that comes back
    // is the new height. The loop only drains reloads when it is woken, so a
    // missing wake-up leaves the panel at the old height and this test times out.
    let plan = Plan {
        arguments: vec!["--surface".to_owned(), "menubar".to_owned()],
        // No pointer events: the only thing that can wake the loop is the reload
        // itself (or the clock, which does not drain reloads).
        pointer: false,
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);
    let before = points(menubar_placement().height);

    fixture.wait_for_frame(OUTPUT_WIDTH, before);

    fixture.write_shell("[menubar]\nheight = 40\n");
    fixture.signal(Signal::HUP);

    fixture.wait_for("the reloaded configure", |observed| {
        observed.configures_sent.contains(&(OUTPUT_WIDTH, 40))
    });
    fixture.wait_for_frame(OUTPUT_WIDTH, 40);

    {
        let observed = fixture.observed();

        assert!(
            observed.acks.len() >= 2,
            "the new configure was acknowledged: {:?}",
            observed.acks
        );
        assert_eq!(
            observed.layer.size,
            Some((0, 40)),
            "the panel asked for the reloaded height"
        );
        assert!(
            observed
                .glass_regions
                .contains(&(0, 0, OUTPUT_WIDTH as i32, 40)),
            "the frosted region followed the pane it describes: {:?}",
            observed.glass_regions
        );
        assert!(
            observed
                .frames
                .iter()
                .any(|frame| frame.bytes == (OUTPUT_WIDTH * 40) as usize * 4),
            "a frame of the new size was committed: {:?}",
            observed
                .frames
                .iter()
                .map(|frame| frame.bytes)
                .collect::<Vec<_>>()
        );
    }

    fixture.signal(Signal::TERM);
    let status = fixture.wait_for_exit();
    assert!(status.success(), "the reloaded panel still stops cleanly");
}

#[test]
fn a_compositor_without_layer_shell_is_refused_with_a_readable_error() {
    let plan = Plan {
        layer_shell: false,
        ..Plan::default()
    };
    let fixture = Fixture::start(plan);

    let status = fixture.wait_for_exit();

    assert!(
        !status.success(),
        "a panel that cannot be placed must not pretend to be: "
    );

    let log = fixture.log();
    assert!(
        log.contains("zwlr_layer_shell_v1"),
        "the failure names the protocol that is missing: {log}"
    );
    assert!(
        log.contains("will not run as an ordinary window"),
        "and says why there is no fallback: {log}"
    );

    let observed = fixture.observed();
    assert_eq!(
        observed.layer_surfaces, 0,
        "nothing was created on a compositor that cannot place it"
    );
}

#[test]
fn the_panel_binds_the_seats_pointer_and_survives_its_events() {
    let fixture = Fixture::start(Plan::default());

    fixture.wait_for("the pointer to be bound", |observed| observed.pointers == 1);
    let placement = taskbar_placement();
    fixture.wait_for_frame(OUTPUT_WIDTH, points(placement.height));
    fixture.wait_for("a second frame after the pointer events", |observed| {
        observed.commits > 1
    });
    fixture.wait_for("the session to still be running", |_| {
        fixture.log().contains("wlr-layer-shell surface")
    });

    let observed = fixture.observed();

    assert_eq!(observed.pointers, 1, "the pointer was bound exactly once");
    assert!(!observed.acks.is_empty());

    fixture.signal(Signal::TERM);
    let status = fixture.wait_for_exit();
    assert!(
        status.success(),
        "pointer input is not a reason to stop: {:?}",
        fixture.log()
    );
}

/// A panel state holding one task, the way the render tests build one.
fn panel_with_a_task(surface: inox_config::PanelSurface) -> (State, WindowKey) {
    let mut shell = inox_config::ShellConfig::default();
    shell.panel.surface = surface;

    let localizer = crate::i18n::load(&inox_common::i18n::Locale::default_locale())
        .expect("the catalogs parse");
    let mut state = State::with_shell(shell, Tokens::builtin_dark(), std::rc::Rc::new(localizer));

    // The opening snapshot request is not what this test is about.
    state.take_outgoing();

    let key = WindowKey::new(WindowId::new(1), SessionGeneration::new(1));
    app::update(
        &mut state,
        Message::Snapshot {
            generation: SessionGeneration::new(1),
            windows: vec![(
                key,
                WindowState {
                    app_id: "firefox".to_owned(),
                    title: "Inox".to_owned(),
                    ..WindowState::default()
                },
            )],
        },
    );

    (state, key)
}

#[test]
fn a_pointer_press_on_a_task_tile_produces_the_activation_message() {
    // The wire cannot show this: the activation goes to a window-service
    // transport this milestone does not have. What it *can* be shown against is
    // the interface the binary drives, with a state that has a tile — the same
    // `Input::pointer` the `wl_pointer` handler calls, and the same
    // `Message::TaskActivated` the render tests assert on.
    let tokens = Tokens::builtin_dark();
    let (mut state, key) = panel_with_a_task(inox_config::PanelSurface::Taskbar);
    let geometry = crate::view::PanelGeometry::new(&tokens, state.config());
    let size = (1280, geometry.height as u32);

    // `Input` is the interface's pointer half: a scene projected from the state
    // and the entries a point is matched against. No renderer is involved, so
    // this asserts what a click *means* without an offscreen Qt application.
    let mut interface = Input::new(size.0, size.1);
    // The first entry starts at the strip's own padding; a click just inside it
    // is a click on that entry. Derived from the tokens rather than written as a
    // number, so a token change moves the click with the tile.
    let point = Point::new(geometry.padding + geometry.inset, geometry.height / 2.0);

    let entered = interface.pointer(&mut state, Pointer::Enter(point));
    assert!(entered.messages.is_empty(), "entering is not an activation");

    let pressed = interface.pointer(&mut state, Pointer::Press(Button::Left));
    assert!(
        pressed.messages.is_empty(),
        "the activation is a release, not a press"
    );

    let released = interface.pointer(&mut state, Pointer::Release(Button::Left));

    assert!(
        matches!(released.messages.as_slice(), [Message::TaskActivated { key: activated }] if *activated == key),
        "the click became the task activation the render tests assert on: {:?}",
        released.messages
    );

    let commands = state.take_outgoing();
    assert!(
        matches!(commands.as_slice(), [OutgoingCommand::Activate { window, .. }] if *window == key),
        "and the panel queued exactly one activation request: {commands:?}"
    );
}

#[test]
fn a_click_on_a_panel_with_no_task_asks_for_nothing() {
    let tokens = Tokens::builtin_dark();
    let (mut state, _) = panel_with_a_task(inox_config::PanelSurface::Taskbar);
    app::update(
        &mut state,
        Message::Snapshot {
            generation: SessionGeneration::new(1),
            windows: Vec::new(),
        },
    );
    let geometry = crate::view::PanelGeometry::new(&tokens, state.config());
    let mut interface = Input::new(1280, geometry.height as u32);
    let point = Point::new(geometry.padding, geometry.height / 2.0);

    let _ = interface.pointer(&mut state, Pointer::Enter(point));
    let _ = interface.pointer(&mut state, Pointer::Press(Button::Left));
    let released = interface.pointer(&mut state, Pointer::Release(Button::Left));

    assert!(
        released.messages.is_empty() && state.take_outgoing().is_empty(),
        "a click where there is no tile means nothing (D01)"
    );
}

/// The protocol's anchor bitfield for a placement.
fn anchor_bits(placement: &PanelPlacement) -> u32 {
    let anchors = &placement.anchors;
    let mut bits = 0;

    if anchors.contains(crate::placement::Anchor::Top) {
        bits |= 1;
    }
    if anchors.contains(crate::placement::Anchor::Bottom) {
        bits |= 2;
    }
    if anchors.contains(crate::placement::Anchor::Left) {
        bits |= 4;
    }
    if anchors.contains(crate::placement::Anchor::Right) {
        bits |= 8;
    }

    bits
}
