//! Off-loop candidate loading and confirmed live application.
//!
//! Two D-Bus interfaces live here. `org.inox.Configuration1` applies a candidate
//! immediately and refuses live display changes: `SIGHUP` and `inoxctl` have no
//! confirmation channel, so they may not switch a mode nobody can confirm.
//! `org.inox.Configuration2` (ADR 0028) adds the trial interface: `Trial` applies
//! a candidate with provisional output preferences, `Confirm` keeps it and
//! `Rollback` puts the previous preferences back. The compositor owns the 30
//! second deadline, so a settings window that disappears still gets its display
//! change reverted.
//!
//! Disk reads and the `snapshot()` comparison happen on the prepare worker, off
//! the event loop. Applying the candidate, arming the deadline and reverting it
//! happen on the loop; `Confirm` and `Rollback` never touch the disk and reach the
//! loop through their own bounded channel.

use super::{CalloopData, RunError, physical};
use crate::trial::{self, ArmedTrial, TRIAL_DEADLINE};
use inox_config::{Appearance, ConfigPaths, InoxConfig, OutputConfig};
use inox_protocols::ipc::inox_ipc::server::inox_ipc::{InoxIpc, ReloadResult};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

const DEADLINE: Duration = Duration::from_secs(5);

/// Why a v1 `Reload` refuses live display changes.
const OUTPUTS_REQUIRE_TRIAL: &str =
    "outputs: live display changes require a trial (org.inox.Configuration2.Trial)";

/// Why a headless session refuses configured outputs on any live change.
const HEADLESS_PRESERVES_STARTUP: &str = "outputs: headless outputs come from --output and require a display trial (org.inox.Configuration2.Trial); input.input_method/environment: explicit headless runtime preserves startup choices";

type Handle = calloop::LoopHandle<'static, CalloopData>;

enum Reply {
    Signal,
    /// A `inox_ipc` client resource. Boxed because the generated resource handle
    /// is by far the largest variant, and an oversized `Trigger` would make the
    /// bounded channel's `TrySendError` unwieldy.
    Wayland(Box<InoxIpc>),
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

/// Which interface asked for the candidate to be prepared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerKind {
    /// `org.inox.Configuration1.Reload`: display changes are refused.
    Reload,
    /// `org.inox.Configuration2.Trial`: display changes are provisional.
    Trial,
}

pub(crate) struct Trigger {
    kind: TriggerKind,
    generation: u64,
    expected: String,
    deadline: Instant,
    reply: Reply,
}
impl Trigger {
    fn new(kind: TriggerKind, generation: u64, expected: String, reply: Reply) -> Self {
        Self {
            kind,
            generation,
            expected,
            deadline: Instant::now() + DEADLINE,
            reply,
        }
    }

    pub(crate) fn wayland(resource: InoxIpc) -> Self {
        Self::new(
            TriggerKind::Reload,
            0,
            String::new(),
            Reply::Wayland(Box::new(resource)),
        )
    }

    /// A `Trial` request: the candidate is compared with the disk exactly like a
    /// `Reload`, and only its application differs.
    fn trial(generation: u64, expected: String, reply: Reply) -> Self {
        Self::new(TriggerKind::Trial, generation, expected, reply)
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
        let applied = ask(&self.sender, |reply| {
            Trigger::new(TriggerKind::Reload, generation, expected, reply)
        })?
        .map_err(zbus::fdo::Error::Failed)?;

        Ok((generation, applied))
    }
}

/// Send one trigger to the prepare worker and block on the event loop's answer.
///
/// The trigger carries its reply channel; this keeps the busy and timeout handling
/// for both D-Bus interfaces in one place.
fn ask(
    sender: &mpsc::SyncSender<Trigger>,
    trigger: impl FnOnce(Reply) -> Trigger,
) -> zbus::fdo::Result<Result<String, String>> {
    let (reply, receiver) = mpsc::sync_channel(1);
    sender
        .try_send(trigger(Reply::Bus(reply)))
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;

    receiver
        .recv_timeout(DEADLINE)
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

/// Which trial request arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrialKind {
    /// Keep the armed trial.
    Confirm,
    /// Put the previous output preferences back.
    Rollback,
}

/// One `Confirm` or `Rollback` waiting for the event loop.
struct TrialRequest {
    kind: TrialKind,
    generation: u64,
    reply: mpsc::SyncSender<Result<(u64, String), String>>,
}

