//! Session-bus and SIGHUP bridge; application state stays on the Qt thread.
//!
//! The channels are the crate's own ([`crate::runtime`]): a request travels to the
//! UI thread over a bounded `std` channel, the UI thread finishes it, and the
//! D-Bus method that asked awaits a one-shot future for the outcome. Nothing here
//! touches Qt, and nothing here applies a configuration itself — `main` does that
//! on the UI thread, which is what keeps a reload a state transition like every
//! other message.
//!
//! A reload that fails keeps the previous state: the outcome is only reported as
//! applied when `State::apply_configuration` ran, and a failure is logged and
//! answered as an error rather than silently dropped (root AGENTS.md §7.1).

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{debug, warn};

use crate::runtime::oneshot;

/// How long a prepared reload stays worth applying.
const DEADLINE: Duration = Duration::from_secs(5);

/// Marks a reload as in flight, and gives the marker back when it is dropped.
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

/// One reload, prepared off the UI thread and applied on it.
#[derive(Debug)]
pub(super) struct Request {
    /// The configuration that was read.
    pub(super) config: InoxConfig,
    /// The index the configuration's locale and hidden-entry rule produce.
    pub(super) resource: super::ReloadResource,
    deadline: Instant,
    completion: Arc<Completion>,
}

impl Request {
    /// Whether the request is still worth applying.
    pub(super) fn is_current(&self) -> bool {
        Instant::now() < self.deadline
    }

    /// Reports the outcome to whoever asked, and clears the in-flight marker.
    pub(super) fn finish(&self, result: Result<(), String>) {
        let result = result.and_then(|()| self.config.snapshot().map_err(|e| e.to_string()));
        if let Err(error) = &result {
            warn!(%error, "configuration reload failed; keeping previous state");
        }
        let sender = self
            .completion
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        let Some(sender) = sender else {
            return;
        };
        if sender.send(result).is_err() {
            // SIGHUP has no caller awaiting a reply, so a closed receiver is
            // expected rather than an error.
            debug!("configuration reload finished with no caller waiting for it");
        }
    }
}

#[derive(Clone)]
struct Endpoint {
    paths: ConfigPaths,
    sender: Arc<Mutex<SyncSender<Request>>>,
    busy: Arc<AtomicBool>,
}

impl Endpoint {
    fn submit(&self, expected: &str) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("configuration reload is busy".into());
        }
        let (sender, receiver) = oneshot::channel();
        let completion = Arc::new(Completion {
            sender: Mutex::new(Some(sender)),
            busy: Arc::clone(&self.busy),
        });
        let deadline = Instant::now() + DEADLINE;
        let config =
            InoxConfig::load(&self.paths, Appearance::System).map_err(|e| e.to_string())?;
        if !expected.is_empty() && config.snapshot().map_err(|e| e.to_string())? != expected {
            return Err("configuration changed after it was saved; read the files again".into());
        }
        let resource = super::prepare_reload(&config, &self.paths)?;
        self.sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_send(Request {
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
    async fn reload(&self, generation: u64, expected: String) -> zbus::fdo::Result<(u64, String)> {
        let receiver = self.submit(&expected).map_err(zbus::fdo::Error::Failed)?;
        let applied = receiver
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((generation, applied))
    }
}

/// The reload source the launcher runs for its whole lifetime.
pub(super) struct Host {
    _connection: Option<zbus::Connection>,
    receiver: Receiver<Request>,
}

impl Host {
    /// Installs the SIGHUP mask and opens the D-Bus interface.
    ///
    /// The mask is installed here, before the Qt application exists, so a SIGHUP
    /// cannot reach the default handler and kill a session mid-frame.
    pub(super) fn start(paths: ConfigPaths) -> anyhow::Result<Self> {
        let signals = calloop::signals::Signals::new(&[calloop::signals::Signal::SIGHUP])?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let endpoint = Endpoint {
            paths,
            sender: Arc::new(Mutex::new(sender)),
            busy: Arc::new(AtomicBool::new(false)),
        };
        let signal_endpoint = endpoint.clone();
        let (ready, started) = mpsc::sync_channel(1);
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
                .name("org.inox.Launcher")
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
            receiver,
        })
    }

    /// Takes the requests that arrived since the last call.
    ///
    /// `pump` calls this, so a reload is applied on the UI thread like every other
    /// message instead of on the thread that received the signal.
    pub(super) fn requests(&self) -> Vec<Request> {
        let mut requests = Vec::new();
        while let Ok(request) = self.receiver.try_recv() {
            requests.push(request);
        }
        requests
    }
}
