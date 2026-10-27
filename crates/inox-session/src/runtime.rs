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
use tracing::{debug, info, warn};
use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::policy::RestartBudget;

const TICK: Duration = Duration::from_millis(50);
const SOCKET: &str = "wayland-inox";

/// A program and literal arguments; never interpreted by a shell.
///
/// The session's own environment is applied to every child; per-component values
/// live on [`ServiceSpec::environment`].
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

/// When a service is started, and therefore whether it is ever restarted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Started as soon as the compositor is ready, and restarted within budget.
    AtStartup,
    /// Started only when something asks for it (`session-control-v1`), and never
    /// restarted automatically.
    ///
    /// This is what the trusted lock client needs: restarting it while the session
    /// is locked asks a compositor that refuses a second lock, and restarting it
    /// after an unlock locks the desktop the person just opened. A service that is
    /// asked for by name is started again by the next request.
    OnDemand,
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
    /// Whether the service is started with the session or on request.
    pub start: Start,
    /// Extra environment for this component alone, layered over the session's.
    ///
    /// This exists for capabilities that must not be handed to every child: the
    /// session lock token authorizes taking the session lock, so putting it in the
    /// session's environment would hand it to the shell and every component it
    /// starts. Only the trusted lock client gets it.
    pub environment: BTreeMap<String, String>,
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
            // Per-service values win over the session's: they are the ones carrying
            // a capability that belongs to this component alone.
            .envs(&self.spec.environment)
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
        info!(component = %self.spec.name, pid = self.child.as_ref().map(Child::id), "started session process");
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

/// The name the lock service is registered under in a session's service table.
pub const LOCK_SERVICE: &str = "lock";

/// What asking for a service to start did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartOutcome {
    /// Queued for the next tick.
    Requested,
    /// It is already running; nothing was changed.
    AlreadyRunning,
    /// This session has no such service.
    NoSuchService,
}

struct Supervisor {
    compositor: Process,
    services: Vec<Process>,
    control: crate::control::Listener,
    last_lock_request: Option<Instant>,
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
    services_killed: bool,
    compositor_killed: bool,
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
    /// Starts a service that is in the table, if it is not already running.
    fn start_service(&mut self, name: &str) -> StartOutcome {
        let now = Instant::now();
        let Some(process) = self.services.iter_mut().find(|p| p.spec.name == name) else {
            return StartOutcome::NoSuchService;
        };

        if process.child.is_some() {
            return StartOutcome::AlreadyRunning;
        }

        // A service that has finished is started again only by being asked for.
        process.done = false;
        process.restart_at = Some(now);
        info!(component = name, "a service was asked to start");

        StartOutcome::Requested
    }

    /// Answers one `lock` request.
    ///
    /// The reply says what the supervisor did, not what the session is: whether the
    /// screen is covered is the compositor's `session_lock_state`.
    fn request_lock(&mut self) -> inox_protocols::control::Reply {
        use inox_protocols::control::Reply;

        let now = Instant::now();

        if !self.ready {
            debug!("a lock was requested before the compositor was ready");

            return Reply::Busy;
        }

        if self
            .last_lock_request
            .is_some_and(|last| now.saturating_duration_since(last) < Duration::from_secs(2))
        {
            debug!("a lock was requested while the previous request is still fresh");

            return Reply::Busy;
        }

        self.last_lock_request = Some(now);

        match self.start_service(LOCK_SERVICE) {
            StartOutcome::Requested => Reply::Dispatched,
            StartOutcome::AlreadyRunning => Reply::Busy,
            StartOutcome::NoSuchService => Reply::Unknown,
        }
    }

    /// Serves every pending control connection.
    fn serve_control(&mut self) {
        let uid = rustix::process::getuid().as_raw();

        loop {
            match self.control.accept() {
                Ok(Some(stream)) => {
                    let crate::control::Served::Request(request, mut stream) =
                        crate::control::serve(stream, uid)
                    else {
                        continue;
                    };

                    let reply = match request {
                        inox_protocols::control::Request::Lock => self.request_lock(),
                    };

                    if let Err(error) = inox_protocols::control::write_reply(&mut stream, reply) {
                        debug!(%error, "failed to answer a control request");
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    warn!(%error, "failed to accept a control connection");

                    break;
                }
            }
        }
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
            if elapsed >= Duration::from_secs(4) && !self.services_killed {
                self.services_killed = true;
                for process in self.services.iter().rev() {
                    if process.child.is_some() {
                        warn!(component = %process.spec.name, "shutdown grace period expired; sending SIGKILL");
                    }
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
            if elapsed >= Duration::from_secs(8) && !self.compositor_killed {
                self.compositor_killed = true;
                if self.compositor.child.is_some() {
                    warn!("compositor shutdown grace period expired; sending SIGKILL");
                }
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
                if process.spec.start == Start::AtStartup {
                    process.restart_at = Some(now);
                }
            }
        }
        for process in &mut self.services {
            if let Some(success) = process.reap()? {
                if process.spec.start == Start::OnDemand {
                    // Asked for, once. Restarting the lock client on a timer would
                    // either be refused by the compositor (already locked) or lock
                    // a session nobody asked to lock again.
                    process.done = true;
                } else if success && process.spec.oneshot {
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
    let handle = event_loop.handle();

    // The control socket exists before any component starts, so a request that
    // arrives while the compositor is still coming up gets `busy` — an answer —
    // rather than a connection refused the caller cannot interpret.
    let control_path = crate::control::path_for(&spec.runtime_dir);
    let control = crate::control::Listener::bind(&control_path)?;
    let control_handle = control.handle()?;
    handle
        .insert_source(
            Generic::new(control_handle, Interest::READ, Mode::Level),
            |_, _, supervisor: &mut Supervisor| {
                supervisor.serve_control();

                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| SessionError::EventLoop(e.to_string()))?;

    let mut compositor = Process::new(ServiceSpec {
        name: "compositor".into(),
        command: spec.compositor,
        required: true,
        oneshot: false,
        start: Start::AtStartup,
        // The lock token reaches the compositor as an argument, not through the
        // environment: it is the compositor's copy of the capability that decides
        // whether a claim is honoured, and that is a startup decision.
        environment: BTreeMap::new(),
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
        control,
        last_lock_request: None,
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
        services_killed: false,
        compositor_killed: false,
        finished: false,
        failure: None,
    };
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
