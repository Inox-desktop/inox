//! Smithay DRM/GBM scanout, with libseat-owned device access (ADR 0012).
//!
//! The first hardware implementation drives one connector on one enumerated GPU.
//! Ordinary clients survive loss of all outputs. Multi-output layout, fractional
//! scaling and PRIME are deliberately not advertised as supported here.

use std::{
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

use inox_config::OutputConfig;
use smithay::{
    backend::{
        SwapBuffersError,
        allocator::{
            Fourcc,
            dmabuf::Dmabuf,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmError, DrmEventMetadata as EventMetadata,
            DrmEventTime as DrmTime, GbmBufferedSurface,
        },
        egl::{EGLContext, EGLDisplay},
        renderer::{
            Bind,
            damage::OutputDamageTracker,
            gles::{GlesError, GlesRenderer},
        },
        session::{Session, libseat::LibSeatSession},
    },
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        drm::control::{self, Device as ControlDevice, connector, crtc},
        rustix::fs::OFlags,
    },
    utils::{Clock, Monotonic, Transform},
};
use tracing::{debug, info, warn};

use crate::{InoxState, backend::frame::FrameToken};

/// Errors that prevent a device from rendering or presenting.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// No usable device, connector, configuration or session is available.
    #[error("DRM backend unavailable: {reason}")]
    Unavailable {
        /// The failing boundary and its diagnostic.
        reason: String,
    },
    /// A device operation failed.
    #[error(transparent)]
    Drm(#[from] DrmError),
    /// A renderer operation failed.
    #[error(transparent)]
    Render(#[from] GlesError),
    /// Buffer allocation or presentation failed.
    #[error(transparent)]
    Swap(#[from] SwapBuffersError),
    /// An I/O operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl BackendError {
    pub(crate) fn retryable(&self) -> bool {
        matches!(self, Self::Swap(SwapBuffersError::TemporaryFailure(_)))
    }
}

fn unavailable(error: impl std::fmt::Display) -> BackendError {
    BackendError::Unavailable {
        reason: error.to_string(),
    }
}

pub(crate) struct SessionDevice {
    fd: Option<OwnedFd>,
    session: LibSeatSession,
}

impl Drop for SessionDevice {
    fn drop(&mut self) {
        if let Some(fd) = self.fd.take()
            && let Err(error) = self.session.close(fd)
        {
            warn!(%error, "failed to release DRM device through libseat");
        }
    }
}

struct Submission {
    token: FrameToken,
    monotonic: Duration,
    realtime: SystemTime,
}

impl Submission {
    fn accepts(&self, metadata: &EventMetadata) -> bool {
        match metadata.time {
            DrmTime::Monotonic(time) => time >= self.monotonic,
            DrmTime::Realtime(time) => time >= self.realtime,
        }
    }
}

struct Scanout {
    buffers: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, FrameToken>,
    output: Output,
    connector: connector::Handle,
    crtc: crtc::Handle,
    mode: control::Mode,
    damage: OutputDamageTracker,
    scene_damage: OutputDamageTracker,
    in_flight: Option<Submission>,
}

/// A single GPU and its current scanout target.
///
/// Drop order releases buffers, EGL/GBM, DRM master, and finally the libseat claim.
pub struct DrmBackend {
    scanout: Option<Scanout>,
    renderer: GlesRenderer,
    gbm: GbmDevice<DrmDeviceFd>,
    device: DrmDevice,
    card: PathBuf,
    claim: Rc<SessionDevice>,
}

impl DrmBackend {
    /// Open an enumerated GPU through libseat, returning its calloop event source.
    ///
    /// # Errors
    /// Returns a typed error if the device or EGL/GBM renderer is unavailable.
    pub fn open(
        session: &mut LibSeatSession,
        path: &Path,
    ) -> Result<(Self, DrmDeviceNotifier), BackendError> {
        let fd = session
            .open(path, OFlags::RDWR | OFlags::CLOEXEC)
            .map_err(unavailable)?;
        let claim = Rc::new(SessionDevice {
            fd: Some(fd),
            session: session.clone(),
        });
        let duplicate = match claim.fd.as_ref() {
            Some(fd) => fd.try_clone()?,
            None => return Err(unavailable("libseat returned no DRM descriptor")),
        };
        let flags = smithay::reexports::rustix::fs::fcntl_getfl(&duplicate)
            .map_err(std::io::Error::from)?;
        smithay::reexports::rustix::fs::fcntl_setfl(&duplicate, flags | OFlags::NONBLOCK)
            .map_err(std::io::Error::from)?;
        // The original descriptor stays owned by SessionDevice, so its libseat
        // device ID is closed exactly once after every graphics reference is gone.
        let fd = DrmDeviceFd::new(duplicate.into());
        let (device, notifier) = DrmDevice::new(fd.clone(), true)?;
        let gbm = GbmDevice::new(fd)?;
        let renderer = create_renderer(gbm.clone())?;
        info!(card = %path.display(), seat = %session.seat(), "opened DRM/GBM renderer");
        Ok((
            Self {
                scanout: None,
                renderer,
                gbm,
                device,
                card: path.to_owned(),
                claim,
            },
            notifier,
        ))
    }

    /// Path of the active GPU.
    pub fn card(&self) -> &Path {
        &self.card
    }

    /// Kernel device ID used to match udev events.
    pub fn device_id(&self) -> smithay::reexports::rustix::fs::Dev {
        self.device.device_id()
    }

    pub(crate) fn lease(&self) -> Rc<SessionDevice> {
        self.claim.clone()
    }

    pub(crate) fn output(&self) -> Option<Output> {
        self.scanout.as_ref().map(|s| s.output.clone())
    }

    pub(crate) fn interval(&self) -> Duration {
        let mhz = self
            .scanout
            .as_ref()
            .map(|s| Mode::from(s.mode).refresh)
            .unwrap_or(60_000)
            .max(1) as u64;
        Duration::from_nanos(1_000_000_000_000 / mhz)
    }

    pub(crate) fn rescan(&mut self, preferences: &[OutputConfig]) -> Result<bool, BackendError> {
        if !self.device.is_active() {
            return Ok(false);
        }
        let resources = self.device.resource_handles()?;
        if let Some(scanout) = &self.scanout {
            let connector = self.device.get_connector(scanout.connector, true)?;
            if connector.state() == connector::State::Connected
                && connector.modes().contains(&scanout.mode)
            {
                return Ok(false);
            }
        }
        self.scanout = None;
        let mut connectors = resources.connectors().to_vec();
        connectors.sort_by_key(|handle| u32::from(*handle));
        for handle in connectors {
            let connector = self.device.get_connector(handle, true)?;
            if connector.state() != connector::State::Connected || connector.modes().is_empty() {
                continue;
            }
            let name = connector.to_string();
            let preference = preferences
                .iter()
                .find(|p| p.name.as_deref() == Some(&name))
                .or_else(|| preferences.iter().find(|p| p.name.is_none()));
            if let Some(preference) = preference
                && preference.scale != 1.0
            {
                return Err(unavailable(format!(
                    "{name}: this backend currently requires output scale 1"
                )));
            }
            let mode = choose_mode(connector.modes(), preference)
                .ok_or_else(|| unavailable(format!("{name}: requested mode is not supported")))?;
            let mut crtcs = Vec::new();
            if let Some(encoder) = connector.current_encoder()
                && let Some(crtc) = self.device.get_encoder(encoder)?.crtc()
            {
                crtcs.push(crtc);
            }
            for encoder in connector.encoders() {
                for crtc in
                    resources.filter_crtcs(self.device.get_encoder(*encoder)?.possible_crtcs())
                {
                    if !crtcs.contains(&crtc) {
                        crtcs.push(crtc);
                    }
                }
            }
            for crtc in crtcs {
                let surface = match self.device.create_surface(crtc, mode, &[handle]) {
                    Ok(surface) => surface,
                    Err(error) => {
                        debug!(%error, %name, ?crtc, "connector cannot use CRTC");
                        continue;
                    }
                };
                let formats = <GlesRenderer as Bind<Dmabuf>>::supported_formats(&self.renderer)
                    .ok_or_else(|| unavailable("renderer has no renderable dmabuf formats"))?;
                let allocator = GbmAllocator::new(
                    self.gbm.clone(),
                    GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
                );
                let buffers = match GbmBufferedSurface::new(
                    surface,
                    allocator,
                    &[
                        Fourcc::Argb8888,
                        Fourcc::Abgr8888,
                        Fourcc::Xrgb8888,
                        Fourcc::Xbgr8888,
                    ],
                    formats,
                ) {
                    Ok(buffers) => buffers,
                    Err(error) => {
                        debug!(%error, %name, ?crtc, "no compatible scanout allocation");
                        continue;
                    }
                };
                let size = connector.size().unwrap_or_default();
                let output = Output::new(
                    name.clone(),
                    PhysicalProperties {
                        size: (size.0 as i32, size.1 as i32).into(),
                        subpixel: Subpixel::Unknown,
                        make: "DRM".to_owned(),
                        model: name.clone(),
                    },
                );
                let output_mode = Mode::from(mode);
                output.set_preferred(output_mode);
                output.change_current_state(
                    Some(output_mode),
                    Some(Transform::Normal),
                    Some(Scale::Integer(1)),
                    Some((0, 0).into()),
                );
                let damage = OutputDamageTracker::from_output(&output);
                let scene_damage = OutputDamageTracker::from_output(&output);
                info!(%name, ?crtc, size = ?output_mode.size, refresh_mhz = output_mode.refresh, "configured physical output");
                self.scanout = Some(Scanout {
                    buffers,
                    output,
                    connector: handle,
                    crtc,
                    mode,
                    damage,
                    scene_damage,
                    in_flight: None,
                });
                return Ok(true);
            }
        }
        Ok(true)
    }

    pub(crate) fn render(
        &mut self,
        state: &mut InoxState,
        token: FrameToken,
    ) -> Result<bool, BackendError> {
        let elements = state.scene_elements(&mut self.renderer)?;
        let scanout = self
            .scanout
            .as_mut()
            .ok_or_else(|| unavailable("no active scanout"))?;
        // Check scene changes independently of swapchain age. An unused buffer
        // needs a full redraw, but must not cause an unchanged scene to be submitted.
        if !state.frame_budget()
            && scanout
                .scene_damage
                .damage_output(1, &elements)
                .map_err(unavailable)?
                .0
                .is_none()
        {
            return Ok(false);
        }
        if state.frame_budget() {
            scanout.damage = OutputDamageTracker::from_output(&scanout.output);
            scanout.scene_damage = OutputDamageTracker::from_output(&scanout.output);
        }
        let (mut dmabuf, age) = scanout
            .buffers
            .next_buffer()
            .map_err(SwapBuffersError::from)?;
        let mut target = self.renderer.bind(&mut dmabuf)?;
        let rendered = scanout
            .damage
            .render_output(
                &mut self.renderer,
                &mut target,
                usize::from(age),
                &elements,
                state.background(),
            )
            .map_err(unavailable)?;
        let Some(damage) = rendered.damage.cloned() else {
            return Ok(false);
        };
        let sync = rendered.sync;
        drop(target);
        let submission = Submission {
            token,
            monotonic: Clock::<Monotonic>::new().now().into(),
            realtime: SystemTime::now(),
        };
        scanout
            .buffers
            .queue_buffer(Some(sync), Some(damage), token)
            .map_err(SwapBuffersError::from)?;
        scanout.in_flight = Some(submission);
        Ok(true)
    }

    pub(crate) fn completed(
        &mut self,
        crtc: crtc::Handle,
        metadata: Option<EventMetadata>,
    ) -> Result<Option<FrameToken>, BackendError> {
        let Some(scanout) = self.scanout.as_mut() else {
            return Ok(None);
        };
        let Some(submission) = scanout.in_flight.as_ref() else {
            return Ok(None);
        };
        if scanout.crtc != crtc
            || !metadata
                .as_ref()
                .is_some_and(|metadata| submission.accepts(metadata))
        {
            return Ok(None);
        }
        let expected = submission.token;
        let completed = scanout
            .buffers
            .frame_submitted()
            .map_err(SwapBuffersError::from)?;
        scanout.in_flight = None;
        Ok(completed.filter(|token| *token == expected))
    }

    pub(crate) fn retry(&mut self) {
        if let Some(scanout) = &mut self.scanout {
            scanout.buffers.reset_buffers();
            scanout.damage = OutputDamageTracker::from_output(&scanout.output);
            scanout.scene_damage = OutputDamageTracker::from_output(&scanout.output);
            scanout.in_flight = None;
        }
    }

    pub(crate) fn pause(&mut self) {
        self.device.pause();
        if let Some(scanout) = &mut self.scanout {
            scanout.in_flight = None;
        }
    }

    pub(crate) fn resume(&mut self) -> Result<(), BackendError> {
        self.device.activate(true)?;
        if let Some(scanout) = &self.scanout {
            scanout.buffers.surface().reset_state()?;
        }
        self.retry();
        Ok(())
    }
}

impl Drop for DrmBackend {
    fn drop(&mut self) {
        // Release the scanout while DRM is still active, before the fd owner.
        self.scanout = None;
        debug!(card = %self.card.display(), seat = %self.claim.session.seat(), "releasing graphics backend");
    }
}

#[allow(unsafe_code)]
fn create_renderer(gbm: GbmDevice<DrmDeviceFd>) -> Result<GlesRenderer, BackendError> {
    // SAFETY: EGLDisplay owns a GBM device clone, which retains its DRM fd. The
    // display is created once for this GPU, used only on the calloop thread, and
    // is dropped before the session's original device descriptor is released.
    let display = unsafe { EGLDisplay::new(gbm) }.map_err(unavailable)?;
    let context = EGLContext::new(&display).map_err(unavailable)?;
    // SAFETY: This new context has never been made current on another thread.
    // The renderer and its display remain on the thread that owns the event loop.
    unsafe { GlesRenderer::new(context) }.map_err(BackendError::from)
}

fn rank(preferred: bool, width: u32, height: u32, refresh: u32) -> (bool, u64, u32) {
    (preferred, u64::from(width) * u64::from(height), refresh)
}

fn choose_mode(
    modes: &[control::Mode],
    preference: Option<&OutputConfig>,
) -> Option<control::Mode> {
    modes
        .iter()
        .filter(|mode| {
            preference.is_none_or(|p| {
                let (width, height) = mode.size();
                i32::from(width) == p.width
                    && i32::from(height) == p.height
                    && mode.vrefresh() == p.refresh_hz as u32
            })
        })
        .max_by_key(|mode| {
            let (width, height) = mode.size();
            rank(
                mode.mode_type().contains(control::ModeTypeFlags::PREFERRED),
                u32::from(width),
                u32::from(height),
                mode.vrefresh(),
            )
        })
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn page_flips_before_the_current_submission_are_ignored() {
        let token = crate::backend::frame::FrameSchedule::default()
            .begin()
            .unwrap();
        let submission = Submission {
            token,
            monotonic: Duration::from_secs(100),
            realtime: SystemTime::UNIX_EPOCH + Duration::from_secs(100),
        };
        assert!(!submission.accepts(&EventMetadata {
            time: DrmTime::Monotonic(Duration::from_secs(99)),
            sequence: 1
        }));
        assert!(submission.accepts(&EventMetadata {
            time: DrmTime::Monotonic(Duration::from_secs(101)),
            sequence: 2
        }));
        assert!(!submission.accepts(&EventMetadata {
            time: DrmTime::Realtime(SystemTime::UNIX_EPOCH + Duration::from_secs(99)),
            sequence: 1
        }));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]
        #[test]
        fn preferred_modes_dominate_resolution_and_refresh(width in any::<u32>(), height in any::<u32>(), hz in any::<u32>()) {
            prop_assert!(rank(true, 1, 1, 1) > rank(false, width, height, hz));
            prop_assert_eq!(rank(false, width, height, hz).1, u64::from(width) * u64::from(height));
        }
    }
}
