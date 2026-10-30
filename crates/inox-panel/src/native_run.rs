//! The panel on its native shell surface: what `main` runs where the build
//! host's Qt can carry the private ABI (ADR 0046 §1, §5, §7, §8).
//!
//! # What is different from [`crate::layer`]
//!
//! [`crate::layer`] owns a `wl_surface` of its own: it computes a placement,
//! asks `zwlr_layer_shell_v1` for a role, renders the QML tree offscreen, uploads
//! the pixels through `wl_shm`, and matches pointer events against the scene it
//! drew. This module owns a *window* instead. Qt created its surface on Qt's own
//! Wayland connection, the compositor gives it an `inox_shell_surface_v1` role,
//! Qt's scene graph draws it on a `wl_surface.frame` clock, and the compositor
//! animates it. Nothing is grabbed, uploaded or read back.
//!
//! What that changes here, and nothing else:
//!
//! - **The loop pumps Qt instead of reading a socket for its surface.** Qt runs
//!   its own event thread, so the surface's events reach this thread only when
//!   [`Surface::pump`] runs Qt's dispatcher. The loop therefore wakes at least
//!   once per [`PUMP`], which is the frame interval — the same cadence the
//!   `wl_surface.frame` clock would give it.
//! - **Input arrives from QML, not from `wl_pointer`.** A `TapHandler` maps a
//!   point onto an entry id the projection assigned and dispatches it; a
//!   `HoverHandler` does the same for what the pointer rests on. What a click
//!   *means* is unchanged: [`parse_action`] produces a [`Button`], the scene
//!   answers with an [`EntryAction`], and
//!   [`message_of`] turns that into the same [`Message`] the keyboard and the
//!   old pointer path produced (ADR 0046 §7).
//! - **A second connection stays.** The window service (`inox-window-manager-v1`)
//!   is not about a surface, so it keeps its own `wayland-client` connection —
//!   which the protocol allows, because it sends snapshots per connection
//!   (ADR 0045). Glass is the opposite case and is *not* here: its object names
//!   a `wl_surface`, so it lives on Qt's connection and is created inside the
//!   native host (ADR 0046 §8).
//!
//! # The state machine is separable from the loop
//!
//! [`Panel`] is the whole of the panel's reaction to the compositor: give it an
//! event, a dispatched action, or a turn of the clock, and it decides what to
//! send next. [`run`] is the thin part — a `calloop` loop, two `wayland-client`
//! sources and a pump interval. That split is what makes the interesting half
//! testable, and `src/headless_tests/native.rs` drives it against a real
//! `inox-compositor`.

use std::rc::Rc;
use std::time::{Duration, Instant};

use calloop::{
    EventLoop, Interest, Mode, PostAction,
    generic::Generic,
    ping::make_ping,
    signals::{Signal, Signals},
};
use chrono::Local;
use inox_common::i18n::Localizer;
use inox_config::{ConfigPaths, PanelSurface, ShellConfig, Theme as Tokens};
use inox_glass::{Capabilities, Request};
use inox_protocols::window_manager::inox_window_manager::client::{
    inox_window_list_v1::{self as window_list, InoxWindowListV1},
    inox_window_manager_v1::InoxWindowManagerV1,
};
use serde_json::Value;
use tracing::{debug, info, warn};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    protocol::wl_registry::{self, WlRegistry},
};

use crate::app::{self, Message, OutgoingCommand, State};
use crate::dock::{DockGeometry, DockView};
use crate::frost;
use crate::layer::{Button, LegError, Outcome, message_of, pointer_button};
use crate::native_surface::{
    Event, Geometry, KeyboardMode, Layer, Role, Surface, anchor, capability,
};
use crate::placement::{self, PanelPlacement};
use crate::reload;
use crate::theme;
use crate::view::{self, EntryAction, Interaction, Scene};
use crate::view_model::PanelViewModel;
use crate::window_service::WindowService;

/// How long the loop may sleep before it pumps Qt again.
///
/// This is the frame interval, not a polling hack: a native surface is drawn on
/// a `wl_surface.frame` clock, so 16 ms is the cadence the compositor would
/// drive it at. It is also the upper bound on how long a click can sit in Qt's
/// queue before the panel reacts, which is one frame — the same latency the
/// compositor's own frame clock gives every other client.
const PUMP: Duration = Duration::from_millis(16);

/// The `inox-glass-v1` version this client asks for: `min(advertised, 2)`.
///
/// Version 2 is the refraction vocabulary. A newer compositor is used at this
/// version, because a client that asked for more would have to guess what the
/// extra version means.
const GLASS_VERSION: u32 = 2;

/// The shell role a surface carries.
///
/// A taskbar and a menu bar are both standing bars along an edge, so both are
/// [`Role::Panel`]; a dock floats above the work area and reserves nothing, so it
/// is [`Role::Dock`]. The role is semantic — placement is [`Geometry`]'s job — and
/// it is what the compositor uses for its own policy (which surface a menu
/// belongs to, which one a screenshot names).
pub const fn role_of(surface: PanelSurface) -> Role {
    match surface {
        PanelSurface::Taskbar | PanelSurface::Menubar => Role::Panel,
        PanelSurface::Dock => Role::Dock,
    }
}

