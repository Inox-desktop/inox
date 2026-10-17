//! A calloop supervisor: Wayland sync readiness, isolated process groups and bounded shutdown.
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use calloop::generic::Generic;
use calloop::signals::{Signal, Signals};
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, Mode, PostAction};
use inox_protocols::standard::core::client::wl_callback;
use rustix::process::{Pid, kill_process_group};
use tracing::{info, warn};
use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::policy::RestartBudget;

const TICK: Duration = Duration::from_millis(50);
const SOCKET: &str = "wayland-inox";

/// A program and literal arguments; never interpreted by a shell.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// Executable path or name resolved through PATH.
    pub program: OsString,
    /// Literal arguments.
    pub arguments: Vec<OsString>,
}

impl CommandSpec {
    /// A command without arguments.
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
        }
    }
}

/// A compositor-dependent component.
#[derive(Debug, Clone)]
pub struct ServiceSpec {
    /// Unique diagnostic identifier.
    pub name: String,
    /// Command to supervise.
    pub command: CommandSpec,
    /// Exhausting this service's restart budget ends the session.
    pub required: bool,
    /// A successful exit completes a bootstrap task instead of restarting it.
    pub oneshot: bool,
}

/// The fixed two-stage session graph: compositor, then independent services.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    /// Compositor command. Its exit always terminates the session.
    pub compositor: CommandSpec,
    /// Components launched after a real wl_display.sync reply.
    pub services: Vec<ServiceSpec>,
    /// Environment from config.toml; session-owned display variables take precedence.
    pub environment: BTreeMap<String, String>,
    /// Absolute, user-owned, private XDG runtime directory.
    pub runtime_dir: PathBuf,
    /// Deadline for the compositor's first protocol response.
    pub startup_timeout: Duration,
}

/// Controls signal registration and optional bounded runs in CI.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Register SIGINT, SIGTERM and SIGHUP in the current thread.
    pub signals: bool,
    /// Request normal shutdown after this duration.
    pub exit_after: Option<Duration>,
    /// The inox-session executable used to reset inherited signal masks before exec.
    /// Required when signal handling is enabled; it runs the internal --exec-child mode.
    pub child_launcher: Option<PathBuf>,
}

/// Session startup, supervision or cleanup failure.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// Invalid graph or runtime directory.
    #[error("invalid session: {0}")]
    Invalid(String),
    /// System boundary failure.
    #[error("session I/O failed: {0}")]
    Io(#[from] io::Error),
    /// The event loop failed.
    #[error("session event loop failed: {0}")]
    EventLoop(String),
    /// A required process or protocol failed.
    #[error("session failed: {0}")]
    Failed(String),
}

