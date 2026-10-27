//! The protocol leg: taking the compositor's session lock, as its client.
//!
//! the session-lock boundary puts the lock state in the compositor and makes this crate the
//! trusted client of `ext-session-lock-v1`. This module is that client — the half
//! of the component that talks to the wire, kept separate from the state machine
//! in [`crate::state`] because the two answer different questions: the state
//! machine owns what a lock *means* (and is exercised without a compositor), this
//! module owns the requests and events the protocol defines.
//!
//! # What it does
//!
//! It connects to the session's Wayland display, claims the trusted lock role over
//! the private `inox-ipc` protocol with the token `inox-session` delivered, asks
//! for the lock, gives every output a lock surface, and draws
//! [`crate::app::State`]'s projection onto each of them — the same widget tree
//! [`crate::render`] gives the render tests, at the size the compositor
//! configured.
//!
//! The state is the same state the interface is drawn from, and this module feeds
//! it: a protocol event becomes an [`app::Message`], and everything the state
//! decides to ask of the outside world comes back out of
//! [`app::State::take_outgoing`] and is turned into protocol requests here. There
//! is no second copy of "is the session locked", which is the point — a protocol
//! leg with its own idea of coverage could confirm a lock the interface believed
//! was still being established.
//!
//! # What it asks of the outside world
//!
//! Authentication is not done here — this crate must not be able to check a
//! password — so an attempt becomes one `auth-v1` conversation on its own thread
//! ([`crate::boundary`]) and its answer comes back through the loop as a
//! [`boundary::Report`]. Only [`crate::contract::AuthOutcome::Success`] can move the
//! state machine towards `Unlocking`, and only that state queues
//! [`OutgoingCommand::Unlock`], which is the one place `unlock_and_destroy` is sent.
//! A signal still unlocks nothing: `SIGTERM` stops the process with the session
//! locked, and the operator is told so.
//!
//! The credential is typed at the lock surface: `wl_seat`/`wl_keyboard` are bound,
//! the keymap the compositor sends is compiled by [`crate::input`], and a key
//! becomes an [`app::Message`] — never a direct decision. What that means for the
//! screen is still the projection's business: input is accepted only once the
//! compositor has confirmed the lock.
//!
//! # What it deliberately does not do
//!
//! It reads no pointer and no touch: the layout lives in QML, so hit-testing here
//! would be a second copy of the layout maths, and the lock screen's whole
//! interaction is typing. It has no input-method or compose support (a dead key
//! types nothing), and no key repeat (nothing in this session synthesises repeats).
//! It has no suspend transport: [`OutgoingCommand::AllowSuspend`] is logged and
//! dropped until logind is wired.
//!

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Write as _},
    os::fd::{AsFd, OwnedFd},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use calloop::{
    EventLoop, Interest, Mode, PostAction,
    channel::{Channel, Event as ChannelEvent, Sender},
    generic::Generic,
    signals::{Signal, Signals},
    timer::{TimeoutAction, Timer},
};
use inox_common::i18n::Localizer;
use inox_config::Theme as Tokens;
use inox_protocols::{
    ipc::inox_ipc::client::inox_ipc as ipc,
    standard::{
        core::client::{
            wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_keyboard::WlKeyboard,
            wl_output::WlOutput, wl_registry::WlRegistry, wl_seat::WlSeat, wl_shm::WlShm,
            wl_shm_pool::WlShmPool, wl_surface::WlSurface,
        },
        ext::session_lock::client::{
            ext_session_lock_manager_v1 as manager, ext_session_lock_surface_v1 as lock_surface,
            ext_session_lock_v1 as lock,
        },
    },
};
use tracing::{debug, error, info, warn};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};

use crate::app::{self, Message, OutgoingCommand, State};
use crate::boundary;
use crate::contract::ServiceFailure;
use crate::input;
use crate::render;
use crate::state::{LockReason, OutputId};
use crate::view_model::LockViewModel;

/// How long the compositor may take to confirm the lock before this client stops
/// waiting for it.
///
/// The wait is bounded because the alternative is a process holding a lock
/// request nobody answered with nothing on screen. The bound is generous rather than tight: confirming means the
/// compositor has drawn a locked frame, which on a software-rendered host is work
/// rather than a round trip. Stopping is safe in the direction that matters —
/// whatever the compositor accepted it keeps, and this process reports that it
/// stopped before the lock was confirmed.
const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(10);

/// How many frames one output may have in flight before this client stops
/// committing new ones.
///
/// A compositor releases a buffer when it replaces it on the surface, so the
/// steady state is one buffer being displayed and one being handed over — two.
/// This leaves room for two more so that a release still in flight cannot stall a
/// redraw. Past this many, the frame already on the surface is left there: it is a
/// covering frame, and a covering frame one state behind is not a failure of the
/// lock. The bound exists so a compositor that never releases anything cannot have
/// this process allocate a frame per redraw for ever.
pub const MAX_FRAMES_IN_FLIGHT: usize = 4;