/// The placement the configured surface asks for, as data.
///
/// The conversion is one-way and total: every field of [`PanelPlacement`] has
/// exactly one field in [`Geometry`], and the anchors become the protocol's
/// bitfield. Nothing here derives anything — that is [`crate::placement`]'s job,
/// and it is shared with the layer leg so the two can never place the same strip
/// differently.
fn geometry_of(placement: &PanelPlacement) -> Geometry {
    let mut anchor_bits = 0;

    for edge in placement.anchors.iter() {
        anchor_bits |= match edge {
            placement::Anchor::Top => anchor::TOP,
            placement::Anchor::Bottom => anchor::BOTTOM,
            placement::Anchor::Left => anchor::LEFT,
            placement::Anchor::Right => anchor::RIGHT,
        };
    }

    let layer = match placement.layer {
        placement::Layer::Background => Layer::Background,
        placement::Layer::Bottom => Layer::Bottom,
        placement::Layer::Top => Layer::Top,
        placement::Layer::Overlay => Layer::Overlay,
    };

    let margin = i32::from(placement.margin);

    Geometry {
        layer,
        anchor: anchor_bits,
        // The protocol's own meaning, kept: a positive thickness to reserve, 0
        // for none, -1 to extend to the edge. `PanelPlacement` carries a `u32`
        // because a layer-surface exclusive zone is unsigned too.
        exclusive_zone: i32::try_from(placement.exclusive_zone).unwrap_or(i32::MAX),
        margin_top: margin,
        margin_right: margin,
        margin_bottom: margin,
        margin_left: margin,
        // A bar is stretched between two opposite anchors, so it asks for no
        // width of its own; a dock has nothing to be sized by and asks for the
        // width of the row it draws.
        width: placement.width.map_or(0, u32::from),
        height: u32::from(placement.height),
    }
}

/// The keyboard mode a placement asks for.
const fn keyboard_of(interactivity: placement::KeyboardInteractivity) -> KeyboardMode {
    match interactivity {
        placement::KeyboardInteractivity::None => KeyboardMode::None,
        placement::KeyboardInteractivity::OnDemand => KeyboardMode::OnDemand,
        placement::KeyboardInteractivity::Exclusive => KeyboardMode::Exclusive,
    }
}

/// What QML decided about the pointer, as Rust reads it.
///
/// QML's whole input vocabulary is these two sentences: "the pointer rests on
/// entry 7" and "the left button was clicked on entry 7". Neither says what the
/// entry *means* — that is [`crate::view`]'s projection and [`crate::app`]'s
/// state, exactly as it was for the pointer path (migration contract §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// The entry the pointer rests on, or `None` when it left every entry.
    Hover {
        /// The entry's index in the projection, when it is over one.
        entry: Option<usize>,
    },
    /// A completed click on an entry.
    Tap {
        /// The entry's index in the projection.
        entry: usize,
        /// Which button produced it.
        button: Button,
    },
}

/// Reads one action QML dispatched, or `None` for anything this build does not
/// understand.
///
/// A malformed or unknown action is dropped rather than treated as an error: the
/// bytes come from the renderer, a renderer bug must not stop the panel, and
/// "the renderer said something I do not know" is exactly what the version skew
/// between a shipped `Main.qml` and a newer crate would look like.
pub fn parse_action(bytes: &[u8]) -> Option<Action> {
    let value: Value = serde_json::from_slice(bytes).ok()?;

    match value.get("type")?.as_str()? {
        "hover" => {
            // `-1` is QML's "no entry", which is the same thing an absent id
            // would mean; a hover that names nothing clears the hover.
            let id = value.get("id")?.as_i64()?;

            Some(Action::Hover {
                entry: usize::try_from(id).ok(),
            })
        }
        "tap" => {
            let id = usize::try_from(value.get("id")?.as_i64()?).ok()?;
            let button = pointer_button(u32::try_from(value.get("button")?.as_i64()?).ok()?)?;

            Some(Action::Tap { entry: id, button })
        }
        _ => None,
    }
}

