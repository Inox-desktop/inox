//! Device event sources and bounded page-flip scheduling for the physical backend.

use std::{
    collections::BTreeMap,
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
use smithay::{output::Output, reexports::rustix::fs::Dev};
use tracing::{debug, error, info, warn};

use super::{CalloopData, RunError};
use crate::backend::{
    DrmBackend,
    frame::{FrameToken, Phase},
    input::InputDevices,
};

type Handle = LoopHandle<'static, CalloopData>;

pub(super) struct PhysicalState {
    backend: Option<DrmBackend>,
    session: LibSeatSession,
    input: InputDevices,
    preferences: Vec<OutputConfig>,
    gpus: BTreeMap<Dev, PathBuf>,
    epoch: u64,
    active: bool,
    ready: bool,
    drm_source: Option<RegistrationToken>,
    sources: Vec<RegistrationToken>,
}

pub(super) struct Prepared {
    pub(super) output: Output,
    pub(super) theme: Theme,
    pub(super) input: InputConfig,
    pub(super) layout: crate::layout::LayoutConfig,
    state: PhysicalState,
    session_source: LibSeatSessionNotifier,
    udev_source: UdevBackend,
    drm_source: DrmDeviceNotifier,
    input_source: LibinputInputBackend,
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
    let gpus: BTreeMap<_, _> = udev_source
        .device_list()
        .map(|(id, path)| (id, path.to_owned()))
        .collect();
    let (backend, drm_source) = open_gpu(&mut session, &gpus, &config.compositor.outputs)?;
    let output = backend
        .output()
        .ok_or_else(|| failure("no usable physical output"))?;
    let (input, input_source) =
        InputDevices::new(session.clone(), config.compositor.input.clone()).map_err(failure)?;
    Ok(Prepared {
        layout: crate::layout::LayoutConfig {
            master_ratio: config.compositor.layout.master_ratio,
            master_count: config.compositor.layout.master_count as usize,
            gap: config.compositor.layout.gap,
        },
        output,
        theme: config.theme,
        input: config.compositor.input,
        state: PhysicalState {
            backend: Some(backend),
            session,
            input,
            preferences: config.compositor.outputs,
            gpus,
            epoch: 0,
            active: true,
            ready: false,
            drm_source: None,
            sources: Vec::new(),
        },
        session_source,
        udev_source,
        drm_source,
        input_source,
    })
}

fn open_gpu(
    session: &mut LibSeatSession,
    gpus: &BTreeMap<Dev, PathBuf>,
    preferences: &[OutputConfig],
) -> Result<(DrmBackend, DrmDeviceNotifier), RunError> {
    let primary = primary_gpu(session.seat()).map_err(failure)?;
    let mut paths: Vec<_> = gpus.values().collect();
    paths.sort();
    paths.sort_by_key(|path| Some(path.as_path()) != primary.as_deref());
    let mut reasons = Vec::new();
    for path in paths {
        match DrmBackend::open(session, path) {
            Ok((mut backend, source)) => match backend.rescan(preferences) {
                Ok(_) if backend.output().is_some() => return Ok((backend, source)),
                Ok(_) => reasons.push(format!("{}: no connected usable output", path.display())),
                Err(error) => reasons.push(format!("{}: {error}", path.display())),
            },
            Err(error) => reasons.push(format!("{}: {error}", path.display())),
        }
    }
    if reasons.is_empty() {
        reasons.push("no DRM cards enumerated on this seat".to_owned());
    }
    Err(failure(reasons.join("; ")))
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
        drm_source,
        input_source,
        ..
    } = prepared;
    state.drm_source = Some(register_drm(handle, &state, drm_source)?);
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
                    physical.gpus.insert(device_id, path);
                    if physical.backend.is_none() && physical.active {
                        recover_gpu(&callback_handle, data, &mut physical);
                    }
                }
                UdevEvent::Removed { device_id } => {
                    physical.gpus.remove(&device_id);
                    if physical
                        .backend
                        .as_ref()
                        .is_some_and(|gpu| gpu.device_id() == device_id)
                    {
                        lose_output(&callback_handle, data, &mut physical);
                        if physical.active {
                            recover_gpu(&callback_handle, data, &mut physical);
                        }
                    }
                }
                UdevEvent::Changed { device_id } => {
                    if physical.active {
                        if physical
                            .backend
                            .as_ref()
                            .is_some_and(|gpu| gpu.device_id() == device_id)
                        {
                            rescan(&callback_handle, data, &mut physical);
                        } else if physical.backend.is_none() {
                            recover_gpu(&callback_handle, data, &mut physical);
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
                    data.frames.pause();
                    cancel_timers(&callback_handle, data);
                    if let Some(gpu) = &mut physical.backend {
                        gpu.pause();
                    }
                    if let Err(error) = physical.input.pause(&mut data.state) {
                        warn!(%error, "input pause failed");
                    }
                    info!("session paused; input and DRM submissions stopped");
                }
                SessionEvent::ActivateSession => {
                    physical.active = true;
                    if let Some(gpu) = &mut physical.backend {
                        if let Err(error) = gpu.resume() {
                            warn!(%error, "DRM resume failed");
                            lose_output(&callback_handle, data, &mut physical);
                        } else {
                            data.frames.resume();
                            rescan(&callback_handle, data, &mut physical);
                            if data.state.output_active()
                                && let Err(error) = physical.input.resume()
                            {
                                fail_output(&callback_handle, data, &mut physical, error);
                            }
                        }
                    }
                    if physical.backend.is_none() {
                        recover_gpu(&callback_handle, data, &mut physical);
                    }
                }
            }
            data.physical = Some(physical);
        })
        .map_err(failure)?;
    state.sources = vec![input_token, udev_token, session_token];
    data.physical = Some(state);
    arm_frame(handle, data);
    Ok(())
}

