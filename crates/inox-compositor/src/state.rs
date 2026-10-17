//! Smithay compositor state and protocol handlers.
//!
//! # Advertised globals
//!
//! Every global the compositor creates, and the version it binds at:
//!
//! - `wl_compositor` (5) and `wl_subcompositor` (1) — [`CompositorState`], which
//!   creates both in one call.
//! - `wl_shm` (2) — [`ShmState`], the only buffer transport besides client-provided
//!   dmabufs, which are not advertised yet.
//! - `wl_seat` (9) — [`SeatState`], carrying the keyboard and pointer capabilities.
//! - `wl_output` (4) and `zxdg_output_manager_v1` (3) — the headless output and
//!   [`OutputManagerState`].
//! - `xdg_wm_base` (6) — [`XdgShellState`], the surface role managed windows use.
//! - `zwlr_layer_shell_v1` (4) — [`WlrLayerShellState`], ordinary desktop layers.
//! - `wp_viewporter` (1) — [`ViewporterState`]. Required by XWayland clients that
//!   scale themselves (ADR 0007); without it `xwayland-satellite` cannot run here.
//! - `wl_data_device_manager` (3) — [`DataDeviceState`], the clipboard and
//!   drag-and-drop channel.
//! - `zwp_primary_selection_device_manager_v1` (1) — [`PrimarySelectionState`], the
//!   X11-style primary selection (middle-click paste).
//! - `inox_ipc` — the private control protocol from `inox-protocols`, at the version
//!   its own interface declares.
//!
//! Client toplevels become managed windows: they are registered in a
//! [`Workspace`], tiled by [`LayoutConfig`] and mapped into a Smithay desktop
//! `Space` that [`InoxState::render_frame`] composites.
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

use std::{
    borrow::Cow,
    collections::HashMap,
    os::unix::io::OwnedFd,
    sync::OnceLock,
    time::{Duration, Instant},
};