/// The panel: its state, its surface, and what the compositor last said.
pub struct Panel {
    /// Everything the panel shows, and the queue of requests it wants carried to
    /// a service.
    state: State,
    /// The native surface: one window with a shell role, on Qt's connection.
    surface: Surface,
    /// What the pointer is doing, as the projection needs to know it.
    interaction: Interaction,
    /// The scene the surface currently carries: its JSON is what QML drew, its
    /// entries are what a click is matched against.
    scene: Option<Scene>,
    /// The projection already sent to QML, so an unchanged state sends nothing.
    drawn: Option<Value>,
    /// The size the compositor configured. Authoritative: a surface that drew at
    /// another size would be showing the wrong thing.
    configured: Option<(u32, u32)>,
    /// The work area the compositor reported: the output minus every exclusive
    /// zone. A menu would be laid out against it; the panel records it because a
    /// strip that lies about the space it has is the failure this protocol
    /// exists to prevent.
    workarea: Option<(i32, i32, i32, i32)>,
    /// The placement the surface is carrying, so a reload or a growing task list
    /// is noticed only when it really changes something.
    applied: Option<PanelPlacement>,
    /// The optional parts of `inox-shell-v1` this compositor implements.
    bits: u32,
    /// The longest enter or exit transition the compositor will draw, in
    /// milliseconds; 0 until it reported one.
    max_transition: u32,
    /// The glass maxima, once the compositor answered. A v2 answer wins over the
    /// v1 one, which is why `liquid` exists.
    capabilities: Option<Capabilities>,
    /// Whether `liquid_capabilities` has answered.
    liquid: bool,
    /// Whether a glass object exists for this surface.
    glass: bool,
    /// Whether the "the effect is not available" line has been said — once,
    /// however many ways it can be unavailable.
    glass_reported: bool,
    /// The glass request the surface is carrying; `None` is the effect off.
    applied_glass: Option<Request>,
    /// Whether the surface currently receives key events.
    focused: bool,
    /// When the clock's next tick is due.
    next_tick: Instant,
    /// Whether `SIGHUP` asked for a reload (the loop owns the host that does it).
    reload_requested: bool,
    /// Whether the reload thread has an answer ready (the ping source sets this).
    reload_pending: bool,
    /// Set by the compositor's `closed`, or by a stop request.
    stopped: bool,
    /// Whether the surface went away rather than the process being asked to stop.
    closed: bool,
    /// The window service's protocol objects and outstanding requests.
    windows: WindowService,
}

impl Panel {
    /// Starts the panel on a native shell surface.
    ///
    /// Order matters and is the ADR's: the role is installed and the placement
    /// sent **before** the window is shown (a shown window has an `xdg_toplevel`
    /// role already, and a `wl_surface` has exactly one), then the glass object
    /// is created, because the compositor answers a surface with no known
    /// desktop layer with a zero maximum.
    ///
    /// # Errors
    ///
    /// [`LegError::Surface`] with the host's own message: a non-Wayland platform,
    /// a compositor without `inox_shell_v1`, a document that produced no window.
    /// There is no fallback — a panel that cannot be placed must not pretend to
    /// be (ADR 0046 §9).
    pub fn start(
        shell: ShellConfig,
        tokens: Tokens,
        localizer: Rc<Localizer>,
    ) -> Result<Self, LegError> {
        let state = State::with_shell(shell, tokens, Rc::clone(&localizer));
        let surface = Surface::start(role_of(state.surface()))
            .map_err(|error| LegError::Surface(error.to_string()))?;

        let mut panel = Self {
            state,
            surface,
            interaction: Interaction::default(),
            scene: None,
            drawn: None,
            configured: None,
            workarea: None,
            applied: None,
            bits: 0,
            max_transition: 0,
            capabilities: None,
            liquid: false,
            glass: false,
            glass_reported: false,
            applied_glass: None,
            focused: false,
            next_tick: Instant::now() + app::TICK,
            reload_requested: false,
            reload_pending: false,
            stopped: false,
            closed: false,
            windows: WindowService::new(),
        };

        let model = panel.state.model();
        let placement = panel.placement(&model);

        panel.place(&placement)?;
        panel.ensure_glass();

        info!(
            surface = ?panel.state.surface(),
            role = ?role_of(panel.state.surface()),
            layer = ?placement.layer,
            exclusive_zone = placement.exclusive_zone,
            "the panel is running on a native inox_shell_surface_v1 surface"
        );

        Ok(panel)
    }

    /// The placement this surface asks for, given the projection it is about to
    /// draw.
    ///
    /// A dock is sized from the projection — its tiles, its tray labels and its
    /// own sentence — because the surface has to ask for a width that covers
    /// everything it will draw; a bar is stretched between its anchors and asks
    /// for none.
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

    /// Sends `placement`, and records it so a repeat is not sent twice.
    fn place(&mut self, placement: &PanelPlacement) -> Result<(), LegError> {
        self.surface
            .place(&geometry_of(placement))
            .map_err(|error| LegError::Surface(error.to_string()))?;

        self.surface
            .keyboard(keyboard_of(placement.keyboard_interactivity))
            .map_err(|error| LegError::Surface(error.to_string()))?;

        self.applied = Some(placement.clone());

        Ok(())
    }

    /// Creates the glass object, when the material asks for one.
    ///
    /// A material with neither a blur nor a refraction creates nothing at all —
    /// which is a different thing from creating an object and then learning the
    /// compositor will not draw on it. Both are degradations, and both are said
    /// once.
    fn ensure_glass(&mut self) {
        if self.glass {
            return;
        }

        let material = theme::glass_material(self.state.tokens());

        if !frost::asks_for_glass(self.state.surface(), material) {
            return;
        }

        match self.surface.start_glass() {
            Ok(()) => {
                self.glass = true;
                debug!("created the panel's inox_glass_v1 object on Qt's connection");
            }
            Err(error) if !self.glass_reported => {
                self.glass_reported = true;
                info!(
                    %error,
                    "the pane is drawn as plain translucency: no frosted glass on this surface"
                );
            }
            Err(_) => {}
        }
    }

