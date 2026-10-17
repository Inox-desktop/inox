//! The session lock's own bookkeeping: who may lock, and when the screen is covered.
//!
//! This module is pure: no Smithay types, no protocol objects, no I/O. The
//! protocol mechanics — the lock object's lifecycle, lock surfaces, the
//! configure/ack handshake and the protocol errors — are Smithay's
//! (`smithay::wayland::session_lock`, `SessionLockHandler`), and the compositor's
//! `state` module owns them. What is left here are the two decisions ADR 0015 and
//! ADR 0022 make the compositor responsible for, because getting them wrong is a
//! security failure rather than a protocol failure:
//!
//! - [`claim`]: may this connection take the trusted lock role? The answer is a
//!   capability the session delivered, never something the client says about
//!   itself.
//! - [`LockCoverage`]: has a locked frame been presented on **every** output yet?
//!   `ext-session-lock-v1` forbids sending `locked` before it has, because a
//!   client that is told it owns the screen may immediately suspend the machine.
//!
//! Both are unit tested and property tested, which is why they live here instead
//! of inside the protocol handlers.

use std::collections::BTreeSet;

use crate::topology::OutputId;

/// The outcome of a client asking for the trusted lock role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    /// The token matched the one the session delivered.
    Authorized,
    /// The compositor has no token this run, so it cannot authorize anyone.
    NoToken,
    /// The presented token is not the session's.
    Mismatch,
}

impl Claim {
    /// Whether the claim grants the role.
    pub const fn is_authorized(self) -> bool {
        matches!(self, Self::Authorized)
    }

    /// Why the claim was refused, for a log line.
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Authorized => "authorized",
            Self::NoToken => "this compositor has no lock token, so locking is unavailable",
            Self::Mismatch => "the presented token is not this session's",
        }
    }
}

/// Decide a lock-role claim against the token the session delivered.
///
/// An empty presented token is a mismatch rather than a match, so a compositor
/// configured with an empty token cannot be claimed by a client that sends
/// nothing — the configuration layer also rejects an empty `--lock-token`.
pub fn claim(expected: Option<&str>, presented: &str) -> Claim {
    match expected {
        None => Claim::NoToken,
        Some(expected) if expected == presented && !presented.is_empty() => Claim::Authorized,
        Some(_) => Claim::Mismatch,
    }
}

/// The outputs that still owe a locked frame before `locked` may be sent.
///
/// Started by [`LockCoverage::begin`] when a lock is accepted. From then on the
/// compositor may only confirm the lock once every advertised output has composed
/// a frame from the locked scene. [`LockCoverage::sync`] is what keeps the two in
/// step when the topology changes: an output that appears during the lock joins
/// the set (it must be covered too), and one that disappears leaves it — a display
/// that is gone cannot present anything, and waiting for it would let a hot-unplug
/// stall the lock for good.
#[derive(Debug, Default)]
pub struct LockCoverage {
    locking: bool,
    live: BTreeSet<OutputId>,
    covered: BTreeSet<OutputId>,
}

impl LockCoverage {
    /// An idle coverage: no lock is being established.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a lock is being established and not yet confirmed.
    pub const fn is_locking(&self) -> bool {
        self.locking
    }

    /// How many advertised outputs still owe a locked frame.
    pub fn outstanding(&self) -> usize {
        self.live.difference(&self.covered).count()
    }

    /// Begin covering `outputs`. Replaces any coverage in progress.
    pub fn begin(&mut self, outputs: impl IntoIterator<Item = OutputId>) {
        self.locking = true;
        self.live = outputs.into_iter().collect();
        self.covered.clear();
    }

    /// Take `outputs` as the advertised set, keeping what has been covered.
    pub fn sync(&mut self, outputs: impl IntoIterator<Item = OutputId>) {
        self.live = outputs.into_iter().collect();
        self.covered.retain(|id| self.live.contains(id));
    }

    /// Note that `id` composed a frame drawn from the locked scene.
    pub fn presented(&mut self, id: OutputId) {
        if self.locking {
            self.covered.insert(id);
        }
    }

    /// Whether every advertised output has presented a locked frame.
    pub fn is_complete(&self) -> bool {
        self.locking && self.covered.is_superset(&self.live)
    }

    /// Take the completed coverage, confirming the lock exactly once.
    ///
    /// Returns `true` the first time the coverage is complete, and `false` on
    /// every later call: the confirmation is a one-shot event, and a second
    /// `locked` would be a protocol violation.
    pub fn take_completed(&mut self) -> bool {
        if self.is_complete() {
            self.locking = false;
            self.live.clear();
            self.covered.clear();

            return true;
        }

        false
    }