#[derive(Clone)]
struct TrialEndpoint {
    prepare: mpsc::SyncSender<Trigger>,
    trial: calloop::channel::SyncSender<TrialRequest>,
}
#[zbus::interface(name = "org.inox.Configuration2")]
impl TrialEndpoint {
    /// Apply a candidate configuration with provisional output preferences.
    ///
    /// The candidate is read, compared with the disk and applied by the event
    /// loop, which arms the 30 second deadline before answering. Any failure puts
    /// the previous output preferences back, so a rejected trial leaves nothing
    /// half applied.
    fn trial(&self, generation: u64, expected: String) -> zbus::fdo::Result<(u64, String)> {
        let applied = ask(&self.prepare, |reply| {
            Trigger::trial(generation, expected, reply)
        })?
        .map_err(zbus::fdo::Error::Failed)?;

        Ok((generation, applied))
    }

    /// Keep the trial `generation` applied and cancel its deadline.
    fn confirm(&self, generation: u64) -> zbus::fdo::Result<(u64, String)> {
        self.ask(TrialKind::Confirm, generation)
    }

    /// Restore the output preferences the trial `generation` replaced.
    ///
    /// Idempotent: an unknown generation changes nothing and still succeeds,
    /// because the settings window and the compositor deadline race at 30 seconds.
    fn rollback(&self, generation: u64) -> zbus::fdo::Result<(u64, String)> {
        self.ask(TrialKind::Rollback, generation)
    }
}
impl TrialEndpoint {
    fn ask(&self, kind: TrialKind, generation: u64) -> zbus::fdo::Result<(u64, String)> {
        let (reply, receiver) = mpsc::sync_channel(1);
        self.trial
            .try_send(TrialRequest {
                kind,
                generation,
                reply,
            })
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        receiver
            .recv_timeout(DEADLINE)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?
            .map_err(zbus::fdo::Error::Failed)
    }
}

/// The one display trial a session may have armed, and its deadline source.
#[derive(Default)]
pub(super) struct TrialState {
    armed: Option<ArmedTrial>,
    timer: Option<calloop::RegistrationToken>,
}
impl TrialState {
    /// No trial is armed and no deadline is set.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The armed trial, for the pure decision rule.
    fn armed(&self) -> Option<&ArmedTrial> {
        self.armed.as_ref()
    }

    fn is_armed(&self) -> bool {
        self.armed.is_some()
    }

    /// Record an applied trial and the deadline source that reverts it.
    fn arm(&mut self, armed: ArmedTrial, timer: calloop::RegistrationToken) {
        self.armed = Some(armed);
        self.timer = Some(timer);
    }

    /// Disarm the trial and remove its deadline source.
    fn finish(&mut self, handle: &Handle) -> Option<ArmedTrial> {
        if let Some(timer) = self.timer.take() {
            handle.remove(timer);
        }

        self.armed.take()
    }

    /// Take the trial whose deadline source has just fired.
    ///
    /// The source removed itself when it returned [`calloop::timer::TimeoutAction::Drop`],
    /// so the token is dropped rather than removed.
    fn timed_out(&mut self) -> Option<ArmedTrial> {
        self.timer = None;

        self.armed.take()
    }

    fn shutdown(&mut self, handle: &Handle) {
        if let Some(timer) = self.timer.take() {
            handle.remove(timer);
        }
    }
}