/// What taking the lock ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The compositor refused: the session was already locked, this connection had
    /// not claimed the lock role, or no more locks are being accepted. Nothing was covered, and nothing is left holding anything.
    Refused,
    /// The lock is no longer held: this client asked the compositor to release it
    /// after a successful authentication ([`Outcome::Unlocked`]), or the
    /// compositor withdrew it with the protocol's `finished` event.
    Released,
    /// The client released the lock on the strength of an authentication success.
    ///
    /// Distinct from [`Outcome::Released`] because the two are different facts: a
    /// withdrawal is the compositor's decision, a release is one this component
    /// caused and is allowed to report.
    Unlocked,
    /// The process was asked to stop after the lock was confirmed. The lock is
    /// deliberately **still held**: a signal is not a credential, and releasing on
    /// one would turn "log out" into "unlock".
    StoppedLocked,
    /// The process was asked to stop before the compositor confirmed the lock.
    ///
    /// Whether anything is locked afterwards is the compositor's answer to give:
    /// this client asked and did not hear back, and a compositor that accepted the
    /// request keeps the session locked when the client disappears.
    StoppedBeforeConfirmation,
}

/// Why the lock could not be taken at all.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The display could not be reached.
    #[error("no Wayland display to lock: {0}")]
    Connect(String),
    /// A protocol the lock needs is not advertised.
    #[error("the compositor did not advertise {0}")]
    MissingGlobal(&'static str),
    /// No token was delivered, so the compositor would refuse the claim anyway.
    #[error("no session lock token was delivered (INOX_LOCK_TOKEN)")]
    NoToken,
    /// The event loop failed.
    #[error("the session lock event loop failed: {0}")]
    EventLoop(String),
    /// A shared-memory frame could not be built.
    #[error("failed to prepare the lock frame: {0}")]
    Frame(#[from] io::Error),
}

/// One frame committed to one lock surface, and whether the compositor is done
/// with it.
struct Frame {
    /// The buffer attached to the surface.
    buffer: WlBuffer,
    /// Whether `wl_buffer.release` has arrived for it. A buffer the compositor may
    /// still be reading must not be freed; one it has released may be.
    released: bool,
}

/// One output, and everything this client holds for it.
struct OutputEntry {
    /// The registry name of the `wl_output` global, which is how an output is
    /// recognised when it is removed.
    global: u32,
    /// What the state machine calls this output.
    id: OutputId,
    /// A name for the log line; the compositor's own name for the connector is not
    /// part of this protocol.
    name: String,
    output: WlOutput,
    surface: Option<WlSurface>,
    role: Option<lock_surface::ExtSessionLockSurfaceV1>,
    /// The interface drawn on this output's lock surface, once it has been
    /// configured to a size this component will draw.
    interface: Option<render::Interface>,
    /// The size the compositor last configured this surface to.
    configured: Option<(u32, u32)>,
    /// The frames committed to this surface that the compositor has not released.
    frames: Vec<Frame>,
}

/// Everything one connection to the compositor holds.
struct Client {
    globals: HashMap<String, (u32, u32)>,
    manager: Option<manager::ExtSessionLockManagerV1>,
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    ipc: Option<ipc::InoxIpc>,
    lock: Option<lock::ExtSessionLockV1>,
    /// One entry per advertised output, with the lock surface covering it.
    outputs: Vec<OutputEntry>,
    /// The lock screen itself: the state every frame is drawn from, and the queue
    /// of requests the state puts on the outside world.
    state: State,
    /// The projection that is on the outputs right now.
    ///
    /// Every output shows the same state, so one copy is enough to answer "has
    /// anything changed since the frames were drawn?" — which is the only reason
    /// this exists.
    drawn: Option<LockViewModel>,
    /// Whether a surface has been configured since the last frame, so the output
    /// it belongs to still needs drawing even though the state has not moved.
    pending_repaint: bool,
    /// Where the authentication boundary listens.
    auth_socket: PathBuf,
    /// The account this session's uid names, or `None` when it could not be
    /// resolved — in which case the screen is told so and cannot offer a prompt.
    auth_account: Option<String>,
    /// The channel the boundary's detached threads answer through. Each attempt is
    /// one [`boundary::authenticate`] call and one [`boundary::Report`] back.
    reports: Sender<boundary::Report>,
    /// The seat, the keyboard this client asked it for, and what a keycode means
    /// under the keymap that keyboard declared.
    seat: Option<WlSeat>,
    keyboard: Option<WlKeyboard>,
    keys: Option<input::Keyboard>,
    /// The lock surface the keyboard is currently focused on, when it is focused
    /// on one of ours.
    focused: Option<WlSurface>,
    /// Whether this client asked the compositor to release the lock after a
    /// successful authentication.
    unlocked: bool,
    locked: bool,
    finished: bool,
    stopped: bool,
    /// Frames committed, which is the observable proof that the client covered
    /// the outputs rather than merely asking to.
    committed: u64,
}

/// The bytes of one pixel in `wl_shm`'s `Argb8888`, which is little-endian
/// `B, G, R, A` — the same order the compositor's own read-back tests assert on.
///
/// Kept here as well as in [`crate::render`] because it is the one place this
/// module still needs it on its own: the test below asserts the frame's alpha, and
/// that assertion is what says a lock frame is a cover.
#[cfg(test)]
fn argb_bytes(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[2], rgb[1], rgb[0], 0xff]
}

/// An anonymous shared-memory file for one frame.
fn frame_memory(name: &str) -> io::Result<OwnedFd> {
    rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::CLOEXEC).map_err(io::Error::from)
}

/// What the state machine calls an output the registry knows by `global`.
fn output_id(global: u32) -> OutputId {
    OutputId::new(format!("output-{global}"))
}

