//! Smithay compositor state and protocol handlers.
//!
//! # Advertised globals
//!
//! Every global the compositor creates, and the version it binds at:
//!
//! - `wl_compositor` (6) and `wl_subcompositor` (1) — [`CompositorState`], which
//!   creates both in one call. Version 6 is what carries
//!   `preferred_buffer_scale`: without it a client cannot be told how large a
//!   buffer to render for a scaled output (ADR 0020).
//! - `wl_shm` (2) — [`ShmState`], the only buffer transport besides client-provided
//!   dmabufs, which are not advertised yet.
//! - `wl_seat` (9) — [`SeatState`], carrying the keyboard and pointer capabilities.
//! - `wl_output` (4) and `zxdg_output_manager_v1` (3) — one advertisement per
//!   output and [`OutputManagerState`].
//! - `xdg_wm_base` (6) — [`XdgShellState`], the surface role managed windows use.
//! - `zwlr_layer_shell_v1` (4) — [`WlrLayerShellState`], ordinary desktop layers.
//! - `wp_viewporter` (1) — [`ViewporterState`]. Required by XWayland clients that
//!   scale themselves (ADR 0007); without it `xwayland-satellite` cannot run here.
//! - `wp_fractional_scale_manager_v1` (1) — [`FractionalScaleManagerState`], the
//!   exact scale a surface should render at on an output whose factor is not a
//!   whole number (ADR 0020).
//! - `wl_data_device_manager` (3) — [`DataDeviceState`], the clipboard and
//!   drag-and-drop channel.
//! - `zwp_primary_selection_device_manager_v1` (1) — [`PrimarySelectionState`], the
//!   X11-style primary selection (middle-click paste).
//! - `ext_session_lock_manager_v1` (1) — [`SessionLockManagerState`], the secure
//!   session lock (ADR 0015, ADR 0022). Only a client that has claimed the trusted
//!   lock role over `inox_ipc` may take the lock; while a lock is held every output
//!   is composited from the locked scene alone and ordinary clients receive no
//!   input.
//! - `zwp_text_input_manager_v3` (1) — application text transactions (ADR 0023).
//! - `zwp_input_method_manager_v2` (1) and `zwp_virtual_keyboard_manager_v1` (1) —
//!   visible only to the compositor-launched input method connection (ADR 0023).
//! - `inox_ipc` — the private control protocol from `inox-protocols`, at the version
//!   its own interface declares.
//!
//! Client toplevels become managed windows: they are registered in a
//! [`Workspace`], tiled by [`LayoutConfig`] and mapped into a Smithay desktop
//! `Space` that [`InoxState::render_frame`] composites.
//!
//! # Outputs, scaling and the global coordinate space
//!
//! The compositor advertises one or more outputs (ADR 0019). Every output has a
//! position in a single global logical coordinate space, so a window, a popup and
//! the pointer all live in the same coordinates and a display to the left of the
//! origin simply has negative ones. Each output carries its own scale factor;
//! rendering, the cursor and layer placement convert to that output's physical
//! pixels, while hit testing, tiling and the work areas stay logical (ADR 0020).
//!
//! [`InoxState::apply_outputs`] is the single place the topology changes. An
//! output that survives keeps its `wl_output` global, its layer surfaces and its
//! windows; one that disappears ends its runtime instance and hands its windows
//! to the primary survivor.
//!
//! # Clipboard, drag-and-drop and focus
//!
//! Both selection protocols follow the keyboard focus: only the focused client is
//! offered the current selection, and only the focused client may set one (Smithay
//! enforces the latter inside the `wl_data_device` handler). `InoxState::sync_focus` is
//! the single place the keyboard focus changes, so it is also the single place the
//! selection focus is handed over — see the note there for why that is what makes the
//! XWayland clipboard bridge work.
//!
//! # The session lock
//!
//! The lock is a security boundary, not a window: while one is held, normal
//! clients contribute nothing to what is on screen and receive no input. Four
//! things enforce that, and each of them is the single place it could go wrong:
//!
//! - [`InoxState::scene_elements`] renders the locked scene (the lock surfaces and
//!   the opaque locked background) instead of the layer/window scene.
//! - [`InoxState::sync_focus`] hands the keyboard to a lock surface and clears the
//!   selection focus, so no ordinary client is granted the clipboard.
//! - [`InoxState::surface_under`] only hit-tests lock surfaces, so pointer events
//!   reach no ordinary client.
//! - [`InoxState::inject_key`] stops intercepting the compositor's own chords, so a
//!   locked session cannot be window-switched or closed from the keyboard.
//!
//! `locked` is not sent to the client until every advertised output has presented
//! a frame drawn from the locked scene — see [`crate::session_lock::LockCoverage`],
//! which is also what makes an output plugged in mid-lock covered before the
//! client is told the screen is its own.
//!
//! A lock client that dies keeps the session locked, and this compositor does not
//! let another client take the lock over. That is the fail-closed direction the
//! protocol allows and ADR 0015 requires; the recovery gap it leaves is recorded
//! in ADR 0022 rather than papered over.
//!
//! Input handling is split in two: Smithay's seat owns the keymap, modifier
//! tracking and focus delivery, while [`crate::input::action_for_key`] turns a
//! key press into a compositor [`Action`]. [`InoxState::inject_key`] and the
//! pointer injection methods feed that path, so keyboard and pointer events can
//! be exercised without a libinput backend (see ADR 0008 for M2, and ADR 0010
//! for the M3b libinput path that will call through the same Smithay seat).
//!
//! The Wayland bindings come from `inox-protocols`: the curated core `wl_*` types
//! and the `inox-ipc` control protocol, so upstream versions are pinned in one
//! place (see the workspace `AGENTS.md`).

mod ime;

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    os::unix::io::OwnedFd,
    sync::OnceLock,
    time::{Duration, Instant},
};

use calloop::LoopSignal;
use inox_protocols::{
    ipc::inox_ipc::server::inox_ipc::{
        InoxIpc, LockState as IpcLockState, ReloadResult, Request as InoxIpcRequest,
    },
    standard::{
        core::server::{
            wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface,
        },
        ext::session_lock::server::{
            ext_session_lock_manager_v1::ExtSessionLockManagerV1,
            ext_session_lock_surface_v1::ExtSessionLockSurfaceV1,
            ext_session_lock_v1::{self as lock_v1, ExtSessionLockV1},
        },
        wlr::layer_shell::server::{
            zwlr_layer_shell_v1::ZwlrLayerShellV1,
            zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
        },
    },
};
use smithay::{
    backend::{
        input::{ButtonState, KeyState},
        renderer::{
            element::{
                AsRenderElements, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::render_elements_from_surface_tree,
            },
            gles::{GlesError, GlesRenderer},
            utils::{on_commit_buffer_handler, with_renderer_surface_state},
        },
    },
    desktop::{
        LayerSurface as DesktopLayerSurface, PopupKind, PopupManager, Space, Window,
        WindowSurfaceType,
    },
    input::{
        Seat, SeatHandler, SeatState,
        keyboard::{FilterResult, KeyboardHandle, Keycode, XkbConfig},
        pointer::{
            AxisFrame, ButtonEvent, CursorImageStatus, CursorImageSurfaceData, MotionEvent,
            PointerHandle,
        },
    },
    output::{Mode, Output, PhysicalProperties, Scale as OutputScale, Subpixel},
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        backend::{ClientData, ClientId, DisconnectReason, GlobalId},
        protocol::wl_data_source::WlDataSource,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial, Size, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            send_surface_state, with_states,
        },
        fractional_scale::{
            FractionalScaleHandler, FractionalScaleManagerState, with_fractional_scale,
        },
        output::{OutputHandler, OutputManagerState},
        seat::WaylandFocus,
        selection::{
            SelectionHandler, SelectionSource, SelectionTarget,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                set_data_device_focus,
            },
            primary_selection::{
                PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
            },
        },
        session_lock::{
            ExtLockSurfaceUserData, LockSurface, SessionLockHandler, SessionLockManagerGlobalData,
            SessionLockManagerState, SessionLockState, SessionLocker,
        },
        shell::{
            wlr_layer::{
                KeyboardInteractivity, Layer, LayerSurface, LayerSurfaceCachedState,
                LayerSurfaceConfigure, WlrLayerShellGlobalData, WlrLayerShellHandler,
                WlrLayerShellState, WlrLayerSurfaceUserData,
            },
            xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState},
        },
        shm::{ShmHandler, ShmState},
        viewporter::ViewporterState,
    },
};
use tracing::{debug, error, info, warn};

use crate::{
    config::CompositorConfig,
    input::{Action, SwallowedPresses, action_for_key},
    layout::LayoutConfig,
    render::{HeadlessRenderer, RenderError, SceneElement},
    session_lock::{Claim, LockCoverage, claim},
    topology::{self, OutputId, Placement},
    workspace::{WindowId, Workspace},
};

/// Physical pixels assumed per inch when translating the headless mode into
/// millimetres for the advertised `wl_output` geometry.
const PIXELS_PER_INCH: i32 = 96;

/// Millimetres per inch, scaled by ten to keep the conversion in integer arithmetic.
const MILLIMETRES_PER_INCH_X10: i32 = 254;

/// How far up a surface tree [`InoxState::output_for_surface`] walks.
///
/// The protocol forbids parent cycles, so a deeper tree than this cannot occur;
/// the bound exists so a malformed one cannot spin the compositor.
const MAX_PARENT_DEPTH: usize = 16;

