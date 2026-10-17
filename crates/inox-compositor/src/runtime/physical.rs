//! Device event sources and per-output page-flip scheduling for the physical backend.
//!
//! Every enumerated GPU that can drive a display gets its own [`GpuBackend`], and
//! every output it drives gets its own frame state and its own pair of timers
//! (ADR 0019). One display failing to present, being unplugged or being paused
//! therefore never stops the others from making progress, which is what O04's
//! "each output keeps its own progress" asks for.
//!
//! The calloop integration stays here: [`GpuBackend`] owns device state, this
//! module owns when it is driven.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use calloop::{
    LoopHandle, RegistrationToken,
    timer::{TimeoutAction, Timer},
};
use inox_config::{Appearance, ConfigPaths, InoxConfig, InputConfig, OutputConfig, Theme};
use smithay::backend::{
    drm::{DrmDeviceNotifier, DrmEvent},
    libinput::LibinputInputBackend,
    session::{
        Event as SessionEvent, Session,
        libseat::{LibSeatSession, LibSeatSessionNotifier},
    },
    udev::{UdevBackend, UdevEvent, primary_gpu},
};
use smithay::reexports::rustix::fs::Dev;
use tracing::{debug, error, info, warn};

use super::{CalloopData, RunError};
use crate::backend::drm::SessionDevice;
use crate::backend::{
    GpuBackend,
    frame::{FrameSchedule, FrameToken, Phase},
    input::InputDevices,
};
use crate::state::OutputDescriptor;

type Handle = LoopHandle<'static, CalloopData>;

/// How long a page flip may stay outstanding before its output is considered dead.
const PAGE_FLIP_TIMEOUT: Duration = Duration::from_secs(2);

/// One GPU this session holds open.
struct GpuState {
    backend: GpuBackend,
    device_id: Dev,
    /// The DRM notifier registration, kept so it can be removed with the device.
    notifier: RegistrationToken,
}

/// The frame state of one output.
///
/// Each output has its own schedule, its own paint deadline and its own watchdog,
/// so a stall on one display is invisible to the others.
#[derive(Debug)]
struct OutputFrames {
    schedule: FrameSchedule,
    paint: Option<RegistrationToken>,
    watchdog: Option<RegistrationToken>,
    next_frame_at: Instant,
    interval: Duration,
}

pub(super) struct PhysicalState {
    gpus: Vec<GpuState>,
    session: LibSeatSession,
    input: InputDevices,
    preferences: Vec<OutputConfig>,
    /// Every DRM device udev has enumerated, including the ones this session does
    /// not drive.
    devices: BTreeMap<Dev, PathBuf>,
    frames: HashMap<String, OutputFrames>,
    active: bool,
    ready: bool,
    sources: Vec<RegistrationToken>,
}

pub(super) struct Prepared {
    pub(super) configuration: InoxConfig,
    pub(super) outputs: Vec<OutputDescriptor>,
    pub(super) input: InputConfig,
    pub(super) theme: Theme,
    pub(super) layout: crate::layout::LayoutConfig,
    state: PhysicalState,
    session_source: LibSeatSessionNotifier,
    udev_source: UdevBackend,
    input_source: LibinputInputBackend,
    /// The devices to register a DRM notifier for, with their udev identity.
    pending: Vec<(GpuBackend, DrmDeviceNotifier, Dev)>,
}

fn failure(error: impl std::fmt::Display) -> RunError {
    RunError::BackendUnavailable {
        reason: error.to_string(),
    }
}