struct PrivateRuntime(PathBuf);
impl PrivateRuntime {
    fn create(base: &Path) -> Result<Self, SessionError> {
        let metadata = fs::symlink_metadata(base)?;
        if !base.is_absolute()
            || !metadata.is_dir()
            || metadata.uid() != rustix::process::getuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(SessionError::Invalid(
                "XDG_RUNTIME_DIR must be absolute, owned by this user and mode 0700".into(),
            ));
        }
        let path = base.join(format!("inox-session-{}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}
impl Drop for PrivateRuntime {
    fn drop(&mut self) {
        for name in [SOCKET.to_owned(), format!("{SOCKET}.lock")] {
            if let Err(error) = fs::remove_file(self.0.join(name))
                && error.kind() != io::ErrorKind::NotFound
            {
                warn!(%error, "failed to remove session socket");
            }
        }
        if let Err(error) = fs::remove_dir(&self.0) {
            warn!(%error, path = %self.0.display(), "failed to remove private session directory");
        }
    }
}

struct Process {
    spec: ServiceSpec,
    child: Option<Child>,
    budget: RestartBudget,
    restart_at: Option<Instant>,
    done: bool,
}
impl Process {
    fn new(spec: ServiceSpec) -> Self {
        Self {
            spec,
            child: None,
            budget: RestartBudget::default(),
            restart_at: None,
            done: false,
        }
    }
    fn spawn(
        &mut self,
        runtime: &Path,
        environment: &BTreeMap<String, String>,
        launcher: Option<&Path>,
    ) -> io::Result<()> {
        let mut command = match launcher {
            Some(launcher) => {
                let mut command = Command::new(launcher);
                command.arg("--exec-child").arg(&self.spec.command.program);
                command
            }
            None => Command::new(&self.spec.command.program),
        };
        command
            .args(&self.spec.command.arguments)
            .envs(environment)
            .env("XDG_RUNTIME_DIR", runtime)
            .env("WAYLAND_DISPLAY", SOCKET)
            .env("XDG_SESSION_TYPE", "wayland")
            .env("XDG_CURRENT_DESKTOP", "Inox")
            .env("XDG_SESSION_DESKTOP", "inox")
            .env_remove("WAYLAND_SOCKET")
            .env_remove("DISPLAY")
            .stdin(Stdio::null())
            .process_group(0);
        self.child = Some(command.spawn()?);
        self.restart_at = None;
        info!(component = %self.spec.name, "started session process");
        Ok(())
    }
    fn signal(&self, signal: rustix::process::Signal) {
        if let Some(child) = &self.child
            && let Some(pid) = Pid::from_raw(child.id() as i32)
            && let Err(error) = kill_process_group(pid, signal)
            && error != rustix::io::Errno::SRCH
        {
            warn!(component = %self.spec.name, %error, "failed to signal process group");
        }
    }
    fn reap(&mut self) -> io::Result<Option<bool>> {
        let status = match self.child.as_mut() {
            Some(child) => child.try_wait()?,
            None => None,
        };
        if let Some(status) = status {
            // A crashed leader must not leave descendants in the old failure domain.
            self.signal(rustix::process::Signal::KILL);
            self.child = None;
            info!(component = %self.spec.name, %status, "session process exited");
            return Ok(Some(status.success()));
        }
        Ok(None)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.child.is_some() {
            self.signal(rustix::process::Signal::KILL);
            if let Some(child) = &mut self.child
                && let Err(error) = child.wait()
            {
                warn!(%error, "failed to reap session process");
            }
        }
    }
}

struct Supervisor {
    compositor: Process,
    services: Vec<Process>,
    runtime: PrivateRuntime,
    environment: BTreeMap<String, String>,
    child_launcher: Option<PathBuf>,
    ready: bool,
    probing: bool,
    started_services: bool,
    startup_deadline: Instant,
    exit_deadline: Option<Instant>,
    stopping: Option<Instant>,
    compositor_stopping: bool,
    finished: bool,
    failure: Option<String>,
}
impl Supervisor {
    fn stop(&mut self, failure: Option<String>) {
        if self.stopping.is_some() {
            return;
        }
        self.failure = failure;
        self.stopping = Some(Instant::now());
        for process in self.services.iter().rev() {
            process.signal(rustix::process::Signal::TERM);
        }
        info!(reason = ?self.failure, "stopping session");
    }
    fn tick(&mut self) -> Result<(), SessionError> {
        let now = Instant::now();
        if self.compositor.reap()?.is_some() && self.stopping.is_none() {
            self.stop(Some(
                "compositor exited; client connections cannot be restored".into(),
            ));
        }
        if self.stopping.is_none() {
            if self.exit_deadline.is_some_and(|deadline| now >= deadline) {
                self.stop(None);
            } else if !self.ready && now >= self.startup_deadline {
                self.stop(Some("compositor readiness timed out".into()));
            }
        }
        if let Some(stopped) = self.stopping {
            for process in self.services.iter_mut().rev() {
                process.reap()?;
            }
            let elapsed = now.saturating_duration_since(stopped);
            if elapsed >= Duration::from_secs(4) {
                for process in self.services.iter().rev() {
                    process.signal(rustix::process::Signal::KILL);
                }
            }
            if !self.compositor_stopping
                && (self.services.iter().all(|p| p.child.is_none())
                    || elapsed >= Duration::from_secs(5))
            {
                self.compositor.signal(rustix::process::Signal::TERM);
                self.compositor_stopping = true;
            }
            if elapsed >= Duration::from_secs(8) {
                self.compositor.signal(rustix::process::Signal::KILL);
            }
            self.finished =
                self.compositor.child.is_none() && self.services.iter().all(|p| p.child.is_none());
            return Ok(());
        }
        if self.ready && !self.started_services {
            self.started_services = true;
            // This reports compositor readiness only: L3 has no service-ready transport yet.
            info!("compositor protocol ready; starting desktop components");
            for process in &mut self.services {
                process.restart_at = Some(now);
            }
        }
        for process in &mut self.services {
            if let Some(success) = process.reap()? {
                if success && process.spec.oneshot {
                    process.done = true;
                } else {
                    process.restart_at = Some(now);
                }
                if !process.done {
                    match process.budget.reserve(now) {
                        Some(delay) => process.restart_at = Some(now + delay),
                        None => {
                            process.done = true;
                            warn!(component = %process.spec.name, "restart budget exhausted");
                            if process.spec.required {
                                let reason = format!(
                                    "required component {} exhausted its restart budget",
                                    process.spec.name
                                );
                                self.stop(Some(reason));
                                return Ok(());
                            }
                        }
                    }
                }
            }
            if !process.done
                && process.restart_at.is_some_and(|deadline| now >= deadline)
                && let Err(error) = process.spawn(
                    &self.runtime.0,
                    &self.environment,
                    self.child_launcher.as_deref(),
                )
            {
                warn!(component = %process.spec.name, %error, "component spawn failed");
                match process.budget.reserve(now) {
                    Some(delay) => process.restart_at = Some(now + delay),
                    None => {
                        process.done = true;
                        if process.spec.required {
                            let reason = format!(
                                "required component {} could not start: {error}",
                                process.spec.name
                            );
                            self.stop(Some(reason));
                            return Ok(());
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct ProbeState {
    replied: bool,
}
impl Dispatch<wl_callback::WlCallback, ()> for ProbeState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.replied = true;
        }
    }
}

/// Validate, start and supervise the session until logout or a required failure.
/// Processes are cleaned up even when event source registration or dispatch fails.
pub fn run(spec: SessionSpec, options: RunOptions) -> Result<(), SessionError> {
    if options.signals && options.child_launcher.is_none() {
        return Err(SessionError::Invalid(
            "signal handling requires the inox-session child launcher".into(),
        ));
    }
    if spec.startup_timeout.is_zero()
        || spec.startup_timeout > Duration::from_secs(60)
        || options
            .exit_after
            .is_some_and(|d| d.is_zero() || d > Duration::from_secs(86400))
        || spec.compositor.program.is_empty()
    {
        return Err(SessionError::Invalid(
            "startup timeout must be 1..=60000 ms, bounded runs 1..=86400000 ms and compositor nonempty".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for service in &spec.services {
        if service.name.is_empty()
            || service.command.program.is_empty()
            || !names.insert(&service.name)
        {
            return Err(SessionError::Invalid(
                "service names must be nonempty and unique; commands must be nonempty".into(),
            ));
        }
    }
    let runtime = PrivateRuntime::create(&spec.runtime_dir)?;
    let mut event_loop =
        EventLoop::<Supervisor>::try_new().map_err(|e| SessionError::EventLoop(e.to_string()))?;
    let mut compositor = Process::new(ServiceSpec {
        name: "compositor".into(),
        command: spec.compositor,
        required: true,
        oneshot: false,
    });
    compositor.spawn(
        &runtime.0,
        &spec.environment,
        options.child_launcher.as_deref(),
    )?;
    let now = Instant::now();
    let mut supervisor = Supervisor {
        compositor,
        services: spec.services.into_iter().map(Process::new).collect(),
        runtime,
        environment: spec.environment,
        child_launcher: options.child_launcher,
        ready: false,
        probing: false,
        started_services: false,
        startup_deadline: now + spec.startup_timeout,
        exit_deadline: options.exit_after.map(|d| now + d),
        stopping: None,
        compositor_stopping: false,
        finished: false,
        failure: None,
    };
    let handle = event_loop.handle();
    handle
        .insert_source(Timer::immediate(), |_, _, _: &mut Supervisor| {
            TimeoutAction::ToDuration(TICK)
        })
        .map_err(|e| SessionError::EventLoop(e.to_string()))?;
    if options.signals {
        let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP])
            .map_err(|e| SessionError::EventLoop(e.to_string()))?;
        handle.insert_source(signals, |event, _, supervisor| {
            if event.signal() == Signal::SIGHUP {
                warn!("session environment is fixed until logout; component reloads remain owned by their services");
            } else { supervisor.stop(None); }
        })
            .map_err(|e| SessionError::EventLoop(e.to_string()))?;
    }
    while !supervisor.finished {
        if !supervisor.probing && supervisor.stopping.is_none() {
            match UnixStream::connect(supervisor.runtime.0.join(SOCKET)) {
                Ok(stream) => {
                    let connection = Connection::from_socket(stream)
                        .map_err(|e| SessionError::Failed(e.to_string()))?;
                    let mut queue = connection.new_event_queue::<ProbeState>();
                    connection.display().sync(&queue.handle(), ());
                    connection
                        .flush()
                        .map_err(|e| SessionError::Failed(e.to_string()))?;
                    let mut state = ProbeState::default();
                    handle
                        .insert_source(
                            Generic::new(connection, Interest::READ, Mode::Level),
                            move |_, connection, supervisor| {
                                let result = (|| -> Result<(), String> {
                                    if let Some(guard) = connection.prepare_read() {
                                        match guard.read() {
                                            Ok(_) => {}
                                            Err(wayland_client::backend::WaylandError::Io(
                                                error,
                                            )) if error.kind() == io::ErrorKind::WouldBlock => {}
                                            Err(error) => return Err(error.to_string()),
                                        }
                                    }
                                    queue
                                        .dispatch_pending(&mut state)
                                        .map_err(|e| e.to_string())?;
                                    Ok(())
                                })();
                                if let Err(error) = result {
                                    supervisor.stop(Some(format!(
                                        "compositor readiness failed: {error}"
                                    )));
                                    return Ok(PostAction::Remove);
                                }
                                if state.replied {
                                    supervisor.ready = true;
                                    Ok(PostAction::Remove)
                                } else {
                                    Ok(PostAction::Continue)
                                }
                            },
                        )
                        .map_err(|e| SessionError::EventLoop(e.to_string()))?;
                    supervisor.probing = true;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        event_loop
            .dispatch(None, &mut supervisor)
            .map_err(|e| SessionError::EventLoop(e.to_string()))?;
        supervisor.tick()?;
    }
    match supervisor.failure.take() {
        Some(reason) => Err(SessionError::Failed(reason)),
        None => Ok(()),
    }
}
