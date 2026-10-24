//! The session-bus and `SIGHUP` bridge: reload requests, prepared off the UI's
//! thread and applied by the panel's own loop.
//!
//! `inoxctl reload` reaches the session through `org.inox.Panel`, and
//! `org.inox.Configuration1.Reload` hands back a snapshot only once the panel has
//! *applied* the candidate — not once the files were read (ADR 0009). That is why
//! the endpoint below never answers by itself: it reads the candidate, puts a
//! [`Request`] on a channel and waits for whatever owns the panel's state to
//! [`Request::finish`] it.
//!
//! What owns the state is [`crate::layer`]'s event loop, so this module no longer
//! needs an `iced` runtime to reach it. `SIGHUP` is watched by **that** loop's
//! signal source, for a reason worth writing down: a `signalfd` only receives a
//! signal that is blocked in every thread of the process, so the mask has to be
//! installed by the first thread to run and the source has to live in a thread
//! that can be woken by it. [`Host::prepare`] is what that source calls; the
//! reading and parsing happen on a thread of its own, because they are file I/O
//! and must not stall the panel's loop.
//!
//! A request that arrives when the loop is blocked in `calloop` would wait for
//! the next pointer or clock event, so the host is given a [`Ping`] and fires it
//! whenever it has prepared something. That is the whole cross-thread contract:
//! the channel carries the data, the ping carries the wake-up.

use iced::futures::channel::{mpsc, oneshot};
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use std::rc::Rc;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tracing::warn;

use calloop::ping::Ping;

use crate::app::State;

const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct Completion {
    sender: Mutex<Option<oneshot::Sender<Result<String, String>>>>,
    busy: Arc<AtomicBool>,
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

/// One prepared configuration reload, waiting for the panel to apply it.
#[derive(Debug, Clone)]
pub struct Request {
    /// The candidate configuration, already parsed and validated.
    pub config: InoxConfig,
    deadline: Instant,
    completion: Arc<Completion>,
}
impl Request {
    /// Whether this request is still worth applying.
    ///
    /// A reload that took longer than the deadline to reach the panel describes a
    /// snapshot the caller has already given up on; applying it would make the
    /// panel's state disagree with the answer the caller received.
    pub fn is_current(&self) -> bool {
        Instant::now() < self.deadline
    }
    /// Answers the caller, and reports a failure in the log while keeping the
    /// panel's previous configuration.
    pub fn finish(&self, result: Result<(), String>) {
        let result = result.and_then(|()| self.config.snapshot().map_err(|e| e.to_string()));
        if let Err(error) = &result {
            warn!(%error, "configuration reload failed; keeping previous state");
        }
        if let Some(sender) = self
            .completion
            .sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            // SIGHUP has no caller awaiting a reply; a closed receiver is expected.
            let _ = sender.send(result);
        }
    }
}

/// Applies a prepared reload to the running panel.
///
/// Everything except the surface itself is applied live. The placement the
/// surface carries — anchors, exclusive zone, margin, height, the menu bar's and
/// the dock's own sections — is data the layer surface re-asks for on the next
/// commit, which is a change of host rather than a change of process, so a reload
/// that changes it is applied and the leg re-commits the surface state.
///
/// **`[panel] surface` is not applied.** It names a different role with a
/// different size and placement: a *different process*. The candidate's value is
/// therefore dropped — with one `warn!` naming both the asked-for and the running
/// surface — and the process keeps the surface it was started as. Failing the
/// whole reload instead would make a session that passes `--surface` (a menu bar
/// and a dock are two processes, so that is the normal case) refuse every reload
/// over a field the flag already overrode.
///
/// # Errors
///
/// Returns the reason the candidate was refused: it expired, or its catalogs
/// could not be loaded.
pub fn apply(state: &mut State, request: &Request) -> Result<(), String> {
    if !request.is_current() {
        return Err("configuration reload expired".to_owned());
    }

    let running = state.shell().panel.surface;
    let mut shell = request.config.shell.clone();

    if shell.panel.surface != running {
        warn!(
            asked = ?shell.panel.surface,
            running = ?running,
            "panel.surface: a different surface is a different process; this one keeps the \
             surface it was started as"
        );
        shell.panel.surface = running;
    }

    let localizer = Rc::new(crate::i18n::load(&request.config.locale).map_err(|e| e.to_string())?);
    state.apply_shell(shell, request.config.theme.clone(), localizer);

    Ok(())
}