fn register_drm(
    handle: &Handle,
    physical: &PhysicalState,
    source: DrmDeviceNotifier,
) -> Result<RegistrationToken, RunError> {
    let epoch = physical.epoch;
    let lease = physical
        .backend
        .as_ref()
        .ok_or_else(|| failure("missing DRM device"))?
        .lease();
    let callback_handle = handle.clone();
    handle
        .insert_source(source, move |event, metadata, data: &mut CalloopData| {
            let Some(mut physical) = data.physical.take() else {
                return;
            };
            if physical.epoch != epoch
                || !physical.active
                || !physical
                    .backend
                    .as_ref()
                    .is_some_and(|gpu| Rc::ptr_eq(&gpu.lease(), &lease))
            {
                data.physical = Some(physical);
                return;
            }
            match event {
                DrmEvent::VBlank(crtc) => {
                    let completed = physical
                        .backend
                        .as_mut()
                        .map(|gpu| gpu.completed(crtc, *metadata));
                    match completed {
                        Some(Ok(Some(token))) if data.frames.complete(token) => {
                            if let Some(timer) = data.watchdog.take() {
                                callback_handle.remove(timer);
                            }
                            data.state.frame_presented();
                            if !physical.ready {
                                physical.ready = true;
                                info!("inox-compositor ready: first physical frame presented");
                            }
                            if data.state.frame_budget_reached() {
                                data.state.request_exit();
                            }
                        }
                        Some(Err(error)) => {
                            fail_output(&callback_handle, data, &mut physical, error)
                        }
                        _ => {}
                    }
                }
                DrmEvent::Error(smithay::backend::drm::DrmError::Access(ref error))
                    if error.source.kind() == std::io::ErrorKind::WouldBlock => {}
                DrmEvent::Error(error) => fail_output(&callback_handle, data, &mut physical, error),
            }
            if let Err(error) = data.display_handle.flush_clients() {
                warn!(%error, "failed to flush presentation events");
            }
            data.physical = Some(physical);
        })
        .map_err(failure)
}

fn rescan(handle: &Handle, data: &mut CalloopData, physical: &mut PhysicalState) {
    let changed = match physical.backend.as_mut() {
        Some(gpu) => gpu.rescan(&physical.preferences),
        None => return,
    };
    match changed {
        Ok(true) => {
            cancel_timers(handle, data);
            data.frames.remove();
            let output = physical.backend.as_ref().and_then(DrmBackend::output);
            if let Err(error) = data.state.replace_output(output.clone()) {
                fail_output(handle, data, physical, error);
                return;
            }
            if output.is_some() {
                data.frames.resume();
                if let Err(error) = physical.input.resume() {
                    fail_output(handle, data, physical, error);
                }
            } else if let Err(error) = physical.input.pause(&mut data.state) {
                warn!(%error, "input pause after output removal failed");
            }
        }
        Ok(false) => {}
        Err(error) => fail_output(handle, data, physical, error),
    }
}

fn recover_gpu(handle: &Handle, data: &mut CalloopData, physical: &mut PhysicalState) {
    match open_gpu(&mut physical.session, &physical.gpus, &physical.preferences) {
        Ok((gpu, source)) => {
            let output = gpu.output();
            physical.epoch = physical.epoch.wrapping_add(1);
            physical.backend = Some(gpu);
            match register_drm(handle, physical, source) {
                Ok(token) => physical.drm_source = Some(token),
                Err(error) => {
                    fail_output(handle, data, physical, error);
                    return;
                }
            }
            if let Err(error) = data.state.replace_output(output) {
                fail_output(handle, data, physical, error);
                return;
            }
            data.frames.resume();
            if let Err(error) = physical.input.resume() {
                fail_output(handle, data, physical, error);
            }
        }
        Err(error) => debug!(%error, "waiting for a usable physical output"),
    }
}

