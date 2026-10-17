//! A calloop client of inhibitor-respecting ext-idle-notify v1.
use crate::policy::{Effect, Event, IdlePolicy};
use calloop::{
    EventLoop, Interest, Mode, PostAction,
    generic::Generic,
    signals::{Signal, Signals},
    timer::{TimeoutAction, Timer},
};
use inox_common::i18n::Localizer;
use inox_protocols::standard::{
    core::client::{wl_callback, wl_registry, wl_seat},
    ext::idle_notify::client::{ext_idle_notification_v1, ext_idle_notifier_v1},
};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};

/// Idle protocol connection, capability or timeout failure.
#[derive(Debug, thiserror::Error)]
#[error("idle protocol failed: {0}")]
pub struct IdleError(pub String);

struct State {
    seat: Option<(u32, wl_seat::WlSeat)>,
    notifier: Option<(u32, ext_idle_notifier_v1::ExtIdleNotifierV1)>,
    notification: Option<ext_idle_notification_v1::ExtIdleNotificationV1>,
    timeout_ms: u32,
    failure: Option<String>,
    stopped: bool,
    policy: IdlePolicy,
    localizer: Localizer,
}
impl State {
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
                if let Effect::RequestLock(id) = state.policy.apply(Event::Idled, now) {
                    // No reviewed lock transport exists. Never infer success from process exit.
                    state.policy.apply(Event::LockFailed(id), now);
                    warn!("{}", state.localizer.message("lock-blocked"));
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
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore ext_idle_notifier_v1::ExtIdleNotifierV1);

/// Listen for compositor activity. Missing capabilities fail within five seconds.
/// This monitor requests neither lock nor suspend while their trusted transports are absent.
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