impl Dispatch<WlRegistry, ()> for Client {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: inox_protocols::standard::core::client::wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use inox_protocols::standard::core::client::wl_registry as registry_event;

        match event {
            registry_event::Event::Global {
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
                    "ext_session_lock_manager_v1" => {
                        state.manager = Some(registry.bind(name, version.min(1), qh, ()));
                    }
                    "inox_ipc" => {
                        state.ipc = Some(registry.bind(name, version.min(2), qh, ()));
                    }
                    // The keyboard, for the credential field. It is not a required
                    // global: a compositor with no seat still has to be covered, and
                    // the screen simply says it cannot take input.
                    "wl_seat" => {
                        state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                    }
                    "wl_output" => {
                        let output: WlOutput = registry.bind(name, version.min(4), qh, ());
                        let id = output_id(name);
                        state.outputs.push(OutputEntry {
                            global: name,
                            id: id.clone(),
                            name: id.to_string(),
                            output,
                            surface: None,
                            role: None,
                            interface: None,
                            configured: None,
                            frames: Vec::new(),
                        });
                        // A display that arrives while the session is locked has to
                        // be covered too. Telling the state is what asks for a lock
                        // surface for it, because the state is what decides what
                        // every output needs.
                        let _ = app::update(&mut state.state, Message::OutputAdded(id));
                    }
                    _ => {}
                }
            }
            registry_event::Event::GlobalRemove { name } => {
                let removed = state.outputs.iter().position(|entry| entry.global == name);

                if let Some(index) = removed {
                    let entry = state.outputs.remove(index);
                    // A display that is unplugged takes its lock surface with it,
                    // and the protocol expects the client to say so. Dropping a
                    // `wayland-client` proxy sends nothing — the destructor is a
                    // request like any other — so it has to be asked for. The role
                    // goes first, because the surface is what it was placed on.
                    if let Some(role) = entry.role {
                        role.destroy();
                    }
                    if let Some(surface) = entry.surface {
                        surface.destroy();
                    }
                    debug!(output = %entry.name, "an output went away; its lock surface was destroyed");
                    let _ = app::update(&mut state.state, Message::OutputRemoved(entry.id));
                }
            }
            _ => {}
        }
    }
}

/// The seat: what this client asks it for, and nothing else.
impl Dispatch<WlSeat, ()> for Client {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: inox_protocols::standard::core::client::wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use inox_protocols::standard::core::client::wl_seat as seat_event;

        if let seat_event::Event::Capabilities { capabilities } = event {
            let has_keyboard = matches!(
                capabilities,
                WEnum::Value(capabilities)
                    if capabilities.contains(seat_event::Capability::Keyboard)
            );

            match (has_keyboard, state.keyboard.is_some()) {
                (true, false) => {
                    state.keyboard = Some(seat.get_keyboard(qh, ()));
                    debug!("the seat offers a keyboard; asking for it");
                }
                (false, true) => {
                    // The keyboard went away. Keys stop meaning anything, and the
                    // screen keeps the covered frame it already has.
                    state.keyboard = None;
                    state.keys = None;
                    state.focused = None;
                    warn!("the seat no longer offers a keyboard; the screen cannot take input");
                }
                _ => {}
            }
        }
    }
}

/// The keyboard: a keymap, then key events that only matter on a lock surface.
impl Dispatch<WlKeyboard, ()> for Client {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: inox_protocols::standard::core::client::wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use inox_protocols::standard::core::client::wl_keyboard as keyboard_event;

        match event {
            keyboard_event::Event::Keymap { format, fd, .. } => {
                use inox_protocols::standard::core::client::wl_keyboard::KeymapFormat;

                if !matches!(format, WEnum::Value(KeymapFormat::XkbV1)) {
                    warn!(
                        ?format,
                        "the compositor sent a keymap format this screen cannot read"
                    );
                    state.keys = None;

                    return;
                }

                state.keys = input::Keyboard::from_keymap_fd(fd);

                if state.keys.is_none() {
                    warn!("the keymap could not be compiled; no key will type anything");
                }
            }
            keyboard_event::Event::Enter { surface, .. } => {
                // One lock surface has the keyboard at a time; which one is the
                // compositor's decision and can move between outputs.
                state.focused = Some(surface);
                if let Some(keys) = state.keys.as_mut() {
                    keys.reset();
                }
            }
            keyboard_event::Event::Leave { surface, .. } => {
                if state.focused.as_ref() == Some(&surface) {
                    state.focused = None;
                }
                if let Some(keys) = state.keys.as_mut() {
                    keys.reset();
                }
            }
            keyboard_event::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(keys) = state.keys.as_mut() {
                    keys.update_modifiers(mods_depressed, mods_latched, mods_locked, group);
                }
            }
            keyboard_event::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                use inox_protocols::standard::core::client::wl_keyboard::KeyState;