pub(super) fn prepare() -> Result<Prepared, RunError> {
    let config = InoxConfig::load(
        &ConfigPaths::discover().map_err(failure)?,
        Appearance::System,
    )
    .map_err(failure)?;
    let (mut session, session_source) = LibSeatSession::new().map_err(failure)?;
    if !session.is_active() {
        return Err(failure("libseat session is inactive"));
    }
    let udev_source = UdevBackend::new(session.seat()).map_err(failure)?;
    let devices: BTreeMap<_, _> = udev_source
        .device_list()
        .map(|(id, path)| (id, path.to_owned()))
        .collect();

    let opened = open_gpus(&mut session, &devices, &config.compositor.outputs)?;
    let outputs: Vec<OutputDescriptor> = opened
        .iter()
        .flat_map(|(backend, _, _)| backend.descriptors())
        .collect();
    if outputs.is_empty() {
        return Err(failure("no usable physical output"));
    }

    let (input, input_source) =
        InputDevices::new(session.clone(), config.compositor.input.clone()).map_err(failure)?;

    Ok(Prepared {
        configuration: config.clone(),
        layout: crate::layout::LayoutConfig {
            master_ratio: config.compositor.layout.master_ratio,
            master_count: config.compositor.layout.master_count as usize,
            gap: config.compositor.layout.gap,
        },
        outputs,
        theme: config.theme,
        input: config.compositor.input,
        state: PhysicalState {
            gpus: Vec::new(),
            session,
            input,
            preferences: config.compositor.outputs,
            devices,
            frames: HashMap::new(),
            active: true,
            ready: false,
            sources: Vec::new(),
        },
        session_source,
        udev_source,
        input_source,
        pending: opened,
    })
}

/// Open every enumerated GPU that can drive at least one connected connector.
///
/// The primary GPU is tried first so the display the firmware brought up keeps
/// driving it; a device that yields no usable output is reported and skipped
/// rather than being fatal, because another one may still work (ADR 0019).
fn open_gpus(
    session: &mut LibSeatSession,
    devices: &BTreeMap<Dev, PathBuf>,
    preferences: &[OutputConfig],
) -> Result<Vec<(GpuBackend, DrmDeviceNotifier, Dev)>, RunError> {
    let primary = primary_gpu(session.seat()).map_err(failure)?;
    let mut paths: Vec<_> = devices.iter().map(|(id, path)| (*id, path)).collect();
    paths.sort_by_key(|(_, path)| *path);
    paths.sort_by_key(|(_, path)| Some(path.as_path()) != primary.as_deref());

    let mut opened = Vec::new();
    let mut reasons = Vec::new();
    for (device_id, path) in paths {
        match GpuBackend::open(session, path) {
            Ok((mut backend, notifier)) => match backend.rescan(preferences) {
                Ok(_) if backend.is_driving() => opened.push((backend, notifier, device_id)),
                Ok(_) => reasons.push(format!("{}: no connected usable output", path.display())),
                Err(error) => reasons.push(format!("{}: {error}", path.display())),
            },
            Err(error) => reasons.push(format!("{}: {error}", path.display())),
        }
    }

    if opened.is_empty() {
        if reasons.is_empty() {
            reasons.push("no DRM cards enumerated on this seat".to_owned());
        }

        return Err(failure(reasons.join("; ")));
    }
    for reason in &reasons {
        warn!(%reason, "GPU skipped");
    }

    Ok(opened)
}

