//! The protocol leg: taking the compositor's session lock, as its client.
//!
//! [ADR 0015] puts the lock state in the compositor and makes this crate the
//! trusted client of `ext-session-lock-v1`. This module is that client — the half
//! of the component that talks to the wire, kept separate from the state machine
//! in [`crate::state`] because the two answer different questions: the state
//! machine owns what a lock *means* (and is exercised without a compositor), this
//! module owns the requests and events the protocol defines.
//!
//! # What it does, and what it deliberately does not
//!
//! It connects to the session's Wayland display, claims the trusted lock role over
//! the private `inox-ipc` protocol with the token `inox-session` delivered, asks
//! for the lock, covers every output with an opaque frame of this component's
//! background colour, and stays there.
//!
//! It does **not** authenticate anyone, and it has no unlock path at all. That is
//! not an omission: ADR 0015 makes PAM (or whichever mechanism), the helper's
//! privileges and the handling of the credential a human security review item, and
//! a test stub does not count as authentication. `main` therefore only runs this
//! module under an explicit flag, and the mode it starts cannot release the lock —
//! a session that enters it is released by logging out, and the compositor says so
//! when the process is signalled.
//!
//! The frame it commits is therefore a cover, not an interface: no prompt, no
//! mask, no keyboard input of any kind. Authentication is what would put a lock
//! screen on it, and authentication is what has not been reviewed.
//!
//! [ADR 0015]: https://github.com/Inox-desktop/inox/blob/main/docs/adr/0015-session-lock-and-failure-boundaries.md

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Write as _},
    os::fd::{AsFd, OwnedFd},
    time::Duration,
};

use calloop::{
    EventLoop, Interest, Mode, PostAction,
    generic::Generic,
    signals::{Signal, Signals},
    timer::{TimeoutAction, Timer},
};
use inox_common::i18n::Localizer;
use inox_protocols::{
    ipc::inox_ipc::client::inox_ipc as ipc,
    standard::{
        core::client::{
            wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_output::WlOutput,
            wl_registry::WlRegistry, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
        },
        ext::session_lock::client::{
            ext_session_lock_manager_v1 as manager, ext_session_lock_surface_v1 as lock_surface,
            ext_session_lock_v1 as lock,
        },
    },
};
use tracing::{debug, error, info, warn};
use wayland_client::{Connection, Dispatch, QueueHandle};

/// How long the registry may take to answer before the compositor is declared
/// unusable, in keeping with the component's other bounded waits.
const REGISTRY_TIMEOUT: Duration = Duration::from_secs(5);

/// What taking the lock ended in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The compositor refused: the session was already locked, this connection had
    /// not claimed the lock role, or no more locks are being accepted (ADR 0022
    /// decision 2). Nothing was covered, and nothing is left holding anything.
    Refused,
    /// The compositor withdrew the lock with the protocol's `finished` event.
    Released,
    /// The process was asked to stop after the lock was confirmed. The lock is
    /// deliberately **still held**: this mode has no way to release it, and
    /// releasing it on a signal would turn "log out" into "unlock".
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
    /// The registry did not answer in time.
    #[error("the compositor's registry did not answer within {REGISTRY_TIMEOUT:?}")]
    RegistryTimeout,
    /// The event loop failed.
    #[error("the session lock event loop failed: {0}")]
    EventLoop(String),
    /// A shared-memory frame could not be built.
    #[error("failed to prepare the lock frame: {0}")]
    Frame(#[from] io::Error),
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
    /// The buffers committed, kept alive until the compositor has released them.
    buffers: Vec<WlBuffer>,
    /// The colour every lock frame is filled with.
    colour: [u8; 3],
    locked: bool,
    finished: bool,
    stopped: bool,
    /// Frames committed, which is the observable proof that the client covered
    /// the outputs rather than merely asking to.
    committed: u64,
}

struct OutputEntry {
    /// The registry name of the `wl_output` global, which is how an output is
    /// recognised when it is removed.
    global: u32,
    /// A name for the log line; the compositor's own name for the connector is not
    /// part of this protocol.
    name: String,
    output: WlOutput,
    surface: Option<WlSurface>,
    role: Option<lock_surface::ExtSessionLockSurfaceV1>,
}

/// The bytes of one pixel in `wl_shm`'s `Argb8888`, which is little-endian
/// `B, G, R, A` — the same order the compositor's own read-back tests assert on.
fn argb_bytes(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[2], rgb[1], rgb[0], 0xff]
}