                match key_state {
                    WEnum::Value(KeyState::Pressed) => state.key_pressed(key),
                    WEnum::Value(KeyState::Released) => {
                        if let Some(keys) = state.keys.as_mut() {
                            keys.release(key);
                        }
                    }
                    WEnum::Value(_) | WEnum::Unknown(_) => {}
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, ()> for Client {
    fn event(
        state: &mut Self,
        buffer: &WlBuffer,
        event: inox_protocols::standard::core::client::wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use inox_protocols::standard::core::client::wl_buffer as buffer_event;

        if let buffer_event::Event::Release = event {
            // The compositor has finished with this frame. It is not destroyed here
            // — destroying a proxy inside its own event handler is the kind of thing
            // that reads as a use-after-free later — it is marked, and the next
            // commit on that output destroys it.
            for entry in &mut state.outputs {
                for frame in &mut entry.frames {
                    if &frame.buffer == buffer {
                        frame.released = true;
                    }
                }
            }
        }
    }
}

impl Client {
    /// Create the lock object the state asked for.
    ///
    /// The protocol asks a client to create a lock surface for every output as
    /// soon as it has a lock object, so the state asks for the lock and for the
    /// outputs' surfaces in the same breath; this is the half that reaches the
    /// wire.
    fn take_lock(&mut self, qh: &QueueHandle<Self>, reason: LockReason) {
        if self.lock.is_some() {
            return;
        }

        let Some(manager) = self.manager.clone() else {
            warn!(?reason, "the lock was asked for with no manager to ask");
            return;
        };

        self.lock = Some(manager.lock(qh, ()));
        info!(?reason, "asked the compositor for the session lock");
    }

    /// Give `id`'s output a lock surface, if it does not have one yet.
    fn cover(&mut self, id: &OutputId, qh: &QueueHandle<Self>) {
        let (Some(lock), Some(compositor)) = (self.lock.clone(), self.compositor.clone()) else {
            return;
        };

        let Some(entry) = self.outputs.iter_mut().find(|entry| &entry.id == id) else {
            return;
        };

        if entry.role.is_some() {
            return;
        }

        let surface = entry
            .surface
            .get_or_insert_with(|| compositor.create_surface(qh, ()))
            .clone();
        entry.role = Some(lock.get_lock_surface(&surface, &entry.output, qh, ()));
        info!(output = %entry.name, "created a lock surface");
    }

    /// Draw every configured output, if anything has changed since the last frame.
    ///
    /// Ask the compositor to take the lock away.
    ///
    /// Only ever reached through [`OutgoingCommand::Unlock`], which
    /// [`crate::app::update`] queues from the one branch that interprets
    /// [`crate::contract::AuthOutcome::Success`]. The compositor destroys the lock
    /// object and every lock surface in response, so the loop ends here rather than
    /// waiting for a `finished` event the protocol does not send for this request.
    fn unlock(&mut self) {
        let Some(lock) = self.lock.take() else {
            warn!("an unlock was asked for with no lock to release");

            return;
        };

        if !self.locked {
            // The protocol refuses this from a client the compositor never
            // confirmed, and asking would only produce that refusal.
            warn!("an unlock was asked for before the compositor confirmed the lock");

            return;
        }

        lock.unlock_and_destroy();
        self.unlocked = true;
        // No `Message::UnlockConfirmed`: the protocol defines no event for a
        // client-initiated release, so feeding one would be inventing a
        // confirmation nobody sent. `Phase::Unlocking` is the last phase this
        // process sees, and the process ending is the only thing left to do.
        info!("the boundary accepted the user: the compositor was asked to release the lock");
    }

    /// One key press, if it happened on a lock surface and means anything.
    fn key_pressed(&mut self, keycode: u32) {
        let Some(surface) = self.focused.as_ref() else {
            return;
        };

        // Keys arrive for whichever surface holds the focus. A surface that is not
        // one of ours is not ours to read either.
        if !self
            .outputs
            .iter()
            .any(|entry| entry.surface.as_ref() == Some(surface))
        {
            return;
        }

        let Some(key) = self.keys.as_mut().and_then(|keys| keys.press(keycode)) else {
            return;
        };

        // The only thing a key can do. Nothing here submits, cancels or unlocks on
        // its own: the key becomes a message, and the state machine decides.
        if let Some(message) = app::key_binding(&key) {
            let _ = app::update(&mut self.state, message);
        }
    }

    /// Acts on one answer from the authentication boundary.
    ///
    /// The generation check is the whole of the safety here, and it is made before
    /// anything is fed to the state: an answer about an attempt that is no longer in
    /// flight — cancelled, superseded, or answered already — is a fact about a
    /// question nobody is asking any more.
    fn auth_report(&mut self, report: boundary::Report) {
        let generation = report.generation();

        if !self.state.boundary().is_current(generation) {
            warn!(%generation, "an answer about an attempt that is not in flight; ignored");

            return;
        }

        match report {
            boundary::Report::Answered { outcome, .. } => {
                let _ = app::update(&mut self.state, Message::AuthAnswered(generation, outcome));
            }
            boundary::Report::Unavailable { failure, .. } => {
                let _ = app::update(&mut self.state, Message::AuthUnavailable(failure));
            }
        }
    }

    /// A lock surface is not repainted by a frame clock: there is no window here
    /// and nothing to drive one. Frames are committed when there is something new
    /// to say — a surface that has just been configured, or a state that has
    /// moved — and otherwise the frame already on the screen is left alone.
    fn repaint(&mut self, qh: &QueueHandle<Self>) {
        let model = self.state.model().clone();

        if self.drawn.as_ref() == Some(&model) && !self.pending_repaint {
            return;
        }

        // Recorded before the frames are committed, not after: committing one
        // tells the state machine that an output is covered, which can move the
        // projection — and the frames this pass draws are of the projection it
        // started with. Noticing that change on the next pass is what keeps a
        // later state change from being mistaken for one already on screen.
        self.drawn = Some(model);

        let mut every_output_drawn = true;
        let mut drew_something = false;

        for index in 0..self.outputs.len() {
            if self.outputs[index].interface.is_none() {
                continue;
            }

            drew_something = true;

            match self.commit_frame(index, qh) {
                Ok(true) => {}
                Ok(false) => every_output_drawn = false,
                Err(error) => {
                    every_output_drawn = false;
                    error!(%error, output = %self.outputs[index].name, "failed to draw the lock screen");
                }
            }
        }

        // An output that refused this pass is asked again on the next one, which
        // the compositor's `wl_buffer.release` is what wakes. Clearing the request
        // regardless would leave that output a state behind until something else
        // happened to change the projection. With nothing configured yet there is
        // nothing to redraw either.
        self.pending_repaint = !every_output_drawn && drew_something;
    }

    /// Draw the lock screen onto an output's surface and commit it.
    ///
    /// Returns whether a frame was committed. Every reason not to is a reason the
    /// output is still covered by the compositor's own locked blank, so
    /// a refusal here is never a hole in the lock — which is why most of them are
    /// logged and not treated as failures.
    fn commit_frame(&mut self, index: usize, qh: &QueueHandle<Self>) -> Result<bool, SessionError> {
        let Some(shm) = self.shm.clone() else {
            return Err(SessionError::MissingGlobal("wl_shm"));
        };

        let covered = {
            let entry = &mut self.outputs[index];

            if entry.role.is_none() {
                return Ok(false);
            }

            let Some(surface) = entry.surface.clone() else {
                return Ok(false);
            };
            let Some((width, height)) = entry.configured else {
                // Nothing has been configured yet, so there is no size the protocol
                // would accept a frame of.
                return Ok(false);
            };
            let Some(interface) = entry.interface.as_mut() else {
                return Ok(false);
            };

            // The two are set together by the configure handler, and if they ever
            // came apart the frame would declare one size and hold another size's
            // pixels. Refusing is the safe answer: the output keeps the covering
            // frame it already has. The comparison is on the integers the
            // configure delivered, never on an `f32` that could round them.
            if interface.dimensions() != (width, height) {
                warn!(
                    output = %entry.name,
                    configured = ?(width, height),
                    drawn = ?interface.dimensions(),
                    "the interface is not the size the surface was configured to; \
                     not committing a frame of the wrong size"
                );
                return Ok(false);
            }

            // Buffers the compositor has finished with can go; the ones it has not
            // are the frames it may still be reading.
            let in_flight = entry.frames.iter().filter(|frame| !frame.released).count();
            if in_flight >= MAX_FRAMES_IN_FLIGHT {
                warn!(
                    output = %entry.name,
                    in_flight,
                    "the compositor has not released the frames already committed to \
                     this output; leaving the covering frame that is there"
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

            let mut file = File::from(frame_memory("inox-lock-frame")?);
            file.write_all(&bytes)?;

            let pool: WlShmPool = shm.create_pool(file.as_fd(), size, qh, ());
            let buffer = pool.create_buffer(
                0,
                width as i32,
                height as i32,
                stride,
                inox_protocols::standard::core::client::wl_shm::Format::Argb8888,
                qh,
                (),
            );
            // The buffer holds its own reference to the memory, so the pool has
            // done its job.
            pool.destroy();

            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, width as i32, height as i32);
            surface.commit();

            // A frame the compositor has released is asked to go, not merely
            // dropped: dropping a `wayland-client` proxy sends nothing, and the
            // compositor keeps a shared-memory mapping plus the pool behind every
            // buffer until `wl_buffer.destroy` or this client's connection ends.
            // A lock lasts hours, so "it will be freed when the process exits" is
            // not good enough — it is a full-screen mapping held for the whole
            // session, one per frame this client ever commits.
            let (released, kept): (Vec<Frame>, Vec<Frame>) =
                entry.frames.drain(..).partition(|frame| frame.released);

            for frame in &released {
                frame.buffer.destroy();
            }

            entry.frames = kept;
            entry.frames.push(Frame {
                buffer,
                released: false,
            });

            debug!(output = %entry.name, width, height, "drew the lock screen onto an output");

            entry.id.clone()
        };

        // Standing here — the protocol has the frame — is what "this output is
        // covered" means, and the state machine is told so rather than being asked
        // to assume it.
        let _ = app::update(&mut self.state, Message::OutputCovered(covered));
        self.committed = self.committed.saturating_add(1);

        Ok(true)
    }
}

impl Dispatch<lock::ExtSessionLockV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &lock::ExtSessionLockV1,
        event: lock::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            lock::Event::Locked => {
                state.locked = true;
                let reported = state.state.machine().reports_locked();

                if reported {
                    info!(
                        outputs = state.outputs.len(),
                        committed = state.committed,
                        "the compositor confirmed the session lock"
                    );
                } else {
                    // The compositor confirms only once every output has a frame,
                    // so this would mean the two sides disagree about coverage. It
                    // is the interface half of the lock saying so, because it is
                    // the half a person would otherwise be looking at a screen
                    // that had not drawn anything yet.
                    warn!(
                        outputs = state.outputs.len(),
                        committed = state.committed,
                        "the compositor confirmed the lock before every output was \
                         recorded as covered"
                    );
                }
            }
            lock::Event::Finished => {
                // The protocol's refusal or withdrawal. Either way there is nothing
                // to unlock: `finished` without `locked` means the lock was never
                // taken, and with `locked` it means the compositor has released it.
                state.finished = true;
                warn!("the compositor withdrew the session lock");
            }
            _ => {}
        }
    }
}