pub(super) fn register(
    handle: &Handle,
    data: &mut CalloopData,
    prepared: Prepared,
) -> Result<(), RunError> {
    let Prepared {
        mut state,
        session_source,
        udev_source,
        input_source,
        pending,
        ..
    } = prepared;
    for (backend, notifier, device_id) in pending {
        let (backend, token) = register_drm(handle, backend, notifier, device_id)?;
        state.gpus.push(GpuState {
            backend,
            device_id,
            notifier: token,
        });
    }
    refresh_topology(handle, data, &mut state);

    let callback_handle = handle.clone();
    let input_token = handle
        .insert_source(input_source, move |event, _, data: &mut CalloopData| {
            if let Some(physical) = &mut data.physical
                && let Err(error) =
                    physical
                        .input
                        .event(event, &mut data.state, &mut physical.session)
            {
                error!(%error, "input-device update failed");
                data.fatal = Some(failure(error));
                data.state.request_exit();
            }
        })
        .map_err(failure)?;
    let udev_token = handle
        .insert_source(udev_source, move |event, _, data: &mut CalloopData| {
            let Some(mut physical) = data.physical.take() else {
                return;
            };
            match event {
                UdevEvent::Added { device_id, path } => {
                    physical.devices.insert(device_id, path);
                    if physical.active && !physical.drives_any_output() {
                        recover_gpus(&callback_handle, data, &mut physical);
                    }
                }
                UdevEvent::Removed { device_id } => {
                    physical.devices.remove(&device_id);
                    if physical.gpus.iter().any(|gpu| gpu.device_id == device_id) {
                        lose_gpu(&callback_handle, data, &mut physical, device_id);
                        if physical.active {
                            recover_gpus(&callback_handle, data, &mut physical);
                        }
                    }
                }
                UdevEvent::Changed { device_id } => {
                    if physical.active {
                        if physical.gpus.iter().any(|gpu| gpu.device_id == device_id) {
                            rescan(&callback_handle, data, &mut physical);
                        } else {
                            recover_gpus(&callback_handle, data, &mut physical);
                        }
                    }
                }
            }
            data.physical = Some(physical);
        })
        .map_err(failure)?;
    let callback_handle = handle.clone();
    let session_token = handle
        .insert_source(session_source, move |event, _, data: &mut CalloopData| {
            let Some(mut physical) = data.physical.take() else {
                return;
            };
            match event {
                SessionEvent::PauseSession => {
                    physical.active = false;
                    physical.pause_frames(&callback_handle);
                    for gpu in &mut physical.gpus {
                        gpu.backend.pause();
                    }
                    if let Err(error) = physical.input.pause(&mut data.state) {
                        warn!(%error, "input pause failed");
                    }
                    info!("session paused; input and DRM submissions stopped");
                }
                SessionEvent::ActivateSession => {
                    physical.active = true;
                    // Failures are collected first: removing a device needs the
                    // whole state, not just the device being iterated.
                    let mut failed = Vec::new();
                    for gpu in &mut physical.gpus {
                        if let Err(error) = gpu.backend.resume() {
                            warn!(%error, card = %gpu.backend.card().display(), "DRM resume failed");
                            failed.push(gpu.device_id);
                        }
                    }
                    for device_id in failed {
                        lose_gpu(&callback_handle, data, &mut physical, device_id);
                    }
                    let had_outputs = physical.drives_any_output();
                    if !had_outputs {
                        recover_gpus(&callback_handle, data, &mut physical);
                    } else {
                        refresh_topology(&callback_handle, data, &mut physical);
                        physical.resume_frames();
                        if data.state.output_active()
                            && let Err(error) = physical.input.resume()
                        {
                            fail_session(&callback_handle, data, &mut physical, error);
                        }
                    }
                }
            }
            data.physical = Some(physical);
        })
        .map_err(failure)?;
    state.sources = vec![input_token, udev_token, session_token];
    data.physical = Some(state);
    if let Some(physical) = data.physical.as_mut() {
        physical.arm_frames(handle);
    }
    Ok(())
}

/// Register one device's DRM notifier.
///
/// `device_id` and the libseat lease are what the callback uses to find the
/// device again: the backend itself lives in the shared state, which the event
/// loop takes out for the duration of a callback anyway.
fn register_drm(
    handle: &Handle,
    backend: GpuBackend,
    source: DrmDeviceNotifier,
    device_id: Dev,
) -> Result<(GpuBackend, RegistrationToken), RunError> {
    let lease = backend.lease();
    let callback_handle = handle.clone();
    let token = handle
        .insert_source(source, move |event, metadata, data: &mut CalloopData| {
            on_drm_event(&callback_handle, data, device_id, &lease, event, *metadata);
        })
        .map_err(failure)?;

    Ok((backend, token))
}

