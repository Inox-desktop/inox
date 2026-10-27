//! A calloop client of inhibitor-respecting ext-idle-notify v1.
use crate::policy::{Effect, Event, IdlePolicy, RequestId};
use crate::transport::LockRequester;
use calloop::{
    EventLoop, Interest, Mode, PostAction,
    generic::Generic,
    signals::{Signal, Signals},
    timer::{TimeoutAction, Timer},
};
use inox_common::i18n::Localizer;
use inox_protocols::{
    ipc::inox_ipc::client::inox_ipc::{Event as IpcEvent, InoxIpc, LockState},
    standard::{
        core::client::{wl_callback, wl_registry, wl_seat},
        ext::idle_notify::client::{ext_idle_notification_v1, ext_idle_notifier_v1},
    },
};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};

/// Idle protocol connection, capability or timeout failure.
#[derive(Debug, thiserror::Error)]
#[error("idle protocol failed: {0}")]
pub struct IdleError(pub String);

struct State {
    seat: Option<(u32, wl_seat::WlSeat)>,
    notifier: Option<(u32, ext_idle_notifier_v1::ExtIdleNotifierV1)>,
    notification: Option<ext_idle_notification_v1::ExtIdleNotificationV1>,
    /// The compositor's report of whether the session is locked, when one has
    /// arrived. `None` is "never told", which is not "unlocked".
    covered: Option<bool>,
    ipc: Option<InoxIpc>,
    requester: Option<LockRequester>,
    timeout_ms: u32,
    failure: Option<String>,
    stopped: bool,
    policy: IdlePolicy,
    localizer: Localizer,
}
impl State {
    /// Asks for a lock, unless the compositor has already told us the screen is
    /// covered.
    ///
    /// The second half matters: a compositor reports `session_lock_state` on bind
    /// and on every *change*. If the session was already locked when the request is
    /// made, waiting for a new event waits forever — so an observed "locked" is
    /// itself the confirmation, and asking again would be asking a compositor that
    /// refuses a second lock.
    fn request_lock(&mut self, id: RequestId, now: Instant) {
        if let Some(effect) = confirm_observed(&mut self.policy, self.covered == Some(true), now) {
            debug_assert!(self.policy.locked());
            self.act(effect);

            return;
        }

        let reply = self.requester.as_ref().map(LockRequester::request);

        match reply {
            // Dispatched or busy: the session understood, and a second press is the
            // retry. The request stays pending until the compositor speaks.
            Some(Ok(inox_protocols::control::Reply::Dispatched))
            | Some(Ok(inox_protocols::control::Reply::Busy)) => {
                info!("{}", self.localizer.message("lock-requested"));
            }
            Some(Ok(inox_protocols::control::Reply::Unknown)) => {
                warn!("{}", self.localizer.message("lock-blocked"));
                self.policy.apply(Event::LockFailed(id), now);
            }
            Some(Err(error)) => {
                debug!(%error, "the session did not answer the lock request");
                warn!("{}", self.localizer.message("lock-blocked"));
                self.policy.apply(Event::LockFailed(id), now);
            }
            None => {
                warn!("{}", self.localizer.message("lock-blocked"));
                self.policy.apply(Event::LockFailed(id), now);
            }
        }
    }

    /// Carries out what the policy decided.
    fn act(&mut self, effect: Effect) {
        match effect {
            // The lock is confirmed, but the suspend transport (logind) is not
            // wired: say so rather than pretending the machine will sleep.
            Effect::AllowSuspend(id) => warn!(
                ?id,
                "the lock is confirmed, but this daemon has no suspend transport; \
                 nothing was suspended"
            ),
            Effect::RefuseSuspend => warn!("the suspend was refused: the lock was not confirmed"),
            Effect::RequestLock(id) => self.request_lock(id, Instant::now()),
            Effect::None => {}
        }
    }