impl Dispatch<lock_surface::ExtSessionLockSurfaceV1, ()> for Client {
    fn event(
        state: &mut Self,
        role: &lock_surface::ExtSessionLockSurfaceV1,
        event: lock_surface::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let lock_surface::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            // The protocol makes acking the first configure a precondition for the
            // first commit, and the committed size an exact requirement.
            role.ack_configure(serial);

            let Some(index) = state
                .outputs
                .iter()
                .position(|entry| entry.role.as_ref() == Some(role))
            else {
                return;
            };

            state.outputs[index].configured = Some((width, height));

            // The size and the interface are set together, because a frame's
            // *pixels* come from the interface and its *dimensions* come from here.
            // Letting them disagree would commit a buffer declaring one size and
            // holding another size's pixels — a short frame, or the rest of the
            // memory as it happened to be.
            let entry = &mut state.outputs[index];

            match entry.interface.as_mut() {
                // A resize that cannot draw the requested size is answered by
                // asking the interface what size it actually has, rather than by
                // trusting it to have taken the new one.
                Some(interface) => {
                    interface.resize(width, height);

                    if interface.dimensions() != (width, height) {
                        entry.interface = None;
                    }
                }
                None => {
                    entry.interface = render::Interface::new(width, height);
                }
            }

            if entry.interface.is_none() {
                entry.configured = None;
                warn!(
                    output = %entry.name,
                    width,
                    height,
                    "this is not a size the lock screen will draw; the output keeps \
                     the compositor's locked blank instead"
                );
                return;
            }

            entry.configured = Some((width, height));

            // Drawn by the loop's repaint pass rather than here: the pass draws
            // every configured output, which is what makes an output that arrives
            // while another one is already covered show the same frame.
            state.pending_repaint = true;
            let _ = qh;
        }
    }
}