fn lose_output(handle: &Handle, data: &mut CalloopData, physical: &mut PhysicalState) {
    cancel_timers(handle, data);
    data.frames.remove();
    physical.epoch = physical.epoch.wrapping_add(1);
    if let Some(token) = physical.drm_source.take() {
        handle.remove(token);
    }
    physical.backend = None;
    if let Err(error) = data.state.replace_output(None) {
        warn!(%error, "failed to remove logical output");
    }
    if let Err(error) = physical.input.pause(&mut data.state) {
        warn!(%error, "failed to pause input without an output");
    }
}

fn fail_output(
    handle: &Handle,
    data: &mut CalloopData,
    physical: &mut PhysicalState,
    reason: impl std::fmt::Display,
) {
    error!(%reason, "physical output failed");
    if !physical.ready {
        data.fatal = Some(failure(&reason));
        data.state.request_exit();
    }
    lose_output(handle, data, physical);
}

fn cancel_timers(handle: &Handle, data: &mut CalloopData) {
    if let Some(token) = data.paint_timer.take() {
        handle.remove(token);
    }
    if let Some(token) = data.watchdog.take() {
        handle.remove(token);
    }
}

pub(super) fn arm_frame(handle: &Handle, data: &mut CalloopData) {
    if data.frames.phase() != Phase::Scheduled
        || data.paint_timer.is_some()
        || data.state.exit_requested()
    {
        return;
    }
    if !data
        .physical
        .as_ref()
        .is_some_and(|p| p.active && p.backend.as_ref().is_some_and(|gpu| gpu.output().is_some()))
    {
        return;
    }
    let callback_handle = handle.clone();
    let delay = data.next_frame_at.saturating_duration_since(Instant::now());
    match handle.insert_source(
        Timer::from_duration(delay),
        move |_, _, data: &mut CalloopData| {
            data.paint_timer = None;
            let Some(token) = data.frames.begin() else {
                return TimeoutAction::Drop;
            };
            let Some(mut physical) = data.physical.take() else {
                return TimeoutAction::Drop;
            };
            if let Some(gpu) = &mut physical.backend {
                data.next_frame_at = Instant::now() + gpu.interval();
                match gpu.render(&mut data.state, token) {
                    Ok(true) => arm_watchdog(&callback_handle, data, token),
                    Ok(false) => {
                        data.frames.complete(token);
                        data.state.frame_callbacks();
                    }
                    Err(error) => {
                        if data.frames.failed(token, error.retryable()) {
                            warn!(%error, "temporary scanout failure; retrying within budget");
                            gpu.retry();
                        } else {
                            fail_output(&callback_handle, data, &mut physical, error);
                        }
                    }
                }
            }
            if let Err(error) = data.display_handle.flush_clients() {
                warn!(%error, "failed to flush frame events");
            }
            data.physical = Some(physical);
            TimeoutAction::Drop
        },
    ) {
        Ok(token) => data.paint_timer = Some(token),
        Err(error) => {
            data.fatal = Some(failure(error));
            data.state.request_exit();
        }
    }
}

fn arm_watchdog(handle: &Handle, data: &mut CalloopData, token: FrameToken) {
    let callback_handle = handle.clone();
    match handle.insert_source(
        Timer::from_duration(Duration::from_secs(2)),
        move |_, _, data: &mut CalloopData| {
            data.watchdog = None;
            if data.frames.phase() == Phase::Submitted(token)
                && let Some(mut physical) = data.physical.take()
            {
                fail_output(
                    &callback_handle,
                    data,
                    &mut physical,
                    "page-flip completion timed out",
                );
                data.physical = Some(physical);
            }
            TimeoutAction::Drop
        },
    ) {
        Ok(token) => data.watchdog = Some(token),
        Err(error) => {
            data.fatal = Some(failure(error));
            data.state.request_exit();
        }
    }
}

pub(super) fn shutdown(handle: &Handle, data: &mut CalloopData) {
    cancel_timers(handle, data);
    if let Some(mut physical) = data.physical.take() {
        if let Err(error) = physical.input.pause(&mut data.state) {
            warn!(%error, "input shutdown failed");
        }
        if let Some(token) = physical.drm_source.take() {
            handle.remove(token);
        }
        // Input/udev sources go first; the session notifier owns libseat and goes last.
        let session_token = physical.sources.pop();
        for token in physical.sources.drain(..) {
            handle.remove(token);
        }
        physical.backend = None;
        if let Some(token) = session_token {
            handle.remove(token);
        }
    }
}
