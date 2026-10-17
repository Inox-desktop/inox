//! Backend-independent frame lifecycle. Device completions carry an epoch token.

/// An individual submission in one output lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameToken {
    epoch: u64,
    sequence: u64,
}

/// The scheduling state of an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Idle,
    Scheduled,
    Submitted(FrameToken),
    Paused,
    Removed,
}

/// One pending submission at most; repeated failure stops until device recovery.
#[derive(Debug)]
pub(crate) struct FrameSchedule {
    phase: Phase,
    epoch: u64,
    sequence: u64,
    dirty: bool,
    failures: u8,
}

impl Default for FrameSchedule {
    fn default() -> Self {
        Self {
            phase: Phase::Scheduled,
            epoch: 0,
            sequence: 0,
            dirty: true,
            failures: 0,
        }
    }
}

impl FrameSchedule {
    pub(crate) fn phase(&self) -> Phase {
        self.phase
    }

    pub(crate) fn damage(&mut self) {
        self.dirty = true;
        if self.phase == Phase::Idle {
            self.phase = Phase::Scheduled;
            self.failures = 0;
        }
    }

    pub(crate) fn begin(&mut self) -> Option<FrameToken> {
        if self.phase != Phase::Scheduled {
            return None;
        }
        self.sequence = self.sequence.wrapping_add(1);
        let token = FrameToken {
            epoch: self.epoch,
            sequence: self.sequence,
        };
        self.phase = Phase::Submitted(token);
        self.dirty = false;
        Some(token)
    }

    pub(crate) fn complete(&mut self, token: FrameToken) -> bool {
        if self.phase != Phase::Submitted(token) {
            return false;
        }
        self.failures = 0;
        self.phase = if self.dirty {
            Phase::Scheduled
        } else {
            Phase::Idle
        };
        true
    }

    pub(crate) fn failed(&mut self, token: FrameToken, retryable: bool) -> bool {
        if self.phase != Phase::Submitted(token) {
            return false;
        }
        self.failures = self.failures.saturating_add(1);
        if retryable && self.failures < 3 {
            self.phase = Phase::Scheduled;
            self.dirty = true;
            true
        } else {
            self.remove();
            false
        }
    }

    pub(crate) fn pause(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.phase = Phase::Paused;
        self.dirty = true;
    }

    pub(crate) fn resume(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.phase = Phase::Scheduled;
        self.dirty = true;
        self.failures = 0;
    }

    pub(crate) fn remove(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.phase = Phase::Removed;
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_frame_idle_and_damage_while_submitted() {
        let mut frames = FrameSchedule::default();
        let first = frames.begin().unwrap();
        assert_eq!(frames.begin(), None);
        assert!(frames.complete(first));
        assert_eq!(frames.phase(), Phase::Idle);
        assert_eq!(frames.begin(), None);
        frames.damage();
        let second = frames.begin().unwrap();
        frames.damage();
        assert!(frames.complete(second));
        assert_eq!(frames.phase(), Phase::Scheduled);
        let third = frames.begin().unwrap();
        assert!(!frames.complete(second));
        assert!(frames.complete(third));
        assert_eq!(frames.phase(), Phase::Idle);
    }

    #[test]
    fn pause_removal_and_recovery_reject_old_completions() {
        let mut frames = FrameSchedule::default();
        let old = frames.begin().unwrap();
        frames.pause();
        frames.damage();
        assert_eq!(frames.begin(), None);
        assert!(!frames.complete(old));
        frames.resume();
        let resumed = frames.begin().unwrap();
        assert!(!frames.complete(old));
        frames.remove();
        assert!(!frames.complete(resumed));
        frames.damage();
        assert_eq!(frames.begin(), None);
        frames.resume();
        assert!(frames.begin().is_some());
    }

    #[test]
    fn retry_budget_is_bounded_and_old_errors_cannot_cancel_new_frames() {
        let mut frames = FrameSchedule::default();
        for attempt in 0..3 {
            let token = frames.begin().unwrap();
            assert_eq!(frames.failed(token, true), attempt < 2);
            assert!(!frames.failed(token, true));
        }
        assert_eq!(frames.phase(), Phase::Removed);
        frames.damage();
        assert_eq!(frames.begin(), None);
        frames.resume();
        let token = frames.begin().unwrap();
        assert!(frames.complete(token));
        frames.damage();
        let fatal = frames.begin().unwrap();
        assert!(!frames.failed(fatal, false));
        assert_eq!(frames.phase(), Phase::Removed);
    }
}
