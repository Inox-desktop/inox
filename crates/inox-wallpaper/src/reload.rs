//! Session-bus and SIGHUP bridge; the state machine stays on the window thread.
//!
//! Two sources wake the component: `SIGHUP` (a hand-rolled signal thread, which
//! has to exist before the configuration is read) and the session bus
//! (`org.inox.Wallpaper`, path `/org/inox/Configuration`). Neither touches the
//! state: a reload is *prepared* here — the configuration is read and its
//! resource decoded — and the prepared request is posted to the host's queue,
//! which the window thread drains in [`crate::runtime::Runtime::pump`].
//!
//! # Why the channels are `std::sync::mpsc`
//!
//! There is no async runtime in this process, so a plain bounded channel per
//! direction is all this bridge needs: one request at a
//! time in, one reply out. The D-Bus method that waits for that reply blocks its
//! own executor for at most [`DEADLINE`], which is the deadline the caller
//! already has; nothing on the window thread ever waits on the bus.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender, SyncSender},
};
use std::time::{Duration, Instant};

use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::warn;

/// How long a prepared reload is still worth applying.
///
/// Also the longest the bus interface waits for the window thread: a caller that
/// asked for a reload gets an answer, even if the answer is "expired".
const DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct Completion {
    sender: Mutex<Option<SyncSender<Result<String, String>>>>,
    busy: Arc<AtomicBool>,
}

impl Drop for Completion {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
pub(super) struct Request {
    pub(super) config: InoxConfig,
    pub(super) resource: super::ReloadResource,
    deadline: Instant,
    completion: Arc<Completion>,
}

impl Request {
    pub(super) fn is_current(&self) -> bool {
        Instant::now() < self.deadline
    }

    pub(super) fn finish(&self, result: Result<(), String>) {
        let result = result.and_then(|()| self.config.snapshot().map_err(|e| e.to_string()));
        if let Err(error) = &result {
            warn!(%error, "configuration reload failed; keeping previous state");
        }
        if let Some(sender) = self
            .completion
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            // SIGHUP has no caller awaiting a reply; a closed receiver is expected.
            let _ = sender.send(result);
        }
    }
}

#[derive(Clone)]
struct Endpoint {
    paths: ConfigPaths,
    wallpaper_dirs: Vec<std::path::PathBuf>,
    sender: Arc<Mutex<Sender<Request>>>,
    busy: Arc<AtomicBool>,
}

impl Endpoint {
    fn submit(&self, expected: &str) -> Result<Receiver<Result<String, String>>, String> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("configuration reload is busy".into());
        }
        let (sender, receiver) = mpsc::sync_channel(1);
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
        let resource = super::prepare_reload(&config, &self.paths, &self.wallpaper_dirs)?;
        self.sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .send(Request {
                config,
                resource,
                deadline,
                completion,
            })
            .map_err(|e| e.to_string())?;
        Ok(receiver)
    }
}

#[zbus::interface(name = "org.inox.Configuration1")]
impl Endpoint {
    /// Prepare the reload and hand it to the window thread.
    ///
    /// The wait is bounded by [`DEADLINE`], and it is the only place this
    /// component blocks its D-Bus executor: the alternative would be a second
    /// asynchronous runtime just to await a channel, which the
    /// no-new-runtime rule does not allow.
    async fn reload(&self, generation: u64, expected: String) -> zbus::fdo::Result<(u64, String)> {
        let receiver = self.submit(&expected).map_err(zbus::fdo::Error::Failed)?;
        let applied = receiver
            .recv_timeout(DEADLINE)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((generation, applied))
    }
}

pub(super) struct Host {
    _connection: Option<zbus::Connection>,
    receiver: std::cell::RefCell<Option<Receiver<Request>>>,
}

impl Host {
    pub(super) fn start(
        paths: ConfigPaths,
        wallpaper_dirs: Vec<std::path::PathBuf>,
    ) -> anyhow::Result<Self> {
        // Install the mask before creating the D-Bus or signal threads.
        let signals = calloop::signals::Signals::new(&[calloop::signals::Signal::SIGHUP])?;
        let (sender, receiver) = mpsc::channel();
        let endpoint = Endpoint {
            paths,
            wallpaper_dirs,
            sender: Arc::new(Mutex::new(sender)),
            busy: Arc::new(AtomicBool::new(false)),
        };
        let signal_endpoint = endpoint.clone();
        let (ready, started) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("configuration signals".into())
            .spawn(move || {
                let run = || -> anyhow::Result<()> {
                    let mut event_loop = calloop::EventLoop::<()>::try_new()?;
                    event_loop
                        .handle()
                        .insert_source(signals, move |_, _, _| {
                            let endpoint = signal_endpoint.clone();
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
                        })
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    ready.send(()).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    event_loop.run(None, &mut (), |_| {})?;
                    Ok(())
                };
                if let Err(error) = run() {
                    warn!(%error, "configuration signal source stopped");
                }
            })?;
        started.recv()?;
        let connection = zbus::block_on(
            zbus::connection::Builder::session()
                .name("org.inox.Wallpaper")
                .serve_at("/org/inox/Configuration", endpoint)
                .build(),
        );
        let connection = match connection {
            Ok(connection) => Some(connection),
            Err(error) => {
                warn!(%error, "configuration bus interface unavailable; SIGHUP remains available");
                None
            }
        };
        Ok(Self {
            _connection: connection,
            receiver: std::cell::RefCell::new(Some(receiver)),
        })
    }

    /// The queue of prepared reloads, taken once so the runtime can own it.
    ///
    /// The host keeps the D-Bus connection alive; the receiver lives with the
    /// runtime.
    pub(super) fn receiver(&self) -> Option<Receiver<Request>> {
        self.receiver.borrow_mut().take()
    }
}
