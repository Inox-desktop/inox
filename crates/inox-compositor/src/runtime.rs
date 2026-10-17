//! The compositor event loop.
//!
//! Calloop drives Wayland sockets, the display and shutdown signals. Headless
//! adds a paced timer; physical sessions add libseat, udev, libinput and DRM
//! sources plus one-shot repaint/completion deadlines. Device event sources and
//! their lifetime guards live in `physical`.
//!
//! The optional `test-input` source exists only in feature-enabled builds and
//! remains inert without an explicit socket path.

use std::{ffi::OsString, io, sync::Arc, time::Duration};

mod input_method;
mod physical;

use calloop::{
    EventLoop, Interest, Mode, PostAction,
    generic::{Generic, NoIoDrop},
    signals::{Signal, Signals},
    timer::{TimeoutAction, Timer},
};
use smithay::{
    reexports::wayland_server::{BindError, Display, DisplayHandle, backend::InitError},
    wayland::socket::ListeningSocketSource,
};
use tracing::{debug, info, warn};

#[cfg(feature = "test-input")]
use smithay::utils::Point;
#[cfg(feature = "test-input")]
use std::{
    os::unix::net::{UnixListener, UnixStream},
    path::Path,
};

#[cfg(feature = "test-input")]
use crate::test_input::{
    Command, CommandError, ConnectionState, TestInputConnection, accept_connection, bind_listener,
    to_ppm,
};
use crate::{
    backend::frame::FrameSchedule,
    config::{BackendKind, CompositorConfig},
    state::{ClientState, InoxState, StateError},
};