/// An anonymous shared-memory file for one frame.
fn frame_memory(name: &str) -> io::Result<OwnedFd> {
    rustix::fs::memfd_create(name, rustix::fs::MemfdFlags::CLOEXEC).map_err(io::Error::from)
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
                    "wl_output" => {
                        let output: WlOutput = registry.bind(name, version.min(4), qh, ());
                        state.outputs.push(OutputEntry {
                            global: name,
                            name: format!("output-{name}"),
                            output,
                            surface: None,
                            role: None,
                        });
                        // A display that arrives while the session is locked has to
                        // be covered too, so a surface is created for it as soon as
                        // it is advertised.
                        state.cover_new_surfaces(qh);
                    }
                    _ => {}
                }
            }
            registry_event::Event::GlobalRemove { name } => {
                state.outputs.retain(|entry| {
                    let keep = entry.global != name;
                    if !keep {
                        // A display that is unplugged takes its lock surface with it;
                        // the protocol expects the client to destroy it, and dropping
                        // the proxy is enough for the compositor to see it go.
                        debug!(output = %entry.name, "an output went away; its lock surface goes with it");
                    }
                    keep
                });
            }
            _ => {}
        }
    }
}

impl Client {
    /// Create a lock surface for every output that does not have one yet.
    ///
    /// The protocol asks a client to do this as soon as it has a lock object, and
    /// again for every output that appears later: the compositor may hold the
    /// `locked` event until it has a covered frame, and this client's cover is its
    /// own surfaces. Before the lock object exists there is nothing to create, so
    /// the call is simply a no-op.
    fn cover_new_surfaces(&mut self, qh: &QueueHandle<Self>) {
        let (Some(lock), Some(compositor)) = (self.lock.as_ref(), self.compositor.as_ref()) else {
            return;
        };
        for entry in self.outputs.iter_mut() {
            if entry.role.is_some() {
                continue;
            }
            let surface = entry
                .surface
                .get_or_insert_with(|| compositor.create_surface(qh, ()))
                .clone();
            entry.role = Some(lock.get_lock_surface(&surface, &entry.output, qh, ()));
            info!(output = %entry.name, "created a lock surface");
        }
    }
}

impl Dispatch<lock::ExtSessionLockV1, ()> for Client {
    fn event(
        state: &mut Self,
        _: &lock::ExtSessionLockV1,
        event: lock::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            lock::Event::Locked => {
                state.locked = true;
                info!(
                    outputs = state.outputs.len(),
                    committed = state.committed,
                    "the compositor confirmed the session lock"
                );
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
        let _ = qh;
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
            if let Err(error) = state.commit_cover(qh, role, width, height) {
                error!(%error, "failed to cover an output with a lock frame");
            }
        }
    }
}

impl Client {
    /// Commit an opaque frame of exactly the configured size to `role`'s surface.
    fn commit_cover(
        &mut self,
        qh: &QueueHandle<Self>,
        role: &lock_surface::ExtSessionLockSurfaceV1,
        width: u32,
        height: u32,
    ) -> Result<(), SessionError> {
        let Some(entry) = self
            .outputs
            .iter()
            .find(|entry| entry.role.as_ref() == Some(role))
        else {
            return Ok(());
        };
        let Some(surface) = entry.surface.clone() else {
            return Ok(());
        };
        let Some(shm) = self.shm.clone() else {
            return Err(SessionError::MissingGlobal("wl_shm"));
        };
        let stride = width.saturating_mul(4);
        let size = stride.saturating_mul(height);
        let (Ok(stride), Ok(size)) = (i32::try_from(stride), i32::try_from(size)) else {
            return Ok(());
        };

        let mut file = File::from(frame_memory("inox-lock-frame")?);
        let row = argb_bytes(self.colour).repeat(width as usize);
        for _ in 0..height {
            file.write_all(&row)?;
        }

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
        pool.destroy();

        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, width as i32, height as i32);
        surface.commit();
        self.buffers.push(buffer);
        self.committed = self.committed.saturating_add(1);
        debug!(output = %entry.name, width, height, "covered an output");

        Ok(())
    }
}