/// Handle one DRM event for the device identified by `device_id`.
fn on_drm_event(
    handle: &Handle,
    data: &mut CalloopData,
    device_id: Dev,
    lease: &Rc<SessionDevice>,
    event: DrmEvent,
    metadata: Option<smithay::backend::drm::DrmEventMetadata>,
) {
    let Some(mut physical) = data.physical.take() else {
        return;
    };
    let index = physical
        .gpus
        .iter()
        .position(|gpu| gpu.device_id == device_id);
    let ours = index.is_some_and(|index| {
        physical.active && Rc::ptr_eq(&physical.gpus[index].backend.lease(), lease)
    });
    if !ours {
        data.physical = Some(physical);

        return;
    }
    let index = index.unwrap_or_default();

    match event {
        DrmEvent::VBlank(crtc) => match physical.gpus[index].backend.completed(crtc, metadata) {
            Ok(Some((key, token))) => {
                // Only a completion that matched the submission in flight counts
                // as a presented frame; a stale one must not advance the count or
                // wake clients that are waiting for their frame callback.
                if physical.complete(handle, &key, token) {
                    data.state.frame_presented();
                }
                if !physical.ready {
                    physical.ready = true;
                    info!("inox-compositor ready: first physical frame presented");
                }
                if data.state.frame_budget_reached() {
                    data.state.request_exit();
                }
            }
            Ok(None) => {}
            Err(error) => fail_device(handle, data, &mut physical, device_id, error),
        },
        DrmEvent::Error(smithay::backend::drm::DrmError::Access(ref error))
            if error.source.kind() == std::io::ErrorKind::WouldBlock => {}
        DrmEvent::Error(error) => fail_device(handle, data, &mut physical, device_id, error),
    }

    if let Err(error) = data.display_handle.flush_clients() {
        warn!(%error, "failed to flush presentation events");
    }
    physical.arm_frames(handle);
    data.physical = Some(physical);
}

impl PhysicalState {
    fn drives_any_output(&self) -> bool {
        self.gpus.iter().any(|gpu| gpu.backend.is_driving())
    }

    /// How many outputs the devices drive between them.
    fn total_outputs(&self) -> usize {
        self.gpus
            .iter()
            .map(|gpu| gpu.backend.descriptors().len())
            .sum()
    }

    /// Record a completion, disarm its watchdog and re-arm the schedule.
    fn complete(&mut self, handle: &Handle, key: &str, token: FrameToken) -> bool {
        let Some(frames) = self.frames.get_mut(key) else {
            return false;
        };
        if !frames.schedule.complete(token) {
            return false;
        }
        if let Some(timer) = frames.watchdog.take() {
            handle.remove(timer);
        }

        true
    }

    /// Drop every frame timer and put every output's schedule back to idle.
    fn pause_frames(&mut self, handle: &Handle) {
        for frames in self.frames.values_mut() {
            if let Some(token) = frames.paint.take() {
                handle.remove(token);
            }
            if let Some(token) = frames.watchdog.take() {
                handle.remove(token);
            }
            frames.schedule.pause();
        }
    }

    fn resume_frames(&mut self) {
        for frames in self.frames.values_mut() {
            frames.schedule.resume();
            frames.next_frame_at = Instant::now();
        }
    }

    /// Make the per-output frame bookkeeping match the outputs in `keys`.
    fn sync_frames(&mut self, handle: &Handle, keys: &[(String, Duration)]) {
        let wanted: Vec<&str> = keys.iter().map(|(key, _)| key.as_str()).collect();
        let stale: Vec<String> = self
            .frames
            .keys()
            .filter(|key| !wanted.contains(&key.as_str()))
            .cloned()
            .collect();
        for key in stale {
            self.remove_frames(handle, &key);
        }
        for (key, interval) in keys {
            self.frames
                .entry(key.clone())
                .and_modify(|frames| frames.interval = *interval)
                .or_insert_with(|| OutputFrames {
                    schedule: FrameSchedule::default(),
                    paint: None,
                    watchdog: None,
                    next_frame_at: Instant::now(),
                    interval: *interval,
                });
        }
    }

    fn remove_frames(&mut self, handle: &Handle, key: &str) {
        if let Some(frames) = self.frames.remove(key) {
            if let Some(token) = frames.paint {
                handle.remove(token);
            }
            if let Some(token) = frames.watchdog {
                handle.remove(token);
            }
        }
    }

