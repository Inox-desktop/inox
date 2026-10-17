//! Fail-closed lock-before-suspend ordering, independent of system transports.
use std::time::{Duration, Instant};

/// Identifier minted for one lock request; confirmations must match exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestId(u64);

/// Input from the compositor or the reviewed system boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// An inhibitor-respecting notification became idle.
    Idled,
    /// Activity resumed. This never unlocks the session.
    Resumed,
    /// Local automatic-policy inhibition changed.
    Inhibited(bool),
    /// Explicit user lock, unaffected by inhibition.
    ManualLock,
    /// Begin a suspend preparation; this is not permission to sleep.
    PrepareSuspend,
    /// All outputs are locked, confirmed by the trusted compositor transport.
    LockConfirmed(RequestId),
    /// The trusted lock transport refused or disconnected.
    LockFailed(RequestId),
    /// Check the deadline using the caller's monotonic clock.
    Tick,
}

/// Work for a transport. The policy itself never launches a command or suspends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Ask the trusted lock boundary to cover all outputs.
    RequestLock(RequestId),
    /// A matching lock completed before the deadline; allow this preparation only.
    AllowSuspend(RequestId),
    /// Keep the machine awake; no successful lock was confirmed in time.
    RefuseSuspend,
    /// No transport action.
    None,
}

/// Observable idle policy with a single outstanding lock request.
#[derive(Debug)]
pub struct IdlePolicy {
    inhibited: bool,
    idle: bool,
    locked: bool,
    generation: u64,
    pending: Option<(RequestId, Instant)>,
    preparing: bool,
    timeout: Duration,
}
impl Default for IdlePolicy {
    fn default() -> Self {
        Self {
            inhibited: false,
            idle: false,
            locked: false,
            generation: 0,
            pending: None,
            preparing: false,
            timeout: Duration::from_secs(5),
        }
    }
}
impl IdlePolicy {
    /// Whether the compositor's trusted confirmation has been received.
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// Reset only after an authenticated unlock from the trusted transport.
    pub fn authenticated_unlock(&mut self) {
        self.locked = false;
        self.pending = None;
        self.preparing = false;
    }

    /// Apply one event. Duplicates, expired and superseded replies authorize nothing.
    pub fn apply(&mut self, event: Event, now: Instant) -> Effect {
        if self.pending.is_some_and(|(_, deadline)| now >= deadline) {
            self.pending = None;
            let preparing = std::mem::take(&mut self.preparing);
            if preparing {
                return Effect::RefuseSuspend;
            }
        }
        match event {
            Event::Idled => {
                if self.idle || self.inhibited {
                    return Effect::None;
                }
                self.idle = true;
                self.request(now)
            }
            Event::Resumed => {
                self.idle = false;
                Effect::None
            }
            Event::Inhibited(value) => {
                self.inhibited = value;
                Effect::None
            }
            Event::ManualLock => self.request(now),
            Event::PrepareSuspend => {
                if self.preparing {
                    return Effect::None;
                }
                self.preparing = true;
                // A fresh confirmation is required even when already locked.
                self.pending = None;
                self.request(now)
            }
            Event::LockConfirmed(id) if self.pending.is_some_and(|(current, _)| current == id) => {
                self.pending = None;
                self.locked = true;
                if std::mem::take(&mut self.preparing) {
                    Effect::AllowSuspend(id)
                } else {
                    Effect::None
                }
            }
            Event::LockFailed(id) if self.pending.is_some_and(|(current, _)| current == id) => {
                self.pending = None;
                if std::mem::take(&mut self.preparing) {
                    Effect::RefuseSuspend
                } else {
                    Effect::None
                }
            }
            Event::LockConfirmed(_) | Event::LockFailed(_) | Event::Tick => Effect::None,
        }
    }
    fn request(&mut self, now: Instant) -> Effect {
        if self.pending.is_some() {
            return Effect::None;
        }
        let Some(generation) = self.generation.checked_add(1) else {
            self.preparing = false;
            return Effect::RefuseSuspend;
        };
        self.generation = generation;
        let id = RequestId(generation);
        self.pending = Some((id, now + self.timeout));
        Effect::RequestLock(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(effect: Effect) -> RequestId {
        match effect {
            Effect::RequestLock(id) => id,
            other => panic!("{other:?}"),
        }
    }
    #[test]
    fn inhibitors_do_not_block_explicit_lock_or_suspend_preparation() {
        let now = Instant::now();
        let mut state = IdlePolicy::default();
        state.apply(Event::Inhibited(true), now);
        assert_eq!(state.apply(Event::Idled, now), Effect::None);
        let id = request(state.apply(Event::ManualLock, now));
        assert_eq!(state.apply(Event::LockConfirmed(id), now), Effect::None);
        assert!(state.locked());
        let id = request(state.apply(Event::PrepareSuspend, now));
        assert_eq!(
            state.apply(Event::LockConfirmed(id), now),
            Effect::AllowSuspend(id)
        );
        assert_eq!(state.apply(Event::LockConfirmed(id), now), Effect::None);
        state.apply(Event::Resumed, now);
        assert!(state.locked());
    }
    #[test]
    fn stale_failed_and_late_replies_never_allow_sleep() {
        let now = Instant::now();
        let mut state = IdlePolicy::default();
        let first = request(state.apply(Event::PrepareSuspend, now));
        assert_eq!(
            state.apply(Event::Tick, now + Duration::from_secs(5)),
            Effect::RefuseSuspend
        );
        let next = request(state.apply(Event::PrepareSuspend, now + Duration::from_secs(6)));
        assert_eq!(
            state.apply(Event::LockConfirmed(first), now + Duration::from_secs(6)),
            Effect::None
        );
        assert_eq!(
            state.apply(Event::LockFailed(next), now + Duration::from_secs(6)),
            Effect::RefuseSuspend
        );
        assert_eq!(
            state.apply(Event::LockConfirmed(next), now + Duration::from_secs(6)),
            Effect::None
        );
        assert!(!state.locked());
    }
    #[test]
    fn idle_events_are_deduplicated_and_resuming_is_not_unlocking() {
        let now = Instant::now();
        let mut state = IdlePolicy::default();
        let id = request(state.apply(Event::Idled, now));
        assert_eq!(state.apply(Event::Idled, now), Effect::None);
        state.apply(Event::LockConfirmed(id), now);
        state.apply(Event::Resumed, now);
        assert!(state.locked());
        state.authenticated_unlock();
        assert!(!state.locked());
    }
    #[test]
    fn confirmation_at_the_deadline_is_too_late() {
        let now = Instant::now();
        let mut state = IdlePolicy::default();
        let id = request(state.apply(Event::PrepareSuspend, now));
        assert_eq!(
            state.apply(Event::LockConfirmed(id), now + Duration::from_secs(5)),
            Effect::RefuseSuspend
        );
        assert!(!state.locked());
    }
}