/// Errors produced while building the compositor state.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// An input value cannot be represented by the seat protocol.
    #[error("invalid input configuration: {0}")]
    InputConfig(String),
    /// The headless renderer could not be created.
    #[error("failed to create the headless renderer")]
    Renderer(#[from] RenderError),

    /// The seat keyboard could not be created, which happens when no XKB keymap
    /// is available (for example when the `xkb-data` package is missing).
    #[error("failed to create the seat keyboard")]
    Keyboard(#[from] smithay::input::keyboard::Error),
}

/// Everything the compositor holds for the session lock (ADR 0015, ADR 0022).
///
/// `client` is set the moment a lock request is honoured — which is *before* the
/// protocol's `locked` event goes out — because from that instant ordinary
/// content must neither be drawn nor interacted with. Confirming the lock is a
/// later, separate step: it waits for [`LockCoverage`] to report that every
/// advertised output has presented a frame drawn from the locked scene.
///
/// The state is deliberately not tied to the lock object's lifetime: a client
/// that dies leaves the session locked, which is what the protocol requires and
/// what keeps a crashing lock UI from unlocking the desktop.
#[derive(Default)]
struct SessionLock {
    /// The accepted lock object, kept as a liveness handle.
    object: Option<ExtSessionLockV1>,
    /// The connection that asked for the lock.
    client: Option<ClientId>,
    /// Whether `locked` has been sent for `object`.
    confirmed: bool,
    /// The outputs that still owe a locked frame.
    coverage: LockCoverage,
    /// The confirmation to use once the coverage is complete.
    pending: Option<SessionLocker>,
    /// Lock surfaces: the connection that created each one, and its output.
    surfaces: Vec<(ClientId, OutputId, LockSurface)>,
    /// The lock surface that holds the keyboard focus.
    focus: Option<WlSurface>,
    /// Whether the loss of the lock's connection has been reported already.
    owner_lost: bool,
}

impl SessionLock {
    /// Whether a lock has been accepted and not released.
    fn accepted(&self) -> bool {
        self.client.is_some()
    }

    /// Whether `lock` is the object that owns the current lock.
    fn holds(&self, lock: &ExtSessionLockV1) -> bool {
        self.object.as_ref() == Some(lock)
    }

    /// Release the lock, whatever the reason.
    fn release(&mut self) {
        self.object = None;
        self.client = None;
        self.confirmed = false;
        self.coverage.cancel();
        self.pending = None;
        self.surfaces.clear();
        self.focus = None;
        self.owner_lost = false;
    }
}

/// The compositor state, shared with every Smithay protocol handler.
pub struct InoxState {
    compositor_state: CompositorState,

    /// Owns the `zxdg_output_manager_v1` global advertised next to `wl_output`.
    ///
    /// Smithay's output dispatch is generic over the compositor state, so this value
    /// is never read: the compositor keeps it to own the global it created.
    #[allow(dead_code)]
    output_manager_state: OutputManagerState,

    /// Owns the `wp_viewporter` global clients scale their buffers with.
    ///
    /// Like `output_manager_state`, Smithay's viewporter dispatch is
    /// generic over the compositor state, so the value is never read: keeping it is
    /// what keeps the global alive.
    #[allow(dead_code)]
    viewporter_state: ViewporterState,

    /// Owns the `wl_data_device_manager` global and the per-seat selection state.
    data_device_state: DataDeviceState,

    /// Owns the `zwp_primary_selection_device_manager_v1` global.
    primary_selection_state: PrimarySelectionState,

    /// The display the compositor sends events on.
    ///
    /// Held so that focus changes can hand the selection focus to the newly focused
    /// client (see [`InoxState::sync_focus`]); the display itself is owned by the event
    /// loop, and this handle is cheap to clone.
    display_handle: DisplayHandle,

    /// The advertised outputs, primary first (ADR 0019).
    ///
    /// Empty only while no display is available: the session and its clients stay
    /// alive then, and the windows keep their identity until an output returns.
    outputs: Vec<OutputEntry>,
    /// Mints [`OutputId`]s, so an output that comes back is a new runtime
    /// instance even when it reuses a connector name.
    next_output_id: u64,
    /// Owns the `wp_fractional_scale_manager_v1` global (ADR 0020).
    ///
    /// Like [`InoxState::output_manager_state`], Smithay's dispatch is generic
    /// over the compositor state, so the value is never read: keeping it is what
    /// keeps the global alive.
    #[allow(dead_code)]
    fractional_scale_state: FractionalScaleManagerState,

    /// Owns the `ext_session_lock_manager_v1` global (ADR 0015, ADR 0022).
    session_lock_state: SessionLockManagerState,
    /// The token a client must present to be allowed to lock, or `None`.
    ///
    /// `None` means this run cannot be locked at all: a compositor with no token
    /// refuses every claim (ADR 0022 decision 3).
    lock_token: Option<String>,
    /// The connections that presented this session's token.
    lock_authorized: HashSet<ClientId>,
    /// The accepted session lock, if any.
    lock: SessionLock,
    /// The bound `inox_ipc` objects that observe the lock state.
    ipc_clients: Vec<InoxIpc>,
    /// Claims refused, exposed so the acceptance tests can assert on the refusals.
    lock_claim_refusals: u64,
    /// Lock requests refused, for the same reason.
    lock_refusals: u64,
    input_config: inox_config::InputConfig,
    cursor_image: CursorImageStatus,
    cursor_visible: bool,
    cursor_buffer: MemoryRenderBuffer,
    repaint: bool,
    background: [f32; 4],

    ime: ime::ImeState,
    seat_state: SeatState<Self>,
    seat: Seat<Self>,
    keyboard: KeyboardHandle<Self>,
    pointer: PointerHandle<Self>,

    xdg_shell_state: XdgShellState,
    layer_shell_state: WlrLayerShellState,
    layers: Vec<LayerEntry>,
    layer_focus: Option<WlSurface>,
    popups: PopupManager,
    shm_state: ShmState,

    /// The desktop space that owns the managed windows and their positions.
    space: Space<Window>,
    /// The tiling order and focus of the managed windows.
    workspace: Workspace,
    /// Maps workspace identifiers to the Smithay windows they stand for.
    windows: HashMap<WindowId, Window>,

    /// Tiling parameters, applied whenever the window set changes.
    layout: LayoutConfig,
    /// Last location reported for the pointer, in global logical coordinates.
    pointer_location: Point<f64, Logical>,

    /// Which bound chords this compositor has already swallowed the press of.
    ///
    /// See [`InoxState::inject_key`] for why the press has to be remembered.
    swallowed_presses: SwallowedPresses,

    renderer: Option<HeadlessRenderer>,
    /// Monotonic origin the frame callbacks are timed against.
    ///
    /// A monotonic clock, not wall-clock time: clients only compare frame times with
    /// each other, and `wl_surface.frame` timestamps must never jump backwards when the
    /// system clock is adjusted.
    start_time: Instant,
    frames_rendered: u64,
    exit_after_frames: Option<u64>,
    exit_requested: bool,
    loop_signal: LoopSignal,
}

/// One output the compositor should advertise (ADR 0019).
///
/// The backends describe *what* they found; the compositor decides *where* it
/// goes, which keeps placement and identity in one place instead of once per
/// backend.
#[derive(Debug, Clone)]
pub struct OutputDescriptor {
    /// The Smithay output to create or keep.
    ///
    /// An output that is already advertised is recognised by [`Self::key`] and,
    /// when the very same value comes back, keeps its global and its layer
    /// surfaces.
    pub output: Output,
    /// Position the user asked for, or `None` to place the output automatically.
    pub position: Option<Point<i32, Logical>>,
    /// Identity used to recognise this output across topology changes.
    ///
    /// Deliberately not the connector name on its own: a machine with two GPUs can
    /// have a `DP-1` on each, and keying on the name alone would collapse them
    /// into one output. The compositor treats this as opaque and lets the backend
    /// choose it — the headless backend uses the configured name, the DRM backend
    /// pairs the card with the connector.
    pub key: String,
}

impl OutputDescriptor {
    /// Describe `output` under its own name, for backends with no name collisions.
    pub fn named(output: Output, position: Option<Point<i32, Logical>>) -> Self {
        Self {
            key: output.name(),
            output,
            position,
        }
    }
}

/// One advertised output and the geometry it occupies.
#[derive(Debug, Clone)]
struct OutputEntry {
    /// Identity the backend reports for this output, used to recognise it again.
    key: String,
    /// Runtime identity, stable for as long as the output is advertised.
    id: OutputId,
    output: Output,
    /// Its `wl_output` global, present while the output is advertised.
    global: Option<GlobalId>,
    /// Top-left corner in the global logical coordinate space.
    position: Point<i32, Logical>,
    /// The output's full logical area, reported by the `Space`.
    area: Rectangle<i32, Logical>,
    /// The part of the area ordinary windows may tile into, after layer
    /// exclusive-zone reservations. Computed by [`InoxState::arrange_layers`].
    work_area: Rectangle<i32, Logical>,
}

struct StateSetup {
    outputs: Vec<OutputDescriptor>,
    renderer: Option<HeadlessRenderer>,
    input: inox_config::InputConfig,
    theme: inox_config::Theme,
    physical: bool,
}

fn rgba_floats(color: inox_config::Rgba) -> [f32; 4] {
    [color.r, color.g, color.b, color.a].map(|channel| f32::from(channel) / 255.0)
}

struct LayerEntry {
    /// The output this surface asked for or was assigned to.
    output: OutputId,
    surface: DesktopLayerSurface,
    /// Placement in the global logical coordinate space.
    geometry: Rectangle<i32, Logical>,
    committed: bool,
    mapped: bool,
    initial_configure: Option<Serial>,
    acknowledged: bool,
}

// Front to back, shared by rendering, hit testing and exclusive keyboard focus.
const LAYER_ORDER: [Layer; 4] = [Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background];

impl InoxState {
    /// Build the state, the headless outputs, the seat and the render target.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::Renderer`] when the EGL/GLES renderer cannot be
    /// created, which happens when no usable GL driver is available, and
    /// [`StateError::Keyboard`] when the XKB keymap cannot be compiled.
    pub fn new(
        display_handle: &DisplayHandle,
        config: &CompositorConfig,
        loop_signal: LoopSignal,
    ) -> Result<Self, StateError> {
        let setup = StateSetup {
            outputs: create_headless_outputs(config),
            renderer: Some(HeadlessRenderer::new()?),
            input: inox_config::InputConfig {
                keyboard_repeat_delay_ms: 200,
                ..Default::default()
            },
            theme: inox_config::Theme::builtin_dark(),
            physical: false,
        };
        Self::build(display_handle, config, loop_signal, setup)
    }

    /// Build the state for outputs the physical backend has already enumerated.
    pub(crate) fn new_physical(
        display_handle: &DisplayHandle,
        config: &CompositorConfig,
        loop_signal: LoopSignal,
        outputs: Vec<OutputDescriptor>,
        input: inox_config::InputConfig,
        theme: inox_config::Theme,
        layout: LayoutConfig,
    ) -> Result<Self, StateError> {
        let mut state = Self::build(
            display_handle,
            config,
            loop_signal,
            StateSetup {
                outputs,
                renderer: None,
                input,
                theme,
                physical: true,
            },
        )?;
        state.set_input_capabilities(false, false)?;
        state.layout = layout;
        Ok(state)
    }

    fn build(
        display_handle: &DisplayHandle,
        config: &CompositorConfig,
        loop_signal: LoopSignal,
        setup: StateSetup,
    ) -> Result<Self, StateError> {
        let StateSetup {
            outputs,
            renderer,
            input,
            theme,
            physical,
        } = setup;
        let repeat_delay = i32::try_from(input.keyboard_repeat_delay_ms).map_err(|_| {
            StateError::InputConfig("keyboard repeat delay exceeds i32::MAX".to_owned())
        })?;
        let repeat_rate = i32::try_from(input.keyboard_repeat_rate_hz).map_err(|_| {
            StateError::InputConfig("keyboard repeat rate exceeds i32::MAX".to_owned())
        })?;
        // `wl_compositor` v6 carries `preferred_buffer_scale`, which is how a
        // client is told how large a buffer to render for its output (ADR 0020).
        let compositor_state = CompositorState::new_v6::<Self>(display_handle);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(display_handle);
        let fractional_scale_state = FractionalScaleManagerState::new::<Self>(display_handle);
        // The secure session lock (ADR 0015, ADR 0022). The global is advertised
        // to every client on purpose: whether a client may *take* the lock is a
        // decision about a request, not about the global's existence, and the
        // protocol has a first-class way to refuse one (`finished`). Hiding the
        // global would also not work as the gate — a client that claims the role
        // reads the registry on the same connection, so the claim necessarily
        // follows the advertisement.
        let session_lock_state = SessionLockManagerState::new::<Self, _>(display_handle, |_| true);
        let ipc_version = inox_ipc_version();
        let ipc_global = display_handle.create_global::<Self, InoxIpc, ()>(ipc_version, ());
        debug!(
            ?ipc_global,
            version = ipc_version,
            "advertised inox_ipc global"
        );

        // XWayland clients scale their buffers through `wp_viewporter`, so a satellite
        // never gets far without it (ADR 0007).
        let viewporter_state = ViewporterState::new::<Self>(display_handle);
        let data_device_state = DataDeviceState::new::<Self>(display_handle);
        let primary_selection_state = PrimarySelectionState::new::<Self>(display_handle);

        let xdg_shell_state = XdgShellState::new::<Self>(display_handle);
        let layer_shell_state = WlrLayerShellState::new::<Self>(display_handle);
        let shm_state = ShmState::new::<Self>(display_handle, []);

        let mut seat_state = SeatState::<Self>::new();
        let mut seat = seat_state.new_wl_seat(display_handle, "seat-0");
        let keyboard = seat.add_keyboard(keyboard_config(&input), repeat_delay, repeat_rate)?;
        let pointer = seat.add_pointer();

        let space = Space::default();

        ime::init(display_handle);

        let mut state = Self {
            compositor_state,
            output_manager_state,
            viewporter_state,
            data_device_state,
            primary_selection_state,
            display_handle: display_handle.clone(),
            outputs: Vec::new(),
            next_output_id: 0,
            fractional_scale_state,
            session_lock_state,
            lock_token: config.lock_token.clone(),
            lock_authorized: HashSet::new(),
            lock: SessionLock::default(),
            ipc_clients: Vec::new(),
            lock_claim_refusals: 0,
            lock_refusals: 0,
            input_config: input,
            cursor_image: CursorImageStatus::default_named(),
            cursor_visible: false,
            cursor_buffer: crate::cursor::fallback(&theme),
            repaint: true,
            background: if physical {
                rgba_floats(theme.colors.background)
            } else {
                crate::render::CLEAR_COLOR
            },
            ime: ime::ImeState::default(),
            seat_state,
            seat,
            keyboard,
            pointer,
            xdg_shell_state,
            layer_shell_state,
            layers: Vec::new(),
            layer_focus: None,
            popups: PopupManager::default(),
            shm_state,
            space,
            workspace: Workspace::new(),
            windows: HashMap::new(),
            layout: LayoutConfig::default(),
            pointer_location: Point::from((0.0, 0.0)),
            swallowed_presses: SwallowedPresses::default(),
            renderer,
            start_time: Instant::now(),
            frames_rendered: 0,
            exit_after_frames: config.exit_after_frames,
            exit_requested: false,
            loop_signal,
        };
        state.apply_outputs(&outputs)?;

        info!(
            outputs = state.outputs.len(),
            physical, "created compositor outputs"
        );

        Ok(state)
    }

    /// Reconcile the advertised outputs with `desired` (ADR 0019).
    ///
    /// This is the single place the output topology changes. An output the
    /// caller keeps — matched by connector name and by the very same [`Output`]
    /// value — keeps its `wl_output` global, its layer surfaces and its windows,
    /// and only has its geometry refreshed. An output that disappears ends its
    /// runtime instance: its layer surfaces are told to close (their clients
    /// recreate them on an output that exists), its global is removed, and the
    /// windows that were on it move to the primary survivor rather than being
    /// left in a coordinate space nothing covers. Windows left over from a moment
    /// with no output at all are adopted by the primary one.
    ///
    /// Scale changes reach clients through `preferred_buffer_scale` on
    /// `wl_compositor` v6 and through `wp_fractional_scale_v1`.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::InvalidSize`] when an output has no mode or a
    /// non-positive one. The check runs before anything is touched, so a rejected
    /// topology leaves the previous one in place.
    pub fn apply_outputs(&mut self, desired: &[OutputDescriptor]) -> Result<(), RenderError> {
        for descriptor in desired {
            let usable = descriptor
                .output
                .current_mode()
                .is_some_and(|mode| mode.size.w > 0 && mode.size.h > 0);
            if !usable {
                return Err(RenderError::InvalidSize {
                    width: 0,
                    height: 0,
                });
            }
        }

        let placements: Vec<Placement> = desired
            .iter()
            .map(|descriptor| Placement {
                size: topology::logical_size(
                    descriptor
                        .output
                        .current_mode()
                        .map(|mode| mode.size)
                        .unwrap_or_default(),
                    descriptor.output.current_scale().fractional_scale(),
                ),
                position: descriptor.position,
            })
            .collect();
        let areas = topology::place(&placements);

        // Take the current table apart: an output that survives is handed over
        // unchanged, and every one that does not is left for teardown below.
        let mut previous: Vec<Option<OutputEntry>> = std::mem::take(&mut self.outputs)
            .into_iter()
            .map(Some)
            .collect();
        let mut next: Vec<OutputEntry> = Vec::with_capacity(desired.len());

        for (descriptor, area) in desired.iter().zip(areas.iter()) {
            let key = &descriptor.key;
            let found = previous
                .iter()
                .position(|slot| slot.as_ref().is_some_and(|entry| &entry.key == key));

            match found.and_then(|index| previous[index].take()) {
                Some(entry) if entry.output == descriptor.output => next.push(OutputEntry {
                    output: descriptor.output.clone(),
                    position: area.loc,
                    area: *area,
                    // A rescan that keeps the same connector keeps everything the
                    // client can see; only the geometry is refreshed.
                    ..entry
                }),
                Some(entry) => {
                    // Same connector, new device instance: the advertisement is
                    // rebuilt, so the surfaces bound to the old one come back on
                    // the new one. The windows stay where they are.
                    debug!(%key, "re-advertising a known output");
                    if let Some(global) = entry.global {
                        self.display_handle.remove_global::<Self>(global);
                    }
                    self.detach_layers_of(entry.id, &entry.output);
                    self.space.unmap_output(&entry.output);
                    next.push(OutputEntry {
                        key: key.clone(),
                        id: entry.id,
                        output: descriptor.output.clone(),
                        global: None,
                        position: area.loc,
                        area: *area,
                        work_area: *area,
                    });
                }
                None => next.push(OutputEntry {
                    key: key.clone(),
                    id: self.mint_output_id(),
                    output: descriptor.output.clone(),
                    global: None,
                    position: area.loc,
                    area: *area,
                    work_area: *area,
                }),
            }
        }

        let survivor = next.first().map(|entry| entry.id);
        for entry in previous.into_iter().flatten() {
            if let Some(global) = entry.global {
                self.display_handle.remove_global::<Self>(global);
            }
            self.workspace.reassign(Some(entry.id), survivor);
            self.detach_layers_of(entry.id, &entry.output);
            self.space.unmap_output(&entry.output);
            info!(output = %entry.key, "output removed");
        }

        for entry in &mut next {
            self.space.map_output(&entry.output, entry.position);
            if entry.global.is_none() {
                entry.global = Some(entry.output.create_global::<Self>(&self.display_handle));
            }
            info!(
                output = %entry.key,
                scale = entry.output.current_scale().fractional_scale(),
                position = ?entry.position,
                size = ?entry.area.size,
                "output advertised"
            );
        }

        self.outputs = next;
        // Windows that outlived every output come back with the first one.
        if let Some(survivor) = survivor {
            self.workspace.reassign(None, Some(survivor));
        }

        self.arrange_layers();
        self.arrange_windows();
        self.pointer_location = self.clamp_pointer(self.pointer_location);
        self.update_surface_scales();
        self.repaint = true;
        // An output that arrived while the session is locked must be covered
        // before the lock client is told the screen is its own; one that left
        // must stop blocking that confirmation. Both happen here because this is
        // the only place the topology changes (ADR 0019).
        self.sync_lock_coverage();
        self.note_lock_owner_loss();
        self.sync_focus();
        self.inject_pointer_motion(self.pointer_location);

        Ok(())
    }

    /// Replace the whole topology with at most one output (ADR 0019).
    ///
    /// The single-output entry point kept for callers that only ever deal with
    /// one display; it is exactly [`InoxState::apply_outputs`] with an empty list
    /// or a lone automatically placed output.
    ///
    /// # Errors
    /// As [`InoxState::apply_outputs`].
    pub fn replace_output(&mut self, output: Option<Output>) -> Result<(), RenderError> {
        let desired: Vec<OutputDescriptor> = output
            .into_iter()
            .map(|output| OutputDescriptor::named(output, None))
            .collect();

        self.apply_outputs(&desired)
    }

    /// Mint the next runtime output identity.
    fn mint_output_id(&mut self) -> OutputId {
        let id = OutputId::new(self.next_output_id);
        self.next_output_id = self.next_output_id.wrapping_add(1);

        id
    }

    /// The output with `id`, if it is still advertised.
    fn output_of(&self, id: OutputId) -> Option<&Output> {
        output_of(&self.outputs, id)
    }

    /// The entry advertising `output`, if this compositor advertises it.
    fn find_output(&self, output: &Output) -> Option<&OutputEntry> {
        self.outputs.iter().find(|entry| &entry.output == output)
    }

    /// Close and forget every layer surface bound to `output`.
    ///
    /// Clients recreate their surfaces on an output that still exists; the
    /// protocol has no way to migrate one, so `close` is the only correct answer
    /// (ADR 0019). The surface is told to leave first — which is why `leaving` is
    /// passed in rather than looked up: by the time this runs the output is no
    /// longer in the table, and the client still needs to hear about the output it
    /// is losing.
    fn detach_layers_of(&mut self, output: OutputId, leaving: &Output) {
        let (closing, keeping): (Vec<LayerEntry>, Vec<LayerEntry>) =
            std::mem::take(&mut self.layers)
                .into_iter()
                .partition(|entry| entry.output == output);
        self.layers = keeping;

        for entry in closing {
            entry.surface.layer_surface().send_close();
            entry
                .surface
                .with_surfaces(|surface, _| leaving.leave(surface));
            if self.layer_focus.as_ref() == Some(entry.surface.wl_surface()) {
                self.layer_focus = None;
            }
        }
    }

    /// The output the pointer is over, if any.
    fn pointer_output(&self) -> Option<OutputId> {
        let point = self.pointer_location;

        self.outputs
            .iter()
            .find(|entry| {
                entry.area.size.w > 0
                    && entry.area.size.h > 0
                    && point.x >= f64::from(entry.area.loc.x)
                    && point.x < f64::from(entry.area.loc.x.saturating_add(entry.area.size.w))
                    && point.y >= f64::from(entry.area.loc.y)
                    && point.y < f64::from(entry.area.loc.y.saturating_add(entry.area.size.h))
            })
            .map(|entry| entry.id)
    }

    /// The output a surface is shown on, following its parents if it has one.
    ///
    /// Sub-surfaces, popups and cursors hang off a toplevel or a layer surface,
    /// and the protocol lets them be created for any surface in the tree, so the
    /// answer has to be found by walking up rather than by looking the surface up
    /// directly.
    fn output_for_surface(&self, surface: &WlSurface) -> Option<OutputId> {
        let mut current = surface.clone();

        // The protocol forbids parent cycles; the bound is a belt-and-braces
        // guard so a malformed tree cannot spin here.
        for _ in 0..MAX_PARENT_DEPTH {
            let window = self
                .windows
                .iter()
                .find(|(_, window)| window_toplevel_surface(window) == Some(&current))
                .map(|(id, _)| *id);
            if let Some(id) = window
                && let Some(output) = self.workspace.output_of(id)
            {
                return Some(output);
            }
            if let Some(entry) = self
                .layers
                .iter()
                .find(|entry| entry.surface.wl_surface() == &current)
            {
                return Some(entry.output);
            }
            current = get_parent(&current).or_else(|| self.ime_popup_parent(&current).cloned())?;
        }

        None
    }

    /// Tell a surface the scale it should render at on its output (ADR 0020).
    ///
    /// Both halves are needed: `preferred_buffer_scale` tells a client that only
    /// understands integer scales how large a buffer to produce, and
    /// `wp_fractional_scale_v1` carries the exact factor. Neither is binding — a
    /// client may ignore both — so the compositor must keep compositing whatever
    /// buffer size it is handed, which it does.
    fn update_surface_scale(&self, surface: &WlSurface) {
        let scale = self
            .output_for_surface(surface)
            .and_then(|id| self.output_of(id))
            .or_else(|| self.outputs.first().map(|entry| &entry.output))
            .map(|output| output.current_scale())
            .unwrap_or(OutputScale::Integer(1));

        with_states(surface, |states| {
            send_surface_state(surface, states, scale.integer_scale(), Transform::Normal);
            with_fractional_scale(states, |state| {
                state.set_preferred_scale(scale.fractional_scale());
            });
        });
    }

    /// Refresh the advertised scale of every surface this compositor shows.
    ///
    /// Called whenever an output's scale can have changed, which is exactly when
    /// the topology changes; individual surfaces are refreshed on commit.
    fn update_surface_scales(&self) {
        self.ime_update_scales();
        for window in self.windows.values() {
            if let Some(surface) = window_toplevel_surface(window) {
                self.update_surface_scale(surface);
            }
        }
        for entry in &self.layers {
            self.update_surface_scale(entry.surface.wl_surface());
        }
    }

    /// Render one frame into every headless output's own render target.
    ///
    /// The desktop space is refreshed so that closed windows are dropped, and
    /// each output composites the part of the space its geometry covers, at that
    /// output's scale. Every mapped window is told that a frame happened.
    ///
    /// # Errors
    ///
    /// Returns a [`RenderError`] when the GLES rendering fails.
    pub fn render_frame(&mut self) -> Result<(), RenderError> {
        let mut renderer = self.renderer.take().ok_or(RenderError::NotHeadless)?;
        let result = self.render_outputs(&mut renderer);
        self.renderer = Some(renderer);
        result?;
        self.frame_presented();
        Ok(())
    }

    /// Composite every advertised output, in advertisement order.
    fn render_outputs(&mut self, renderer: &mut HeadlessRenderer) -> Result<(), RenderError> {
        let outputs: Vec<(String, (i32, i32), f64, Output)> = self
            .outputs
            .iter()
            .map(|entry| {
                let size = entry
                    .output
                    .current_mode()
                    .map(|mode| mode.size)
                    .unwrap_or_default();

                (
                    entry.output.name(),
                    (size.w, size.h),
                    entry.output.current_scale().fractional_scale(),
                    entry.output.clone(),
                )
            })
            .collect();

        for (name, size, scale, output) in outputs {
            let elements = self
                .scene_elements(renderer.renderer_mut(), &output)
                .map_err(RenderError::from)?;
            renderer.render_elements(&name, size, scale, &elements)?;
        }

        Ok(())
    }

    /// The render elements that make up `output`, front to back.
    ///
    /// Every position is expressed in that output's physical pixels, relative to
    /// its own top-left corner, which is what `OutputDamageTracker` and the
    /// headless target both expect. The scale is the output's, so a window on a
    /// 150 % display is composited from logical geometry into a 1.5x larger
    /// buffer instead of being stretched afterwards (ADR 0020).
    pub(crate) fn scene_elements(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
    ) -> Result<Vec<SceneElement>, GlesError> {
        self.space.refresh();
        self.popups.cleanup();
        self.ime_update_popups();

        let Some(entry) = self.outputs.iter().find(|entry| &entry.output == output) else {
            return Ok(Vec::new());
        };
        let geometry = entry.area;
        let id = entry.id;
        let scale = output.current_scale().fractional_scale();
        let mut elements = Vec::new();

        // The pointer is drawn once, on the output it is actually over; drawing
        // it on every output would leave ghosts on the others.
        if self.cursor_visible && self.pointer_output() == Some(id) {
            match &self.cursor_image {
                CursorImageStatus::Hidden => {}
                CursorImageStatus::Surface(surface) if surface.is_alive() => {
                    let hotspot = with_states(surface, |states| {
                        states
                            .data_map
                            .get::<CursorImageSurfaceData>()
                            .and_then(|data| data.lock().ok().map(|data| data.hotspot))
                    })
                    .unwrap_or_default();
                    let location =
                        (self.pointer_location - hotspot.to_f64() - geometry.loc.to_f64())
                            .to_physical_precise_round(scale);
                    elements.extend(render_elements_from_surface_tree(
                        renderer,
                        surface,
                        location,
                        scale,
                        1.0,
                        Kind::Cursor,
                    ));
                }
                _ => {
                    let location = (self.pointer_location - geometry.loc.to_f64())
                        .to_physical_precise_round(scale);
                    elements.push(SceneElement::Cursor(
                        MemoryRenderBufferRenderElement::from_buffer(
                            renderer,
                            location,
                            &self.cursor_buffer,
                            None,
                            None,
                            None,
                            Kind::Cursor,
                        )?,
                    ));
                }
            }
        }

        // The locked scene is the lock surfaces and nothing else. Composing this
        // output is also what marks it covered for `LockCoverage`, which is what
        // the `locked` event waits for: from here on, no frame this output shows
        // can contain an ordinary client's pixels.
        if self.lock.accepted() {
            self.note_lock_owner_loss();
            self.lock_render_elements(renderer, id, geometry, scale, &mut elements);
            self.lock.coverage.presented(id);

            return Ok(elements);
        }

        self.ime_render_elements(renderer, geometry, scale, &mut elements);
        for layer in [Layer::Overlay, Layer::Top] {
            self.layer_render_elements(layer, id, geometry, scale, renderer, &mut elements);
        }
        elements.extend(
            self.space
                .render_elements_for_region(renderer, &geometry, scale, 1.0)
                .into_iter()
                .map(SceneElement::Surface),
        );
        for layer in [Layer::Bottom, Layer::Background] {
            self.layer_render_elements(layer, id, geometry, scale, renderer, &mut elements);
        }

        Ok(elements)
    }

    fn layer_render_elements(
        &self,
        layer: Layer,
        output: OutputId,
        region: Rectangle<i32, Logical>,
        scale: f64,
        renderer: &mut GlesRenderer,
        elements: &mut Vec<SceneElement>,
    ) {
        for entry in self.layers.iter().rev().filter(|entry| {
            entry.mapped && entry.output == output && entry.surface.layer() == layer
        }) {
            elements.extend(entry.surface.render_elements::<SceneElement>(
                renderer,
                (entry.geometry.loc - region.loc).to_physical_precise_round(scale),
                scale.into(),
                1.0,
            ));
        }
    }

    /// Whether the session is locked.
    ///
    /// True from the moment a lock is accepted, which is *before* the protocol's
    /// `locked` event goes out: the outputs are blanked first, so the two moments
    /// in which ordinary content could still be interacted with are covered by
    /// the same flag.
    pub fn session_locked(&self) -> bool {
        self.lock.accepted()
    }

    /// How many lock-role claims this compositor refused.
    pub fn lock_claim_refusals(&self) -> u64 {
        self.lock_claim_refusals
    }

    /// How many lock requests this compositor refused.
    pub fn lock_refusals(&self) -> u64 {
        self.lock_refusals
    }

    /// Render the locked scene for one output.
    ///
    /// Only the lock surfaces: no layer surface, no window, no popup. Whatever is
    /// left is the opaque background the renderer clears to, which is the
    /// protocol's "blank all outputs with an opaque color such that their normal
    /// content is fully hidden" — including on an output the lock client has not
    /// drawn on yet.
    fn lock_render_elements(
        &self,
        renderer: &mut GlesRenderer,
        output: OutputId,
        region: Rectangle<i32, Logical>,
        scale: f64,
        elements: &mut Vec<SceneElement>,
    ) {
        let origin = Point::from((-region.loc.x, -region.loc.y)).to_physical_precise_round(scale);
        for (client, id, surface) in &self.lock.surfaces {
            // Only the connection that owns the lock may put pixels on a locked
            // screen: Smithay creates a lock surface object for any client that
            // asks, so the ownership check has to happen here.
            if Some(client) != self.lock.client.as_ref() || *id != output || !surface.alive() {
                continue;
            }
            elements.extend(render_elements_from_surface_tree(
                renderer,
                surface.wl_surface(),
                origin,
                scale,
                1.0,
                Kind::Unspecified,
            ));
        }
    }

    /// Release the session lock and restore the ordinary scene.
    fn release_lock(&mut self, reason: &str) {
        if !self.lock.accepted() {
            return;
        }
        self.lock.release();
        info!(reason, "session lock released");
        self.repaint = true;
        self.sync_focus();
        self.inject_pointer_motion(self.pointer_location);
        self.broadcast_lock_state();
    }

    /// Keep the lock's coverage in step with the advertised outputs.
    fn sync_lock_coverage(&mut self) {
        if !self.lock.accepted() || self.lock.confirmed {
            return;
        }
        let ids: Vec<OutputId> = self.outputs.iter().map(|entry| entry.id).collect();
        self.lock.coverage.sync(ids.iter().copied());
        self.lock
            .surfaces
            .retain(|(_, id, surface)| ids.contains(id) && surface.alive());
    }

    /// Confirm the lock once every output has composed a locked frame.
    ///
    /// The protocol forbids sending `locked` earlier, because a client told it
    /// owns the screen may suspend the machine straight away.
    fn settle_lock(&mut self) {
        if !self.lock.accepted() || self.lock.confirmed {
            return;
        }
        if !self.lock.coverage.take_completed() {
            return;
        }
        self.lock.confirmed = true;
        info!(
            outputs = self.outputs.len(),
            "every output is covered; confirming the session lock"
        );
        if let Some(locker) = self.lock.pending.take() {
            locker.lock();
        }
    }

    /// The lock state as `inox_ipc` reports it.
    fn lock_state_for_ipc(&self) -> IpcLockState {
        if self.lock.accepted() {
            IpcLockState::Locked
        } else {
            IpcLockState::Unlocked
        }
    }

    /// Tell every bound `inox_ipc` client whether the session is locked.
    ///
    /// The compositor owns the lock, so it is the authority observers such as the
    /// idle daemon and the notification daemon read; the event grants them
    /// nothing, which is why it can be a broadcast (ADR 0022 decision 4).
    fn broadcast_lock_state(&mut self) {
        let state = self.lock_state_for_ipc();
        self.ipc_clients.retain(InoxIpc::is_alive);
        for client in &self.ipc_clients {
            client.session_lock_state(state);
        }
    }

    /// Report once that the connection owning the lock is gone.
    ///
    /// The session stays locked (protocol and ADR 0015), so this is the only
    /// signal that something needs a human: nothing can unlock it again.
    fn note_lock_owner_loss(&mut self) {
        if !self.lock.accepted() || self.lock.owner_lost {
            return;
        }
        let alive = self
            .lock
            .object
            .as_ref()
            .is_some_and(ExtSessionLockV1::is_alive);
        if alive {
            return;
        }
        self.lock.owner_lost = true;
        error!(
            "the lock client is gone while the session is locked: the session stays locked and \
             this compositor does not hand the lock to another client (ADR 0022)"
        );
    }

    pub(crate) fn take_repaint(&mut self) -> bool {
        std::mem::take(&mut self.repaint)
    }
    pub(crate) fn background(&self) -> [f32; 4] {
        self.background
    }
    pub(crate) fn frame_presented(&mut self) {
        self.frames_rendered = self.frames_rendered.saturating_add(1);
        self.settle_lock();
        self.send_frame_callbacks();
    }
    pub(crate) fn frame_callbacks(&mut self) {
        self.send_frame_callbacks();
    }
    pub(crate) fn frame_budget(&self) -> bool {
        self.exit_after_frames.is_some()
    }

    /// Send a `wl_surface.frame` callback to every mapped surface.
    ///
    /// Without this a client that waits for its frame callback before drawing the next
    /// frame never gets one, so it never repaints: a real client (an XWayland toplevel,
    /// a GTK application) that asks for a frame first would stay blank. Note that the
    /// callbacks only reach the client once the display is flushed, which the event loop
    /// does after every frame it renders.
    ///
    /// Each surface is told about the output it is on, which is the output whose
    /// frame it is waiting for.
    fn send_frame_callbacks(&mut self) {
        let time = self.start_time.elapsed();
        let outputs: Vec<(OutputId, Output)> = self
            .outputs
            .iter()
            .map(|entry| (entry.id, entry.output.clone()))
            .collect();
        let windows: Vec<(WindowId, Window)> = self
            .windows
            .iter()
            .map(|(id, window)| (*id, window.clone()))
            .collect();

        for (id, output) in &outputs {
            self.ime_send_frames(output, time);
            // The closures report the output the surface is actually on, so the
            // throttle never gates a callback; `ZERO` states that rather than
            // leaving it to depend on the output comparison alone.
            for window_id in self.workspace.windows_on(*id) {
                if let Some((_, window)) = windows.iter().find(|(id, _)| *id == window_id) {
                    window.send_frame(output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
                }
            }
            for entry in self
                .layers
                .iter()
                .filter(|entry| entry.mapped && entry.output == *id)
            {
                entry
                    .surface
                    .send_frame(output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
            }
        }
    }

    /// Number of frames rendered so far.
    pub fn frames_rendered(&self) -> u64 {
        self.frames_rendered
    }

    /// Whether the configured frame budget has been rendered.
    ///
    /// Always `false` when no budget was configured.
    pub fn frame_budget_reached(&self) -> bool {
        budget_reached(self.exit_after_frames, self.frames_rendered)
    }

    /// Read back the centre pixel of the render target, as RGBA bytes.
    ///
    /// Exposed so that headless integration tests can verify that a frame really
    /// reached the render target.
    ///
    /// # Errors
    ///
    /// Returns a [`RenderError`] when the readback fails.
    pub fn read_frame_center_pixel(&mut self) -> Result<[u8; 4], RenderError> {
        let name = self.primary_output_name();
        self.renderer
            .as_mut()
            .ok_or(RenderError::NotHeadless)?
            .read_center_pixel(&name)
    }

    /// Read back the whole render target as tightly packed RGBA8 bytes.
    ///
    /// The format the PPM writer of the test-input channel consumes; the pixel
    /// integration tests use [`InoxState::read_frame_center_pixel`] instead.
    ///
    /// # Errors
    ///
    /// Returns a [`RenderError`] when the readback fails.
    pub fn read_frame_rgba(&mut self) -> Result<Vec<u8>, RenderError> {
        let name = self.primary_output_name();
        self.renderer
            .as_mut()
            .ok_or(RenderError::NotHeadless)?
            .read_frame_rgba(&name)
    }

    /// Name of the primary output, or an empty string when none is advertised.
    ///
    /// The readback accessors key on it, and a read with no output advertised
    /// reports [`RenderError::UnknownOutput`] rather than a stale frame.
    fn primary_output_name(&self) -> String {
        self.outputs
            .first()
            .map(|entry| entry.output.name())
            .unwrap_or_default()
    }

    /// Size of the primary output in pixels, as `(width, height)`.
    ///
    /// `(0, 0)` while no output is advertised.
    pub fn output_size(&self) -> (i32, i32) {
        self.outputs
            .first()
            .and_then(|entry| entry.output.current_mode())
            .map(|mode| (mode.size.w, mode.size.h))
            .unwrap_or((0, 0))
    }

    /// The logical rectangle the primary output leaves to ordinary windows.
    ///
    /// The primary output is the first advertised one, which is the only output
    /// every earlier milestone ever had.
    pub fn work_area(&self) -> Rectangle<i32, Logical> {
        self.outputs
            .first()
            .map(|entry| entry.work_area)
            .unwrap_or_default()
    }

    /// The work area of every advertised output, in advertisement order.
    pub fn work_areas(&self) -> Vec<(OutputId, Rectangle<i32, Logical>)> {
        self.outputs
            .iter()
            .map(|entry| (entry.id, entry.work_area))
            .collect()
    }

    /// Number of layer surfaces with a committed buffer.
    pub fn layer_count(&self) -> usize {
        self.layers.iter().filter(|entry| entry.mapped).count()
    }

    /// The primary output, or `None` while no display is available.
    pub fn output(&self) -> Option<&Output> {
        self.outputs.first().map(|entry| &entry.output)
    }

    /// Every advertised output, primary first.
    pub fn outputs(&self) -> impl Iterator<Item = &Output> {
        self.outputs.iter().map(|entry| &entry.output)
    }

    /// How many outputs are advertised.
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// Whether at least one output is currently available and advertised.
    pub fn output_active(&self) -> bool {
        !self.outputs.is_empty()
    }

    /// The bounding box of every output, in global logical coordinates.
    ///
    /// Absolute input devices (tablets, touchscreens) map their unit range onto
    /// this box, so one device covers the whole arrangement.
    pub fn input_area(&self) -> Rectangle<i32, Logical> {
        let areas: Vec<Rectangle<i32, Logical>> =
            self.outputs.iter().map(|entry| entry.area).collect();

        topology::bounds(&areas)
    }

    /// The output a managed window is tiled on, if it has one.
    pub fn window_output(&self, id: WindowId) -> Option<OutputId> {
        self.workspace.output_of(id)
    }

    /// Apply capabilities enumerated at the input-device boundary.
    ///
    /// Headless tests can drive this same path without mocking seat logic.
    ///
    /// # Errors
    /// Returns an error when the configured keyboard cannot be created.
    pub fn set_input_capabilities(
        &mut self,
        keyboard: bool,
        pointer: bool,
    ) -> Result<(), StateError> {
        if keyboard && self.seat.get_keyboard().is_none() {
            let delay = i32::try_from(self.input_config.keyboard_repeat_delay_ms)
                .map_err(|_| StateError::InputConfig("repeat delay exceeds i32::MAX".to_owned()))?;
            let rate = i32::try_from(self.input_config.keyboard_repeat_rate_hz)
                .map_err(|_| StateError::InputConfig("repeat rate exceeds i32::MAX".to_owned()))?;
            self.keyboard =
                self.seat
                    .add_keyboard(keyboard_config(&self.input_config), delay, rate)?;
        } else if !keyboard && self.seat.get_keyboard().is_some() {
            self.reset_keyboard();
            let handle = self.keyboard.clone();
            handle.set_focus(self, None, SERIAL_COUNTER.next_serial());
            self.seat.remove_keyboard();
        }
        if pointer && self.seat.get_pointer().is_none() {
            self.pointer = self.seat.add_pointer();
            self.cursor_image = CursorImageStatus::default_named();
        } else if !pointer && self.seat.get_pointer().is_some() {
            let handle = self.pointer.clone();
            handle.unset_grab(
                self,
                SERIAL_COUNTER.next_serial(),
                smithay::utils::Clock::<smithay::utils::Monotonic>::new()
                    .now()
                    .as_millis(),
            );
            let event = MotionEvent {
                location: self.pointer_location,
                serial: SERIAL_COUNTER.next_serial(),
                time: smithay::utils::Clock::<smithay::utils::Monotonic>::new()
                    .now()
                    .as_millis(),
            };
            handle.motion(self, None, &event);
            handle.frame(self);
            self.seat.remove_pointer();
        }
        self.cursor_visible = pointer;
        self.repaint = true;
        self.sync_focus();
        Ok(())
    }

    fn reset_keyboard(&mut self) {
        if self.seat.get_keyboard().is_none() {
            return;
        }
        for key in self.keyboard.pressed_keys() {
            self.inject_key(key, KeyState::Released);
        }
        let keyboard = self.keyboard.clone();
        keyboard.unset_grab(self);
    }

    pub(crate) fn reset_input(&mut self) {
        self.reset_keyboard();
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.unset_grab(
                self,
                SERIAL_COUNTER.next_serial(),
                smithay::utils::Clock::<smithay::utils::Monotonic>::new()
                    .now()
                    .as_millis(),
            );
        }
    }

    /// Move a pointer position into the union of the output areas.
    ///
    /// Unplugging the display the pointer was on leaves it outside every output,
    /// so the position is pulled back onto the closest one instead of being
    /// dropped somewhere nothing can be clicked (ADR 0019).
    pub(crate) fn clamp_pointer(&self, location: Point<f64, Logical>) -> Point<f64, Logical> {
        let areas: Vec<Rectangle<i32, Logical>> =
            self.outputs.iter().map(|entry| entry.area).collect();

        topology::clamp(location, &areas)
    }

    pub(crate) fn inject_pointer_axis(&mut self, frame: AxisFrame) {
        if self.seat.get_pointer().is_none() {
            return;
        }
        let pointer = self.pointer.clone();
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    /// The seat advertised to clients, carrying the keyboard and pointer capabilities.
    pub fn seat(&self) -> &Seat<Self> {
        &self.seat
    }

    /// Whether a clean shutdown has been requested.
    pub fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    /// Ask the event loop to stop after the current iteration.
    ///
    /// Repeated requests are ignored so that a second signal cannot restart the
    /// shutdown sequence.
    pub fn request_exit(&mut self) {
        if !self.exit_requested {
            self.exit_requested = true;
            self.loop_signal.stop();
        }
    }

    /// The managed windows in tiling order, master first.
    pub fn managed_windows(&self) -> Vec<WindowId> {
        self.workspace.windows()
    }

    /// Number of managed windows.
    pub fn window_count(&self) -> usize {
        self.workspace.len()
    }

    /// The focused window, if any.
    pub fn focused_window(&self) -> Option<WindowId> {
        self.workspace.focused()
    }

    /// The tiling parameters in use.
    pub fn layout(&self) -> LayoutConfig {
        self.layout
    }

    /// The surface currently holding the keyboard focus, if any.
    pub fn keyboard_focus(&self) -> Option<WlSurface> {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
    }

    /// The last location reported for the pointer, in workspace coordinates.
    pub fn pointer_location(&self) -> Point<f64, Logical> {
        self.pointer_location
    }

    /// The location a managed window is mapped at inside the workspace, if any.
    pub fn window_location(&self, id: WindowId) -> Option<Point<i32, Logical>> {
        let window = self.windows.get(&id)?;
        self.space.element_location(window)
    }

    /// Feed a key press or release into the seat.
    ///
    /// Returns the compositor [`Action`] the key resolved to, or `None` when the
    /// event was not bound (and was forwarded to the focused client) or when the
    /// release of a swallowed chord arrived.
    ///
    /// Smithay runs the filter for a release exactly as it does for a press, and it
    /// resolves the binding from the modifier state of the moment, which can change
    /// between the two halves of one chord. So the press decides whether the chord is
    /// the compositor's, [`SwallowedPresses`] carries that decision to the release, and
    /// only a press can trigger an [`Action`].
    pub fn inject_key(&mut self, keycode: Keycode, state: KeyState) -> Option<Action> {
        self.seat.get_keyboard()?;
        self.ime_sync_grab();
        let keyboard = self.keyboard.clone();
        let serial = SERIAL_COUNTER.next_serial();

        let action = keyboard
            .input(
                self,
                keycode,
                state,
                serial,
                smithay::utils::Clock::<smithay::utils::Monotonic>::new()
                    .now()
                    .as_millis(),
                |data, modifiers, keysym| {
                    // A locked session has no compositor chords: every key belongs
                    // to the lock surface. Without this, the desktop's own
                    // bindings would still switch windows or close them behind a
                    // locked screen.
                    if data.lock.accepted() {
                        return FilterResult::Forward;
                    }
                    let action = match state {
                        KeyState::Pressed => action_for_key(keysym.modified_sym(), modifiers),
                        KeyState::Released => None,
                    };

                    if data
                        .swallowed_presses
                        .swallow(keycode, state, action.is_some())
                    {
                        FilterResult::Intercept(action)
                    } else {
                        FilterResult::Forward
                    }
                },
            )
            .flatten();

        if let Some(action) = action {
            self.perform(action);
        }

        action
    }

    /// Move the pointer to `location` and deliver the resulting enter/motion events.
    pub fn inject_pointer_motion(&mut self, location: Point<f64, Logical>) {
        self.repaint |= self.cursor_visible && self.pointer_location != location;
        self.pointer_location = location;
        if self.seat.get_pointer().is_none() {
            return;
        }
        let focus = self.surface_under(location);

        let pointer = self.pointer.clone();
        let serial = SERIAL_COUNTER.next_serial();
        let event = MotionEvent {
            location,
            serial,
            time: smithay::utils::Clock::<smithay::utils::Monotonic>::new()
                .now()
                .as_millis(),
        };

        pointer.motion(self, focus, &event);
        pointer.frame(self);
    }

    /// Deliver a pointer button press or release at the current location.
    pub fn inject_pointer_button(&mut self, button: u32, state: ButtonState) {
        if self.seat.get_pointer().is_none() {
            return;
        }
        if state == ButtonState::Pressed && !self.pointer.is_grabbed() {
            self.focus_at_pointer();
        }
        let pointer = self.pointer.clone();
        let serial = SERIAL_COUNTER.next_serial();
        let event = ButtonEvent {
            serial,
            time: smithay::utils::Clock::<smithay::utils::Monotonic>::new()
                .now()
                .as_millis(),
            button,
            state,
        };

        pointer.button(self, &event);
        pointer.frame(self);
    }

    /// Apply a compositor action triggered by a key binding.
    fn perform(&mut self, action: Action) {
        match action {
            Action::FocusNext => {
                self.layer_focus = None;
                self.workspace.focus_next();
                self.sync_focus();
            }
            Action::FocusPrev => {
                self.layer_focus = None;
                self.workspace.focus_prev();
                self.sync_focus();
            }
            Action::Quit => self.request_exit(),
            Action::SwitchVt(_) => {}
        }
    }

    /// Register a freshly created toplevel as a managed window.
    ///
    /// A new window lands on the output under the pointer, falling back to the
    /// primary one: that is the display the user was looking at when the window
    /// appeared.
    fn add_window(&mut self, surface: ToplevelSurface) {
        let Some(output) = self
            .pointer_output()
            .or_else(|| self.outputs.first().map(|e| e.id))
        else {
            // No display at all: the window is still registered so it can be shown
            // as soon as one comes back.
            let id = self.workspace.add(OutputId::new(u64::MAX));
            self.workspace.assign(id, None);
            self.windows.insert(id, Window::new_wayland_window(surface));

            return;
        };

        let id = self.workspace.add(output);
        self.windows.insert(id, Window::new_wayland_window(surface));

        info!(
            id = id.get(),
            windows = self.workspace.len(),
            "mapped toplevel"
        );

        self.arrange_windows();
        self.sync_focus();
    }

    /// Forget the window built from a destroyed toplevel.
    fn remove_window(&mut self, surface: &ToplevelSurface) {
        let Some(id) = self.window_id_for_surface(surface.wl_surface()) else {
            return;
        };

        if let Some(window) = self.windows.remove(&id) {
            self.space.unmap_elem(&window);
        }
        self.workspace.remove(id);

        info!(
            id = id.get(),
            windows = self.workspace.len(),
            "unmapped toplevel"
        );

        self.arrange_windows();
        self.sync_focus();
    }

    /// Identifier of the managed window built from `surface`, if any.
    fn window_id_for_surface(&self, surface: &WlSurface) -> Option<WindowId> {
        self.windows
            .iter()
            .find(|(_, window)| window_toplevel_surface(window) == Some(surface))
            .map(|(id, _)| *id)
    }

    /// Suggest each managed window its tiled geometry and map it into the space.
    ///
    /// Every output tiles its own windows inside its own work area, in the global
    /// coordinate space, so a window on the second display is placed next to the
    /// first display's area rather than on top of it (ADR 0019).
    fn arrange_windows(&mut self) {
        self.repaint = true;
        let work_areas: Vec<(OutputId, Rectangle<i32, Logical>)> = self
            .outputs
            .iter()
            .map(|entry| (entry.id, entry.work_area))
            .collect();
        let tiles = self.workspace.arrange(&work_areas, self.layout);
        let mut mapped: Vec<Window> = Vec::with_capacity(tiles.len());

        for (id, tile) in &tiles {
            let Some(window) = self.windows.get(id).cloned() else {
                continue;
            };

            if let Some(toplevel) = window.toplevel() {
                let size = clamp_size(tile.size);
                toplevel.with_pending_state(|state| {
                    state.size = Some(size);
                });
                toplevel.send_pending_configure();
            }

            self.space.map_element(window.clone(), tile.loc, false);
            mapped.push(window);
        }

        let stale: Vec<Window> = self
            .space
            .elements()
            .filter(|window| !mapped.contains(window))
            .cloned()
            .collect();
        for window in stale {
            self.space.unmap_elem(&window);
        }

        self.space.refresh();
    }

    /// Point the keyboard focus at the focused window and activate it in the space.
    fn sync_focus(&mut self) {
        let exclusive = LAYER_ORDER[..2].iter().find_map(|layer| {
            self.layers
                .iter()
                .rev()
                .find(|entry| {
                    entry.mapped
                        && entry.surface.layer() == *layer
                        && entry.surface.cached_state().keyboard_interactivity
                            == KeyboardInteractivity::Exclusive
                })
                .map(|entry| entry.surface.wl_surface().clone())
        });
        let on_demand = self
            .layer_focus
            .as_ref()
            .filter(|surface| {
                self.layers.iter().any(|entry| {
                    entry.mapped
                        && entry.surface.wl_surface() == *surface
                        && entry.surface.can_receive_keyboard_focus()
                })
            })
            .cloned();
        // While the session is locked the keyboard belongs to a lock surface and
        // to nothing else. The candidates above are discarded, not merged: a layer
        // surface asking for exclusive focus, or the workspace's own idea of the
        // focused window, must not survive a lock.
        let focused = if self.lock.accepted() {
            self.lock.focus.clone().filter(|surface| surface.is_alive())
        } else {
            exclusive.or(on_demand).or_else(|| {
                self.workspace
                    .focused()
                    .and_then(|id| self.windows.get(&id))
                    .and_then(|window| window.wl_surface())
                    .map(Cow::into_owned)
            })
        };

        // The selection protocols follow the keyboard focus: a client is only offered the
        // clipboard (and only allowed to set one) while it is focused. This is what makes
        // the X11 clipboard bridge work: `xwayland-satellite` is an ordinary Wayland
        // client, so the focus handed to it here is the only thing that lets X11
        // selections through.
        //
        // What this buys is that an unfocused client is never *granted* the selection.
        // Smithay does not revoke an offer a client already holds: `send_selection`
        // only ever targets the newly focused client, so the client that lost the focus
        // is sent nothing at all — not even when the focus is set to `None`. Dropping
        // the offer is the client's own duty, and the protocol states it plainly
        // ("the data_offer is valid ... until the client loses keyboard focus").
        let focused =
            focused.filter(|_| self.seat.get_keyboard().is_some() && !self.outputs.is_empty());
        self.repaint |= self.keyboard.current_focus() != focused;
        // A locked session offers the selection to nobody. The lock surface takes
        // the keyboard, but it is not a client of the desktop: granting it the
        // clipboard would be a way out of the lock for anything sharing its
        // connection, and no ordinary client is focused to receive one either.
        let client = if self.lock.accepted() {
            None
        } else {
            focused.as_ref().and_then(|surface| surface.client())
        };

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, focused.clone(), serial);
        }

        self.ime_sync_focus(focused);
        set_data_device_focus(&self.display_handle, &self.seat, client.clone());
        set_primary_focus(&self.display_handle, &self.seat, client);

        if let Some(id) = self.workspace.focused()
            && let Some(window) = self.windows.get(&id).cloned()
        {
            self.space.raise_element(&window, true);
        }
    }

    /// The surface under a pointer location, together with its absolute position.
    fn surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.outputs.is_empty() {
            return None;
        }
        // A locked session hit-tests lock surfaces only, so pointer motion and
        // button presses reach no ordinary client — not a window, not a layer
        // surface, not a popup.
        if self.lock.accepted() {
            return self.lock_surface_under(location);
        }
        if let Some(surface) = self.ime_surface_under(location) {
            return Some(surface);
        }
        for layer in [Layer::Overlay, Layer::Top] {
            if let Some(surface) = self.layer_under(layer, location) {
                return Some(surface);
            }
        }
        if let Some((window, window_location)) = self.space.element_under(location) {
            let relative = location - window_location.to_f64();
            if let Some((surface, surface_location)) =
                window.surface_under(relative, WindowSurfaceType::ALL)
            {
                return Some((surface, (window_location + surface_location).to_f64()));
            }
        }
        [Layer::Bottom, Layer::Background]
            .iter()
            .find_map(|layer| self.layer_under(*layer, location))
    }

    /// The lock surface covering `location`, if the lock client has one there.
    ///
    /// A lock surface is configured to its whole output, so the containment test
    /// is the output's area. Sub-surfaces of a lock surface are deliberately not
    /// descended into: they would be additional surfaces on a locked screen, and
    /// the protocol's lock surfaces are meant to be one per output.
    fn lock_surface_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        for (client, id, surface) in &self.lock.surfaces {
            if Some(client) != self.lock.client.as_ref() || !surface.alive() {
                continue;
            }
            let covers = self
                .outputs
                .iter()
                .any(|entry| entry.id == *id && entry.area.to_f64().contains(location));
            if covers {
                return Some((surface.wl_surface().clone(), Point::from((0.0, 0.0))));
            }
        }

        None
    }

    fn layer_under(
        &self,
        layer: Layer,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.layers
            .iter()
            .rev()
            .filter(|entry| entry.mapped && entry.surface.layer() == layer)
            .find_map(|entry| {
                let relative = location - entry.geometry.loc.to_f64();
                entry
                    .surface
                    .surface_under(relative, WindowSurfaceType::ALL)
                    .map(|(surface, offset)| (surface, (entry.geometry.loc + offset).to_f64()))
            })
    }

    fn focus_at_pointer(&mut self) {
        let Some((surface, _)) = self.surface_under(self.pointer_location) else {
            return;
        };
        if self.lock.accepted() {
            // Clicking a lock surface moves the keyboard focus between lock
            // surfaces. It must not touch the workspace or the layer focus, which
            // is why the two branches below are unreachable while locked.
            let is_lock_surface = self.lock.surfaces.iter().any(|(client, _, lock)| {
                Some(client) == self.lock.client.as_ref() && lock.wl_surface() == &surface
            });
            if is_lock_surface {
                self.lock.focus = Some(surface);
                self.sync_focus();
            }

            return;
        }
        if self.ime_surface_under(self.pointer_location).is_some() {
            return;
        }
        if let Some(entry) = self.layers.iter().find(|entry| {
            entry.mapped
                && entry
                    .surface
                    .surface_under(
                        self.pointer_location - entry.geometry.loc.to_f64(),
                        WindowSurfaceType::ALL,
                    )
                    .is_some_and(|(target, _)| target == surface)
        }) {
            if entry.surface.can_receive_keyboard_focus() {
                self.layer_focus = Some(entry.surface.wl_surface().clone());
            }
        } else if let Some(id) = self.window_id_for_surface(&surface) {
            self.layer_focus = None;
            self.workspace.focus(id);
        }
        self.sync_focus();
    }

    /// Place every committed layer surface and recompute each output's work area.
    ///
    /// Each output has its own area and its own reservation chain, so a panel on
    /// one display never shrinks the work area of another (ADR 0019). The pure
    /// placement and reservation rules in [`crate::layers`] are unchanged; only
    /// the area they are applied to differs per output.
    fn arrange_layers(&mut self) {
        self.repaint = true;

        let areas: Vec<(OutputId, Rectangle<i32, Logical>)> = self
            .outputs
            .iter()
            .map(|entry| (entry.id, entry.area))
            .collect();
        let mut work_areas: Vec<(OutputId, Rectangle<i32, Logical>)> =
            Vec::with_capacity(areas.len());

        for (id, output_area) in &areas {
            let mut area = *output_area;
            // Reservations precede neutral surfaces regardless of creation order.
            for exclusive in [true, false] {
                for layer in LAYER_ORDER {
                    for entry in self.layers.iter_mut().rev().filter(|entry| {
                        entry.output == *id && entry.committed && entry.surface.layer() == layer
                    }) {
                        let cached = entry.surface.cached_state();
                        let reserves = matches!(
                            cached.exclusive_zone,
                            smithay::wayland::shell::wlr_layer::ExclusiveZone::Exclusive(_)
                        );
                        if reserves != exclusive {
                            continue;
                        }
                        let source = if cached.exclusive_zone
                            == smithay::wayland::shell::wlr_layer::ExclusiveZone::DontCare
                        {
                            *output_area
                        } else {
                            area
                        };
                        entry.geometry = crate::layers::geometry(source, cached);
                        let surface = entry.surface.layer_surface();
                        surface.with_pending_state(|state| state.size = Some(entry.geometry.size));
                        if entry.initial_configure.is_none() {
                            entry.initial_configure = Some(surface.send_configure());
                        } else {
                            surface.send_pending_configure();
                        }
                        if entry.mapped {
                            crate::layers::reserve(&mut area, cached);
                        }
                    }
                }
            }
            work_areas.push((*id, area));
        }

        let mut changed = false;
        for entry in &mut self.outputs {
            if let Some((_, area)) = work_areas.iter().find(|(id, _)| *id == entry.id)
                && entry.work_area != *area
            {
                entry.work_area = *area;
                changed = true;
            }
        }
        if changed {
            self.arrange_windows();
        }
        self.sync_focus();
    }

    fn commit_layer(&mut self, surface: &WlSurface) {
        let Some(entry) = self
            .layers
            .iter_mut()
            .find(|entry| entry.surface.wl_surface() == surface)
        else {
            return;
        };
        let cached = entry.surface.cached_state();
        if cached.size.w < 0 || cached.size.h < 0 {
            entry.surface.layer_surface().shell_surface().post_error(
                zwlr_layer_surface_v1::Error::InvalidSize,
                "layer dimensions exceed the supported coordinate range",
            );
            return;
        }
        let has_buffer =
            with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false);
        if has_buffer && !entry.acknowledged {
            entry.surface.layer_surface().shell_surface().post_error(
                zwlr_layer_surface_v1::Error::InvalidSurfaceState,
                "a buffer requires acknowledgement of the initial configure",
            );
            return;
        }
        // Read through the field rather than through `self.output_of`: the entry
        // above already borrows `self.layers`, and a method call would borrow all
        // of `self` with it.
        let target = output_of(&self.outputs, entry.output).cloned();
        if entry.mapped && !has_buffer {
            entry.mapped = false;
            entry.committed = false;
            entry.initial_configure = None;
            entry.acknowledged = false;
            if let Some(target) = target {
                entry
                    .surface
                    .with_surfaces(|surface, _| target.leave(surface));
            }
            let layer = entry.surface.layer();
            with_states(surface, |states| {
                let reset = LayerSurfaceCachedState {
                    layer,
                    ..Default::default()
                };
                *states
                    .cached_state
                    .get::<LayerSurfaceCachedState>()
                    .current() = reset;
                *states
                    .cached_state
                    .get::<LayerSurfaceCachedState>()
                    .pending() = reset;
            });
        } else {
            entry.committed = true;
            entry.mapped = has_buffer;
            if has_buffer && let Some(target) = target {
                entry
                    .surface
                    .with_surfaces(|surface, _| target.enter(surface));
            }
        }
        self.update_surface_scale(surface);
        self.arrange_layers();
    }

    fn remove_layer(&mut self, surface: &WlSurface) {
        if let Some(index) = self
            .layers
            .iter()
            .position(|entry| entry.surface.wl_surface() == surface)
        {
            let entry = self.layers.remove(index);
            let target = self.output_of(entry.output).cloned();
            if let Some(target) = target {
                entry
                    .surface
                    .with_surfaces(|surface, _| target.leave(surface));
            }
            if self.layer_focus.as_ref() == Some(surface) {
                self.layer_focus = None;
            }
            self.arrange_layers();
        }
    }
}

