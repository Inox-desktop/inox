//! Smithay DRM/GBM scanout, with libseat-owned device access.
//!
//! One [`GpuBackend`] wraps one DRM device: its own libseat claim, DRM node, GBM
//! device and EGL/GLES renderer, plus one scanout per connector it drives.
//! The compositor opens every enumerated GPU that can drive a display and lets
//! each one drive its own connectors (ADR 0019), so a machine with an integrated
//! and a discrete GPU uses both without sharing a renderer between them — which
//! is what GBM allocation and the damage tracker require anyway.
//!
//! There is no cross-GPU copy path here. That is complete for the buffers this
//! compositor hands out, because it advertises `wl_shm` only and shared memory is
//! importable by any GPU. It is the reason `linux-dmabuf` must not be advertised
//! before a copy path exists: a client buffer living on a GPU that cannot import
//! it would simply not appear (ADR 0019, decision 3).

use std::{
    collections::HashSet,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
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
    utils::{Clock, Logical, Monotonic, Point, Transform},
};
use tracing::{debug, info, warn};

use crate::{InoxState, backend::frame::FrameToken, state::OutputDescriptor};

/// Whether the "frosted glass is not implemented here" warning has been logged.
///
/// The limitation is a property of the backend, not of a frame, so it is reported
/// once per process rather than on every frame a frosted pane is on screen.
static GLASS_WARNING_SHOWN: AtomicBool = AtomicBool::new(false);

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

/// One page flip waiting for its completion event.
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

/// One connector this device drives, and the scanout state it keeps.
struct Scanout {
    buffers: GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, FrameToken>,
    output: Output,
    connector: connector::Handle,
    crtc: crtc::Handle,
    mode: control::Mode,
    damage: OutputDamageTracker,
    scene_damage: OutputDamageTracker,
    in_flight: Option<Submission>,
    /// Where the configuration asked the output to sit, if it said.
    position: Option<Point<i32, Logical>>,
}

/// A single GPU and the connectors it currently drives.
///
/// Drop order releases the scanouts, then EGL/GBM, then DRM master, and finally
/// the libseat claim.
pub struct GpuBackend {
    scanouts: Vec<Scanout>,
    renderer: GlesRenderer,
    gbm: GbmDevice<DrmDeviceFd>,
    device: DrmDevice,
    card: PathBuf,
    claim: Rc<SessionDevice>,
}