pub(super) struct Controller {
    sender: mpsc::SyncSender<Trigger>,
    _connection: Option<zbus::Connection>,
}
impl Controller {
    pub(super) fn signal(&self) {
        if let Err(error) = self.sender.try_send(Trigger::new(
            TriggerKind::Reload,
            0,
            String::new(),
            Reply::Signal,
        )) {
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
    let apply_handle = handle.clone();
    handle
        .insert_source(prepared_receiver, move |event, _, data| {
            if let calloop::channel::Event::Msg(prepared) = event {
                let Prepared {
                    config,
                    trigger,
                    settled,
                } = prepared;
                let result = config.and_then(|config| {
                    if Instant::now() >= trigger.deadline {
                        return Err("configuration reload expired".into());
                    }
                    match trigger.kind {
                        TriggerKind::Reload => {
                            apply(data, &config)?;
                            config.snapshot().map_err(|e| e.to_string())
                        }
                        TriggerKind::Trial => {
                            apply_trial(&apply_handle, data, &config, trigger.generation)
                        }
                    }
                });
                trigger.reply.send(result);
                // The worker has exited only when the event loop is shutting down.
                let _ = settled.send(());
                if let Err(error) = data.display_handle.flush_clients() {
                    warn!(%error, "cannot flush configuration reply");
                }
            }
        })
        .map_err(|e| RunError::EventSource {
            name: "configuration replies",
            reason: e.to_string(),
        })?;

    // `Confirm` and `Rollback` never read the disk, so they have their own bounded
    // channel: only the two disk-reading kinds reach the prepare worker.
    let (trial_sender, trial_channel) = calloop::channel::sync_channel::<TrialRequest>(1);
    let trial_handle = handle.clone();
    handle
        .insert_source(trial_channel, move |event, _, data| {
            let calloop::channel::Event::Msg(request) = event else {
                return;
            };
            let result = serve_trial(&trial_handle, data, &request);
            // A reply that cannot be delivered to a client that already timed out
            // must not become a success: only the loop's decision is sent.
            let _ = request.reply.send(result);
            if let Err(error) = data.display_handle.flush_clients() {
                warn!(%error, "cannot flush trial reply");
            }
        })
        .map_err(|e| RunError::EventSource {
            name: "configuration trials",
            reason: e.to_string(),
        })?;

    let endpoint = Endpoint {
        sender: sender.clone(),
    };
    let trial_endpoint = TrialEndpoint {
        prepare: sender.clone(),
        trial: trial_sender,
    };
    let connection = zbus::block_on(
        zbus::connection::Builder::session()
            .name("org.inox.Compositor")
            .serve_at("/org/inox/Configuration", endpoint)
            .serve_at("/org/inox/Configuration", trial_endpoint)
            .build(),
    );
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

/// Drop the trial deadline when the event loop shuts down.
pub(super) fn shutdown(handle: &Handle, data: &mut CalloopData) {
    data.trial.shutdown(handle);
}

/// Carry out one `Confirm` or `Rollback` on the event loop.
///
/// The reply is whatever the pure rule decided: a stale `Confirm` fails, a stale
/// `Rollback` succeeds without changing anything.
fn serve_trial(
    handle: &Handle,
    data: &mut CalloopData,
    request: &TrialRequest,
) -> Result<(u64, String), String> {
    let decision = match request.kind {
        TrialKind::Confirm => trial::Request::Confirm(request.generation),
        TrialKind::Rollback => trial::Request::Rollback(request.generation),
    };

    match trial::decide(data.trial.armed(), decision, Instant::now()) {
        trial::Verdict::Keep => {
            data.trial.finish(handle);
            info!(generation = request.generation, "display trial confirmed");
            snapshot(data).map(|applied| (request.generation, applied))
        }
        trial::Verdict::Revert => {
            let Some(armed) = data.trial.finish(handle) else {
                return Err("the display trial disappeared before it could be reverted".into());
            };
            let previous = armed.previous().to_vec();
            let restored = revert_outputs(handle, data, &previous);
            match request.kind {
                TrialKind::Rollback => {
                    restored?;
                    info!(
                        generation = request.generation,
                        "display trial rolled back to the previous outputs"
                    );
                    snapshot(data).map(|applied| (request.generation, applied))
                }
                TrialKind::Confirm => {
                    // The deadline is the same rule as the consumer's countdown, so
                    // a confirmation that arrives after it must not keep the mode.
                    if let Err(error) = restored {
                        warn!(%error, "failed to restore the previous outputs after a late confirmation");
                    }
                    Err("display trial expired before it was confirmed".into())
                }
            }
        }
        trial::Verdict::Stale => match request.kind {
            TrialKind::Rollback => snapshot(data).map(|applied| (request.generation, applied)),
            TrialKind::Confirm => {
                Err("display trial generation does not match the pending trial".into())
            }
        },
        trial::Verdict::NoTrial => Err("no display trial is pending".into()),
        trial::Verdict::Idle => snapshot(data).map(|applied| (request.generation, applied)),
        trial::Verdict::NotYet => Err("the display trial deadline has not been reached".into()),
    }
}

/// The current semantic snapshot, as both interfaces return it.
///
/// A headless runtime never activates a configuration, so there is nothing to
/// compare against and the empty string is returned; the consumer reads a mismatch
/// as "not applied", which is the truth on that path.
fn snapshot(data: &CalloopData) -> Result<String, String> {
    match &data.active_configuration {
        Some(config) => config.snapshot().map_err(|e| e.to_string()),
        None => Ok(String::new()),
    }
}

fn validate(
    data: &CalloopData,
    config: &InoxConfig,
    outputs_are_provisional: bool,
) -> Result<(), String> {
    if let Some(old) = &data.active_configuration {
        if old.compositor.outputs != config.compositor.outputs
            && (!outputs_are_provisional || data.physical.is_none())
        {
            // Headless outputs belong to `--output`: there is no device state a
            // trial could apply or roll back.
            return Err(if data.physical.is_none() {
                HEADLESS_PRESERVES_STARTUP
            } else {
                OUTPUTS_REQUIRE_TRIAL
            }
            .into());
        }
        if old.compositor.input.input_method != config.compositor.input.input_method {
            return Err("input.input_method: live engine replacement is not available".into());
        }
        if old.root.environment != config.root.environment {
            return Err("environment: running process environments cannot be replaced".into());
        }
    } else if !config.compositor.outputs.is_empty()
        || !config.compositor.input.input_method.is_empty()
        || !config.root.environment.is_empty()
    {
        return Err(HEADLESS_PRESERVES_STARTUP.into());
    }

    Ok(())
}

/// Apply the non-display half of a candidate, with the existing keyboard rollback.
fn apply_body(data: &mut CalloopData, config: &InoxConfig) -> Result<(), String> {
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

    Ok(())
}

fn apply(data: &mut CalloopData, config: &InoxConfig) -> Result<(), String> {
    validate(data, config, false)?;
    apply_body(data, config)?;
    data.active_configuration = Some(config.clone());

    Ok(())
}

/// Apply a candidate whose output preferences are provisional (ADR 0028).
///
/// Returns the applied snapshot. The trial is only armed once everything
/// succeeded; any failure — an output that could not be driven, an invalid
/// keyboard configuration, a deadline source that cannot be registered — restores
/// the preferences that were in force first, so the caller never sees a
/// half-applied display change.
fn apply_trial(
    handle: &Handle,
    data: &mut CalloopData,
    config: &InoxConfig,
    generation: u64,
) -> Result<String, String> {
    if data.trial.is_armed() {
        return Err("a display trial is already pending".into());
    }
    validate(data, config, true)?;

    let mut physical = data.physical.take();
    let previous = match physical.as_ref() {
        Some(physical) => physical.preferences().to_vec(),
        None => Vec::new(),
    };
    let applied = match physical.as_mut() {
        Some(physical) => {
            physical::apply_output_preferences(handle, data, physical, &config.compositor.outputs)
        }
        None => Ok(()),
    };
    if let Err(error) = applied {
        let restored = match physical.as_mut() {
            Some(physical) => physical::apply_output_preferences(handle, data, physical, &previous),
            None => Ok(()),
        };
        data.physical = physical;

        return Err(restore_failure(error, restored));
    }
    // Input reconfiguration goes through the shared state, so the device state has
    // to be back in place before the rest of the candidate is applied.
    data.physical = physical;

    if let Err(error) = apply_body(data, config) {
        return Err(restore_failure(
            error,
            restore_preferences(handle, data, &previous),
        ));
    }

    let deadline = Instant::now() + TRIAL_DEADLINE;
    let timer = match arm_expiry(handle, deadline) {
        Ok(timer) => timer,
        Err(error) => {
            return Err(restore_failure(
                error,
                restore_preferences(handle, data, &previous),
            ));
        }
    };
    data.trial
        .arm(ArmedTrial::new(generation, previous, deadline), timer);
    data.active_configuration = Some(config.clone());
    info!(
        generation,
        "display trial applied; waiting for confirmation"
    );

    config.snapshot().map_err(|e| e.to_string())
}

/// Register the one-shot deadline that reverts an armed trial.
fn arm_expiry(handle: &Handle, deadline: Instant) -> Result<calloop::RegistrationToken, String> {
    let callback = handle.clone();

    handle
        .insert_source(
            calloop::timer::Timer::from_deadline(deadline),
            move |_, _, data: &mut CalloopData| {
                if let Some(armed) = data.trial.timed_out() {
                    info!(
                        generation = armed.generation(),
                        "display trial expired; restoring the previous outputs"
                    );
                    if let Err(error) = revert_outputs(&callback, data, armed.previous()) {
                        warn!(%error, "failed to restore the previous outputs after a display trial expired");
                    }
                }

                calloop::timer::TimeoutAction::Drop
            },
        )
        .map_err(|error| error.to_string())
}

/// The display half of a rollback: put the previous output preferences back and
/// make the active configuration describe them again.
///
/// Theme, language, input devices and layout keep the values the trial applied:
/// only display preferences are provisional (ADR 0028), and nothing is written to
/// disk here — the settings application owns persistence.
fn revert_outputs(
    handle: &Handle,
    data: &mut CalloopData,
    previous: &[OutputConfig],
) -> Result<(), String> {
    restore_preferences(handle, data, previous)?;
    if let Some(config) = data.active_configuration.as_mut() {
        config.compositor.outputs = previous.to_vec();
    }

    Ok(())
}

fn restore_preferences(
    handle: &Handle,
    data: &mut CalloopData,
    preferences: &[OutputConfig],
) -> Result<(), String> {
    let Some(mut physical) = data.physical.take() else {
        return Ok(());
    };
    let restored = physical::apply_output_preferences(handle, data, &mut physical, preferences);
    data.physical = Some(physical);

    restored
}

fn restore_failure(error: String, restored: Result<(), String>) -> String {
    match restored {
        Ok(()) => error,
        Err(restore) => format!("{error}; restoring the previous outputs failed: {restore}"),
    }
}