    fn initialize(&mut self, qh: &QueueHandle<Self>) {
        match (&self.seat, &self.notifier) {
            (Some((_, seat)), Some((_, notifier))) => {
                self.notification =
                    Some(notifier.get_idle_notification(self.timeout_ms, seat, qh, ()));
                info!("{}", self.localizer.message("starting"));
            }
            _ => {
                self.failure =
                    Some("compositor must advertise wl_seat and ext_idle_notifier_v1".into())
            }
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_seat" && state.seat.is_none() => {
                state.seat = Some((name, registry.bind(name, version.min(1), qh, ())));
            }
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "ext_idle_notifier_v1" && state.notifier.is_none() => {
                state.notifier = Some((name, registry.bind(name, version.min(1), qh, ())));
            }
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "inox_ipc" && state.ipc.is_none() => {
                state.ipc = Some(registry.bind(name, version.min(2), qh, ()));
            }
            wl_registry::Event::GlobalRemove { name }
                if state.seat.as_ref().is_some_and(|(id, _)| *id == name)
                    || state.notifier.as_ref().is_some_and(|(id, _)| *id == name) =>
            {
                state.failure = Some("idle notifier or selected seat was removed".into());
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.initialize(qh);
        }
    }
}
impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ext_idle_notification_v1::ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let now = Instant::now();
        match event {
            ext_idle_notification_v1::Event::Idled => {
                let effect = state.policy.apply(Event::Idled, now);

                if let Effect::RequestLock(id) = effect {
                    state.request_lock(id, now);
                }
            }
            ext_idle_notification_v1::Event::Resumed => {
                state.policy.apply(Event::Resumed, now);
                info!("{}", state.localizer.message("resumed"));
            }
            _ => {}
        }
    }
}
impl Dispatch<InoxIpc, ()> for State {
    fn event(
        state: &mut Self,
        _: &InoxIpc,
        event: IpcEvent,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let IpcEvent::SessionLockState { state: value } = event else {
            return;
        };

        let now = Instant::now();

        // An unknown state is never read as "unlocked": that is the mistake that
        // would make a locked session sleep or take the next press as an unlock.
        match value.into_result() {
            Ok(LockState::Locked) => {
                state.covered = Some(true);

                if let Some(id) = state.policy.pending() {
                    let effect = state.policy.apply(Event::LockConfirmed(id), now);
                    state.act(effect);
                }
            }
            Ok(LockState::Unlocked) => {
                state.covered = Some(false);

                if state.policy.locked() {
                    state.policy.authenticated_unlock();
                    info!("{}", state.localizer.message("unlocked"));
                }
            }
            Err(_) | Ok(_) => warn!(
                "the compositor reported a session lock state this version of the daemon does \
                 not know; it is ignored rather than read as unlocked"
            ),
        }
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore ext_idle_notifier_v1::ExtIdleNotifierV1);

/// Listen for compositor activity. Missing capabilities fail within five seconds.
///
/// A lock is requested through `session-control-v1` and confirmed only by the
/// compositor's `session_lock_state`; the suspend transport (logind) is still absent,
/// so a confirmed lock ends at "the screen is covered", not at "the machine sleeps".
pub fn run(timeout: Duration, localizer: Localizer) -> Result<(), IdleError> {
    let timeout_ms = u32::try_from(timeout.as_millis())
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| {
            IdleError("timeout must fit a positive uint32 millisecond duration".into())
        })?;
    let connection = Connection::connect_to_env().map_err(|e| IdleError(e.to_string()))?;
    let mut queue = connection.new_event_queue::<State>();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    connection.display().sync(&qh, ());
    connection.flush().map_err(|e| IdleError(e.to_string()))?;
    let mut state = State {
        seat: None,
        notifier: None,
        notification: None,
        covered: None,
        ipc: None,
        requester: LockRequester::discover(),
        timeout_ms,
        failure: None,
        stopped: false,
        policy: IdlePolicy::default(),
        localizer,
    };
    let mut event_loop = EventLoop::<State>::try_new().map_err(|e| IdleError(e.to_string()))?;
    let handle = event_loop.handle();
    let reader = connection.clone();
    handle
        .insert_source(
            Generic::new(reader, Interest::READ, Mode::Level),
            move |_, connection, state| {
                if let Some(guard) = connection.prepare_read() {
                    match guard.read() {
                        Ok(_) => {}
                        Err(wayland_client::backend::WaylandError::Io(error))
                            if error.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(error) => state.failure = Some(error.to_string()),
                    }
                }
                if let Err(error) = queue.dispatch_pending(state) {
                    state.failure = Some(error.to_string());
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| IdleError(e.to_string()))?;
    handle
        .insert_source(
            Timer::from_duration(Duration::from_secs(5)),
            |_, _, state| {
                if state.notification.is_none() {
                    state.failure = Some("registry synchronization timed out".into());
                }
                TimeoutAction::Drop
            },
        )
        .map_err(|e| IdleError(e.to_string()))?;
    let signals = Signals::new(&[Signal::SIGINT, Signal::SIGTERM, Signal::SIGHUP])
        .map_err(|e| IdleError(e.to_string()))?;
    handle
        .insert_source(signals, |_, _, state| state.stopped = true)
        .map_err(|e| IdleError(e.to_string()))?;
    while !state.stopped && state.failure.is_none() {
        connection.flush().map_err(|e| IdleError(e.to_string()))?;
        event_loop
            .dispatch(None, &mut state)
            .map_err(|e| IdleError(e.to_string()))?;
    }
    if let Some(notification) = &state.notification {
        notification.destroy();
    }
    if let Some((_, notifier)) = &state.notifier {
        notifier.destroy();
    }
    if let Err(error) = connection.flush() {
        warn!(%error, "failed to flush idle protocol cleanup");
    }
    match state.failure {
        Some(error) => Err(IdleError(error)),
        None => Ok(()),
    }
}

/// Settles a pending request from a lock the compositor has already reported.
///
/// `session_lock_state` is sent on bind and on every change, so a request made while
/// the session is already locked produces no new event. Reading the observed state
/// as the confirmation is what keeps that case from waiting for ever — and reading
/// it only when the compositor said "locked" is what keeps silence from becoming a
/// confirmation.
fn confirm_observed(policy: &mut IdlePolicy, covered: bool, now: Instant) -> Option<Effect> {
    if !covered {
        return None;
    }

    let id = policy.pending()?;

    Some(policy.apply(Event::LockConfirmed(id), now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(policy: &mut IdlePolicy, now: Instant) -> RequestId {
        match policy.apply(Event::Idled, now) {
            Effect::RequestLock(id) => id,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_observed_lock_settles_a_pending_request() {
        let now = Instant::now();
        let mut policy = IdlePolicy::default();
        let _ = pending(&mut policy, now);

        let effect = confirm_observed(&mut policy, true, now);

        assert_eq!(
            effect,
            Some(Effect::None),
            "nothing to suspend; the lock simply landed"
        );
        assert!(
            policy.locked(),
            "the session counts as locked from the observation"
        );
        assert_eq!(policy.pending(), None);
    }

    #[test]
    fn silence_is_not_a_confirmation() {
        let now = Instant::now();
        let mut policy = IdlePolicy::default();
        let id = pending(&mut policy, now);

        assert_eq!(confirm_observed(&mut policy, false, now), None);
        assert!(!policy.locked());
        assert_eq!(
            policy.pending(),
            Some(id),
            "the request is still outstanding"
        );
    }

    #[test]
    fn an_observation_with_nothing_pending_confirms_nothing() {
        let now = Instant::now();
        let mut policy = IdlePolicy::default();

        assert_eq!(confirm_observed(&mut policy, true, now), None);
        assert!(
            !policy.locked(),
            "no request was made, so no lock was confirmed"
        );
    }
}