    /// Carries the glass decision to the wire, and re-sends it when it changes.
    ///
    /// The decision itself — the clamps, the region, the reasons to draw plain
    /// translucency — is [`crate::frost`] over [`inox_glass::decide`]; this is
    /// only the transport. Nothing is sent before the compositor has answered,
    /// because both maxima are part of the request.
    fn sync_glass(&mut self) {
        if !self.glass {
            return;
        }

        // On a version 2 object the version 1 answer is deliberately **not**
        // acted on: it arrives first so a v1-only client is not left with a mute
        // object, but it cannot say whether the backdrop can be refracted, and
        // acting on it would send a blur-only request and immediately replace it
        // with the real one. Both answers arrive in the same batch.
        if self.surface.glass_version() >= GLASS_VERSION && !self.liquid {
            return;
        }

        let Some(capabilities) = self.capabilities else {
            return;
        };

        let material = theme::glass_material(self.state.tokens());
        let size = self.configured.unwrap_or((0, 0));

        let wanted = match frost::decide(self.state.surface(), material, capabilities, size) {
            Some(inox_glass::Outcome::Effect(request)) => Some(request),
            Some(inox_glass::Outcome::Plain(reason)) => {
                if !self.glass_reported {
                    match reason {
                        // The one line a real degradation gets: named once, and
                        // never a claimed effect.
                        inox_glass::Plain::CompositorCannotBlur => {
                            self.glass_reported = true;
                            info!(
                                "the compositor will not frost this surface's backdrop \
                                 (inox-glass-v1 max_radius = 0); the pane is drawn as plain \
                                 translucency"
                            );
                        }
                        inox_glass::Plain::RefractionDisabled => {
                            self.glass_reported = true;
                            info!(
                                "the compositor will not refract this surface's edge \
                                 (inox-glass-v1 liquid_capabilities report no refraction); the \
                                 pane is drawn as plain translucency"
                            );
                        }
                        inox_glass::Plain::BlurDisabled | inox_glass::Plain::EmptyPane => {}
                    }
                }

                None
            }
            None => None,
        };

        if wanted == self.applied_glass {
            return;
        }

        let request = wanted.unwrap_or_else(frost::off);

        if let Err(error) = self.surface.glass(&request) {
            warn!(%error, "failed to send the pane's glass parameters");
            return;
        }

        if wanted.is_some() {
            debug!(
                radius = request.radius,
                region = ?request.region,
                corners = ?request.corners,
                refraction = ?request.refraction,
                "asked the compositor for the pane's liquid glass"
            );
        }

        self.applied_glass = wanted;
    }

    /// Sends the motion hint the configuration's tokens ask for.
    ///
    /// Only once `capabilities` has arrived: a client that asked for an effect
    /// the compositor did not offer would make the shell and the compositor
    /// disagree about what the user is looking at, and reduced motion turns
    /// every transition into the instantaneous state change the protocol already
    /// guarantees.
    fn sync_transition(&mut self) -> Result<(), LegError> {
        let motion = self.state.tokens().motion;

        let effects =
            if motion.reduce_motion || !Surface::offered(self.bits, capability::TRANSITIONS) {
                0
            } else {
                // `effect.fade`: the compositor cross-fades the surface's own final
                // frame. A shell that animated the same movement itself would fight
                // it for the same pixels.
                1
            };

        let duration = if effects == 0 {
            0
        } else {
            motion.slow_ms.min(self.max_transition)
        };

        // An empty origin mask means the centre, which is what a bar wants; the
        // curve is `ease_out`, the desktop's own motion language.
        self.surface
            .transition(effects, effects, duration, 1, 0)
            .map_err(|error| LegError::Surface(error.to_string()))
    }

    /// The longest transition the compositor will draw, in milliseconds.
    ///
    /// Zero until `capabilities` arrives, which is why the hint is only sent
    /// after it: a duration the compositor never reported is a request it would
    /// clamp to nothing anyway.
    pub const fn max_transition(&self) -> u32 {
        self.max_transition
    }

    /// Drains Qt's dispatch once.
    ///
    /// The panel's only way of hearing the compositor: `pump` moves the surface's
    /// answers through Qt's dispatcher into the queue [`Panel::step`] reads, and
    /// QML's actions into the one [`Panel::drain_actions`] empties.
    pub fn pump(&mut self) {
        self.surface.pump();
    }

    /// Drains the compositor's answers and QML's actions, then sends whatever
    /// the new state asks for.
    ///
    /// # Errors
    ///
    /// [`LegError::Surface`] when the window is gone. A panel with no surface has
    /// nothing to show, and reporting it is what makes `main` exit non-zero
    /// instead of leaving a silent process behind.
    pub fn step(&mut self, now: Instant, qh: &QueueHandle<Self>) -> Result<(), LegError> {
        self.drain_events()?;
        self.drain_actions();
        self.service(qh);

        if now >= self.next_tick {
            // `update` does no I/O: the clock is this loop's, and the tick is
            // what produces the next projection.
            app::update(&mut self.state, Message::Tick(Local::now()));
            self.next_tick = now + app::TICK;
        }

        self.sync()
    }

