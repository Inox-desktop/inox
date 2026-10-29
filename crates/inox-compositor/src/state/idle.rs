//! Idle tracking: `zwp_idle_inhibit_manager_v1` and `ext_idle_notifier_v1`.
//!
//! # What this layer is for
//!
//! `inox-idle` (L4) asks the compositor for an idle notification and, when the
//! `idled` event arrives, asks the session to lock. Until this module existed the
//! compositor had neither global, so the daemon had nothing to subscribe to and
//! idle-triggered locking could not happen at all. Both halves live here because
//! they are the same question — "has the user been inactive long enough" — with
//! one client able to hold the answer off.
//!
//! # The countdown
//!
//! Every notification carries an absolute `due` instant on the compositor's
//! monotonic clock. It starts at "creation + timeout" (the protocol counts from
//! the creation of the notification) and is pushed forward to "now + timeout" by
//! any input event the seat processes. [`InoxState::tick_idle`] is called from the
//! event loop every [`TICK`]; it is the only place `idled` is sent, and the
//! `idle` flag on each notification is what keeps the protocol's "never send
//! `idled` twice without a `resumed` in between" invariant.
//!
//! A tick, rather than a calloop timer per notification, because the compositor
//! state does not own the loop: its handle is typed for the runtime's own data,
//! so a timer registered here could not borrow this state. One timer of the
//! runtime's costs a wake-up per [`TICK`] and needs no ownership plumbing.
//!
//! # Inhibition
//!
//! A `zwp_idle_inhibitor_v1` is a request from a client that is showing something
//! the user is watching (a video, a presentation). The protocol only lets it
//! count if its surface is **visible**, and this compositor takes that literally:
//! [`InoxState::idle_inhibited`] accepts an inhibitor only when its surface is
//! one this compositor actually composites — a mapped layer surface, a window's
//! surface, or a lock surface. A client that creates an inhibitor on a surface it
//! never maps, or never gives a role to, keeps nothing alive.
//!
//! That rule is the reason a hostile client cannot hold the session unlocked
//! forever by inhibiting idle: to inhibit it has to put something on screen, and
//! whatever it puts there is what the user sees while the screen stays unlocked.
//!
//! While any inhibitor counts, the countdown does not accumulate: each tick
//! pushes every pending notification's deadline to "now + timeout", and a
//! notification that is already idle is resumed. Holding the lock request back is
//! therefore visible to the client that asked for it, instead of the event
//! arriving late once the inhibitor goes away.

use std::time::Instant;

use inox_protocols::standard::ext::idle_notify::server::{
    ext_idle_notification_v1::{self as notification, ExtIdleNotificationV1},
    ext_idle_notifier_v1::{self as notifier, ExtIdleNotifierV1},
};
use smithay::wayland::idle_inhibit::IdleInhibitHandler;

use super::*;

/// How often the compositor checks whether a notification has come due.
///
/// 100 ms is below what a screen blanking timeout can distinguish and well above
/// what an idle wake-up costs. It is also the granularity at which an inhibitor
/// that disappears restarts the countdown.
pub(crate) const TICK: Duration = Duration::from_millis(100);

/// The version `ext_idle_notifier_v1` is advertised at.
///
/// Version 2 adds `get_input_idle_notification`, which must ignore idle
/// inhibitors. Advertising 1 keeps clients on the request that does respect them,
/// which is the one this compositor implements.
const VERSION: u32 = 1;

/// One client's idle notification.
struct Notification {
    /// The protocol object the events are sent on.
    object: ExtIdleNotificationV1,
    /// The timeout the client asked for, kept to push the deadline forward.
    timeout: Duration,
    /// When this notification becomes due, on the compositor's monotonic clock.
    due: Instant,
    /// Whether `idled` has been sent and `resumed` is still owed.
    idle: bool,
}

/// The idle notifications and inhibitors of one compositor run.
pub(crate) struct IdleState {
    /// The `ext_idle_notifier_v1` global. Dropping this removes it.
    #[allow(dead_code)]
    global: GlobalId,
    /// The live notifications, in creation order.
    notifications: Vec<Notification>,
    /// The surfaces whose clients asked to inhibit idle.
    ///
    /// Kept even when the surface is not (or no longer) displayable: whether an
    /// inhibitor counts is decided per tick from the scene, not at the moment the
    /// request arrives, because a surface can be mapped and unmapped afterwards.
    inhibitors: Vec<WlSurface>,
}

impl IdleState {
    /// Advertise `ext_idle_notifier_v1` and start the clock.
    pub(super) fn new(display_handle: &DisplayHandle) -> Self {
        let global = display_handle.create_global::<InoxState, ExtIdleNotifierV1, ()>(VERSION, ());

        Self {
            global,
            notifications: Vec::new(),
            inhibitors: Vec::new(),
        }
    }

    /// The surfaces whose clients asked for idle to be inhibited.
    pub(super) fn inhibitors(&self) -> &[WlSurface] {
        &self.inhibitors
    }

    /// Record an inhibitor. It only takes effect once its surface is displayed.
    fn inhibit(&mut self, surface: WlSurface) {
        if !self.inhibitors.contains(&surface) {
            self.inhibitors.push(surface);
        }
    }

    /// Forget an inhibitor its client explicitly dropped.
    fn uninhibit(&mut self, surface: &WlSurface) {
        self.inhibitors.retain(|entry| entry != surface);
    }

    /// Create a notification, due `timeout` from now.
    fn add(&mut self, object: ExtIdleNotificationV1, timeout: Duration) {
        self.notifications.push(Notification {
            object,
            timeout,
            due: Instant::now() + timeout,
            idle: false,
        });
    }

