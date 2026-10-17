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
        self.sender
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_send(Request {
                config,
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
    _connection: Option<zbus::blocking::Connection>,
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
        let connection = zbus::blocking::connection::Builder::session()
            .and_then(|builder| builder.name("org.inox.Panel"))
            .and_then(|builder| builder.serve_at("/org/inox/Configuration", endpoint))
            .and_then(|builder| builder.build());
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

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::{StreamExt, executor::block_on};
    use inox_panel::{app::State, i18n};
    use std::rc::Rc;

    fn endpoint(paths: ConfigPaths) -> (Endpoint, mpsc::Receiver<Request>) {
        let (sender, receiver) = mpsc::channel(1);
        (
            Endpoint {
                paths,
                sender: Arc::new(Mutex::new(sender)),
                busy: Arc::new(AtomicBool::new(false)),
            },
            receiver,
        )
    }

    #[test]
    fn an_expired_preparation_cannot_update_live_preferences() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        let old = InoxConfig::load(&paths, Appearance::System).expect("defaults");
        let mut state = State::new(
            old.shell.panel.clone(),
            old.theme,
            Rc::new(i18n::load(&old.locale).expect("catalog")),
        );
        std::fs::write(dir.path().join("shell.toml"), "[panel]\nheight = 48\n").expect("candidate");
        let (endpoint, mut receiver) = endpoint(paths);
        let reply = endpoint.submit("").expect("prepare");
        let mut request = block_on(receiver.next()).expect("candidate");
        request.deadline = Instant::now() - Duration::from_secs(1);
        let _task = super::super::update_runtime(
            &mut state,
            super::super::RuntimeMessage::Reload(Box::new(request)),
        );
        assert_eq!(state.config(), &old.shell.panel);
        assert!(
            block_on(reply)
                .expect("reply")
                .expect_err("expired")
                .contains("expired")
        );
    }

    #[test]
    fn malformed_or_mismatched_candidates_release_the_gate_and_do_not_reach_the_ui() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        let (endpoint, mut receiver) = endpoint(paths);
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

    #[test]
    fn real_bus_reply_waits_for_application_and_echoes_the_actual_snapshot() {
        let dir = tempfile::tempdir().expect("directory");
        let paths = ConfigPaths::new(dir.path());
        let old = InoxConfig::load(&paths, Appearance::System).expect("defaults");
        let mut state = State::new(
            old.shell.panel,
            old.theme,
            Rc::new(i18n::load(&old.locale).expect("catalog")),
        );
        std::fs::write(
            dir.path().join("shell.toml"),
            "[panel]\nheight = 48\nshow_clock = false\n",
        )
        .expect("write");
        let config = InoxConfig::load(&paths, Appearance::System).expect("candidate");
        let expected = config.snapshot().expect("snapshot");
        let (endpoint, mut receiver) = endpoint(paths);
        let (server_socket, client_socket) =
            std::os::unix::net::UnixStream::pair().expect("sockets");
        let guid = zbus::Guid::generate();
        let server = std::thread::spawn(move || {
            zbus::blocking::connection::Builder::async_io_unix_stream(server_socket)
                .server(guid)
                .expect("guid")
                .p2p()
                .serve_at("/org/inox/Configuration", endpoint)
                .expect("interface")
                .build()
                .expect("server")
        });
        let client = zbus::blocking::connection::Builder::async_io_unix_stream(client_socket)
            .p2p()
            .method_timeout(Duration::from_secs(6))
            .build()
            .expect("client");
        let _server = server.join().expect("thread");
        let wanted = expected.clone();
        let (sent, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let proxy = zbus::blocking::Proxy::new(
                &client,
                "org.inox.Panel",
                "/org/inox/Configuration",
                "org.inox.Configuration1",
            )
            .expect("proxy");
            sent.send(proxy.call::<_, _, (u64, String)>("Reload", &(42u64, wanted)))
                .expect("send");
        });
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
        let _task = super::super::update_runtime(
            &mut state,
            super::super::RuntimeMessage::Reload(Box::new(request)),
        );
        assert_eq!(state.config().height, 48);
        assert!(!state.config().show_clock);
        let (generation, actual) = result
            .recv_timeout(Duration::from_secs(6))
            .expect("bounded reply")
            .expect("success");
        assert_eq!(generation, 42);
        assert_eq!(actual, expected);
    }
}