    /// Reads every event the surface queued.
    fn drain_events(&mut self) -> Result<(), LegError> {
        while let Some(event) = self.surface.next_event() {
            match event {
                Event::Capabilities {
                    bits,
                    max_transition_ms,
                } => {
                    self.bits = bits;
                    self.max_transition = max_transition_ms;
                    info!(
                        bits,
                        max_transition_ms, "the compositor reported its inox-shell-v1 capabilities"
                    );
                    self.sync_transition()?;
                }
                Event::Configure { width, height } => {
                    if self.configured != Some((width, height)) {
                        debug!(width, height, "the surface was configured");
                        self.configured = Some((width, height));
                        // The entries move with the size, so the scene a click is
                        // matched against is stale from here on.
                        self.scene = None;
                    }
                }
                Event::Workarea {
                    x,
                    y,
                    width,
                    height,
                } => {
                    debug!(x, y, width, height, "the compositor reported the work area");
                    self.workarea = Some((x, y, width, height));
                }
                Event::Keyboard { focused } => {
                    debug!(focused, "the surface's keyboard state changed");
                    self.focused = focused;
                }
                Event::Closed => {
                    warn!(
                        "the compositor closed the panel's surface; stopping without a placed \
                         surface"
                    );
                    self.closed = true;
                    self.stopped = true;
                }
                Event::GlassCapabilities { max_radius } => {
                    if !self.liquid {
                        self.capabilities = Some(Capabilities::from_legacy(max_radius));
                    }
                }
                Event::LiquidCapabilities {
                    max_radius,
                    max_refraction_height,
                    max_refraction_amount,
                    flags,
                } => {
                    self.liquid = true;
                    self.capabilities = Some(Capabilities::from_liquid(
                        max_radius,
                        max_refraction_height,
                        max_refraction_amount,
                        flags,
                    ));
                    debug!(
                        max_radius,
                        max_refraction_height,
                        max_refraction_amount,
                        flags,
                        "the compositor reported its liquid-glass capabilities"
                    );
                }
                // Popups are the menu work ADR 0046 P3 describes and the panel
                // has nothing to open yet: a surface with no menu never receives
                // one of these, and logging is the honest answer until it does.
                Event::PopupDismissed { popup, reason } => {
                    debug!(popup, ?reason, "a popup was dismissed");
                }
                Event::PopupConfigure {
                    popup,
                    width,
                    height,
                } => {
                    debug!(popup, width, height, "a popup was configured");
                }
                Event::PopupKeyboard { popup, focused } => {
                    debug!(popup, focused, "a popup's keyboard state changed");
                }
            }
        }

        Ok(())
    }

    /// Applies every action QML dispatched.
    fn drain_actions(&mut self) {
        while let Some(bytes) = self.surface.next_action() {
            let Some(action) = parse_action(&bytes) else {
                debug!(
                    action = %String::from_utf8_lossy(&bytes),
                    "ignoring an action this build does not understand"
                );

                continue;
            };

            self.apply(action);
        }
    }

    /// Applies one action: a hover changes what is drawn, a click becomes the
    /// message the entry means.
    fn apply(&mut self, action: Action) {
        match action {
            Action::Hover { entry } => {
                // A hover is drawn from the entry's primary (left) action: which
                // button the user will use does not change what the entry looks
                // like while they rest on it.
                let hovered =
                    entry.and_then(|id| self.scene.as_ref().and_then(|scene| scene.action_of(id)));

                if self.interaction.hovered != hovered {
                    self.interaction.hovered = hovered;
                    self.drawn = None;
                }
            }
            Action::Tap { entry, button } => {
                let action: Option<EntryAction> = self
                    .scene
                    .as_ref()
                    .and_then(|scene| scene.entries.get(entry))
                    .and_then(|entry| entry.action_for(button));

                if let Some(message) = action.and_then(message_of) {
                    debug!(?message, "a click became a message");
                    app::update(&mut self.state, message);
                }
            }
        }
    }

    /// Carries out what the panel asked of the outside world.
    ///
    /// [`crate::window_service::WindowService`] turns each queued command into
    /// the `inox-window-manager-v1` request of the same name. Nothing here
    /// settles a request: the protocol confirms it with the state event it
    /// produced, which arrives through [`Panel`]'s dispatch below.
    fn service(&mut self, qh: &QueueHandle<Self>) {
        for command in self.state.take_outgoing() {
            match &command {
                OutgoingCommand::Snapshot => {
                    if self.windows.request_snapshot(qh) {
                        debug!("the panel asked the window service for its snapshot");
                    } else {
                        debug!(
                            "no inox-window-manager-v1; the panel's snapshot request has nowhere \
                             to go"
                        );
                    }
                }
                command => {
                    if !self.windows.dispatch(command) {
                        debug!(
                            "no window-list observer yet; the panel's request waits for the \
                             snapshot that creates one"
                        );
                    }
                }
            }
        }
    }