    /// Arm one paint timer per output that has a frame due.
    fn arm_frames(&mut self, handle: &Handle) {
        if !self.active {
            return;
        }
        let due: Vec<String> = self
            .frames
            .iter()
            .filter(|(_, frames)| {
                frames.schedule.phase() == Phase::Scheduled && frames.paint.is_none()
            })
            .map(|(key, _)| key.clone())
            .collect();

        for key in due {
            let Some(frames) = self.frames.get(&key) else {
                continue;
            };
            let delay = frames
                .next_frame_at
                .saturating_duration_since(Instant::now());
            let callback = handle.clone();
            let armed = key.clone();
            match handle.insert_source(
                Timer::from_duration(delay),
                move |_, _, data: &mut CalloopData| {
                    paint(&callback, data, &armed);
                    TimeoutAction::Drop
                },
            ) {
                Ok(token) => {
                    if let Some(frames) = self.frames.get_mut(&key) {
                        frames.paint = Some(token);
                    }
                }
                Err(error) => warn!(%error, %key, "failed to arm a paint timer"),
            }
        }
    }

    fn damage_all(&mut self) {
        for frames in self.frames.values_mut() {
            frames.schedule.damage();
        }
    }

    /// The output keys and their refresh intervals, in advertisement order.
    fn output_intervals(&self) -> Vec<(String, Duration)> {
        self.gpus
            .iter()
            .flat_map(|gpu| {
                gpu.backend
                    .descriptors()
                    .into_iter()
                    .map(|descriptor| descriptor.key)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .map(move |key| {
                        let interval = gpu.backend.interval(&key);
                        (key, interval)
                    })
            })
            .collect()
    }
}

/// Draw and submit one output's next frame.
fn paint(handle: &Handle, data: &mut CalloopData, key: &str) {
    let Some(mut physical) = data.physical.take() else {
        return;
    };
    let Some(frames) = physical.frames.get_mut(key) else {
        data.physical = Some(physical);

        return;
    };
    frames.paint = None;
    let Some(token) = frames.schedule.begin() else {
        data.physical = Some(physical);

        return;
    };
    let interval = frames.interval;
    frames.next_frame_at = Instant::now() + interval;

    let Some(index) = physical.gpus.iter().position(|gpu| gpu.backend.drives(key)) else {
        if let Some(frames) = physical.frames.get_mut(key) {
            frames.schedule.complete(token);
        }
        data.physical = Some(physical);

        return;
    };

    match physical.gpus[index]
        .backend
        .render(&mut data.state, key, token)
    {
        Ok(true) => arm_watchdog(handle, &mut physical, key, token),
        Ok(false) => {
            if let Some(frames) = physical.frames.get_mut(key) {
                frames.schedule.complete(token);
            }
            data.state.frame_callbacks();
        }
        Err(error) => {
            let retryable = error.retryable();
            let retrying = physical
                .frames
                .get_mut(key)
                .is_some_and(|frames| frames.schedule.failed(token, retryable));
            if retrying {
                warn!(%error, %key, "temporary scanout failure; retrying within budget");
                physical.gpus[index].backend.retry(key);
            } else {
                fail_output(handle, data, &mut physical, key, error);
            }
        }
    }

    if let Err(error) = data.display_handle.flush_clients() {
        warn!(%error, "failed to flush frame events");
    }
    physical.arm_frames(handle);
    data.physical = Some(physical);
}

fn arm_watchdog(handle: &Handle, physical: &mut PhysicalState, key: &str, token: FrameToken) {
    let callback = handle.clone();
    let watched = key.to_owned();
    let armed = handle.insert_source(
        Timer::from_duration(PAGE_FLIP_TIMEOUT),
        move |_, _, data: &mut CalloopData| {
            let Some(mut physical) = data.physical.take() else {
                return TimeoutAction::Drop;
            };
            let stalled = physical
                .frames
                .get(&watched)
                .is_some_and(|frames| frames.schedule.phase() == Phase::Submitted(token));
            if let Some(frames) = physical.frames.get_mut(&watched) {
                frames.watchdog = None;
            }
            if stalled {
                fail_output(
                    &callback,
                    data,
                    &mut physical,
                    &watched,
                    "page-flip completion timed out",
                );
            }
            data.physical = Some(physical);

            TimeoutAction::Drop
        },
    );

    match armed {
        Ok(token) => {
            if let Some(frames) = physical.frames.get_mut(key) {
                frames.watchdog = Some(token);
            }
        }
        Err(error) => warn!(%error, %key, "failed to arm the page-flip watchdog"),
    }
}