/// The advertisement of `id` inside `outputs`.
fn output_of(outputs: &[OutputEntry], id: OutputId) -> Option<&Output> {
    outputs
        .iter()
        .find(|entry| entry.id == id)
        .map(|entry| &entry.output)
}

/// The toplevel surface backing `window`, if it is a Wayland window.
fn window_toplevel_surface(window: &Window) -> Option<&WlSurface> {
    window.toplevel().map(ToplevelSurface::wl_surface)
}

/// Coerce a tiled size into one the protocol accepts, which requires it to be positive.
fn clamp_size(size: Size<i32, Logical>) -> Size<i32, Logical> {
    Size::from((size.w.max(1), size.h.max(1)))
}

/// The XKB configuration used for the seat keyboard.
///
/// Physical sessions use the loaded input configuration; headless tests pass
/// deterministic defaults explicitly.
fn keyboard_config(input: &inox_config::InputConfig) -> XkbConfig<'_> {
    XkbConfig {
        rules: "evdev",
        model: "pc105",
        layout: &input.keyboard_layout,
        variant: &input.keyboard_variant,
        options: None,
    }
}

/// Create the headless outputs the configuration asks for (ADR 0019).
///
/// Without `--output` this is the single 1280x720 output every earlier milestone
/// had, so the headless path is unchanged unless more displays are requested.
fn create_headless_outputs(config: &CompositorConfig) -> Vec<OutputDescriptor> {
    config
        .outputs
        .iter()
        .map(|configured| {
            let (width, height) = configured.size();

            let output = Output::new(
                configured.name.clone(),
                PhysicalProperties {
                    size: (millimetres(width), millimetres(height)).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Inox".to_owned(),
                    model: "Headless".to_owned(),
                },
            );

            let mode = Mode {
                size: (width, height).into(),
                refresh: configured.refresh_mhz(),
            };
            output.set_preferred(mode);
            output.change_current_state(
                Some(mode),
                Some(Transform::Normal),
                Some(configured_scale(configured.scale)),
                Some(Point::from((0, 0))),
            );

            OutputDescriptor {
                key: configured.name.clone(),
                output,
                position: configured.position.map(Point::from),
            }
        })
        .collect()
}