    /// Forget a notification whose object went away.
    fn remove(&mut self, object: &ExtIdleNotificationV1) {
        self.notifications.retain(|entry| &entry.object != object);
    }

    /// Drop inhibitors and notifications whose objects are gone.
    ///
    /// A client that disconnects, or a surface that is destroyed while its
    /// inhibitor still exists, must not keep anything alive: `uninhibit` is only
    /// called for an explicit `destroy`, which the protocol documents.
    fn prune(&mut self) {
        self.inhibitors.retain(WlSurface::is_alive);
        self.notifications.retain(|entry| entry.object.is_alive());
    }

    /// Push every pending deadline forward: input started the countdown again.
    ///
    /// Called from the four input entry points, which are the only places user
    /// activity enters this compositor — the physical backend and the test channel
    /// both go through them.
    pub(super) fn note_activity(&mut self) {
        let now = Instant::now();

        for entry in &mut self.notifications {
            entry.due = now + entry.timeout;
            if entry.idle {
                entry.object.resumed();
                entry.idle = false;
            }
        }
    }

    /// Send `idled` to every notification that has come due.
    ///
    /// Returns whether anything was sent, so the caller knows a flush is owed.
    /// A `resumed` sent while an inhibitor counts is one of those things: the
    /// client on the other end is waiting for it just as much as for an `idled`.
    fn tick(&mut self, now: Instant, inhibited: bool) -> bool {
        let mut sent = false;

        if inhibited {
            // The countdown does not run while an inhibitor counts, and a
            // notification that is already idle stops being idle: a client that
            // queued a lock request behind `idled` has to be able to withdraw it.
            for entry in &mut self.notifications {
                entry.due = now + entry.timeout;
                if entry.idle {
                    entry.object.resumed();
                    entry.idle = false;
                    sent = true;
                }
            }

            return sent;
        }

        for entry in &mut self.notifications {
            if !entry.idle && now >= entry.due {
                entry.object.idled();
                entry.idle = true;
                sent = true;
            }
        }

        sent
    }
}

impl InoxState {
    /// Advance the idle clock and send the events the tick owes.
    ///
    /// Called from the event loop on every [`TICK`]; returns whether a client was
    /// sent something, which is when the display needs flushing.
    pub(crate) fn tick_idle(&mut self) -> bool {
        let now = Instant::now();
        self.idle.prune();
        let inhibited = self.idle_inhibited();
        let sent = self.idle.tick(now, inhibited);

        if sent {
            debug!("sent an idle notification event");
        }

        sent
    }

    /// Whether a client is holding the idle countdown off.
    ///
    /// An inhibitor counts only while its surface is one this compositor draws;
    /// see the module docs for why that is the security-relevant reading of the
    /// protocol's "visible surface".
    fn idle_inhibited(&self) -> bool {
        self.idle
            .inhibitors()
            .iter()
            .any(|surface| self.surface_is_displayed(surface))
    }

    /// Whether `surface` is part of what this compositor composites.
    ///
    /// A window counts only when it is *visible* — on its output's active
    /// workspace and not minimized — because a window on another workspace has a
    /// buffer and a live surface while showing nothing, which is exactly the state
    /// an inhibitor is not allowed to hide behind. A layer surface counts when it
    /// is mapped, and a lock surface while a lock is held.
    ///
    /// Subsurfaces and popups are not listed: an inhibitor on one of them is not
    /// something clients do, and saying so here is cheaper than walking a surface
    /// tree on every tick.
    fn surface_is_displayed(&self, surface: &WlSurface) -> bool {
        self.windows.iter().any(|(id, window)| {
            self.workspace.is_visible(*id)
                && window.wl_surface().is_some_and(|own| &*own == surface)
        }) || self
            .layers
            .iter()
            .any(|entry| entry.mapped && entry.surface.wl_surface() == surface)
            || self
                .lock
                .surfaces
                .iter()
                .any(|(_, _, lock)| lock.wl_surface() == surface)
    }
}

impl IdleInhibitHandler for InoxState {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle.inhibit(surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle.uninhibit(&surface);
    }
}

impl GlobalDispatch<ExtIdleNotifierV1, ()> for InoxState {
    fn bind(
        _state: &mut Self,
        _display_handle: &DisplayHandle,
        _client: &Client,
        notifier: New<ExtIdleNotifierV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(notifier, ());
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for InoxState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _notifier: &ExtIdleNotifierV1,
        request: notifier::Request,
        _data: &(),
        _display_handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            notifier::Request::GetIdleNotification { id, timeout, seat } => {
                // This compositor has exactly one seat, and Smithay 0.7 exposes
                // no way back from a client's `wl_seat` object to the `Seat` it
                // was bound from, so the argument cannot be compared with
                // anything. Every notification is therefore driven by the one
                // seat's input, which is the same answer for the only seat a
                // well-behaved client can name. The object is kept alive so that
                // a seat that is destroyed while the notification exists is
                // still a live object, not a dangling one.
                let _ = &seat;
                let object = data_init.init(id, ());
                state
                    .idle
                    .add(object, Duration::from_millis(u64::from(timeout)));
            }
            // The manager is only a factory: the notifications outlive it.
            notifier::Request::Destroy => {}
            // `Request` is `#[non_exhaustive]`, and `get_input_idle_notification`
            // belongs to version 2, which this global does not advertise.
            _ => {}
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for InoxState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _notification: &ExtIdleNotificationV1,
        _request: notification::Request,
        _data: &(),
        _display_handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // `destroy` is a destructor: wayland-server removes the object once the
        // request has been dispatched, and `destroyed` below forgets it.
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        notification: &ExtIdleNotificationV1,
        _data: &(),
    ) {
        state.idle.remove(notification);
    }
}