/// Bring the compositor's topology in line with what the devices drive now.
fn refresh_topology(handle: &Handle, data: &mut CalloopData, physical: &mut PhysicalState) {
    let outputs: Vec<OutputDescriptor> = physical
        .gpus
        .iter()
        .flat_map(|gpu| gpu.backend.descriptors())
        .collect();

    if let Err(error) = data.state.apply_outputs(&outputs) {
        warn!(%error, "failed to apply the output topology");
    }

    physical.sync_frames(handle, &physical.output_intervals());

    // A paused session owns no devices: the topology is updated so clients see the
    // right thing when it resumes, but nothing may be submitted or dispatched in
    // the meantime.
    if !physical.active {
        for frames in physical.frames.values_mut() {
            frames.schedule.pause();
        }

        return;
    }

    if outputs.is_empty() {
        if let Err(error) = physical.input.pause(&mut data.state) {
            warn!(%error, "input pause after output removal failed");
        }
        for frames in physical.frames.values_mut() {
            frames.schedule.remove();
        }

        return;
    }

    // Damage rather than resume: `resume` bumps the schedule's epoch, which would
    // orphan the completion of a flip that is already in flight on another output.
    // `damage` schedules the next frame without invalidating anything pending.
    physical.damage_all();
    if let Err(error) = physical.input.resume() {
        warn!(%error, "input resume after output change failed");
    }
}

/// Stop driving one output, keeping every other display and the session alive.
///
/// The exception is the first frame: a compositor that was asked for a physical
/// output and never managed to present one has failed, and must say so rather
/// than sit with no display and no error (ADR 0012).
fn fail_output(
    handle: &Handle,
    data: &mut CalloopData,
    physical: &mut PhysicalState,
    key: &str,
    reason: impl std::fmt::Display,
) {
    error!(%reason, %key, "output failed");
    if !physical.ready && physical.total_outputs() <= 1 {
        data.fatal = Some(failure(&reason));
        data.state.request_exit();

        return;
    }

    drop_output(handle, data, physical, key);
}

/// Stop driving one output, keeping every other display and the session alive.
fn drop_output(handle: &Handle, data: &mut CalloopData, physical: &mut PhysicalState, key: &str) {
    physical.remove_frames(handle, key);
    for gpu in &mut physical.gpus {
        gpu.backend.drop_scanout(key);
    }
    refresh_topology(handle, data, physical);
}

/// A whole device went away or failed.
fn lose_gpu(handle: &Handle, data: &mut CalloopData, physical: &mut PhysicalState, device_id: Dev) {
    let Some(index) = physical
        .gpus
        .iter()
        .position(|gpu| gpu.device_id == device_id)
    else {
        return;
    };
    let gpu = physical.gpus.remove(index);
    handle.remove(gpu.notifier);
    let keys = gpu
        .backend
        .descriptors()
        .into_iter()
        .map(|descriptor| descriptor.key)
        .collect::<Vec<_>>();
    for key in keys {
        physical.remove_frames(handle, &key);
    }
    drop(gpu);

    refresh_topology(handle, data, physical);
}