/// The Smithay scale a configured factor becomes (ADR 0020).
///
/// Clients that only understand integer scales are told the ceiling, so they
/// render at least as many pixels as the display has and the compositor
/// downscales; clients that bind `wp_fractional_scale_v1` get the exact value.
/// A non-finite or non-positive factor is treated as `1.0` — the configuration
/// layer rejects those, and this keeps a hand-built one inert rather than absurd.
fn configured_scale(scale: f64) -> OutputScale {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let advertised = scale.ceil().clamp(1.0, f64::from(i32::MAX)) as i32;

    OutputScale::Custom {
        advertised_integer: advertised,
        fractional: scale,
    }
}

/// Convert a pixel dimension to millimetres at [`PIXELS_PER_INCH`].
fn millimetres(pixels: i32) -> i32 {
    pixels.saturating_mul(MILLIMETRES_PER_INCH_X10) / (PIXELS_PER_INCH * 10)
}

/// Whether `rendered` frames satisfy the configured `budget`.
///
/// A `None` budget means "run until signalled", so it is never reached.
fn budget_reached(budget: Option<u64>, rendered: u64) -> bool {
    matches!(budget, Some(budget) if rendered >= budget)
}

/// Per-client data attached to every Wayland client.
///
/// The compositor requires every client to be registered through
/// [`DisplayHandle::insert_client`] with this state; see
/// [`CompositorHandler::client_compositor_state`].
#[derive(Default)]
pub struct ClientState {
    compositor_state: CompositorClientState,
    input_method: bool,
}

