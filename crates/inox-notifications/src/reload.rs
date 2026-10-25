//! Session-bus and SIGHUP bridge; application state stays on the Qt thread.
//!
//! A reload is prepared on a thread of its own — the file is read, parsed and
//! compared before the daemon is told anything — and then handed to the Qt thread
//! on a channel ([`Host::poll`], drained by [`crate::runtime::Runtime::pump`]).
//! The daemon applies it through the same `State::apply_configuration` the unit
//! tests drive, and answers the caller through a one-shot [`Reply`], because the
//! D-Bus method has to return what was applied.
//!
//! Nothing here touches [`crate::app::State`]: the state is not `Send`, and the
//! point of the split is that the thread that reads `notifications.toml` is never
//! the thread that owns the popup stack.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::warn;

use crate::runtime::{Completer, Reply, reply_channel};

/// How long a prepared reload stays applicable.
///
/// A reload is a snapshot of the files: if the daemon has not applied it within
/// this window, what is on disk may have moved on, and answering "applied" for a
/// snapshot nobody read would be a lie.
const DEADLINE: Duration = Duration::from_secs(5);

/// The giving end of one reload's completion, and whether a reload is in flight.
#[derive(Debug)]
struct Completion {
    sender: Mutex<Option<Completer<Result<String, String>>>>,
    busy: Arc<AtomicBool>,
}

impl Drop for Completion {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::Release);
    }
}

/// One prepared reload, waiting for the Qt thread.
#[derive(Debug, Clone)]
pub struct Request {
    /// The configuration that was read.
    pub config: InoxConfig,
    deadline: Instant,
    completion: Arc<Completion>,
}

impl Request {
    /// Whether the prepared configuration is still fresh.
    pub fn is_current(&self) -> bool {
        Instant::now() < self.deadline
    }

    /// Answers whoever asked for the reload, with the snapshot the daemon applied.
    ///
    /// SIGHUP has no caller awaiting a reply; a closed receiver is expected there
    /// and is not an error.
    pub fn finish(&self, result: Result<(), String>) {
        let result =
            result.and_then(|()| self.config.snapshot().map_err(|error| error.to_string()));

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
            let _ = sender.complete(result);
        }
    }
}

/// The bus interface and the SIGHUP handler, which both prepare reloads.
#[derive(Clone)]
struct Endpoint {
    paths: ConfigPaths,
    sender: Arc<Mutex<SyncSender<Request>>>,
    busy: Arc<AtomicBool>,
}

impl Endpoint {
    /// Reads the configuration and queues it for the daemon.
    fn submit(&self, expected: &str) -> Result<Reply<Result<String, String>>, String> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err("configuration reload is busy".into());
        }

        let (completer, reply) = reply_channel();
        let completion = Arc::new(Completion {
            sender: Mutex::new(Some(completer)),
            busy: Arc::clone(&self.busy),
        });
        let deadline = Instant::now() + DEADLINE;
        let config =
            InoxConfig::load(&self.paths, Appearance::System).map_err(|error| error.to_string())?;

        if !expected.is_empty() && config.snapshot().map_err(|error| error.to_string())? != expected
        {
            return Err("configuration changed after it was saved; read the files again".into());
        }

        match self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .try_send(Request {
                config,
                deadline,
                completion,
            }) {
            Ok(()) => Ok(reply),
            Err(TrySendError::Full(_)) => Err("a configuration reload is already queued".into()),
            Err(TrySendError::Disconnected(_)) => {
                Err("the daemon is not applying configuration reloads".into())
            }
        }
    }
}

#[zbus::interface(name = "org.inox.Configuration1")]
impl Endpoint {
    /// Applies a reload and reports the snapshot that was applied.
    async fn reload(&self, generation: u64, expected: String) -> zbus::fdo::Result<(u64, String)> {
        let reply = self.submit(&expected).map_err(zbus::fdo::Error::Failed)?;
        let applied = reply
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;

        Ok((generation, applied))
    }
}

/// The bridge's thread-side half: the reload queue the Qt thread drains.
pub struct Host {
    _connection: Option<zbus::Connection>,
    receiver: Receiver<Request>,
}

impl Host {
    /// Starts the SIGHUP handler and, when a session bus is there, the
    /// `org.inox.Configuration1` interface.
    ///
    /// # Errors
    ///
    /// When the signal mask cannot be installed: a session whose SIGHUP would
    /// kill the daemon instead of reloading it is a reason not to start.
    pub fn start(paths: ConfigPaths) -> anyhow::Result<Self> {
        // Install the mask before creating the D-Bus or Qt threads.
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
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

                    ready
                        .send(())
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
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
                .name("org.inox.Notifications")
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

    /// The next prepared reload, if one is waiting.
    ///
    /// Called by [`crate::runtime::Runtime::pump`] on the Qt thread; never blocks.
    pub fn poll(&self) -> Option<Request> {
        self.receiver.try_recv().ok()
    }
}