#[derive(Clone)]
struct Endpoint {
    paths: ConfigPaths,
    sender: Arc<Mutex<mpsc::Sender<Request>>>,
    busy: Arc<AtomicBool>,
    /// What wakes the panel's loop after a request was queued.
    wake: Ping,
}
impl Endpoint {
    fn submit(&self, expected: &str) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("configuration reload is busy".into());
        }
        let (sender, receiver) = oneshot::channel();
        let completion = Arc::new(Completion {
            sender: Mutex::new(Some(sender)),
            busy: self.busy.clone(),
        });
        let deadline = Instant::now() + DEADLINE;
        let config =
            InoxConfig::load(&self.paths, Appearance::System).map_err(|e| e.to_string())?;
        if !expected.is_empty() && config.snapshot().map_err(|e| e.to_string())? != expected {
            return Err("configuration changed after it was saved; read the files again".into());
        }
        self.sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_send(Request {
                config,
                deadline,
                completion,
            })
            .map_err(|e| e.to_string())?;
        // The channel carries the request; this carries the wake-up. Without it
        // the request would sit there until something else made the loop return
        // from `dispatch`.
        self.wake.ping();
        Ok(receiver)
    }
}
#[zbus::interface(name = "org.inox.Configuration1")]
impl Endpoint {
    async fn reload(&self, generation: u64, expected: String) -> zbus::fdo::Result<(u64, String)> {
        let receiver = self.submit(&expected).map_err(zbus::fdo::Error::Failed)?;
        let applied = receiver
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((generation, applied))
    }
}

/// The reload host of a running panel.
pub struct Host {
    _connection: Option<zbus::Connection>,
    receiver: std::cell::RefCell<Option<mpsc::Receiver<Request>>>,
    endpoint: Endpoint,
}
impl Host {
    /// Starts serving `org.inox.Panel`.
    ///
    /// `wake` is fired whenever a request has been queued, so the caller's event
    /// loop can drain [`Host::take_requests`] without polling. Serving the bus is
    /// best-effort: a session without one logs a warning and keeps `SIGHUP`
    /// working, which is the same degradation the endpoint has always had.
    pub fn start(paths: ConfigPaths, wake: Ping) -> Self {
        let (sender, receiver) = mpsc::channel(1);
        let endpoint = Endpoint {
            paths,
            sender: Arc::new(Mutex::new(sender)),
            busy: Arc::new(AtomicBool::new(false)),
            wake,
        };
        let connection = zbus::block_on(
            zbus::connection::Builder::session()
                .name("org.inox.Panel")
                .serve_at("/org/inox/Configuration", endpoint.clone())
                .build(),
        );
        let connection = match connection {
            Ok(connection) => Some(connection),
            Err(error) => {
                warn!(%error, "configuration bus interface unavailable; SIGHUP remains available");
                None
            }
        };
        Self {
            _connection: connection,
            receiver: std::cell::RefCell::new(Some(receiver)),
            endpoint,
        }
    }

    /// Prepares a reload from the files, because `SIGHUP` asked for one.
    ///
    /// The candidate is read and parsed on a thread of its own: `submit` does
    /// file I/O, and the caller is the panel's event loop, which must not block
    /// on a disk. The request itself is handed over the channel like any other,
    /// and the application happens where the state is.
    pub fn prepare(&self) {
        prepare(&self.endpoint);
    }

    /// Takes everything the bus or `SIGHUP` has prepared, without blocking.
    ///
    /// A non-blocking drain rather than an `EventSource` of its own: the host is
    /// woken by the [`Ping`] it was given, so by the time this is called the
    /// channel has whatever arrived and nothing is missed.
    pub fn take_requests(&self) -> Vec<Request> {
        let mut guard = self.receiver.borrow_mut();
        let Some(receiver) = guard.as_mut() else {
            return Vec::new();
        };
        let mut requests = Vec::new();

        while let Ok(request) = receiver.try_recv() {
            requests.push(request);
        }

        requests
    }
}