impl ClientState {
    /// Connection data for an explicitly launched, trusted input method.
    ///
    /// Never use this for clients accepted on the public Wayland socket.
    pub fn input_method() -> Self {
        Self {
            input_method: true,
            ..Self::default()
        }
    }
}

impl ClientData for ClientState {
    fn initialized(&self, client_id: ClientId) {
        info!(?client_id, "wayland client connected");
    }

    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        info!(?client_id, ?reason, "wayland client disconnected");
    }
}

impl OutputHandler for InoxState {}

impl CompositorHandler for InoxState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        match client.get_data::<ClientState>() {
            Some(client_state) => &client_state.compositor_state,
            None => fallback_client_state(),
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        self.repaint = true;
        on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
            && let Err(error) = popup.send_configure()
        {
            warn!(%error, "failed to configure popup");
        }
        self.commit_layer(surface);
        self.update_surface_scale(surface);

        if let Some(id) = self.window_id_for_surface(surface)
            && let Some(window) = self.windows.get(&id)
        {
            // Refresh the bounding box the space uses for hit-testing and tiling.
            window.on_commit();
        }
        self.ime_update_popups();
    }

    fn destroyed(&mut self, surface: &WlSurface) {
        self.ime_surface_destroyed(surface);
        self.remove_layer(surface);
    }
}

