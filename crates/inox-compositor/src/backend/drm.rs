//! DRM/KMS real backend scaffold (ADR 0010 §M3a).
//!
//! The M2.5 milestone replaces the headless render path with a libseat session
//! plus a DRM/KMS device driving a GBM surface and vblank-paced page flips. This
//! module currently lands the **shape**: it opens the session, enumerates the
//! first DRM card, and reports a typed error at the render/present boundary so
//! callers can distinguish "device not available" from "device available but
//! GBM/vblank wiring still pending".
//!
//! The actual GBM swapchain, primary-plane atomic commit and vblank event source
//! arrive in the follow-up commits of M3a (see ADR 0010 §"影响面" for the exact
//! Smithay entry points); until then the compositor falls back to the headless
//! path in CI, which continues to satisfy PRD §13 M1/M2 DoD.

use std::path::{Path, PathBuf};

use smithay::backend::session::Session;
use smithay::backend::session::libseat::{LibSeatSession, LibSeatSessionNotifier};
use smithay::reexports::rustix::fs::OFlags;
use tracing::{debug, info};

/// Errors that keep the DRM backend from taking over the compositor.
///
/// Both variants are recoverable at the runtime level: the event loop can either
/// fall back to headless (in `--backend=auto`) or abort with a clear message (in
/// `--backend=drm`).
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// The libseat session could not be opened, or the DRM device could not be
    /// reached through it. The environment has no usable session manager and no
    /// uaccess grant on the DRM card.
    #[error("no DRM device available through libseat: {reason}")]
    Unavailable {
        /// The underlying reason, either the libseat error or the failed device
        /// open, rendered as text so it can travel across `RunError`.
        reason: String,
    },

    /// The session and DRM card opened successfully but the GBM swapchain and
    /// vblank-driven present path defined by ADR 0010 §M3a are not yet wired.
    /// Callers on `--backend=auto` treat this as a headless fallback signal;
    /// explicit `--backend=drm` surfaces it to the user.
    #[error(
        "DRM session open on seat {seat} (card {}), but the GBM/vblank present path is pending (ADR 0010 §M3a)",
        .card.display()
    )]
    Pending {
        /// Seat name reported by libseat, e.g. `seat0`.
        seat: String,
        /// Path of the first DRM card the session was able to open.
        card: PathBuf,
    },
}

/// Path of the DRM card M3a probes through libseat.
///
/// ADR 0010 §1 pins `card0` for the M3a scaffold; multi-GPU and prime
/// enumeration are follow-up concerns.
pub const DRM_CARD: &str = "/dev/dri/card0";

/// A live DRM backend: a libseat session plus the card it claimed.
///
/// The session and its notifier are the pieces M3a extends once the GBM surface
/// and page-flip loop land; keeping them attached to this struct makes the
/// follow-up a matter of filling in [`DrmBackend::present`] rather than
/// reshaping the runtime.
pub struct DrmBackend {
    session: LibSeatSession,
    notifier: LibSeatSessionNotifier,
    seat_name: String,
    card: PathBuf,
}

impl DrmBackend {
    /// Open a libseat session and probe a DRM card through it.
    ///
    /// The session is opened through [`LibSeatSession::new`], which internally
    /// picks logind first and seatd second (ADR 0010 §1). On success, the card
    /// at [`DRM_CARD`] is opened read/write through the session to confirm
    /// device access. The card FD is dropped before returning because the GBM
    /// swapchain that would consume it is not wired yet — holding it would
    /// keep DRM mastership from being released cleanly on a fallback path.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::Unavailable`] when libseat cannot start or the
    /// session cannot open the DRM card.
    pub fn open() -> Result<Self, BackendError> {
        let (mut session, notifier) =
            LibSeatSession::new().map_err(|error| BackendError::Unavailable {
                reason: format!("libseat: {error}"),
            })?;

        let seat_name = session.seat();
        info!(seat = %seat_name, "opened libseat session");

        let card = PathBuf::from(DRM_CARD);
        let flags = OFlags::RDWR | OFlags::CLOEXEC;

        let owned_fd = session
            .open(&card, flags)
            .map_err(|error| BackendError::Unavailable {
                reason: format!("open {}: {error:?}", card.display()),
            })?;

        // The card FD is deliberately released here: the GBM surface that would
        // consume it is not wired up yet, and leaving the FD open would keep the
        // session in an active device claim across a headless fallback.
        drop(owned_fd);
        debug!(card = %card.display(), "probed DRM card through libseat");

        Ok(Self {
            session,
            notifier,
            seat_name,
            card,
        })
    }

    /// Seat name reported by libseat, e.g. `seat0`.
    pub fn seat_name(&self) -> &str {
        &self.seat_name
    }

    /// Path of the DRM card this backend claimed.
    pub fn card(&self) -> &Path {
        &self.card
    }

    /// Borrow the session notifier, which M3a registers on the calloop to react
    /// to `Event::Paused` / `Event::Resumed` (VT switch and lid close).
    pub fn notifier(&mut self) -> &mut LibSeatSessionNotifier {
        &mut self.notifier
    }

    /// Borrow the underlying session; the M3a render path will use it to open
    /// the DRM card FD that the GBM device ultimately binds against.
    pub fn session(&mut self) -> &mut LibSeatSession {
        &mut self.session
    }

    /// The `present` entry point the M3a frame loop will call once the GBM
    /// swapchain and vblank plumbing land. Until then it always reports the
    /// backend as pending, which is how `--backend=auto` decides to fall back.
    pub fn present(&self) -> Result<(), BackendError> {
        Err(BackendError::Pending {
            seat: self.seat_name.clone(),
            card: self.card.clone(),
        })
    }
}