/// Try to open a usable GPU again after one was lost.
fn recover_gpus(handle: &Handle, data: &mut CalloopData, physical: &mut PhysicalState) {
    let currently = physical
        .gpus
        .iter()
        .map(|gpu| gpu.device_id)
        .collect::<Vec<_>>();
    let candidates: Vec<(Dev, PathBuf)> = physical
        .devices
        .iter()
        .filter(|(id, _)| !currently.contains(id))
        .map(|(id, path)| (*id, path.clone()))
        .collect();

    for (device_id, path) in candidates {
        let opened =
            GpuBackend::open(&mut physical.session, &path).and_then(|(mut backend, source)| {
                backend.rescan(&physical.preferences)?;

                Ok((backend, source))
            });
        let (backend, source) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                debug!(%error, path = %path.display(), "GPU still unusable");
                continue;
            }
        };
        if !backend.is_driving() {
            continue;
        }
        match register_drm(handle, backend, source, device_id) {
            Ok((backend, notifier)) => physical.gpus.push(GpuState {
                backend,
                device_id,
                notifier,
            }),
            Err(error) => {
                warn!(%error, "failed to register a recovered GPU");
                continue;
            }
        }
    }

    refresh_topology(handle, data, physical);
    if physical.drives_any_output() {
        physical.ready = true;
    }
}

fn rescan(handle: &Handle, data: &mut CalloopData, physical: &mut PhysicalState) {
    let mut changed = false;
    for gpu in &mut physical.gpus {
        match gpu.backend.rescan(&physical.preferences) {
            Ok(updated) => changed |= updated,
            Err(error) => {
                warn!(%error, card = %gpu.backend.card().display(), "rescan failed");
            }
        }
    }

    if changed {
        refresh_topology(handle, data, physical);
    }
}

/// One device failed: only that device is taken down.
///
/// Before the first frame has been presented a failure is fatal, because the
/// compositor was asked to produce a physical output and could not. Afterwards
/// the session survives with whatever other displays are left, and the device is
/// picked up again by the next udev event (ADR 0012).
fn fail_device(
    handle: &Handle,
    data: &mut CalloopData,
    physical: &mut PhysicalState,
    device_id: Dev,
    reason: impl std::fmt::Display,
) {
    error!(%reason, "DRM device failed");
    if !physical.ready {
        data.fatal = Some(failure(&reason));
        data.state.request_exit();

        return;
    }

    lose_gpu(handle, data, physical, device_id);
}

/// A failure with no single device to blame: every output is taken down.
fn fail_session(
    handle: &Handle,
    data: &mut CalloopData,
    physical: &mut PhysicalState,
    reason: impl std::fmt::Display,
) {
    error!(%reason, "physical session failed");
    if !physical.ready {
        data.fatal = Some(failure(&reason));
        data.state.request_exit();

        return;
    }

    let device_ids: Vec<Dev> = physical.gpus.iter().map(|gpu| gpu.device_id).collect();
    for device_id in device_ids {
        lose_gpu(handle, data, physical, device_id);
    }
}

pub(super) fn schedule(handle: &Handle, data: &mut CalloopData, repaint: bool) {
    if let Some(physical) = data.physical.as_mut() {
        if repaint || data.state.frame_budget() {
            physical.damage_all();
        }
        physical.arm_frames(handle);
    }
}

pub(super) fn shutdown(handle: &Handle, data: &mut CalloopData) {
    if let Some(mut physical) = data.physical.take() {
        let keys: Vec<String> = physical.frames.keys().cloned().collect();
        for key in keys {
            physical.remove_frames(handle, &key);
        }
        if let Err(error) = physical.input.pause(&mut data.state) {
            warn!(%error, "input shutdown failed");
        }
        for gpu in &mut physical.gpus {
            handle.remove(gpu.notifier);
        }
        physical.gpus.clear();
        // Input/udev sources go first; the session notifier owns libseat and goes last.
        let session_token = physical.sources.pop();
        for token in physical.sources.drain(..) {
            handle.remove(token);
        }
        if let Some(token) = session_token {
            handle.remove(token);
        }
    }
}

pub(super) fn apply_input(
    data: &mut CalloopData,
    config: inox_config::InputConfig,
) -> Result<(), String> {
    match data.physical.as_mut() {
        Some(physical) => physical.input.reconfigure(config),
        None => Ok(()),
    }
}