use calloop::LoopSignal;
use inox_protocols::{
    ipc::inox_ipc::server::inox_ipc::{InoxIpc, ReloadResult, Request as InoxIpcRequest},
    standard::{
        core::server::{
            wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface,
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
        compositor::{CompositorClientState, CompositorHandler, CompositorState, with_states},
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
use tracing::{debug, info, warn};

use crate::{
    config::CompositorConfig,
    input::{Action, SwallowedPresses, action_for_key},
    layout::LayoutConfig,
    render::{HeadlessRenderer, RenderError, SceneElement},
    workspace::{WindowId, Workspace},
};

/// Physical pixels assumed per inch when translating the headless mode into
/// millimetres for the advertised `wl_output` geometry.
const PIXELS_PER_INCH: i32 = 96;

/// Millimetres per inch, scaled by ten to keep the conversion in integer arithmetic.
const MILLIMETRES_PER_INCH_X10: i32 = 254;

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

    output: Output,
    output_active: bool,
    output_global: Option<GlobalId>,
    input_config: inox_config::InputConfig,
    cursor_image: CursorImageStatus,
    cursor_visible: bool,
    cursor_buffer: MemoryRenderBuffer,
    repaint: bool,
    background: [f32; 4],

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
    /// Rectangle the windows are tiled into, derived from the output mode.
    work_area: Rectangle<i32, Logical>,
    /// Last location reported for the pointer, in workspace coordinates.
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

struct StateSetup {
    output: Output,
    renderer: Option<HeadlessRenderer>,
    input: inox_config::InputConfig,
    theme: inox_config::Theme,
    physical: bool,
}

fn rgba_floats(color: inox_config::Rgba) -> [f32; 4] {
    [color.r, color.g, color.b, color.a].map(|channel| f32::from(channel) / 255.0)
}

struct LayerEntry {
    surface: DesktopLayerSurface,
    geometry: Rectangle<i32, Logical>,
    committed: bool,
    mapped: bool,
    initial_configure: Option<Serial>,
    acknowledged: bool,
}

// Front to back, shared by rendering, hit testing and exclusive keyboard focus.
const LAYER_ORDER: [Layer; 4] = [Layer::Overlay, Layer::Top, Layer::Bottom, Layer::Background];

impl InoxState {
    /// Build the state, the headless output, the seat and the render target.
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
        let output = create_headless_output(config);
        let setup = StateSetup {
            output,
            renderer: Some(HeadlessRenderer::new(config.output_size())?),
            input: inox_config::InputConfig {
                keyboard_repeat_delay_ms: 200,
                ..Default::default()
            },
            theme: inox_config::Theme::builtin_dark(),
            physical: false,
        };
        Self::build(display_handle, config, loop_signal, setup)
    }

    pub(crate) fn new_physical(
        display_handle: &DisplayHandle,
        config: &CompositorConfig,
        loop_signal: LoopSignal,
        output: Output,
        input: inox_config::InputConfig,
        theme: inox_config::Theme,
        layout: LayoutConfig,
    ) -> Result<Self, StateError> {
        let mut state = Self::build(
            display_handle,
            config,
            loop_signal,
            StateSetup {
                output,
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
            output,
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
        let compositor_state = CompositorState::new::<Self>(display_handle);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(display_handle);
        let output_global = Some(output.create_global::<Self>(display_handle));
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

        let mut space = Space::default();
        space.map_output(&output, (0, 0));

        let work_area = Rectangle::from_size(
            output
                .current_mode()
                .map(|mode| mode.size.to_logical(1))
                .unwrap_or_default(),
        );

        info!(
            output = %output.name(),
            width = work_area.size.w,
            height = work_area.size.h,
            physical,
            "created compositor output"
        );

        Ok(Self {
            compositor_state,
            output_manager_state,
            viewporter_state,
            data_device_state,
            primary_selection_state,
            display_handle: display_handle.clone(),
            output,
            output_active: true,
            output_global,
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
            work_area,
            pointer_location: Point::from((0.0, 0.0)),
            swallowed_presses: SwallowedPresses::default(),
            renderer,
            start_time: Instant::now(),
            frames_rendered: 0,
            exit_after_frames: config.exit_after_frames,
            exit_requested: false,
            loop_signal,
        })
    }

    /// Render one frame of the headless output.
    ///
    /// The desktop space is refreshed so that closed windows are dropped, the
    /// render elements for the whole work area are produced and drawn over the
    /// background colour, and every mapped window is told that a frame happened.
    ///
    /// # Errors
    ///
    /// Returns a [`RenderError`] when the GLES rendering fails.
    pub fn render_frame(&mut self) -> Result<(), RenderError> {
        let mut renderer = self.renderer.take().ok_or(RenderError::NotHeadless)?;
        let result = self
            .scene_elements(renderer.renderer_mut())
            .map_err(RenderError::from)
            .and_then(|elements| renderer.render_elements(&elements));
        self.renderer = Some(renderer);
        result?;
        self.frame_presented();
        Ok(())
    }

    pub(crate) fn scene_elements(
        &mut self,
        renderer: &mut GlesRenderer,
    ) -> Result<Vec<SceneElement>, GlesError> {
        self.space.refresh();
        self.popups.cleanup();
        if !self.output_active {
            return Ok(Vec::new());
        }
        let mut elements = Vec::new();
        if self.cursor_visible {
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
                        (self.pointer_location - hotspot.to_f64()).to_physical_precise_round(1.0);
                    elements.extend(render_elements_from_surface_tree(
                        renderer,
                        surface,
                        location,
                        1.0,
                        1.0,
                        Kind::Cursor,
                    ));
                }
                _ => {
                    elements.push(SceneElement::Cursor(
                        MemoryRenderBufferRenderElement::from_buffer(
                            renderer,
                            self.pointer_location.to_physical(1.0),
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
        for layer in [Layer::Overlay, Layer::Top] {
            self.layer_render_elements(layer, renderer, &mut elements);
        }
        elements.extend(
            self.space
                .render_elements_for_region(
                    renderer,
                    &Rectangle::from_size(self.output_size().into()),
                    1.0f64,
                    1.0,
                )
                .into_iter()
                .map(SceneElement::Surface),
        );
        for layer in [Layer::Bottom, Layer::Background] {
            self.layer_render_elements(layer, renderer, &mut elements);
        }
        Ok(elements)
    }

    fn layer_render_elements(
        &self,
        layer: Layer,
        renderer: &mut GlesRenderer,
        elements: &mut Vec<SceneElement>,
    ) {
        for entry in self
            .layers
            .iter()
            .rev()
            .filter(|entry| entry.mapped && entry.surface.layer() == layer)
        {
            elements.extend(entry.surface.render_elements::<SceneElement>(
                renderer,
                entry.geometry.loc.to_physical_precise_round(1.0),
                1.0.into(),
                1.0,
            ));
        }
    }

    pub(crate) fn take_repaint(&mut self) -> bool {
        std::mem::take(&mut self.repaint)
    }
    pub(crate) fn background(&self) -> [f32; 4] {
        self.background
    }
    pub(crate) fn frame_presented(&mut self) {
        self.frames_rendered = self.frames_rendered.saturating_add(1);
        self.send_frame_callbacks();
    }
    pub(crate) fn frame_callbacks(&mut self) {
        self.send_frame_callbacks();
    }
    pub(crate) fn frame_budget(&self) -> bool {
        self.exit_after_frames.is_some()
    }

    /// Send a `wl_surface.frame` callback to every mapped window.
    ///
    /// Without this a client that waits for its frame callback before drawing the next
    /// frame never gets one, so it never repaints: a real client (an XWayland toplevel,
    /// a GTK application) that asks for a frame first would stay blank. Note that the
    /// callbacks only reach the client once the display is flushed, which the event loop
    /// does after every frame it renders.
    fn send_frame_callbacks(&mut self) {
        let time = self.start_time.elapsed();

        for window in self.windows.values() {
            // Every managed window lives on the single output, and the closure reports
            // exactly that, so the throttle never gates a callback; `ZERO` states that
            // rather than leaving it to depend on the output comparison alone.
            window.send_frame(&self.output, time, Some(Duration::ZERO), |_, _| {
                Some(self.output.clone())
            });
        }
        for entry in self.layers.iter().filter(|entry| entry.mapped) {
            entry
                .surface
                .send_frame(&self.output, time, Some(Duration::ZERO), |_, _| {
                    Some(self.output.clone())
                });
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
        self.renderer
            .as_mut()
            .ok_or(RenderError::NotHeadless)?
            .read_center_pixel()
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
        self.renderer
            .as_mut()
            .ok_or(RenderError::NotHeadless)?
            .read_frame_rgba()
    }

    /// Size of the headless output in pixels, as `(width, height)`.
    pub fn output_size(&self) -> (i32, i32) {
        if !self.output_active {
            return (0, 0);
        }
        let size = self
            .output
            .current_mode()
            .map(|mode| mode.size)
            .unwrap_or_default();
        (size.w, size.h)
    }

    /// The logical rectangle available to ordinary windows after layer reservations.
    pub fn work_area(&self) -> Rectangle<i32, Logical> {
        self.work_area
    }

    /// Number of layer surfaces with a committed buffer.
    pub fn layer_count(&self) -> usize {
        self.layers.iter().filter(|entry| entry.mapped).count()
    }

    /// The headless output advertised to clients.
    pub fn output(&self) -> &Output {
        &self.output
    }

    /// Whether an output is currently available and advertised.
    pub fn output_active(&self) -> bool {
        self.output_active
    }

    /// Replace the active output, preserving ordinary clients across device removal.
    ///
    /// Layers on a removed output receive `closed`; their clients recreate them for
    /// the new output. This first hardware backend owns one active output at a time.
    ///
    /// # Errors
    /// Returns a render error when a headless target cannot be resized.
    pub fn replace_output(&mut self, output: Option<Output>) -> Result<(), RenderError> {
        if let Some(output) = &output
            && let Some(renderer) = &mut self.renderer
            && let Some(mode) = output.current_mode()
        {
            renderer.resize((mode.size.w, mode.size.h))?;
        }
        let same = output.as_ref() == Some(&self.output) && self.output_active;
        if !same {
            if let Some(global) = self.output_global.take() {
                self.display_handle.remove_global::<Self>(global);
            }
            self.space.unmap_output(&self.output);
            for entry in self.layers.drain(..) {
                entry.surface.layer_surface().send_close();
                entry
                    .surface
                    .with_surfaces(|surface, _| self.output.leave(surface));
            }
            self.layer_focus = None;
        }
        self.output_active = output.is_some();
        if let Some(output) = output {
            self.output = output;
            if !same {
                self.output_global = Some(self.output.create_global::<Self>(&self.display_handle));
            }
            self.space.map_output(&self.output, (0, 0));
            self.work_area = Rectangle::from_size(self.output_size().into());
            self.arrange_windows();
            self.arrange_layers();
            self.pointer_location = self.clamp_pointer(self.pointer_location);
        } else {
            self.work_area = Rectangle::default();
        }
        self.repaint = true;
        self.sync_focus();
        self.inject_pointer_motion(self.pointer_location);
        Ok(())
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
            self.seat.remove_pointer();
        }
        self.cursor_visible = pointer;
        self.repaint = true;
        self.sync_focus();
        Ok(())
    }

    fn reset_keyboard(&mut self) {
        for key in self.keyboard.pressed_keys() {
            self.inject_key(key, KeyState::Released);
        }
        let keyboard = self.keyboard.clone();
        keyboard.unset_grab(self);
    }

    pub(crate) fn reset_input(&mut self) {
        self.reset_keyboard();
        let pointer = self.pointer.clone();
        pointer.unset_grab(
            self,
            SERIAL_COUNTER.next_serial(),
            smithay::utils::Clock::<smithay::utils::Monotonic>::new()
                .now()
                .as_millis(),
        );
    }

    pub(crate) fn clamp_pointer(&self, location: Point<f64, Logical>) -> Point<f64, Logical> {
        let (width, height) = self.output_size();
        let clamp = |value: f64, size: i32| {
            if value.is_finite() {
                value.clamp(0.0, f64::from((size - 1).max(0)))
            } else {
                0.0
            }
        };
        (clamp(location.x, width), clamp(location.y, height)).into()
    }

    pub(crate) fn inject_pointer_axis(&mut self, frame: AxisFrame) {
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
    pub fn managed_windows(&self) -> &[WindowId] {
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
        self.keyboard.current_focus()
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
    fn add_window(&mut self, surface: ToplevelSurface) {
        let id = self.workspace.add();
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
    fn arrange_windows(&mut self) {
        self.repaint = true;
        let tiles = self.workspace.arrange(self.work_area, self.layout);
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
        let focused = exclusive.or(on_demand).or_else(|| {
            self.workspace
                .focused()
                .and_then(|id| self.windows.get(&id))
                .and_then(|window| window.wl_surface())
                .map(Cow::into_owned)
        });

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
        let focused = focused.filter(|_| self.seat.get_keyboard().is_some() && self.output_active);
        self.repaint |= self.keyboard.current_focus() != focused;
        let client = focused.as_ref().and_then(|surface| surface.client());

        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.keyboard.clone();
        keyboard.set_focus(self, focused, serial);

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
        if !self.output_active {
            return None;
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

    fn arrange_layers(&mut self) {
        self.repaint = true;
        let output_area = Rectangle::from_size(self.output_size().into());
        let mut area = output_area;
        // Reservations precede neutral surfaces regardless of creation order.
        for exclusive in [true, false] {
            for layer in LAYER_ORDER {
                for entry in self
                    .layers
                    .iter_mut()
                    .rev()
                    .filter(|entry| entry.committed && entry.surface.layer() == layer)
                {
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
                        output_area
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
        if self.work_area != area {
            self.work_area = area;
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
        if entry.mapped && !has_buffer {
            entry.mapped = false;
            entry.committed = false;
            entry.initial_configure = None;
            entry.acknowledged = false;
            entry
                .surface
                .with_surfaces(|surface, _| self.output.leave(surface));
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
            if has_buffer {
                entry
                    .surface
                    .with_surfaces(|surface, _| self.output.enter(surface));
            }
        }
        self.arrange_layers();
    }

    fn remove_layer(&mut self, surface: &WlSurface) {
        if let Some(index) = self
            .layers
            .iter()
            .position(|entry| entry.surface.wl_surface() == surface)
        {
            let entry = self.layers.remove(index);
            entry
                .surface
                .with_surfaces(|surface, _| self.output.leave(surface));
            if self.layer_focus.as_ref() == Some(surface) {
                self.layer_focus = None;
            }
            self.arrange_layers();
        }
    }
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

/// Create the headless output and advertise it to clients.
fn create_headless_output(config: &CompositorConfig) -> Output {
    let (width, height) = config.output_size();

    let output = Output::new(
        config.output_name.clone(),
        PhysicalProperties {
            size: (millimetres(width), millimetres(height)).into(),
            subpixel: Subpixel::Unknown,
            make: "Inox".to_owned(),
            model: "Headless".to_owned(),
        },
    );

    let mode = Mode {
        size: (width, height).into(),
        refresh: config.refresh_mhz(),
    };
    output.set_preferred(mode);
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(OutputScale::Integer(1)),
        Some(Point::from((0, 0))),
    );

    output
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

        let Some(id) = self.window_id_for_surface(surface) else {
            return;
        };

        if let Some(window) = self.windows.get(&id) {
            // Refresh the bounding box the space uses for hit-testing and tiling.
            window.on_commit();
        }
    }

    fn destroyed(&mut self, surface: &WlSurface) {
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
        if !self.output_active {
            surface.send_close();
            return;
        }
        if let Some(output) = output
            && Output::from_resource(&output).as_ref() != Some(&self.output)
        {
            surface.send_close();
            return;
        }
        self.layers.push(LayerEntry {
            surface: DesktopLayerSurface::new(surface, namespace),
            geometry: Rectangle::default(),
            committed: false,
            mapped: false,
            initial_configure: None,
            acknowledged: false,
        });
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
        _state: &mut Self,
        _client: &Client,
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
            // `Request` is `#[non_exhaustive]`: a later protocol revision may add
            // requests, which this milestone ignores rather than second-guessing.
            _ => {}
        }
    }
}

impl GlobalDispatch<InoxIpc, ()> for InoxState {
    fn bind(
        _state: &mut Self,
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
    }
}

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