    /// Projects the state, and sends what changed: the placement, the projection,
    /// the glass parameters.
    fn sync(&mut self) -> Result<(), LegError> {
        let model = self.state.model();
        let placement = self.placement(&model);

        if self.applied.as_ref() != Some(&placement) {
            debug!(
                width = ?placement.width,
                height = placement.height,
                "the panel's placement changed; asking the compositor for it"
            );
            self.place(&placement)?;
        }

        // Before the first configure there is no size to lay anything out at.
        let Some((width, height)) = self.configured else {
            return Ok(());
        };

        let (tokens, shell, localizer) = (
            self.state.tokens(),
            self.state.shell(),
            self.state.localizer(),
        );
        let scene = view::scene(
            &model,
            tokens,
            shell,
            localizer,
            width as f32,
            height as f32,
            &self.interaction,
        );

        if self.drawn.as_ref() != Some(&scene.json) {
            self.surface
                .set_model(&scene.json)
                .map_err(|error| LegError::Surface(error.to_string()))?;

            self.drawn = Some(scene.json.clone());
        }

        self.scene = Some(scene);
        self.sync_glass();

        Ok(())
    }

    /// Applies one configuration reload the bus or `SIGHUP` prepared.
    pub fn apply_reload(&mut self, request: &reload::Request) {
        let result = reload::apply(&mut self.state, request);
        let applied = result.is_ok();

        request.finish(result);

        if applied {
            info!("applied a configuration reload");
            // The tokens may have moved the material under the surface: ask for
            // an object if the new material wants one, and re-project.
            self.drawn = None;
            self.ensure_glass();
            self.applied_glass = None;
        }
    }

    /// Whether the loop should stop.
    pub const fn is_stopped(&self) -> bool {
        self.stopped
    }

    /// The outcome the run ended in.
    pub const fn outcome(&self) -> Outcome {
        if self.closed {
            Outcome::ConnectionLost
        } else {
            Outcome::Stopped
        }
    }

    /// Destroys what this process holds, in the order the protocol wants.
    ///
    /// The window service's observer goes first, so the compositor never has to
    /// serve events for a list whose client is already tearing down. The surface
    /// is unmapped after it, which is what makes the compositor dismiss any popup
    /// this surface owns with `parent_hidden`.
    pub fn teardown(&mut self) {
        self.windows.teardown();
        self.surface.hide();
    }

    /// The size the compositor configured, for tests and logs.
    pub const fn configured(&self) -> Option<(u32, u32)> {
        self.configured
    }

    /// The work area the compositor reported.
    pub const fn workarea(&self) -> Option<(i32, i32, i32, i32)> {
        self.workarea
    }

    /// Whether a glass object exists for this surface.
    ///
    /// Not the same question as "is the effect on": the object exists as soon as
    /// the material asks for one, and what the compositor will draw is
    /// [`Panel::glass_capabilities`].
    pub const fn has_glass(&self) -> bool {
        self.glass
    }

    /// The glass maxima the compositor reported, once it reported them.
    pub const fn glass_capabilities(&self) -> Option<Capabilities> {
        self.capabilities
    }

    /// The projection the surface is carrying, when one was sent.
    pub const fn scene(&self) -> Option<&Scene> {
        self.scene.as_ref()
    }
}

impl Dispatch<WlRegistry, ()> for Panel {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
            && interface == "inox_window_manager_v1"
        {
            // Optional by design: a compositor without it leaves the panel with
            // its clock and tray and a readable "window service is not available"
            // state plus a retry (D05, ADR 0045 §4). The missing global is
            // reported after the registry has been read, not here.
            state
                .windows
                .bind_manager(registry.bind(name, version.min(1), qh, ()));
        }
    }
}

impl Dispatch<InoxWindowManagerV1, ()> for Panel {
    fn event(
        _state: &mut Self,
        _manager: &InoxWindowManagerV1,
        _event: <InoxWindowManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The manager advertises no events: it is a factory for list objects,
        // which is what carries the state.
    }
}

impl Dispatch<InoxWindowListV1, ()> for Panel {
    fn event(
        state: &mut Self,
        _list: &InoxWindowListV1,
        event: window_list::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(message) = state.windows.event(event) else {
            return;
        };

        app::update(&mut state.state, message);
    }
}