// The interfaces this client only consumes, or does not consume at all.
wayland_client::delegate_noop!(Client: ignore WlCompositor);
wayland_client::delegate_noop!(Client: ignore WlSurface);
wayland_client::delegate_noop!(Client: ignore WlShm);
wayland_client::delegate_noop!(Client: ignore WlShmPool);
wayland_client::delegate_noop!(Client: ignore WlOutput);
wayland_client::delegate_noop!(Client: ignore manager::ExtSessionLockManagerV1);

impl Dispatch<ipc::InoxIpc, ()> for Client {
    fn event(
        _: &mut Self,
        _: &ipc::InoxIpc,
        _: ipc::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// Take the session lock and hold it until the process is asked to stop.
///
/// # Errors
///
/// Returns [`SessionError`] when the display cannot be reached, when a protocol
/// the lock needs is missing, or when no token was delivered — every one of which
/// is a state the caller must report rather than paper over.
pub fn take(
    token: Option<&str>,
    localizer: Rc<Localizer>,
    tokens: Tokens,
) -> Result<Outcome, SessionError> {
    let Some(token) = token.filter(|token| !token.is_empty()) else {
        return Err(SessionError::NoToken);
    };

    // The signal source is created before anything else, and that order is
    // load-bearing: `signalfd` only receives a signal that is blocked in *every*
    // thread, and a thread created before the mask is installed does not inherit
    // it. The offscreen QML host is exactly such a component — it is built when
    // the first output is covered, further down this function — so the mask goes
    // in first and the source is handed to the loop by `drive`.
    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP])
        .map_err(|error| SessionError::EventLoop(error.to_string()))?;

    let connection =
        Connection::connect_to_env().map_err(|error| SessionError::Connect(error.to_string()))?;
    let mut queue = connection.new_event_queue::<Client>();
    let qh = queue.handle();
    // The registry object is kept alive by the connection; every bind below goes
    // through it from the dispatch handler.
    let _registry = connection.display().get_registry(&qh, ());

    // The channel the boundary's threads answer through. It is created here, before
    // the client, because the client holds the sending half: an attempt is dispatched
    // from the pump and its answer arrives as an event on this loop, never as a
    // blocking call inside it.
    let (reports, report_channel) = calloop::channel::channel();

    // The boundary is probed before anything is drawn, so the first frame is honest.
    // `auth_account` is the password-database entry for the uid this process runs
    // under (never the environment), and a socket with nobody listening is a
    // boundary that is not running — both are said on screen rather than papered
    // over with a password prompt nobody could check.
    let auth_socket = boundary::socket();
    let auth_account = boundary::session_account();
    let boundary_available = auth_account.is_some() && boundary::is_available(&auth_socket);

    let mut client = Client {
        globals: HashMap::new(),
        manager: None,
        compositor: None,
        shm: None,
        ipc: None,
        lock: None,
        outputs: Vec::new(),
        state: State::new(localizer.clone(), tokens),
        drawn: None,
        pending_repaint: false,
        auth_socket,
        auth_account,
        reports,
        seat: None,
        keyboard: None,
        keys: None,
        focused: None,
        unlocked: false,
        locked: false,
        finished: false,
        stopped: false,
        committed: 0,
    };

    // The registry has to be read before anything can be bound, and the wait is
    // bounded by the event loop below rather than by a retry loop.
    connection
        .flush()
        .map_err(|error| SessionError::Connect(error.to_string()))?;
    queue
        .roundtrip(&mut client)
        .map_err(|error| SessionError::Connect(error.to_string()))?;