    /// Abandon coverage without confirming: the lock was refused or released.
    pub fn cancel(&mut self) {
        self.locking = false;
        self.live.clear();
        self.covered.clear();
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn outputs(ids: &[u64]) -> Vec<OutputId> {
        ids.iter().copied().map(OutputId::new).collect()
    }

    #[test]
    fn a_claim_needs_the_sessions_token() {
        assert_eq!(claim(Some("s3cret"), "s3cret"), Claim::Authorized);
        assert!(claim(Some("s3cret"), "s3cret").is_authorized());
        assert_eq!(claim(None, "s3cret"), Claim::NoToken);
        assert_eq!(claim(Some("s3cret"), "other"), Claim::Mismatch);
        assert_eq!(claim(Some("s3cret"), ""), Claim::Mismatch);
        assert_eq!(
            claim(Some(""), ""),
            Claim::Mismatch,
            "an empty configured token must not be matchable by an empty claim"
        );
    }

    #[test]
    fn refusals_explain_themselves_without_echoing_the_token() {
        for refusal in [Claim::NoToken, Claim::Mismatch] {
            let reason = refusal.reason();
            assert!(!reason.is_empty());
            assert!(!reason.contains("s3cret"), "{reason}");
        }
    }

    #[test]
    fn coverage_waits_for_every_output_and_confirms_once() {
        let mut coverage = LockCoverage::new();
        assert!(!coverage.is_locking());
        assert!(!coverage.is_complete());

        coverage.begin(outputs(&[1, 2]));
        assert!(coverage.is_locking());
        assert_eq!(coverage.outstanding(), 2);
        assert!(!coverage.take_completed(), "nothing has been presented");

        coverage.presented(OutputId::new(1));
        assert_eq!(coverage.outstanding(), 1);
        assert!(
            !coverage.take_completed(),
            "one output is still showing unlocked content"
        );

        coverage.presented(OutputId::new(2));
        assert!(coverage.is_complete());
        assert!(coverage.take_completed());
        assert!(
            !coverage.take_completed(),
            "the confirmation is a one-shot event"
        );
        assert!(!coverage.is_locking());
    }

    #[test]
    fn no_outputs_means_the_requirement_is_vacuously_met() {
        let mut coverage = LockCoverage::new();
        coverage.begin(std::iter::empty());

        assert!(coverage.is_complete());
        assert!(coverage.take_completed());
    }

    #[test]
    fn an_output_that_appears_while_locking_must_be_covered_too() {
        let mut coverage = LockCoverage::new();
        coverage.begin(outputs(&[1]));
        coverage.presented(OutputId::new(1));
        assert!(coverage.is_complete());

        coverage.sync(outputs(&[1, 2]));
        assert!(
            !coverage.is_complete(),
            "a display plugged in during the lock must be covered before the \
             client is told it owns the screen"
        );
        assert!(!coverage.take_completed());
        assert_eq!(coverage.outstanding(), 1);

        coverage.presented(OutputId::new(2));
        assert!(coverage.take_completed());
    }

    #[test]
    fn an_output_that_disappears_stops_blocking_the_lock() {
        let mut coverage = LockCoverage::new();
        coverage.begin(outputs(&[1, 2]));
        coverage.presented(OutputId::new(1));

        coverage.sync(outputs(&[1]));
        assert!(
            coverage.take_completed(),
            "a display that is gone cannot present a frame, and waiting for it \
             would leave the session covered but never confirmed"
        );
    }

    #[test]
    fn covering_a_returning_output_is_required_again() {
        let mut coverage = LockCoverage::new();
        coverage.begin(outputs(&[1]));
        coverage.presented(OutputId::new(1));

        // Unplugged and plugged back in: it is a different runtime instance, and
        // whatever it shows now has never been rendered from the locked scene.
        coverage.sync(std::iter::empty());
        coverage.sync(outputs(&[1]));
        assert!(!coverage.is_complete());
        assert_eq!(coverage.outstanding(), 1);
    }

    #[test]
    fn a_presentation_after_confirmation_changes_nothing() {
        let mut coverage = LockCoverage::new();
        coverage.begin(outputs(&[1]));
        coverage.presented(OutputId::new(1));
        assert!(coverage.take_completed());

        coverage.presented(OutputId::new(1));
        coverage.sync(outputs(&[1, 3]));
        assert!(!coverage.is_locking());
        assert!(
            !coverage.is_complete(),
            "coverage is over; a later topology change must not re-open it"
        );
    }

    #[test]
    fn cancelling_abandons_coverage_without_confirming() {
        let mut coverage = LockCoverage::new();
        coverage.begin(outputs(&[1]));
        coverage.cancel();

        assert!(!coverage.is_locking());
        assert!(!coverage.is_complete());
        assert!(!coverage.take_completed());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// The lock may never be confirmed while an advertised output is still
        /// outstanding, and may never be confirmed twice.
        #[test]
        fn confirmation_requires_every_advertised_output(
            first in proptest::collection::vec(0u64..5, 0..5),
            live in proptest::collection::vec(0u64..5, 0..5),
            before in proptest::collection::vec(proptest::bool::ANY, 0..5),
            after in proptest::collection::vec(proptest::bool::ANY, 0..5),
        ) {
            let mut coverage = LockCoverage::new();
            coverage.begin(first.iter().copied().map(OutputId::new));

            // What the compositor composed for while the lock was being
            // established, then the topology as it stands afterwards, then what
            // it composed for after that.
            let mut covered: BTreeSet<OutputId> = first
                .iter()
                .enumerate()
                .filter(|(index, _)| before.get(*index).copied().unwrap_or(false))
                .map(|(_, id)| OutputId::new(*id))
                .collect();
            for id in &covered {
                coverage.presented(*id);
            }

            let live: BTreeSet<OutputId> = live.iter().copied().map(OutputId::new).collect();
            coverage.sync(live.iter().copied());
            // An output that is gone is no longer covered; whatever it showed is
            // irrelevant, and a returning one is a new instance that must be
            // covered again.
            covered.retain(|id| live.contains(id));

            for (index, id) in live.iter().enumerate() {
                if after.get(index).copied().unwrap_or(false) {
                    coverage.presented(*id);
                    covered.insert(*id);
                }
            }

            let complete = covered.is_superset(&live);
            prop_assert_eq!(coverage.is_complete(), complete, "{:?} vs {:?}", covered, live);
            prop_assert_eq!(coverage.take_completed(), complete);
            prop_assert!(!coverage.take_completed());
        }
    }
}