/// Runs the panel on its native shell surface until the process is asked to stop.
///
/// Two connections, deliberately (ADR 0046 §8): Qt owns the one the surface and
/// the glass object live on, and this one carries the window service. The loop
/// is `calloop`'s — signals, the reload host's ping and the pump interval — and
/// Qt's dispatcher is pumped from it rather than owning the thread.
///
/// # Errors
///
/// [`LegError::Surface`] when the surface cannot start, [`LegError::Connect`]
/// when the window service's connection cannot be made, and
/// [`LegError::EventLoop`] when the loop cannot be built. Every one of those is a
/// state `main` reports and exits non-zero for.
pub fn run(
    shell: ShellConfig,
    tokens: Tokens,
    localizer: Rc<Localizer>,
    paths: ConfigPaths,
) -> Result<Outcome, LegError> {
    // Every signal is taken **first**, before anything spawns a thread: Qt starts
    // its Wayland event thread when the application object is created, and a
    // `calloop` signal source blocks the signal for the thread that installs it.
    // A `SIGTERM` delivered to a thread that does not block it has its default
    // action, which kills the process before it can tear the surface down.
    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP])
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    let mut panel = Panel::start(shell, tokens, Rc::clone(&localizer))?;

    let connection =
        Connection::connect_to_env().map_err(|error| LegError::Connect(error.to_string()))?;
    let mut queue = connection.new_event_queue::<Panel>();
    let qh = queue.handle();
    // The registry object is kept alive by the connection; the bind happens in
    // the dispatch handler.
    let _registry = connection.display().get_registry(&qh, ());

    connection
        .flush()
        .map_err(|error| LegError::Connect(error.to_string()))?;
    queue
        .roundtrip(&mut panel)
        .map_err(|error| LegError::Connect(error.to_string()))?;

    // The window service is optional in exactly the same way the layer leg
    // treats it: unavailable is a designed state, not a failure. The panel says
    // so once, moves to `ServiceState::Unavailable` — which draws the existing
    // sentence and its retry button (D05) — and keeps running.
    if !panel.windows.is_available() && panel.windows.report_unavailable() {
        app::update(&mut panel.state, WindowService::unavailable_message());
    }

    let (wake, wake_source) =
        make_ping().map_err(|error| LegError::EventLoop(error.to_string()))?;
    let reload = reload::Host::start(paths, wake);

    let mut event_loop: EventLoop<Panel> =
        EventLoop::try_new().map_err(|error| LegError::EventLoop(error.to_string()))?;
    let handle = event_loop.handle();

    handle
        .insert_source(signals, |event, _, panel: &mut Panel| {
            match event.signal() {
                // The reload host reads and parses the candidate on a thread of its
                // own and hands the request back over the channel, which the ping
                // then wakes this loop for.
                Signal::SIGHUP => panel.reload_requested = true,
                signal => {
                    info!(?signal, "stopping the panel");
                    panel.stopped = true;
                }
            }
        })
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    handle
        .insert_source(wake_source, |_, _, panel: &mut Panel| {
            panel.reload_pending = true;
        })
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    // The window service's connection is a `calloop` source, and the queue it
    // dispatches into travels with the closure: a `QueueHandle` is a handle into
    // that queue, not a second one, so the main loop keeps using `qh`.
    let reader = connection.clone();

    handle
        .insert_source(
            Generic::new(reader, Interest::READ, Mode::Level),
            move |_, connection, panel: &mut Panel| {
                if let Some(guard) = connection.prepare_read() {
                    match guard.read() {
                        Ok(_) => {}
                        Err(wayland_client::backend::WaylandError::Io(error))
                            if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) => {
                            warn!(%error, "the window service's connection failed");
                            panel.stopped = true;
                        }
                    }
                }

                if let Err(error) = queue.dispatch_pending(panel) {
                    warn!(%error, "failed to dispatch window service events");
                    panel.stopped = true;
                }

                Ok(PostAction::Continue)
            },
        )
        .map_err(|error| LegError::EventLoop(error.to_string()))?;

    // The loop is one expression so that teardown happens on **every** way out,
    // including the failures: the requests still have to reach the compositor
    // before the process goes away, and a surface left mapped for a client that
    // is leaving is a strip the desktop keeps free for nobody.
    let outcome = serve(&mut panel, &connection, &qh, &mut event_loop, &reload);

    panel.teardown();

    outcome?;

    Ok(panel.outcome())
}

/// The loop itself: pump, react, and wait at most [`PUMP`] for the next reason
/// to act.
fn serve(
    panel: &mut Panel,
    connection: &Connection,
    qh: &QueueHandle<Panel>,
    event_loop: &mut EventLoop<Panel>,
    reload: &reload::Host,
) -> Result<(), LegError> {
    while !panel.is_stopped() {
        panel.pump();

        if panel.reload_requested {
            panel.reload_requested = false;
            info!("reloading the panel's configuration, as SIGHUP asked");
            reload.prepare();
        }

        if panel.reload_pending {
            panel.reload_pending = false;

            for request in reload.take_requests() {
                panel.apply_reload(&request);
            }
        }

        panel.step(Instant::now(), qh)?;

        connection
            .flush()
            .map_err(|error| LegError::Connect(error.to_string()))?;

        // A bounded wait rather than a blocking one: the panel has no signal for
        // "Qt has something", so the frame interval is what bounds its latency.
        event_loop
            .dispatch(Some(PUMP), panel)
            .map_err(|error| LegError::EventLoop(error.to_string()))?;
    }

    Ok(())
}