    for interface in [
        "wl_compositor",
        "wl_shm",
        "ext_session_lock_manager_v1",
        "inox_ipc",
    ] {
        if !client.globals.contains_key(interface) {
            return Err(SessionError::MissingGlobal(interface));
        }
    }
    if client.outputs.is_empty() {
        warn!("this compositor advertises no outputs; the lock will cover nothing");
    }

    // Claim the trusted lock role before asking for the lock: the compositor only
    // accepts a lock from a connection that has claimed it,
    // and it refuses rather than guessing. The token is a capability, not an
    // identity claim.
    if let Some(ipc) = client.ipc.as_ref() {
        ipc.claim_lock_role(token.to_owned());
        debug!("claimed the trusted session lock role");
    }

    // The two facts the state needs before anything is drawn, in the order the
    // projection reads them: the compositor offers the protocol (reaching here says
    // so), and whether there is an authentication boundary this screen can use.
    // Reporting the second one is what makes the first frame honest: a screen with
    // nobody to check a password says exactly that, and its credential field is
    // disabled from the very first frame instead of after a failed attempt.
    let _ = app::update(&mut client.state, Message::ProtocolPresent);

    if boundary_available && client.keyboard.is_some() {
        debug!(
            socket = %client.auth_socket.display(),
            "the authentication boundary is listening and the keyboard is bound"
        );
    } else if boundary_available {
        // A prompt with no way to type into it is the same lie in the other
        // direction: the screen would invite a password and drop every key.
        warn!(
            "the compositor offered no keyboard, so this screen cannot take a \
             credential; it will say so instead of asking for one"
        );

        let _ = app::update(
            &mut client.state,
            Message::AuthUnavailable(ServiceFailure::NoInput),
        );
    } else {
        let account_missing = client.auth_account.is_none();
        let failure = if account_missing {
            warn!(
                uid = rustix::process::getuid().as_raw(),
                "the uid this session runs under does not resolve to an account name; \
                 there is nothing to authenticate as"
            );

            ServiceFailure::NotProvided
        } else {
            warn!(
                socket = %client.auth_socket.display(),
                "no authentication boundary is listening; the screen cannot check a \
                 password and will say so"
            );

            ServiceFailure::Disconnected
        };

        let _ = app::update(&mut client.state, Message::AuthUnavailable(failure));
    }

    // Ask for the lock, which is what asks for a surface on every output.
    let _ = app::update(&mut client.state, Message::LockRequested(LockReason::User));
    pump(&mut client, &qh);

    connection
        .flush()
        .map_err(|error| SessionError::Connect(error.to_string()))?;
    info!("{}", localizer.message("lock-taking"));

    let outcome = drive(
        &connection,
        &mut queue,
        &mut client,
        &localizer,
        signals,
        report_channel,
    )?;
    if let Some(lock) = client.lock.take() {
        // Never `unlock_and_destroy` here: a signal is not a credential, and
        // releasing on one would turn "log out" into "unlock". Destroying the
        // object only tells the compositor this client is finished with it.
        if !client.locked && !client.finished && !client.unlocked {
            lock.destroy();
        }
    }

    Ok(outcome)
}

/// Do what the lock screen has asked of the outside world.
///
/// [`app::State`] decides and queues; this is the only thing that carries it out.
fn pump(client: &mut Client, qh: &QueueHandle<Client>) {
    for command in client.state.take_outgoing() {
        match command {
            OutgoingCommand::Lock(reason) => client.take_lock(qh, reason),
            OutgoingCommand::CoverOutput(id) => client.cover(&id, qh),
            OutgoingCommand::Authenticate(attempt) => {
                let generation = attempt.generation();

                let Some(account) = client.auth_account.clone() else {
                    // The screen was told at startup that this session's uid has no
                    // account name to authenticate as, so the field is disabled and
                    // nothing is typed; getting here means the projection and the
                    // transport disagree, and the attempt is answered rather than
                    // dropped so the screen cannot sit on "checking" for ever.
                    warn!(
                        %generation,
                        "an attempt reached the transport, but this session has no \
                         account to authenticate as; it was answered as unusable"
                    );
                    let _ = app::update(
                        &mut client.state,
                        Message::AuthUnavailable(ServiceFailure::NotProvided),
                    );

                    continue;
                };

                info!(%generation, "handing an attempt to the authentication boundary");
                boundary::authenticate(
                    client.auth_socket.clone(),
                    attempt,
                    account,
                    &client.reports,
                );
            }
            OutgoingCommand::Unlock => client.unlock(),
            OutgoingCommand::AllowSuspend(generation) => warn!(
                %generation,
                "a suspend was allowed, but this mode has no suspend transport: \
                 nothing was signalled"
            ),
            OutgoingCommand::RefuseSuspend(reason) => warn!(
                ?reason,
                "a suspend was refused, but this mode has no suspend transport: \
                 nothing was signalled"
            ),
        }
    }
}