impl WlrLayerShellHandler for InoxState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        // A surface that named no output takes the primary one; one that named an
        // output this compositor does not advertise is closed, because there is no
        // display it could be shown on (ADR 0019).
        let target = match &output {
            Some(resource) => Output::from_resource(resource)
                .and_then(|named| self.find_output(&named).map(|entry| entry.id)),
            None => self.outputs.first().map(|entry| entry.id),
        };
        let Some(target) = target else {
            surface.send_close();
            return;
        };

        let wl_surface = surface.wl_surface().clone();
        self.layers.push(LayerEntry {
            output: target,
            surface: DesktopLayerSurface::new(surface, namespace),
            geometry: Rectangle::default(),
            committed: false,
            mapped: false,
            initial_configure: None,
            acknowledged: false,
        });
        self.update_surface_scale(&wl_surface);
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: LayerSurfaceConfigure) {
        if let Some(entry) = self
            .layers
            .iter_mut()
            .find(|entry| entry.surface.wl_surface() == &surface)
            && entry
                .initial_configure
                .is_some_and(|initial| configure.serial >= initial)
        {
            entry.acknowledged = true;
        }
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        self.remove_layer(surface.wl_surface());
    }
}

impl SeatHandler for InoxState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, focused: Option<&WlSurface>) {
        debug!(focused = focused.is_some(), "keyboard focus changed");
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.repaint |= self.cursor_image != image;
        self.cursor_image = image;
    }
}

