//! Off-loop candidate loading and confirmed live application.

use super::{CalloopData, RunError, physical};
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use inox_protocols::ipc::inox_ipc::server::inox_ipc::{InoxIpc, ReloadResult};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

const DEADLINE: Duration = Duration::from_secs(5);

enum Reply {
    Signal,
    Wayland(InoxIpc),
    Bus(mpsc::SyncSender<Result<String, String>>),
}
impl Reply {
    fn send(self, result: Result<String, String>) {
        match &result {
            Ok(_) => info!("live configuration applied"),
            Err(error) => warn!(%error, "configuration reload failed"),
        }
        match self {
            Self::Signal => {}
            Self::Wayland(resource) => resource.config_reloaded(if result.is_ok() {
                ReloadResult::Success
            } else {
                ReloadResult::Rejected
            }),
            Self::Bus(sender) => {
                // The client may have reached its timeout or disconnected.
                let _ = sender.send(result);
            }
        }
    }
}

pub(crate) struct Trigger {
    expected: String,
    deadline: Instant,
    reply: Reply,
}
impl Trigger {
    pub(crate) fn wayland(resource: InoxIpc) -> Self {
        Self {
            expected: String::new(),
            deadline: Instant::now() + DEADLINE,
            reply: Reply::Wayland(resource),
        }
    }
}
struct Prepared {
    config: Result<InoxConfig, String>,
    trigger: Trigger,
    settled: mpsc::SyncSender<()>,
}

#[derive(Clone)]
struct Endpoint {
    sender: mpsc::SyncSender<Trigger>,
}
#[zbus::interface(name = "org.inox.Configuration1")]
impl Endpoint {
    fn reload(&self, generation: u64, expected: String) -> zbus::fdo::Result<(u64, String)> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(Trigger {
                expected,
                deadline: Instant::now() + DEADLINE,
                reply: Reply::Bus(sender),
            })
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        let applied = receiver
            .recv_timeout(DEADLINE)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?
            .map_err(zbus::fdo::Error::Failed)?;
        Ok((generation, applied))
    }
}

pub(super) struct Controller {
    sender: mpsc::SyncSender<Trigger>,
    _connection: Option<zbus::blocking::Connection>,
}
impl Controller {
    pub(super) fn signal(&self) {
        if let Err(error) = self.sender.try_send(Trigger {
            expected: String::new(),
            deadline: Instant::now() + DEADLINE,
            reply: Reply::Signal,
        }) {
            warn!(%error, "configuration reload is busy");
        }
    }
}

pub(super) fn register(
    handle: &calloop::LoopHandle<'static, CalloopData>,
    data: &mut CalloopData,
) -> Result<(), RunError> {
    let paths = ConfigPaths::discover().map_err(|e| RunError::EventSource {
        name: "configuration paths",
        reason: e.to_string(),
    })?;
    let (sender, receiver) = mpsc::sync_channel::<Trigger>(1);
    let (prepared_sender, prepared_receiver) = calloop::channel::channel::<Prepared>();
    std::thread::Builder::new()
        .name("configuration prepare".into())
        .spawn(move || {
            while let Ok(trigger) = receiver.recv() {
                let config = (|| {
                    if Instant::now() >= trigger.deadline {
                        return Err("configuration reload expired".into());
                    }
                    let config =
                        InoxConfig::load(&paths, Appearance::System).map_err(|e| e.to_string())?;
                    if !trigger.expected.is_empty()
                        && config.snapshot().map_err(|e| e.to_string())? != trigger.expected
                    {
                        return Err(
                            "configuration changed after it was saved; read the files again".into(),
                        );
                    }
                    Ok(config)
                })();
                let (settled, wait) = mpsc::sync_channel(1);
                if prepared_sender
                    .send(Prepared {
                        config,
                        trigger,
                        settled,
                    })
                    .is_err()
                {
                    break;
                }
                // Preparation is serialized with actual application, not just disk reads.
                if wait.recv().is_err() {
                    break;
                }
            }
        })
        .map_err(|e| RunError::EventSource {
            name: "configuration worker",
            reason: e.to_string(),
        })?;
    handle
        .insert_source(prepared_receiver, |event, _, data| {
            if let calloop::channel::Event::Msg(prepared) = event {
                let result = prepared.config.and_then(|config| {
                    if Instant::now() >= prepared.trigger.deadline {
                        return Err("configuration reload expired".into());
                    }
                    apply(data, &config)?;
                    config.snapshot().map_err(|e| e.to_string())
                });
                prepared.trigger.reply.send(result);
                // The worker has exited only when the event loop is shutting down.
                let _ = prepared.settled.send(());
                if let Err(error) = data.display_handle.flush_clients() {
                    warn!(%error, "cannot flush configuration reply");
                }
            }
        })
        .map_err(|e| RunError::EventSource {
            name: "configuration replies",
            reason: e.to_string(),
        })?;
    let endpoint = Endpoint {
        sender: sender.clone(),
    };
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name("org.inox.Compositor"))
        .and_then(|b| b.serve_at("/org/inox/Configuration", endpoint))
        .and_then(|b| b.build());
    let connection = match connection {
        Ok(connection) => Some(connection),
        Err(error) => {
            warn!(%error, "configuration bus interface unavailable; Wayland and SIGHUP remain available");
            None
        }
    };
    data.state.reload_requests = Some(sender.clone());
    data.reload = Some(Controller {
        sender,
        _connection: connection,
    });
    Ok(())
}

fn apply(data: &mut CalloopData, config: &InoxConfig) -> Result<(), String> {
    if let Some(old) = &data.active_configuration {
        if old.compositor.outputs != config.compositor.outputs {
            return Err(
                "outputs: live display changes require server-side trial and rollback".into(),
            );
        }
        if old.compositor.input.input_method != config.compositor.input.input_method {
            return Err("input.input_method: live engine replacement is not available".into());
        }
        if old.root.environment != config.root.environment {
            return Err("environment: running process environments cannot be replaced".into());
        }
    } else if !config.compositor.outputs.is_empty()
        || !config.compositor.input.input_method.is_empty()
    {
        return Err(
            "outputs/input.input_method: explicit headless runtime preserves command-line choices"
                .into(),
        );
    }
    data.state
        .apply_configuration(config)
        .map_err(|e| e.to_string())?;
    if let Err(error) = physical::apply_input(data, config.compositor.input.clone()) {
        if let Some(old) = data.active_configuration.clone()
            && let Err(rollback) = data.state.apply_configuration(&old)
        {
            return Err(format!("{error}; keyboard rollback failed: {rollback}"));
        }
        return Err(error);
    }
    data.active_configuration = Some(config.clone());
    Ok(())
}
