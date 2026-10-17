//! The explicitly selected input method's private connection and lifetime.

use std::{
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::{Child, Command, Stdio},
    time::Instant,
};

use super::*;

const MAX_ATTEMPTS: u32 = 4;
const SUPERVISION_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct InputMethodProcess {
    argv: Vec<String>,
    socket_name: OsString,
    child: Option<Child>,
    connection: Option<UnixStream>,
    attempts: u32,
    next_start: Instant,
}

impl InputMethodProcess {
    fn start(&mut self, display: &mut DisplayHandle) -> io::Result<()> {
        let Some(program) = self.argv.first() else {
            return Ok(());
        };
        self.attempts += 1;
        let (server, child_socket) = UnixStream::pair()?;
        let connection = server.try_clone()?;
        let stdin: OwnedFd = child_socket.into();
        let child = Command::new(std::env::current_exe()?)
            .arg("--exec-input-method")
            .arg(program)
            .args(self.argv.iter().skip(1))
            .env("WAYLAND_SOCKET", "0")
            .env("WAYLAND_DISPLAY", &self.socket_name)
            .env_remove("INOX_LOCK_TOKEN")
            .stdin(Stdio::from(stdin))
            .spawn()?;
        self.child = Some(child);
        self.connection = Some(connection);
        if let Err(error) = display.insert_client(server, Arc::new(ClientState::input_method())) {
            self.stop();
            return Err(error);
        }
        info!(
            attempt = self.attempts,
            "started input method on a private Wayland connection"
        );
        Ok(())
    }

    fn close_connection(&mut self) {
        if let Some(connection) = self.connection.take()
            && let Err(error) = connection.shutdown(std::net::Shutdown::Both)
        {
            debug!(%error, "input method connection was already closed");
        }
    }

    fn stop(&mut self) {
        self.close_connection();
        if let Some(mut child) = self.child.take() {
            if let Err(error) = child.kill() {
                debug!(%error, "input method already exited");
            }
            if let Err(error) = child.wait() {
                warn!(%error, "failed to reap input method");
            }
        }
    }

    fn service(&mut self, display: &mut DisplayHandle) {
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => return,
                Ok(Some(status)) => {
                    warn!(%status, attempt = self.attempts, "input method exited");
                    self.child = None;
                    self.close_connection();
                    self.next_start =
                        Instant::now() + Duration::from_secs(1u64 << self.attempts.min(3));
                    if self.attempts == MAX_ATTEMPTS {
                        warn!("input method restart budget exhausted");
                    }
                }
                Err(error) => {
                    warn!(%error, "failed to inspect input method process");
                    return;
                }
            }
        }
        if self.attempts < MAX_ATTEMPTS && Instant::now() >= self.next_start {
            if let Err(error) = self.start(display) {
                warn!(%error, attempt = self.attempts, "failed to start input method; ordinary keyboard input remains available");
                self.next_start =
                    Instant::now() + Duration::from_secs(1u64 << self.attempts.min(3));
            }
            if self.attempts == MAX_ATTEMPTS && self.child.is_none() {
                warn!("input method restart budget exhausted");
            }
        }
    }
}

impl Drop for InputMethodProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) fn register(
    handle: &calloop::LoopHandle<'static, CalloopData>,
    data: &mut CalloopData,
    argv: &[String],
    socket_name: &OsString,
) -> Result<(), RunError> {
    if argv.is_empty() {
        return Ok(());
    }
    let mut process = InputMethodProcess {
        argv: argv.to_vec(),
        socket_name: socket_name.clone(),
        child: None,
        connection: None,
        attempts: 0,
        next_start: Instant::now(),
    };
    process.service(&mut data.display_handle);
    data.input_method = Some(process);
    handle
        .insert_source(Timer::from_duration(SUPERVISION_INTERVAL), |_, _, data| {
            if let Some(process) = data.input_method.as_mut() {
                process.service(&mut data.display_handle);
            }
            TimeoutAction::ToDuration(SUPERVISION_INTERVAL)
        })
        .map(drop)
        .map_err(|error| RunError::EventSource {
            name: "input method supervision",
            reason: error.to_string(),
        })
}