/// Reads the candidate and queues it, on a thread that is not the panel's.
fn prepare(endpoint: &Endpoint) {
    let endpoint = endpoint.clone();

    if let Err(error) = std::thread::Builder::new()
        .name("configuration prepare".into())
        .spawn(move || {
            if let Err(error) = endpoint.submit("") {
                warn!(%error, "SIGHUP configuration reload failed");
            }
        })
    {
        warn!(%error, "cannot prepare configuration reload");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::{StreamExt, executor::block_on};

    fn endpoint(paths: ConfigPaths, wake: Ping) -> (Endpoint, mpsc::Receiver<Request>) {
        let (sender, receiver) = mpsc::channel(1);
        (
            Endpoint {
                paths,
                sender: Arc::new(Mutex::new(sender)),
                busy: Arc::new(AtomicBool::new(false)),
                wake,
            },
            receiver,
        )
    }

    /// Applies `request` the way the panel's loop does, and answers the caller.
    fn apply_request(state: &mut State, request: Request) {
        let result = apply(state, &request);

        request.finish(result);
    }

    #[test]
    fn an_expired_preparation_cannot_update_live_preferences() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        let old = InoxConfig::load(&paths, Appearance::System).expect("defaults");
        let mut state = State::new(
            old.shell.panel.clone(),
            old.theme,
            Rc::new(crate::i18n::load(&old.locale).expect("catalog")),
        );
        std::fs::write(dir.path().join("shell.toml"), "[panel]\nheight = 48\n").expect("candidate");
        let (ping, _source) = calloop::ping::make_ping().expect("a ping");
        let (endpoint, mut receiver) = endpoint(paths, ping);
        let reply = endpoint.submit("").expect("prepare");
        let mut request = block_on(receiver.next()).expect("candidate");
        request.deadline = Instant::now() - Duration::from_secs(1);
        apply_request(&mut state, request);
        assert_eq!(state.config(), &old.shell.panel);
        assert!(
            block_on(reply)
                .expect("reply")
                .expect_err("expired")
                .contains("expired")
        );
    }

    #[test]
    fn a_live_reload_keeps_the_surface_the_process_was_started_as() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        let old = InoxConfig::load(&paths, Appearance::System).expect("defaults");
        let mut state = State::new(
            old.shell.panel.clone(),
            old.theme.clone(),
            Rc::new(crate::i18n::load(&old.locale).expect("catalog")),
        );
        let running = state.surface();
        // The candidate names another surface *and* a new height: the surface is
        // refused and the rest of the file is applied.
        std::fs::write(
            dir.path().join("shell.toml"),
            "[panel]\nsurface = \"dock\"\nheight = 48\n",
        )
        .expect("candidate");
        let candidate = InoxConfig::load(&paths, Appearance::System).expect("candidate");
        let request = Request {
            config: candidate,
            deadline: Instant::now() + DEADLINE,
            completion: Arc::new(Completion {
                sender: Mutex::new(None),
                busy: Arc::new(AtomicBool::new(false)),
            }),
        };

        apply(&mut state, &request).expect("a reload is applied");

        assert_eq!(
            state.surface(),
            running,
            "a different surface is a different process: this one keeps the surface it runs as"
        );
        assert_eq!(
            state.config().height,
            48,
            "and the rest of the same file is not thrown away with it"
        );
    }

    #[test]
    fn malformed_or_mismatched_candidates_release_the_gate_and_do_not_reach_the_ui() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        let (ping, _source) = calloop::ping::make_ping().expect("a ping");
        let (endpoint, mut receiver) = endpoint(paths, ping);
        std::fs::write(dir.path().join("shell.toml"), "[panel]\nheight = 'bad'\n").expect("write");
        assert!(
            endpoint
                .submit("")
                .expect_err("invalid config")
                .contains("panel.height")
        );
        std::fs::write(dir.path().join("shell.toml"), "[panel]\nheight = 48\n").expect("write");
        assert!(endpoint.submit("wrong snapshot").is_err());
        let reply = endpoint
            .submit("")
            .expect("the failed requests released the gate");
        assert!(
            endpoint.submit("").is_err(),
            "an outstanding apply is serialized"
        );
        let request = block_on(receiver.next()).expect("candidate");
        assert_eq!(request.config.shell.panel.height, 48);
        request.finish(Err("panel refused the candidate".into()));
        assert_eq!(
            block_on(reply).expect("reply"),
            Err("panel refused the candidate".into())
        );
        drop(request);
        assert!(
            endpoint.submit("").is_ok(),
            "rejection does not wedge reloads"
        );
    }

    /// A prepared request is handed over with a wake-up, so a loop blocked in
    /// `calloop` notices it.
    #[test]
    fn preparing_a_reload_pings_the_panel() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        std::fs::write(dir.path().join("shell.toml"), "[panel]\nheight = 48\n").expect("write");
        let (ping, source) = calloop::ping::make_ping().expect("a ping");
        let (endpoint, mut receiver) = endpoint(paths, ping);

        let _reply = endpoint.submit("").expect("prepare");

        assert!(
            block_on(receiver.next()).is_some(),
            "the request is in the channel the host drains"
        );

        // The wake-up is what a loop blocked in `dispatch` waits for.
        let mut event_loop = calloop::EventLoop::<bool>::try_new().expect("an event loop");

        event_loop
            .handle()
            .insert_source(source, |(), _, woke| *woke = true)
            .expect("the ping source");
        let mut woke = false;
        event_loop
            .dispatch(Duration::ZERO, &mut woke)
            .expect("dispatch");

        assert!(
            woke,
            "the panel's loop was woken for the request it has to apply"
        );
    }

    /// The `SIGHUP` path: the file is read off the panel's thread and the
    /// request reaches the channel the loop drains.
    #[test]
    fn preparing_a_reload_reads_the_candidate_off_the_loops_thread() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        std::fs::write(dir.path().join("shell.toml"), "[panel]\nheight = 48\n").expect("write");
        let (ping, _source) = calloop::ping::make_ping().expect("a ping");
        let (endpoint, mut receiver) = endpoint(paths, ping);

        prepare(&endpoint);

        let deadline = Instant::now() + Duration::from_secs(5);
        let request = loop {
            if let Ok(request) = receiver.try_recv() {
                break request;
            }

            assert!(Instant::now() < deadline, "the reload was not prepared");
            std::thread::sleep(Duration::from_millis(1));
        };

        assert_eq!(request.config.shell.panel.height, 48);
        assert!(
            request.is_current(),
            "a prepared reload is worth applying when it arrives"
        );
    }

    #[test]
    fn real_bus_reply_waits_for_application_and_echoes_the_actual_snapshot() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        let old = InoxConfig::load(&paths, Appearance::System).expect("defaults");
        let mut state = State::new(
            old.shell.panel,
            old.theme,
            Rc::new(crate::i18n::load(&old.locale).expect("catalog")),
        );
        std::fs::write(
            dir.path().join("shell.toml"),
            "[panel]\nheight = 48\nshow_clock = false\n",
        )
        .expect("write");
        let config = InoxConfig::load(&paths, Appearance::System).expect("candidate");
        let expected = config.snapshot().expect("snapshot");
        let (ping, _source) = calloop::ping::make_ping().expect("a ping");
        let (endpoint, mut receiver) = endpoint(paths, ping);
        let (server_socket, client_socket) =
            std::os::unix::net::UnixStream::pair().expect("sockets");
        let guid = zbus::Guid::generate();
        let server = std::thread::spawn(move || {
            zbus::block_on(
                zbus::connection::Builder::unix_stream(server_socket)
                    .server(guid)
                    .p2p()
                    .serve_at("/org/inox/Configuration", endpoint)
                    .build(),
            )
            .expect("server")
        });
        let client = zbus::block_on(
            zbus::connection::Builder::unix_stream(client_socket)
                .p2p()
                .method_timeout(Duration::from_secs(6))
                .build(),
        )
        .expect("client");
        let _server = server.join().expect("thread");
        let wanted = expected.clone();
        let (sent, result) = std::sync::mpsc::channel();
        let (call, requests) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let proxy = zbus::block_on(zbus::Proxy::new(
                &client,
                "org.inox.Panel",
                "/org/inox/Configuration",
                "org.inox.Configuration1",
            ))
            .expect("proxy");
            for () in requests {
                sent.send(zbus::block_on(
                    proxy.call::<_, _, (u64, String)>("Reload", &(42u64, wanted.clone())),
                ))
                .expect("send");
            }
        });
        call.send(()).expect("first reload");
        let deadline = Instant::now() + Duration::from_secs(5);
        let request = loop {
            if let Ok(request) = receiver.try_recv() {
                break request;
            }
            assert!(Instant::now() < deadline, "request was not delivered");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(
            result.try_recv().is_err(),
            "disk reads are not an application acknowledgement"
        );
        apply_request(&mut state, request);
        assert_eq!(state.config().height, 48);
        assert!(!state.config().show_clock);
        let (generation, actual) = result
            .recv_timeout(Duration::from_secs(6))
            .expect("bounded reply")
            .expect("success");
        assert_eq!(generation, 42);
        assert_eq!(actual, expected);
        std::fs::write(
            dir.path().join("shell.toml"),
            "[panel]\nheight = 'broken'\n",
        )
        .expect("invalid candidate");
        call.send(()).expect("second reload");
        let error = result
            .recv_timeout(Duration::from_secs(6))
            .expect("bounded rejection")
            .expect_err("the consumer refuses malformed configuration");
        assert!(
            matches!(error, zbus::Error::FDO(ref error) if matches!(error.as_ref(), zbus::fdo::Error::Failed(reason) if reason.contains("panel.height"))),
            "standard errors retain their typed rejection and field: {error:?}"
        );
        assert_eq!(
            state.config().height,
            48,
            "a rejected D-Bus reload keeps the running preferences"
        );
    }
}