/// Serve the compositor until a signal arrives, the lock is withdrawn, or a
/// successful authentication releases it.
fn drive(
    connection: &Connection,
    queue: &mut wayland_client::EventQueue<Client>,
    client: &mut Client,
    localizer: &Localizer,
    signals: Signals,
    reports: Channel<boundary::Report>,
) -> Result<Outcome, SessionError> {
    let mut event_loop: EventLoop<Client> =
        EventLoop::try_new().map_err(|error| SessionError::EventLoop(error.to_string()))?;
    let handle = event_loop.handle();

    // Taken before the source below takes ownership of the queue, and used for the
    // whole loop: it is a handle into the same connection, not a second queue.
    let qh = queue.handle();

    // The other half of the boundary channel: an attempt's answer arrives as a
    // message on this loop, which is what keeps the frames coming while the helper
    // thinks.
    handle
        .insert_source(reports, |event, _, client| {
            if let ChannelEvent::Msg(report) = event {
                client.auth_report(report);
            }
        })
        .map_err(|error| SessionError::EventLoop(error.to_string()))?;

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
                            client.stopped = true;
                        }
                    }
                }
                if let Err(error) = queue.dispatch_pending(client) {
                    warn!(%error, "failed to dispatch compositor events");
                    client.stopped = true;
                }

                Ok(PostAction::Continue)
            },
        )
        .map_err(|error| SessionError::EventLoop(error.to_string()))?;

    handle
        .insert_source(signals, |event, _, client| {
            // The lock is deliberately not released here: this mode has no way to
            // authenticate anyone, so "the process was told to stop" must not mean
            // "the desktop is available again".
            info!(signal = ?event.signal(), "stopping without releasing the session lock");
            client.stopped = true;
        })
        .map_err(|error| SessionError::EventLoop(error.to_string()))?;

    handle
        .insert_source(
            Timer::from_duration(CONFIRMATION_TIMEOUT),
            |_, _, client| {
                // The lock has not been confirmed. Whatever the compositor
                // accepted it keeps — this client stops waiting rather than holding
                // a request nobody answered, and reports that it stopped before the
                // lock was confirmed.
                if !client.locked {
                    warn!("the compositor did not confirm the session lock in time");
                    client.stopped = true;
                }
                TimeoutAction::Drop
            },
        )
        .map_err(|error| SessionError::EventLoop(error.to_string()))?;

    // The Fluent message is the one an operator reads, so it is emitted where the
    // localizer is, not from the protocol handler: the handler's own `tracing` line
    // is the raw fact, this is the sentence.
    let mut announced = false;
    while !client.stopped && !client.finished && !client.unlocked {
        connection
            .flush()
            .map_err(|error| SessionError::Connect(error.to_string()))?;
        event_loop
            .dispatch(None, client)
            .map_err(|error| SessionError::EventLoop(error.to_string()))?;
        // Protocol events have changed the state — an output appeared, a surface
        // was configured, the compositor answered, the boundary replied — and what
        // the state decided about them reaches the wire here.
        pump(client, &qh);
        client.repaint(&qh);
        if client.locked && !announced {
            announced = true;
            info!("{}", localizer.message("lock-taken"));
        }
    }

    if client.unlocked {
        // The unlock request was queued by `pump` after the last flush of the loop,
        // so it is still in this connection's outgoing buffer. Flushing here is what
        // puts it on the wire: without it the process would exit with the request
        // unsent and the session locked, which is the worst of both answers.
        connection
            .flush()
            .map_err(|error| SessionError::Connect(error.to_string()))?;

        // The compositor destroys the lock object and every lock surface in
        // response, so there is nothing left for this process to do.
        return Ok(Outcome::Unlocked);
    }

    if client.finished {
        // `finished` after `locked` is the compositor releasing a lock it had
        // confirmed — an administrative decision on its side, not a failure here.
        // Before `locked` it is the protocol's refusal, which is what a compositor
        // sends when the session is already locked or this connection never
        // claimed the trusted role.
        return Ok(if client.locked {
            Outcome::Released
        } else {
            Outcome::Refused
        });
    }

    Ok(if client.locked {
        Outcome::StoppedLocked
    } else {
        Outcome::StoppedBeforeConfirmation
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read as _, Seek as _};

    use super::*;

    #[test]
    fn a_frame_pixel_is_little_endian_argb() {
        // `wl_shm`'s `Argb8888` names the *words*: the byte order on the wire is
        // B, G, R, A, which is what the compositor's own pixel assertions use.
        assert_eq!(argb_bytes([0x11, 0x22, 0x33]), [0x33, 0x22, 0x11, 0xff]);
        assert_eq!(
            argb_bytes([0x11, 0x22, 0x33])[3],
            0xff,
            "the lock frame is opaque: a translucent cover would show the session \
             behind it"
        );
    }

    #[test]
    fn a_frame_file_holds_exactly_what_was_written() {
        // The compositor maps this fd for the whole pool size, so the frame the
        // client writes has to be exactly that long: a short file would fault the
        // compositor's mapping rather than show a wrong colour.
        let mut file =
            File::from(frame_memory("inox-lock-test").expect("memfd_create is available"));
        let frame = [0x33u8, 0x22, 0x11, 0xff].repeat(4);
        file.write_all(&frame)
            .expect("the frame memory is writable");
        file.seek(io::SeekFrom::Start(0)).expect("the frame seeks");
        let mut read = vec![0u8; frame.len()];
        file.read_exact(&mut read).expect("the frame reads back");

        assert_eq!(read, frame, "the frame is the bytes that were written");
        assert!(
            read.as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0xff),
            "every pixel of a lock frame is opaque"
        );
    }

    #[test]
    fn an_output_gets_the_state_machines_own_name_for_it() {
        assert_eq!(output_id(7).as_str(), "output-7");
    }
}