impl GpuBackend {
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
                scanouts: Vec::new(),
                renderer,
                gbm,
                device,
                card: path.to_owned(),
                claim,
            },
            notifier,
        ))
    }

    /// Path of the GPU.
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

    /// Whether this device currently drives at least one connector.
    pub(crate) fn is_driving(&self) -> bool {
        !self.scanouts.is_empty()
    }

    /// The identity of one scanout: its card paired with its connector.
    ///
    /// A connector name alone is not unique across a machine's GPUs — two cards
    /// can each have a `DP-1` — so the card is part of the key (ADR 0019). The
    /// name the clients see stays the plain connector name.
    fn key_of(&self, scanout: &Scanout) -> String {
        format!("{}:{}", self.card.display(), scanout.output.name())
    }

    /// Whether `key` names one of this device's scanouts.
    pub(crate) fn drives(&self, key: &str) -> bool {
        self.scanouts
            .iter()
            .any(|scanout| self.key_of(scanout) == key)
    }

    /// Stop driving `key`, returning whether it was one of this device's outputs.
    pub(crate) fn drop_scanout(&mut self, key: &str) -> bool {
        let found = self
            .scanouts
            .iter()
            .position(|scanout| self.key_of(scanout) == key);
        let Some(index) = found else {
            return false;
        };
        self.scanouts.remove(index);

        true
    }

    /// The outputs this device drives, as the compositor's topology sees them.
    pub(crate) fn descriptors(&self) -> Vec<OutputDescriptor> {
        self.scanouts
            .iter()
            .map(|scanout| OutputDescriptor {
                key: self.key_of(scanout),
                output: scanout.output.clone(),
                position: scanout.position,
            })
            .collect()
    }

    /// The refresh interval of `key`, or 60 Hz when it is unknown.
    pub(crate) fn interval(&self, key: &str) -> Duration {
        let mhz = self
            .scanouts
            .iter()
            .find(|scanout| self.key_of(scanout) == key)
            .map(|scanout| Mode::from(scanout.mode).refresh)
            .unwrap_or(60_000)
            .max(1) as u64;

        Duration::from_nanos(1_000_000_000_000 / mhz)
    }

    /// Reconcile the scanouts with the connectors that are connected now.
    ///
    /// Returns whether anything the compositor can see changed: a scanout that
    /// appeared, disappeared, or moved to a different mode or scale. A connector
    /// whose requested mode is unavailable keeps the scanout it already had, which
    /// is what "reject the change and keep the previous state" means per output
    /// (O03) — the failure never takes the whole device down with it.
    ///
    /// # Errors
    /// Returns a typed error when the device itself cannot be interrogated.
    pub(crate) fn rescan(&mut self, preferences: &[OutputConfig]) -> Result<bool, BackendError> {
        if !self.device.is_active() {
            return Ok(false);
        }

        let resources = self.device.resource_handles()?;
        let mut connectors = resources.connectors().to_vec();
        connectors.sort_by_key(|handle| u32::from(*handle));

        let mut changed = false;
        let mut claimed_crtcs: HashSet<crtc::Handle> = HashSet::new();
        let mut unnamed = 0usize;

        for handle in connectors {
            let connector = self.device.get_connector(handle, true)?;
            let name = connector.to_string();

            let existing = self
                .scanouts
                .iter()
                .position(|scanout| scanout.connector == handle);

            if connector.state() != connector::State::Connected || connector.modes().is_empty() {
                if let Some(index) = existing {
                    debug!(%name, "connector disconnected");
                    self.scanouts.remove(index);
                    changed = true;
                }
                continue;
            }

            // A named entry matches its connector; the entries that name no
            // connector take the remaining ones in the order they are listed.
            let preference = match preferences
                .iter()
                .find(|preference| preference.name.as_deref() == Some(&name))
            {
                Some(preference) => Some(preference.clone()),
                None => {
                    let picked = preferences
                        .iter()
                        .filter(|preference| preference.name.is_none())
                        .nth(unnamed)
                        .cloned();
                    if picked.is_some() {
                        unnamed += 1;
                    }
                    picked
                }
            };

            let Some(mode) = choose_mode(connector.modes(), preference.as_ref()) else {
                if existing.is_some() {
                    warn!(%name, "requested mode is not supported; keeping the current one");
                } else {
                    warn!(%name, "requested mode is not supported");
                }
                if let Some(index) = existing {
                    claimed_crtcs.insert(self.scanouts[index].crtc);
                }
                continue;
            };

            if let Some(index) = existing {
                claimed_crtcs.insert(self.scanouts[index].crtc);
                let scanout = &mut self.scanouts[index];
                if scanout.mode != mode {
                    scanout.mode = mode;
                    apply_mode(scanout, mode)?;
                    changed = true;
                }
                changed |= apply_scale_and_position(scanout, preference.as_ref());
                continue;
            }

            match self.create_scanout(
                handle,
                &connector,
                mode,
                &claimed_crtcs,
                preference.as_ref(),
            ) {
                Ok(scanout) => {
                    claimed_crtcs.insert(scanout.crtc);
                    self.scanouts.push(scanout);
                    changed = true;
                }
                Err(error) => debug!(%error, %name, "connector has no usable scanout"),
            }
        }

        Ok(changed)
    }

    /// Whether this device actually drives what `preferences` ask of it (ADR 0028).
    ///
    /// [`GpuBackend::rescan`] keeps a connector's current scanout when the requested
    /// mode is unavailable, so a successful rescan is not evidence that the request
    /// took effect. This reads the device state back and compares it with the
    /// request:
    ///
    /// - every connected connector this device drives is visited in the order
    ///   `rescan` visits it, and the preference `rescan` would apply to it must be
    ///   matched by a scanout whose mode size, refresh and scale equal the request
    ///   and whose recorded position equals the requested one;
    /// - a connected connector that no preference covers at all is not checked:
    ///   nothing was requested for it;
    /// - a preference naming a connector that is not connected on this device is
    ///   skipped, because another device may drive it — or nobody does, which
    ///   `rescan` accepts as well;
    /// - preferences without a name are positional (`rescan` takes them in order,
    ///   per device), so at least as many of them must have been applied as were
    ///   requested. A positional arrangement that spans devices is therefore
    ///   checked per device; name the connectors to arrange them across GPUs.
    ///
    /// # Errors
    /// Returns a reason naming the connector and the mode that did not take.
    pub(crate) fn preferences_in_effect(&self, preferences: &[OutputConfig]) -> Result<(), String> {
        if !self.device.is_active() {
            return Err(format!("{}: the device is not active", self.card.display()));
        }
        let resources = self
            .device
            .resource_handles()
            .map_err(|error| format!("{}: {error}", self.card.display()))?;
        let mut handles = resources.connectors().to_vec();
        handles.sort_by_key(|handle| u32::from(*handle));

        let mut driven = Vec::with_capacity(handles.len());
        for handle in handles {
            let connector = self
                .device
                .get_connector(handle, true)
                .map_err(|error| format!("{}: {error}", self.card.display()))?;
            if connector.state() != connector::State::Connected || connector.modes().is_empty() {
                continue;
            }
            let name = connector.to_string();
            let applied = self
                .scanouts
                .iter()
                .find(|scanout| scanout.output.name() == name)
                .map(applied_output);
            driven.push((name, applied));
        }

        check_preferences(&driven, preferences)
    }

    /// Build the scanout state for one connector on a free CRTC.
    fn create_scanout(
        &mut self,
        handle: connector::Handle,
        connector: &connector::Info,
        mode: control::Mode,
        claimed: &HashSet<crtc::Handle>,
        preference: Option<&OutputConfig>,
    ) -> Result<Scanout, BackendError> {
        let name = connector.to_string();
        let resources = self.device.resource_handles()?;
        let mut candidates = Vec::new();
        if let Some(encoder) = connector.current_encoder()
            && let Some(crtc) = self.device.get_encoder(encoder)?.crtc()
        {
            candidates.push(crtc);
        }
        for encoder in connector.encoders() {
            for crtc in resources.filter_crtcs(self.device.get_encoder(*encoder)?.possible_crtcs())
            {
                if !candidates.contains(&crtc) {
                    candidates.push(crtc);
                }
            }
        }
        // A CRTC already driving another connector of this device cannot drive a
        // second one, so it is skipped rather than stealing the working output.
        candidates.retain(|crtc| !claimed.contains(crtc));

        let formats = <GlesRenderer as Bind<Dmabuf>>::supported_formats(&self.renderer)
            .ok_or_else(|| unavailable("renderer has no renderable dmabuf formats"))?;

        for crtc in candidates {
            let surface = match self.device.create_surface(crtc, mode, &[handle]) {
                Ok(surface) => surface,
                Err(error) => {
                    debug!(%error, %name, ?crtc, "connector cannot use CRTC");
                    continue;
                }
            };
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
                formats.clone(),
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
                Some(scale_of(preference)),
                Some(Point::from((0, 0))),
            );
            let damage = OutputDamageTracker::from_output(&output);
            let scene_damage = OutputDamageTracker::from_output(&output);
            info!(
                %name,
                ?crtc,
                size = ?output_mode.size,
                refresh_mhz = output_mode.refresh,
                scale = output.current_scale().fractional_scale(),
                "configured physical output"
            );

            return Ok(Scanout {
                buffers,
                output,
                connector: handle,
                crtc,
                mode,
                damage,
                scene_damage,
                in_flight: None,
                position: preference
                    .and_then(|p| p.position)
                    .map(|p| Point::from((p.x, p.y))),
            });
        }

        Err(unavailable(format!(
            "{name}: no CRTC can drive this connector"
        )))
    }

    /// Render the output `name` and queue its buffer for scanout.
    ///
    /// Returns whether a buffer was actually submitted: an unchanged scene is not
    /// submitted again, so an idle desktop costs no page flips.
    ///
    /// # Errors
    /// Returns a typed error when rendering or queueing fails.
    pub(crate) fn render(
        &mut self,
        state: &mut InoxState,
        key: &str,
        token: FrameToken,
    ) -> Result<bool, BackendError> {
        let renderer = &mut self.renderer;
        let card = self.card.clone();
        let scanout = self
            .scanouts
            .iter_mut()
            .find(|scanout| format!("{}:{}", card.display(), scanout.output.name()) == key)
            .ok_or_else(|| unavailable(format!("no active scanout for {key}")))?;

        let scene = state.scene_elements(renderer, &scanout.output)?;
        // Frosted glass is not implemented on this path: `OutputDamageTracker`
        // derives the damage it queues from the elements' own damage snapshots,
        // and a frosted passthrough has no honest damage description there yet.
        // The scene stays sharp, and the protocol's `capabilities` event reports
        // `max_radius = 0` on this backend so no client is promised a blur that
        // would not be drawn (see `crate::glass` and `AGENTS.md`). Reported once,
        // not once per frame.
        if !scene.glass.is_empty() && !GLASS_WARNING_SHOWN.swap(true, Ordering::Relaxed) {
            warn!(
                panes = scene.glass.len(),
                "a layer surface asked for frosted glass, which the DRM backend does not \
                 implement yet: the scene is left sharp and capabilities reported no maximum"
            );
        }
        let elements = &scene.elements;
        // Check scene changes independently of swapchain age. An unused buffer
        // needs a full redraw, but must not cause an unchanged scene to be submitted.
        if !state.frame_budget()
            && scanout
                .scene_damage
                .damage_output(1, elements)
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
        let mut target = renderer.bind(&mut dmabuf)?;
        let rendered = scanout
            .damage
            .render_output(
                renderer,
                &mut target,
                usize::from(age),
                elements,
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

    /// Accept a page-flip completion for `crtc`, naming the output it belongs to.
    ///
    /// Returns `None` for a completion that does not belong to the submission in
    /// flight — a stale event from before a pause, a retry or a mode change.
    ///
    /// # Errors
    /// Returns a typed error when the scanout state cannot be advanced.
    pub(crate) fn completed(
        &mut self,
        crtc: crtc::Handle,
        metadata: Option<EventMetadata>,
    ) -> Result<Option<(String, FrameToken)>, BackendError> {
        let Some(scanout) = self
            .scanouts
            .iter_mut()
            .find(|scanout| scanout.crtc == crtc)
        else {
            return Ok(None);
        };
        let Some(submission) = scanout.in_flight.as_ref() else {
            return Ok(None);
        };
        if !metadata
            .as_ref()
            .is_some_and(|metadata| submission.accepts(metadata))
        {
            return Ok(None);
        }
        let expected = submission.token;
        let key = format!("{}:{}", self.card.display(), scanout.output.name());
        let completed = scanout
            .buffers
            .frame_submitted()
            .map_err(SwapBuffersError::from)?;
        scanout.in_flight = None;

        Ok(completed
            .filter(|token| *token == expected)
            .map(|token| (key, token)))
    }

    pub(crate) fn retry(&mut self, key: &str) {
        let card = self.card.clone();
        if let Some(scanout) = self
            .scanouts
            .iter_mut()
            .find(|scanout| format!("{}:{}", card.display(), scanout.output.name()) == key)
        {
            reset_scanout(scanout);
        }
    }

    pub(crate) fn pause(&mut self) {
        self.device.pause();
        for scanout in &mut self.scanouts {
            scanout.in_flight = None;
        }
    }

    /// Rebuild the device and scanout state after a session resumed.
    ///
    /// # Errors
    /// Returns a typed error when the device cannot be activated again.
    pub(crate) fn resume(&mut self) -> Result<(), BackendError> {
        self.device.activate(true)?;
        for scanout in &mut self.scanouts {
            scanout.buffers.surface().reset_state()?;
            reset_scanout(scanout);
        }

        Ok(())
    }
}

impl Drop for GpuBackend {
    fn drop(&mut self) {
        // Release the scanouts while DRM is still active, before the fd owner.
        self.scanouts.clear();
        debug!(card = %self.card.display(), seat = %self.claim.session.seat(), "releasing graphics backend");
    }
}

/// Put a scanout back to "everything needs redrawing".
fn reset_scanout(scanout: &mut Scanout) {
    scanout.buffers.reset_buffers();
    scanout.damage = OutputDamageTracker::from_output(&scanout.output);
    scanout.scene_damage = OutputDamageTracker::from_output(&scanout.output);
    scanout.in_flight = None;
}

/// Move an existing scanout to `mode`.
fn apply_mode(scanout: &mut Scanout, mode: control::Mode) -> Result<bool, BackendError> {
    scanout.buffers.surface().use_mode(mode)?;
    let output_mode = Mode::from(mode);
    scanout.output.set_preferred(output_mode);
    scanout
        .output
        .change_current_state(Some(output_mode), None, None, None);
    scanout.buffers.surface().commit_pending();
    reset_scanout(scanout);

    Ok(true)
}

/// Apply a configuration's scale and position to an existing scanout.
///
/// Returns whether anything changed, so a rescan that only re-reads the same
/// values does not count as a topology change.
fn apply_scale_and_position(scanout: &mut Scanout, preference: Option<&OutputConfig>) -> bool {
    let scale = scale_of(preference);
    let position = preference
        .and_then(|p| p.position)
        .map(|p| Point::from((p.x, p.y)));
    // `Scale` is not `PartialEq`, so both halves are compared as numbers.
    let current = scanout.output.current_scale();
    let changed = current.integer_scale() != scale.integer_scale()
        || (current.fractional_scale() - scale.fractional_scale()).abs() > f64::EPSILON
        || scanout.position != position;

    if changed {
        scanout
            .output
            .change_current_state(None, None, Some(scale), None);
        scanout.position = position;
        reset_scanout(scanout);
    }

    changed
}

/// The Smithay scale a configuration asks for (ADR 0020).
///
/// Integer-only clients are told the ceiling; `wp_fractional_scale_v1` carries
/// the exact factor. Without a preference the output runs at 1:1.
fn scale_of(preference: Option<&OutputConfig>) -> Scale {
    let Some(preference) = preference else {
        return Scale::Integer(1);
    };
    let scale = requested_scale(preference);

    Scale::Custom {
        advertised_integer: scale.ceil().clamp(1.0, f64::from(i32::MAX)) as i32,
        fractional: scale,
    }
}

#[allow(unsafe_code)]
fn create_renderer(gbm: GbmDevice<DrmDeviceFd>) -> Result<GlesRenderer, BackendError> {
    // SAFETY: EGLDisplay owns a GBM device clone, which retains its DRM fd. Each
    // GpuBackend creates exactly one display for its own device, uses it only on
    // the calloop thread, and drops it before the session's original device
    // descriptor is released.
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

/// The applied state of one scanout, as the preference check reads it.
#[derive(Debug, Clone, PartialEq)]
struct AppliedOutput {
    /// Physical width in pixels.
    width: i32,
    /// Physical height in pixels.
    height: i32,
    /// Refresh rate in hertz, as the DRM mode reports it.
    refresh_hz: u32,
    /// Fractional scale in force.
    scale: f64,
    /// Recorded position, or `None` when the configuration left it to the topology.
    position: Option<(i32, i32)>,
}

/// Read the applied state off one scanout.
fn applied_output(scanout: &Scanout) -> AppliedOutput {
    // `create_scanout` and `apply_mode` both set the current mode from the DRM mode,
    // so the advertised mode cannot drift from `scanout.mode`; the fallback only
    // covers an output whose state was never changed.
    let mode = match scanout.output.current_mode() {
        Some(mode) => mode,
        None => Mode::from(scanout.mode),
    };
    AppliedOutput {
        width: mode.size.w,
        height: mode.size.h,
        // `choose_mode` matched the DRM vrefresh, while `Mode::refresh` is
        // recomputed from the pixel clock and can differ by a fraction of a hertz.
        refresh_hz: scanout.mode.vrefresh(),
        scale: scanout.output.current_scale().fractional_scale(),
        position: scanout.position.map(|position| (position.x, position.y)),
    }
}

/// Check one device's driven connectors against the preferences that apply to them.
///
/// `driven` lists the connectors that are connected on the device, in the order
/// `rescan` visits them; each entry pairs the name clients see with the state of
/// the scanout driving it, or `None` when the connector is connected but no
/// scanout could be built for it. Assignment mirrors `rescan` exactly, including
/// the positional counter, so an entry that `rescan` skipped because its requested
/// mode is unsupported is reported rather than passed over.
fn check_preferences(
    driven: &[(String, Option<AppliedOutput>)],
    preferences: &[OutputConfig],
) -> Result<(), String> {
    let positional = || preferences.iter().filter(|entry| entry.name.is_none());
    let requested = positional().count();
    let mut applied = 0usize;

    for (name, output) in driven {
        let preference = match preferences
            .iter()
            .find(|entry| entry.name.as_deref() == Some(name.as_str()))
        {
            Some(preference) => Some(preference),
            None => {
                let picked = positional().nth(applied);
                if picked.is_some() {
                    applied += 1;
                }
                picked
            }
        };
        let Some(preference) = preference else {
            // Nothing was requested for this connector.
            continue;
        };
        let Some(output) = output else {
            return Err(format!(
                "{name}: requested {}x{}@{}Hz could not be driven",
                preference.width, preference.height, preference.refresh_hz
            ));
        };

        let requested_refresh = preference.refresh_hz.max(1) as u32;
        if output.width != preference.width
            || output.height != preference.height
            || output.refresh_hz != requested_refresh
        {
            return Err(format!(
                "{name}: requested {}x{}@{}Hz but the output runs {}x{}@{}Hz",
                preference.width,
                preference.height,
                preference.refresh_hz,
                output.width,
                output.height,
                output.refresh_hz
            ));
        }
        let requested_scale = requested_scale(preference);
        if (output.scale - requested_scale).abs() > f64::EPSILON {
            return Err(format!(
                "{name}: requested scale {requested_scale} but the output runs {}",
                output.scale
            ));
        }
        let requested_position = preference.position.map(|position| (position.x, position.y));
        if output.position != requested_position {
            return Err(format!(
                "{name}: requested position {requested_position:?} but the output is at {:?}",
                output.position
            ));
        }
    }

    if applied < requested {
        return Err(format!(
            "{applied} of {requested} outputs named by position are driven"
        ));
    }

    Ok(())
}

/// The scale factor a preference actually asks for.
///
/// [`scale_of`] replaces a non-positive or non-finite factor with 1:1, so the
/// check has to do the same or it would report a state the compositor applied.
fn requested_scale(preference: &OutputConfig) -> f64 {
    let scale = preference.scale;
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
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

    #[test]
    fn a_configured_scale_is_advertised_upwards_to_integer_clients() {
        for (scale, advertised) in [(1.0, 1), (1.25, 2), (1.5, 2), (2.0, 2), (0.75, 1), (3.5, 4)] {
            let preference = OutputConfig {
                scale,
                ..OutputConfig::default()
            };
            let applied = scale_of(Some(&preference));

            assert_eq!(applied.integer_scale(), advertised, "scale {scale}");
            assert!((applied.fractional_scale() - scale).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn without_a_preference_an_output_runs_at_one_to_one() {
        let applied = scale_of(None);

        assert_eq!(applied.integer_scale(), 1);
        assert!((applied.fractional_scale() - 1.0).abs() < f64::EPSILON);
    }

    fn applied(
        width: i32,
        height: i32,
        refresh_hz: u32,
        scale: f64,
        position: Option<(i32, i32)>,
    ) -> AppliedOutput {
        AppliedOutput {
            width,
            height,
            refresh_hz,
            scale,
            position,
        }
    }

    fn named(name: &str, width: i32, height: i32, refresh_hz: i32, scale: f64) -> OutputConfig {
        OutputConfig {
            name: Some(name.to_owned()),
            width,
            height,
            refresh_hz,
            scale,
            position: Some(inox_config::Position { x: 0, y: 0 }),
        }
    }

    #[test]
    fn a_pinned_preference_is_in_effect_only_when_every_field_matches() {
        let preference = named("HDMI-A-1", 1920, 1080, 60, 1.5);
        let driven = vec![(
            "HDMI-A-1".to_owned(),
            Some(applied(1920, 1080, 60, 1.5, Some((0, 0)))),
        )];

        assert!(check_preferences(&driven, std::slice::from_ref(&preference)).is_ok());

        let wrong_mode = vec![(
            "HDMI-A-1".to_owned(),
            Some(applied(1280, 720, 60, 1.5, Some((0, 0)))),
        )];
        let error = check_preferences(&wrong_mode, std::slice::from_ref(&preference))
            .expect_err("an unsupported mode must be reported");
        assert!(error.contains("HDMI-A-1"), "{error}");
        assert!(error.contains("1920x1080@60Hz"), "{error}");

        let wrong_refresh = vec![(
            "HDMI-A-1".to_owned(),
            Some(applied(1920, 1080, 30, 1.5, Some((0, 0)))),
        )];
        assert!(check_preferences(&wrong_refresh, std::slice::from_ref(&preference)).is_err());

        let wrong_scale = vec![(
            "HDMI-A-1".to_owned(),
            Some(applied(1920, 1080, 60, 1.0, Some((0, 0)))),
        )];
        let error = check_preferences(&wrong_scale, std::slice::from_ref(&preference))
            .expect_err("a scale the device did not take must be reported");
        assert!(error.contains("scale"), "{error}");

        let wrong_position = vec![(
            "HDMI-A-1".to_owned(),
            Some(applied(1920, 1080, 60, 1.5, Some((100, 0)))),
        )];
        let error = check_preferences(&wrong_position, std::slice::from_ref(&preference))
            .expect_err("a position the device did not take must be reported");
        assert!(error.contains("position"), "{error}");
    }

    #[test]
    fn a_requested_connector_without_a_scanout_is_reported() {
        let preference = named("HDMI-A-1", 1920, 1080, 60, 1.0);
        let driven = vec![("HDMI-A-1".to_owned(), None)];

        let error = check_preferences(&driven, std::slice::from_ref(&preference))
            .expect_err("a connected connector that could not be driven must be reported");
        assert!(error.contains("could not be driven"), "{error}");
    }

    #[test]
    fn a_preference_for_another_device_is_ignored() {
        let preference = named("DP-1", 2560, 1440, 60, 1.0);
        let driven = vec![(
            "HDMI-A-1".to_owned(),
            Some(applied(1920, 1080, 60, 1.0, None)),
        )];

        assert!(check_preferences(&driven, std::slice::from_ref(&preference)).is_ok());
    }

    #[test]
    fn a_connector_no_preference_covers_is_not_checked() {
        let driven = vec![(
            "HDMI-A-1".to_owned(),
            Some(applied(1024, 768, 50, 2.0, None)),
        )];

        assert!(check_preferences(&driven, &[]).is_ok());
    }

    #[test]
    fn positional_preferences_are_counted_against_the_connectors_they_land_on() {
        let positional = OutputConfig {
            width: 1920,
            height: 1080,
            refresh_hz: 60,
            scale: 1.0,
            ..OutputConfig::default()
        };
        let driven = vec![
            ("DP-1".to_owned(), Some(applied(1920, 1080, 60, 1.0, None))),
            (
                "HDMI-A-1".to_owned(),
                Some(applied(1024, 768, 50, 1.0, None)),
            ),
        ];

        assert!(check_preferences(&driven, std::slice::from_ref(&positional)).is_ok());

        let error = check_preferences(&driven[..1], &[positional.clone(), OutputConfig::default()])
            .expect_err("a positional preference with no output left must be reported");
        assert!(error.contains("named by position"), "{error}");
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