impl DataDeviceHandler for InoxState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl PrimarySelectionHandler for InoxState {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

/// Clipboard selection transitions.
///
/// The payload never passes through the compositor — clients hand it to each other over
/// a pipe — so this only reports what changed, for the log and for a reader who wants to
/// see the focus rules in action.
impl SelectionHandler for InoxState {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        match &source {
            Some(source) => debug!(
                target = ?ty,
                mime_types = source.mime_types().len(),
                "selection set by the focused client"
            ),
            None => debug!(target = ?ty, "selection cleared by the focused client"),
        }
    }

    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        _fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        // Only reachable for a compositor-set selection, which Inox does not provide yet;
        // client-set selections are forwarded to the source client by Smithay instead.
        debug!(target = ?ty, %mime_type, "client asked for a compositor-side selection");
    }
}

/// Client-initiated drag and drop.
///
/// The transfer itself needs no compositor code: Smithay's `DnDGrab` takes over the
/// pointer once a client answers an implicit grab with `start_drag`, and routes the
/// enter/motion/drop/action events of the drag from there. All the compositor has to do
/// is keep delivering pointer motion to the seat, which is what
/// [`InoxState::inject_pointer_motion`] and, in M3b, the libinput backend do.
impl ClientDndGrabHandler for InoxState {
    fn started(
        &mut self,
        source: Option<WlDataSource>,
        icon: Option<WlSurface>,
        _seat: Seat<Self>,
    ) {
        debug!(
            offered = source.is_some(),
            icon = icon.is_some(),
            "client started a drag"
        );
    }

    fn dropped(&mut self, target: Option<WlSurface>, validated: bool, _seat: Seat<Self>) {
        debug!(
            dropped_on_a_surface = target.is_some(),
            validated, "client drag ended"
        );
    }
}

/// Server-initiated drag and drop.
///
/// Inox never starts a drag itself, so there is nothing to react to; the trait is
/// required by [`DataDeviceHandler`] regardless.
impl ServerDndGrabHandler for InoxState {}

impl ShmHandler for InoxState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl BufferHandler for InoxState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {
        // Buffers are owned by the renderer's import cache and released there.
    }
}

impl XdgShellHandler for InoxState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        self.add_window(surface);
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
        });
        if let Err(error) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            warn!(%error, "failed to track popup");
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // Popup pointer grabs arrive with the popup milestone.
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| state.geometry = positioner.get_geometry());
        surface.send_repositioned(token);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.remove_window(&surface);
    }
}

/// The `inox_ipc` protocol version the compositor advertises and answers with.
///
/// Read from the interface generated in `inox-protocols` so the reported version
/// can never drift from the protocol definition.
fn inox_ipc_version() -> u32 {
    <InoxIpc as Resource>::interface().version
}