/// Errors that abort the compositor event loop.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// Headless rendering failed.
    #[error(transparent)]
    Render(#[from] crate::render::RenderError),
    /// The calloop event loop could not be created.
    #[error("failed to create the event loop")]
    EventLoop(#[from] calloop::Error),

    /// The Wayland display could not be created.
    #[error("failed to create the wayland display")]
    Display(#[from] InitError),

    /// The Wayland socket could not be bound.
    #[error("failed to bind the wayland socket")]
    Socket(#[from] BindError),

    /// The compositor state could not be created.
    #[error("failed to create the compositor state")]
    State(#[from] StateError),

    /// A calloop event source could not be registered.
    ///
    /// The underlying [`calloop::InsertError`] is kept as a rendered reason because it
    /// is not `Send + Sync` for every source type.
    #[error("failed to register the {name} event source: {reason}")]
    EventSource {
        /// Which event source failed to register.
        name: &'static str,
        /// The rendered registration error.
        reason: String,
    },

    /// The `--test-input` control socket could not be created.
    ///
    /// Only reachable in builds with the `test-input` feature, where the option is
    /// accepted at all.
    #[error("failed to bind the test input socket {path}: {reason}")]
    TestInputSocket {
        /// The socket path that was asked for.
        path: String,
        /// The rendered bind failure.
        reason: String,
    },

    /// The compositor was explicitly asked to run the DRM backend but the
    /// session, device or present path is not available on this system.
    ///
    /// Both `auto` and `drm` require hardware. `headless` is explicit.
    #[error("the DRM backend is not runnable here: {reason}")]
    BackendUnavailable {
        /// The failing session, device, allocation or presentation boundary.
        reason: String,
    },
}

/// State shared with every calloop event source.
struct CalloopData {
    input_method: Option<input_method::InputMethodProcess>,
    state: InoxState,
    display_handle: DisplayHandle,
    /// The physical backend, when one is running.
    ///
    /// Its per-output frame schedules and timers live inside it, so one display
    /// cannot hold up another (ADR 0019).
    physical: Option<physical::PhysicalState>,
    /// The headless frame schedule; unused while a physical backend runs.
    frames: FrameSchedule,
    fatal: Option<RunError>,
    /// The live `--test-input` connection, if one is open.
    ///
    /// The registration token is kept next to the connection state so that the listener
    /// can replace the current connection with the next one: calloop reuses registration
    /// slots, so the token must be removed before its source goes away.
    #[cfg(feature = "test-input")]
    test_input: Option<(calloop::RegistrationToken, TestInputConnection)>,
}

/// Run the compositor until it is signalled to exit or its frame budget is reached.
///
/// Returns `Ok(())` after a clean shutdown, which is the M1 acceptance criterion:
/// termination signals must lead to exit status 0 rather than killing the process.
///
/// # Errors
///
/// Returns a [`RunError`] when the event loop, the Wayland display, the listening
/// socket or an event source cannot be set up, or when the compositor state cannot
/// be built.
pub fn run(config: CompositorConfig) -> Result<(), RunError> {
    // Headless is an explicit choice. Auto selects hardware, never an invisible
    // session with a success/ready report after device acquisition has failed.
    let prepared = if config.backend == BackendKind::Headless {
        None
    } else {
        Some(physical::prepare()?)
    };

    let mut event_loop: EventLoop<'static, CalloopData> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();
    let loop_signal = event_loop.get_signal();

    let display: Display<InoxState> = Display::new()?;
    let display_handle = display.handle();

    let state = match &prepared {
        Some(prepared) => InoxState::new_physical(
            &display_handle,
            &config,
            loop_signal,
            prepared.outputs.clone(),
            prepared.input.clone(),
            prepared.theme.clone(),
            prepared.layout,
        )?,
        None => InoxState::new(&display_handle, &config, loop_signal)?,
    };

    let mut data = CalloopData {
        input_method: None,
        state,
        display_handle,
        physical: None,
        frames: FrameSchedule::default(),
        fatal: None,
        #[cfg(feature = "test-input")]
        test_input: None,
    };

    // Registered before the listening socket is bound, so that a compositor which has
    // already bound its socket is guaranteed to handle termination signals.
    register_signal_source(&loop_handle)?;

    let listening_socket = match config.socket_name.as_deref() {
        Some(name) => ListeningSocketSource::with_name(name)?,
        None => ListeningSocketSource::new_auto()?,
    };
    let socket_name = listening_socket.socket_name().to_os_string();

    register_socket_source(&loop_handle, listening_socket, socket_name.clone())?;
    register_display_source(&loop_handle, display)?;
    if let Some(prepared) = prepared {
        physical::register(&loop_handle, &mut data, prepared)?;
    } else {
        register_frame_source(&loop_handle, frame_interval(&config))?;
    }

    // In a default build the option cannot be set, so this block is compiled out and the
    // loop keeps exactly the four sources it always had.
    #[cfg(feature = "test-input")]
    {
        if let Some(path) = config.test_input_path.as_deref() {
            register_test_input_source(&loop_handle, path)?;
        }
    }

    if data.physical.is_none() {
        info!(socket = ?socket_name, "inox-compositor ready (explicit headless backend)");
    } else {
        info!(socket = ?socket_name, "Wayland socket bound; waiting for first physical presentation");
    }

    input_method::register(&loop_handle, &mut data, &config.input_method, &socket_name)?;

    let scheduling_handle = loop_handle.clone();
    let result = event_loop.run(None, &mut data, move |data| {
        let repaint = data.state.take_repaint();
        if data.physical.is_some() {
            physical::schedule(&scheduling_handle, data, repaint);
        } else if repaint || data.state.frame_budget() {
            data.frames.damage();
        }
    });
    physical::shutdown(&loop_handle, &mut data);
    drop(data.input_method.take());
    result?;
    if let Some(error) = data.fatal.take() {
        return Err(error);
    }

    info!(
        frames = data.state.frames_rendered(),
        "inox-compositor stopped"
    );

    Ok(())
}

/// Frame interval for the timer driving rendering, derived from the output refresh rate.
fn frame_interval(config: &CompositorConfig) -> Duration {
    let refresh_hz = config.refresh_hz().max(1) as u64;

    Duration::from_nanos(1_000_000_000 / refresh_hz)
}

/// Accept client connections and register them on the display.
fn register_socket_source(
    loop_handle: &calloop::LoopHandle<'static, CalloopData>,
    listening_socket: ListeningSocketSource,
    socket_name: OsString,
) -> Result<(), RunError> {
    loop_handle
        .insert_source(
            listening_socket,
            move |stream, _, data: &mut CalloopData| match data
                .display_handle
                .insert_client(stream, Arc::new(ClientState::default()))
            {
                Ok(client) => {
                    debug!(client = ?client.id(), socket = ?socket_name, "client connected");
                }
                Err(error) => warn!(%error, "rejected a client connection"),
            },
        )
        .map(drop)
        .map_err(|source| RunError::EventSource {
            name: "wayland socket",
            reason: source.to_string(),
        })
}

/// Drive the Wayland display whenever its backend reports readability.
fn register_display_source(
    loop_handle: &calloop::LoopHandle<'static, CalloopData>,
    display: Display<InoxState>,
) -> Result<(), RunError> {
    loop_handle
        .insert_source(
            Generic::new(display, Interest::READ, Mode::Level),
            |_, source, data: &mut CalloopData| {
                if let Err(error) = service_clients(source, &mut data.state) {
                    warn!(%error, "failed to service wayland clients");
                }

                Ok(PostAction::Continue)
            },
        )
        .map(drop)
        .map_err(|source| RunError::EventSource {
            name: "wayland display",
            reason: source.to_string(),
        })
}

/// Dispatch pending client requests and flush outgoing buffers.
#[allow(unsafe_code)]
fn service_clients(
    source: &mut NoIoDrop<Display<InoxState>>,
    state: &mut InoxState,
) -> io::Result<usize> {
    // SAFETY: the `Generic` source owns the `Display` for the whole lifetime of the
    // event loop. This callback neither moves the `Display` out of the source nor
    // drops the source, which is exactly what `Generic::get_mut` requires, and the
    // mutable reference never escapes this function.
    let display = unsafe { source.get_mut() };

    let dispatched = display.dispatch_clients(state)?;
    display.flush_clients()?;

    Ok(dispatched)
}

/// Turn `SIGINT` and `SIGTERM` into a clean shutdown request.
fn register_signal_source(
    loop_handle: &calloop::LoopHandle<'static, CalloopData>,
) -> Result<(), RunError> {
    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM])?;

    loop_handle
        .insert_source(signals, |event, _, data: &mut CalloopData| {
            info!(signal = ?event.signal(), "shutdown requested");
            data.state.request_exit();
        })
        .map(drop)
        .map_err(|source| RunError::EventSource {
            name: "signal",
            reason: source.to_string(),
        })
}

/// Render frames at the output refresh rate until shutdown is requested.
fn register_frame_source(
    loop_handle: &calloop::LoopHandle<'static, CalloopData>,
    interval: Duration,
) -> Result<(), RunError> {
    loop_handle
        .insert_source(
            Timer::from_duration(interval),
            move |_, _, data: &mut CalloopData| {
                if data.state.take_repaint() || data.state.frame_budget() {
                    data.frames.damage();
                }
                let Some(token) = data.frames.begin() else {
                    return TimeoutAction::ToDuration(interval);
                };
                if let Err(error) = data.state.render_frame() {
                    warn!(%error, "failed to render a frame, shutting down");
                    data.fatal = Some(RunError::Render(error));
                    data.state.request_exit();

                    return TimeoutAction::Drop;
                }

                data.frames.complete(token);
                // The frame callbacks `render_frame` just queued — and any selection
                // offer a focus change queued before it — only leave the compositor once
                // the display is flushed, and the display source fires on incoming data
                // alone. Flushing here is what keeps a client that waits for its frame
                // callback from waiting forever.
                if let Err(error) = data.display_handle.flush_clients() {
                    warn!(%error, "failed to flush clients after a frame");
                }

                debug!(frames = data.state.frames_rendered(), "rendered frame");

                if data.state.frame_budget_reached() {
                    info!(
                        frames = data.state.frames_rendered(),
                        "frame budget reached, shutting down"
                    );
                    data.state.request_exit();

                    return TimeoutAction::Drop;
                }

                TimeoutAction::ToDuration(interval)
            },
        )
        .map(drop)
        .map_err(|source| RunError::EventSource {
            name: "frame timer",
            reason: source.to_string(),
        })
}

/// Serve test commands on the control socket `--test-input` named.
///
/// Two sources make this up, both compiled only with the `test-input` feature (see
/// [`crate::test_input`] for what the feature is and why it is off by default): the
/// listener registered here, and one source per accepted connection, inserted by the
/// listener and removed again when the peer goes away. They are separate sources because
/// calloop's safe sources own exactly one file descriptor each, and the connection is only
/// known once it has been accepted.
#[cfg(feature = "test-input")]
fn register_test_input_source(
    loop_handle: &calloop::LoopHandle<'static, CalloopData>,
    path: &Path,
) -> Result<(), RunError> {
    let listener = bind_listener(path).map_err(|error| RunError::TestInputSocket {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;

    let acceptor = loop_handle.clone();
    loop_handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            move |_, listener: &mut NoIoDrop<UnixListener>, data: &mut CalloopData| {
                accept_test_input(listener, &acceptor, data);

                Ok(PostAction::Continue)
            },
        )
        .map(drop)
        .map_err(|source| RunError::EventSource {
            name: "test input",
            reason: source.to_string(),
        })?;

    debug!(path = %path.display(), "test-input control socket ready");

    Ok(())
}

/// Take over the connection the listener has been asked about.
///
/// Connections are served one at a time: a client that reconnects replaces the previous
/// one, so a test never has to guess which socket its acknowledgements travel on. Closing
/// the connection from the client side therefore re-arms the listener on the next accept.
#[cfg(feature = "test-input")]
fn accept_test_input(
    listener: &UnixListener,
    loop_handle: &calloop::LoopHandle<'static, CalloopData>,
    data: &mut CalloopData,
) {
    let Some(accepted) = accept_connection(listener) else {
        return;
    };

    let stream = match accepted {
        Ok(stream) => stream,
        Err(error) => {
            debug!(%error, "test-input connection is unusable, dropping it");

            return;
        }
    };

    if let Some((token, _)) = data.test_input.take() {
        loop_handle.remove(token);
    }

    match loop_handle.insert_source(
        Generic::new(stream, Interest::READ, Mode::Level),
        |_, stream: &mut NoIoDrop<UnixStream>, data: &mut CalloopData| {
            serve_test_input(stream, data)
        },
    ) {
        Ok(token) => data.test_input = Some((token, TestInputConnection::default())),
        Err(error) => {
            // The listener stays armed, so the client can simply reconnect.
            debug!(%error, "test-input connection could not be served");
        }
    }
}

/// Run one service pass over the live connection.
#[cfg(feature = "test-input")]
fn serve_test_input(stream: &UnixStream, data: &mut CalloopData) -> io::Result<PostAction> {
    // The connection is taken out of the shared state for the duration of the call so that
    // applying a command can borrow the compositor state, which lives in the same struct.
    let Some((token, mut connection)) = data.test_input.take() else {
        // Nothing owns this source any more; stop serving it.
        return Ok(PostAction::Remove);
    };

    let state = connection.service(stream, |command| apply_test_command(command, data));

    match state {
        ConnectionState::Open => {
            data.test_input = Some((token, connection));

            Ok(PostAction::Continue)
        }
        ConnectionState::Closed => {
            debug!("test-input connection closed");

            Ok(PostAction::Remove)
        }
    }
}

/// Apply one command from the control socket.
///
/// The reply the client receives is built from what this returns, so returning `Ok` is
/// the statement "the command took effect and the clients have been told".
#[cfg(feature = "test-input")]
fn apply_test_command(command: Command, data: &mut CalloopData) -> Result<(), CommandError> {
    match command {
        Command::Key { keycode, state } => {
            data.state.inject_key(keycode, state);
        }
        Command::Motion { x, y } => data.state.inject_pointer_motion(Point::from((x, y))),
        Command::Button { code, state } => data.state.inject_pointer_button(code, state),
        Command::DumpFrame { path } => dump_frame(&mut data.state, &path)?,
    }

    // An acknowledgement must mean "the clients have been sent the events", not merely
    // "the state changed": a test that acted on the weaker promise would still have to
    // sleep. Flushing here is what lets it avoid that.
    data.display_handle
        .flush_clients()
        .map_err(CommandError::Flush)?;

    Ok(())
}

/// Render one frame and write it to `path` as a binary PPM image.
#[cfg(feature = "test-input")]
fn dump_frame(state: &mut InoxState, path: &Path) -> Result<(), CommandError> {
    // The readback returns whatever the render target currently holds, so the frame the
    // test wants to look at has to be drawn first.
    state.render_frame()?;

    let (width, height) = state.output_size();
    let rgba = state.read_frame_rgba()?;
    let ppm = to_ppm(&rgba, width.max(0) as u32, height.max(0) as u32)?;

    std::fs::write(path, ppm).map_err(|source| CommandError::Dump {
        path: path.display().to_string(),
        source,
    })
}