// The interfaces this client only consumes, or does not consume at all.
wayland_client::delegate_noop!(Client: ignore WlCompositor);
wayland_client::delegate_noop!(Client: ignore WlSurface);
wayland_client::delegate_noop!(Client: ignore WlBuffer);
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
pub fn take(token: Option<&str>, localizer: &Localizer) -> Result<Outcome, SessionError> {
    let Some(token) = token.filter(|token| !token.is_empty()) else {
        return Err(SessionError::NoToken);
    };

    let connection =
        Connection::connect_to_env().map_err(|error| SessionError::Connect(error.to_string()))?;
    let mut queue = connection.new_event_queue::<Client>();
    let qh = queue.handle();
    // The registry object is kept alive by the connection; every bind below goes
    // through it from the dispatch handler.
    let _registry = connection.display().get_registry(&qh, ());

    let mut client = Client {
        globals: HashMap::new(),
        manager: None,
        compositor: None,
        shm: None,
        ipc: None,
        lock: None,
        outputs: Vec::new(),
        buffers: Vec::new(),
        colour: LOCK_FRAME_RGB,
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
    // accepts a lock from a connection that has claimed it (ADR 0022 decision 3),
    // and it refuses rather than guessing. The token is a capability, not an
    // identity claim.
    if let Some(ipc) = client.ipc.as_ref() {
        ipc.claim_lock_role(token.to_owned());
        debug!("claimed the trusted session lock role");
    }

    let Some(manager) = client.manager.clone() else {
        return Err(SessionError::MissingGlobal("ext_session_lock_manager_v1"));
    };
    client.lock = Some(manager.lock(&qh, ()));
    // One lock surface per output, created before the compositor can confirm
    // anything: the protocol asks for them immediately, and a compositor that
    // waits for a covered frame waits for these.
    client.cover_new_surfaces(&qh);
    connection
        .flush()
        .map_err(|error| SessionError::Connect(error.to_string()))?;
    info!("{}", localizer.message("lock-taking"));

    let outcome = drive(&connection, &mut queue, &mut client, localizer)?;
    if let Some(lock) = client.lock.take() {
        // Never `unlock_and_destroy`: releasing the session is not this component's
        // to do while it has no authentication boundary, and a signal is not a
        // credential. Destroying the object only tells the compositor this client
        // is finished with it.
        if !client.locked && !client.finished {
            lock.destroy();
        }
    }

    Ok(outcome)
}

/// The colour of a lock frame: an opaque, unlit background, because this build has
/// no reviewed interface to draw on it.
const LOCK_FRAME_RGB: [u8; 3] = [0x11, 0x22, 0x33];

/// Serve the compositor until a signal arrives or the lock is withdrawn.
fn drive(
    connection: &Connection,
    queue: &mut wayland_client::EventQueue<Client>,
    client: &mut Client,
    localizer: &Localizer,
) -> Result<Outcome, SessionError> {
    let mut event_loop: EventLoop<Client> =
        EventLoop::try_new().map_err(|error| SessionError::EventLoop(error.to_string()))?;
    let handle = event_loop.handle();

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

    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP])
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
        .insert_source(Timer::from_duration(REGISTRY_TIMEOUT), |_, _, client| {
            if client.lock.is_none() {
                warn!("{}", "no lock object");
                client.stopped = true;
            }
            TimeoutAction::Drop
        })
        .map_err(|error| SessionError::EventLoop(error.to_string()))?;

    // The Fluent message is the one an operator reads, so it is emitted where the
    // localizer is, not from the protocol handler: the handler's own `tracing` line
    // is the raw fact, this is the sentence.
    let mut announced = false;
    while !client.stopped && !client.finished {
        connection
            .flush()
            .map_err(|error| SessionError::Connect(error.to_string()))?;
        event_loop
            .dispatch(None, client)
            .map_err(|error| SessionError::EventLoop(error.to_string()))?;
        if client.locked && !announced {
            announced = true;
            info!("{}", localizer.message("lock-taken"));
        }
    }

    if client.finished {
        // `finished` after `locked` is the compositor releasing a lock it had
        // confirmed — an administrative decision on its side, not a failure here.
        // Before `locked` it is the protocol's refusal, which is what a compositor
        // sends when the session is already locked or this connection never
        // claimed the trusted role (ADR 0022 decisions 2 and 3).
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
            argb_bytes(LOCK_FRAME_RGB),
            [0x33, 0x22, 0x11, 0xff],
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
        let row = argb_bytes(LOCK_FRAME_RGB).repeat(4);
        file.write_all(&row).expect("the frame memory is writable");
        file.seek(io::SeekFrom::Start(0)).expect("the frame seeks");
        let mut read = vec![0u8; row.len()];
        file.read_exact(&mut read).expect("the frame reads back");

        assert_eq!(
            read, row,
            "the covered frame is the colour that was written"
        );
        assert!(
            read.chunks(4).all(|pixel| pixel[3] == 0xff),
            "every pixel of a lock frame is opaque"
        );
    }
}