/// Runs the panel's loop with the same pump interval, for a test that drives it.
///
/// Not part of the shipped path: `run` is. It exists so
/// `src/headless_tests/native.rs` can exercise the state machine on a real
/// compositor without a clock of its own.
#[cfg(test)]
impl Panel {
    /// Steps until `done` accepts the panel, or `limit` passes.
    pub fn drive_until(&mut self, done: impl Fn(&Panel) -> bool, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;

        while Instant::now() < deadline {
            self.pump();

            // The window service has no connection in this test — one would need
            // a second registry and there is nothing to assert about a service
            // no compositor offers here — so only the surface half runs.
            if self.drain_events().is_err() {
                return false;
            }

            self.drain_actions();

            if self.sync().is_err() {
                return false;
            }

            if done(self) {
                return true;
            }

            std::thread::sleep(Duration::from_millis(5));
        }

        done(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taskbar_and_a_menu_bar_are_panels_and_a_dock_is_a_dock() {
        assert_eq!(role_of(PanelSurface::Taskbar), Role::Panel);
        assert_eq!(role_of(PanelSurface::Menubar), Role::Panel);
        assert_eq!(role_of(PanelSurface::Dock), Role::Dock);
    }

    #[test]
    fn a_taskbars_placement_becomes_the_protocols_own_numbers() {
        let config = inox_config::PanelConfig::default();
        let placement = PanelPlacement::derive(&config);
        let geometry = geometry_of(&placement);

        assert_eq!(geometry.layer, Layer::Top);
        assert_eq!(
            geometry.anchor,
            anchor::BOTTOM | anchor::LEFT | anchor::RIGHT,
            "the default taskbar sits on the bottom edge and spans the output"
        );
        assert_eq!(
            geometry.exclusive_zone,
            i32::try_from(placement.exclusive_zone).expect("a reserved strip fits in an i32"),
            "the strip the compositor keeps free is the placement's, unchanged"
        );
        assert_eq!(geometry.margin_top, i32::from(placement.margin));
        assert_eq!(geometry.margin_bottom, i32::from(placement.margin));
        assert_eq!(
            geometry.width, 0,
            "a bar anchored to both horizontal edges asks for no width: the compositor gives it \
             the output's"
        );
        assert_eq!(geometry.height, u32::from(placement.height));
    }

    #[test]
    fn a_surface_that_is_its_own_width_asks_for_it() {
        let placement = PanelPlacement {
            anchors: placement::Anchors::default(),
            exclusive_zone: 0,
            margin: 8,
            height: 68,
            width: Some(320),
            keyboard_interactivity: placement::KeyboardInteractivity::None,
            layer: placement::Layer::Top,
            autohide: placement::Autohide::Never,
        };

        let geometry = geometry_of(&placement);

        assert_eq!(geometry.width, 320, "a dock is as wide as the row it draws");
        assert_eq!(geometry.exclusive_zone, 0);
    }

    #[test]
    fn every_layer_and_keyboard_policy_reaches_the_protocol() {
        for (layer, expected) in [
            (placement::Layer::Background, Layer::Background),
            (placement::Layer::Bottom, Layer::Bottom),
            (placement::Layer::Top, Layer::Top),
            (placement::Layer::Overlay, Layer::Overlay),
        ] {
            let placement = PanelPlacement {
                anchors: placement::Anchors::default(),
                exclusive_zone: 0,
                margin: 0,
                height: 1,
                width: None,
                keyboard_interactivity: placement::KeyboardInteractivity::None,
                layer,
                autohide: placement::Autohide::Never,
            };

            assert_eq!(geometry_of(&placement).layer, expected);
        }

        assert_eq!(
            keyboard_of(placement::KeyboardInteractivity::None),
            KeyboardMode::None
        );
        assert_eq!(
            keyboard_of(placement::KeyboardInteractivity::OnDemand),
            KeyboardMode::OnDemand
        );
        assert_eq!(
            keyboard_of(placement::KeyboardInteractivity::Exclusive),
            KeyboardMode::Exclusive
        );
    }

    #[test]
    fn a_hover_names_an_entry_or_none() {
        assert_eq!(
            parse_action(br#"{"type":"hover","id":3}"#),
            Some(Action::Hover { entry: Some(3) })
        );
        assert_eq!(
            parse_action(br#"{"type":"hover","id":-1}"#),
            Some(Action::Hover { entry: None }),
            "-1 is QML's \"no entry\", which is the same thing as an absent one"
        );
    }

    #[test]
    fn a_tap_carries_the_entry_and_the_button_it_used() {
        for (code, button) in [
            (272, Button::Left),
            (273, Button::Right),
            (274, Button::Middle),
        ] {
            assert_eq!(
                parse_action(format!(r#"{{"type":"tap","id":7,"button":{code}}}"#).as_bytes()),
                Some(Action::Tap { entry: 7, button })
            );
        }
    }

    #[test]
    fn a_button_the_panel_has_no_meaning_for_is_dropped() {
        assert_eq!(
            parse_action(br#"{"type":"tap","id":7,"button":200}"#),
            None,
            "a button outside `BTN_LEFT`..`BTN_EXTRA` is not a panel verb"
        );
        assert_eq!(
            parse_action(br#"{"type":"tap","id":7,"button":275}"#),
            Some(Action::Tap {
                entry: 7,
                button: Button::Back
            }),
            "a side button is still a button; the entry is what has no meaning for it, and \
             `Entry::action_for` is where that is decided"
        );
    }

    #[test]
    fn a_malformed_action_is_dropped_rather_than_fatal() {
        for bytes in [
            b"".as_slice(),
            b"not json".as_slice(),
            br#"{"id":3}"#.as_slice(),
            br#"{"type":"tap","id":3}"#.as_slice(),
            br#"{"type":"scroll","id":3}"#.as_slice(),
            br#"{"type":"hover","id":"three"}"#.as_slice(),
        ] {
            assert_eq!(
                parse_action(bytes),
                None,
                "{} is not an action this build knows",
                String::from_utf8_lossy(bytes)
            );
        }
    }
}
