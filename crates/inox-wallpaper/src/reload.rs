//! Session-bus and SIGHUP bridge; application state stays on the Iced thread.

use iced::futures::channel::{mpsc, oneshot};
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};
use tracing::warn;

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

#[derive(Debug, Clone)]
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
            .unwrap_or_else(|e| e.into_inner())
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
    sender: Arc<Mutex<mpsc::Sender<Request>>>,
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
            busy: self.busy.clone(),
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
            .unwrap_or_else(|e| e.into_inner())
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
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((generation, applied))
    }
}

pub(super) struct Host {
    _connection: Option<zbus::Connection>,
    receiver: std::cell::RefCell<Option<mpsc::Receiver<Request>>>,
}
impl Host {
    pub(super) fn start(paths: ConfigPaths) -> anyhow::Result<Self> {
        // Install the mask before creating the D-Bus or Iced threads.
        let signals = calloop::signals::Signals::new(&[calloop::signals::Signal::SIGHUP])?;
        let (sender, receiver) = mpsc::channel(1);
        let endpoint = Endpoint {
            paths,
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
    pub(super) fn task(&self) -> iced::Task<super::RuntimeMessage> {
        match self.receiver.borrow_mut().take() {
            Some(receiver) => iced::Task::run(receiver, |request| {
                super::RuntimeMessage::Reload(Box::new(request))
            }),
            None => iced::Task::none(),
        }
    }
}