/// Service the private `inox-ipc` control protocol.
///
/// M1 only has to prove the compositor genuinely consumes `inox-protocols`, so the
/// handler is deliberately small: `get_version` reports the protocol version, and
/// `reload_config` is answered with [`ReloadResult::Unsupported`] because
/// configuration reloading arrives in a later milestone. Answering explicitly lets
/// the shell tell "not implemented yet" apart from a transport failure.
impl Dispatch<InoxIpc, ()> for InoxState {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &InoxIpc,
        request: InoxIpcRequest,
        _data: &(),
        _display_handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            // `destroy` is a destructor: wayland-server removes the object once the
            // request has been dispatched, so there is nothing to clean up here.
            InoxIpcRequest::Destroy => {}
            InoxIpcRequest::GetVersion => resource.version(inox_ipc_version()),
            InoxIpcRequest::ReloadConfig => resource.config_reloaded(ReloadResult::Unsupported),
            // The first privileged request the compositor accepts (acceptance S04,
            // ADR 0015's privilege table). The token is a capability the session
            // delivered; what the client says about itself is not consulted, and a
            // compositor with no token refuses everyone.
            InoxIpcRequest::ClaimLockRole { token } => {
                match claim(state.lock_token.as_deref(), &token) {
                    Claim::Authorized => {
                        state.lock_authorized.insert(client.id());
                        info!("a client claimed the trusted session lock role");
                        resource.version(inox_ipc_version());
                    }
                    refused => {
                        state.lock_claim_refusals = state.lock_claim_refusals.saturating_add(1);
                        state.lock_authorized.remove(&client.id());
                        warn!(
                            reason = refused.reason(),
                            "refused a trusted session lock role claim"
                        );
                    }
                }
            }
            // `Request` is `#[non_exhaustive]`: a later protocol revision may add
            // requests, which this milestone ignores rather than second-guessing.
            _ => {}
        }
    }
}

impl GlobalDispatch<InoxIpc, ()> for InoxState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<InoxIpc>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let ipc = data_init.init(resource, ());

        // The protocol asks the compositor to report its version as soon as the
        // global is bound, so a client does not have to send `get_version` first.
        ipc.version(inox_ipc_version());
        // The lock state is a broadcast every observer needs from the start: an
        // idle daemon that binds while the session is already locked must not have
        // to guess, and neither must the notification daemon.
        ipc.session_lock_state(state.lock_state_for_ipc());
        // Held so `session_lock_state` can reach every observer on a change. A
        // destroyed object is dropped by the next broadcast.
        state.ipc_clients.push(ipc);
    }
}

/// The `ext-session-lock-v1` server side (ADR 0015, ADR 0022).
///
/// Smithay implements the protocol — the lock object, the surfaces, the
/// configure/ack handshake and the protocol errors. What is here is the policy
/// and the locked scene, and three decisions worth stating:
///
/// - **Only a claimed connection may lock.** An unauthorized connection gets
///   `finished`, which is what dropping the [`SessionLocker`] sends: the protocol
///   has a first-class refusal, so a policy refusal does not have to become a
///   protocol error or a disconnected client.
/// - **One lock at a time, and no takeover.** A session that is already locked
///   refuses a second lock, including one from a restarted lock UI. The protocol
///   allows a compositor to hand a held lock over, and ADR 0022 records why this
///   one does not: Smithay's per-output lock bookkeeping has no public reset, so a
///   second client's surfaces for those outputs would be rejected as duplicates.
///   The result is the fail-closed direction the protocol also allows — a lock UI
///   that dies leaves a locked session that only a logout ends.
/// - **The lock is confirmed only after every output is covered.** See
///   [`LockCoverage`]: `locked` is what a client uses to justify suspending the
///   machine, so it must not arrive while any output could still be showing
///   unlocked content.
impl SessionLockHandler for InoxState {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        let owner = confirmation
            .ext_session_lock()
            .client()
            .map(|client| client.id());

        if self.lock.accepted() {
            self.lock_refusals = self.lock_refusals.saturating_add(1);
            warn!(
                "refused a session lock request: the session is already locked (ADR 0022 does \
                 not hand a held lock to another client)"
            );

            return;
        }
        let authorized = owner
            .as_ref()
            .is_some_and(|owner| self.lock_authorized.contains(owner));
        let Some(owner) = owner.filter(|_| authorized) else {
            self.lock_refusals = self.lock_refusals.saturating_add(1);
            warn!(
                "refused a session lock request from a connection that has not claimed the \
                 trusted lock role"
            );

            return;
        };

        self.lock.object = Some(confirmation.ext_session_lock().clone());
        self.lock.client = Some(owner);
        self.lock.confirmed = false;
        self.lock.owner_lost = false;
        self.lock.focus = None;
        let outputs: Vec<OutputId> = self.outputs.iter().map(|entry| entry.id).collect();
        self.lock.coverage.begin(outputs);
        self.lock.pending = Some(confirmation);
        // Chords pressed before the lock belong to the desktop, not to the lock
        // surface: a release whose press was swallowed by the compositor must not
        // be handed to the client that is now focused.
        self.swallowed_presses = SwallowedPresses::default();
        self.repaint = true;
        info!(
            outputs = self.outputs.len(),
            "session lock accepted; covering every output"
        );
        self.sync_focus();
        self.broadcast_lock_state();
    }

    fn unlock(&mut self) {
        // Only the pre-checked dispatch arm below reaches this, so the connection
        // asking really is the one that holds the lock and really has been sent
        // `locked`.
        self.release_lock("the lock client unlocked the session");
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(client) = surface.wl_surface().client().map(|client| client.id()) else {
            debug!("a lock surface arrived without a client");

            return;
        };
        if Some(&client) != self.lock.client.as_ref() {
            // Smithay creates a lock surface object for any client that asks, and
            // sending the client an error here would be about the lock object it
            // does not own; ignoring the surface is enough to keep its pixels off
            // the locked screen.
            debug!("ignored a lock surface from a connection that does not hold the lock");

            return;
        }
        let Some(entry) = Output::from_resource(&output)
            .and_then(|output| self.outputs.iter().find(|entry| entry.output == output))
        else {
            debug!("ignored a lock surface for an output this compositor does not advertise");

            return;
        };
        let size = entry.area.size;
        let id = entry.id;

        // The size is set here, before Smithay sends the initial configure for
        // this surface (`LockSurface::send_configure` runs after this callback
        // returns), so the client is configured to exactly its output's logical
        // size — which the protocol makes an exact requirement it must match.
        surface.with_pending_state(|state| {
            state.size = Some(Size::from((
                u32::try_from(size.w).unwrap_or(0),
                u32::try_from(size.h).unwrap_or(0),
            )));
        });
        if self.lock.focus.is_none() {
            self.lock.focus = Some(surface.wl_surface().clone());
        }
        self.lock.surfaces.push((client, id, surface));
        self.repaint = true;
        self.sync_focus();
    }

    // `ack_configure` is deliberately not implemented: Smithay's configure type is
    // not nameable outside smithay, and there is nothing to apply when the client
    // acknowledges — the commit that follows the ack marks the surface damaged
    // through `CompositorHandler::commit`, which is what draws the next frame.
}

/// The lock object's requests, pre-checked before Smithay sees them.
///
/// Smithay's `unlock_and_destroy` arm posts `invalid_unlock` when the lock was
/// never confirmed and then unlocks anyway — it clears its output bookkeeping and
/// calls [`SessionLockHandler::unlock`] without returning. A connection that was
/// refused the lock, or that never received `locked`, could therefore end a lock
/// it does not own. This arm refuses that request instead of forwarding it, so
/// "only the lock's owner, and only after `locked`, may unlock" is a property of
/// this compositor rather than of the protocol implementation underneath it.
impl Dispatch<ExtSessionLockV1, SessionLockState> for InoxState {
    fn request(
        state: &mut Self,
        client: &Client,
        lock: &ExtSessionLockV1,
        request: lock_v1::Request,
        data: &SessionLockState,
        handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if matches!(&request, lock_v1::Request::UnlockAndDestroy)
            && !(state.lock.confirmed && state.lock.holds(lock))
        {
            lock.post_error(
                lock_v1::Error::InvalidUnlock,
                "the session is not locked by this connection",
            );
            warn!("refused unlock_and_destroy from a connection that does not hold the lock");

            return;
        }
        <SessionLockManagerState as Dispatch<ExtSessionLockV1, SessionLockState, Self>>::request(
            state, client, lock, request, data, handle, data_init,
        );
    }
}

smithay::reexports::wayland_server::delegate_global_dispatch!(InoxState: [ExtSessionLockManagerV1: SessionLockManagerGlobalData] => SessionLockManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(InoxState: [ExtSessionLockManagerV1: ()] => SessionLockManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(InoxState: [ExtSessionLockSurfaceV1: ExtLockSurfaceUserData] => SessionLockManagerState);

/// Fallback used when a client carries no [`ClientState`].
///
/// Every client is registered with a [`ClientState`] through
/// [`DisplayHandle::insert_client`], so this is unreachable in practice. Returning
/// a shared default keeps the compositor alive instead of panicking on an
/// unexpected client.
fn fallback_client_state() -> &'static CompositorClientState {
    static FALLBACK: OnceLock<CompositorClientState> = OnceLock::new();

    FALLBACK.get_or_init(|| {
        warn!("client without the Inox client state, using a shared default");
        CompositorClientState::default()
    })
}

smithay::delegate_compositor!(InoxState);
smithay::delegate_output!(InoxState);
smithay::delegate_seat!(InoxState);
smithay::delegate_xdg_shell!(InoxState);
smithay::reexports::wayland_server::delegate_dispatch!(InoxState: [ZwlrLayerShellV1: ()] => WlrLayerShellState);
smithay::reexports::wayland_server::delegate_global_dispatch!(InoxState: [ZwlrLayerShellV1: WlrLayerShellGlobalData] => WlrLayerShellState);

impl Dispatch<ZwlrLayerSurfaceV1, WlrLayerSurfaceUserData> for InoxState {
    fn request(
        state: &mut Self,
        client: &Client,
        resource: &ZwlrLayerSurfaceV1,
        request: zwlr_layer_surface_v1::Request,
        data: &WlrLayerSurfaceUserData,
        handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // Smithay 0.7 casts these unsigned protocol values to signed Size fields,
        // whose constructor asserts nonnegative dimensions in debug builds.
        if let zwlr_layer_surface_v1::Request::SetSize { width, height } = &request
            && (*width > i32::MAX as u32 || *height > i32::MAX as u32)
        {
            resource.post_error(
                zwlr_layer_surface_v1::Error::InvalidSize,
                "layer dimensions exceed the supported coordinate range",
            );
            return;
        }
        <WlrLayerShellState as Dispatch<ZwlrLayerSurfaceV1, WlrLayerSurfaceUserData, Self>>::request(
            state, client, resource, request, data, handle, data_init,
        );
    }

    fn destroyed(
        state: &mut Self,
        client: ClientId,
        resource: &ZwlrLayerSurfaceV1,
        data: &WlrLayerSurfaceUserData,
    ) {
        <WlrLayerShellState as Dispatch<ZwlrLayerSurfaceV1, WlrLayerSurfaceUserData, Self>>::destroyed(
            state, client, resource, data,
        );
    }
}
/// Fractional scaling (ADR 0020).
///
/// Creating the object is the whole protocol interaction: the surface is told its
/// scale through its own surface update, and a client that never
/// creates the object is told the integer ceiling through
/// `preferred_buffer_scale` on `wl_compositor` v6 instead.
impl FractionalScaleHandler for InoxState {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        self.update_surface_scale(&surface);
    }
}

smithay::delegate_fractional_scale!(InoxState);
smithay::delegate_shm!(InoxState);
smithay::delegate_viewporter!(InoxState);
smithay::delegate_data_device!(InoxState);
smithay::delegate_primary_selection!(InoxState);

#[cfg(test)]
mod tests {
    use super::budget_reached;

    #[test]
    fn without_a_budget_the_compositor_never_stops_by_itself() {
        assert!(!budget_reached(None, 0));
        assert!(!budget_reached(None, u64::MAX));
    }

    #[test]
    fn the_budget_is_reached_at_the_configured_frame_count() {
        assert!(!budget_reached(Some(3), 0));
        assert!(!budget_reached(Some(3), 2));
        assert!(budget_reached(Some(3), 3));
        assert!(budget_reached(Some(3), 4));
    }

    #[test]
    fn a_budget_of_one_frame_is_reached_after_the_first_frame() {
        assert!(!budget_reached(Some(1), 0));
        assert!(budget_reached(Some(1), 1));
    }
}
