//! The real PAM boundary, exercised as far as a CI runner can take it.
//!
//! These tests link and call the real `libpam`: a `pam_start` happens, a
//! conversation answers, a return code comes back and is mapped. What they cannot
//! show is that any particular PAM stack authenticates anybody — that needs
//! `/etc/pam.d/inox-auth`, a real account and a real password, which is an E4 step
//! for a person at a keyboard (ADR 0029 §验收). On a runner without that file,
//! Linux-PAM falls back to `other`, which denies, so what is asserted here is the
//! property that matters in CI: **a wrong password is a verdict, not a panic**, and
//! **a session PAM will not open is an error, not a panic** — including the session
//! leader's session-only stack, which is the half ADR 0037 §决策 5 added.

#![cfg(feature = "server")]

use std::path::PathBuf;

use inox_auth::boundary::{Boundary, Verdict};
use inox_auth::pam::{self, PamBoundary};
use inox_auth::session::Account;
use inox_common::secret::Secret;

/// An account that exists on every machine and is refused by the boundary's policy;
/// what these tests exercise is PAM's answer, not the policy.
fn account() -> Account {
    Account {
        username: "nobody".to_owned(),
        uid: 65534,
        gid: 65534,
        groups: vec![65534],
        home: PathBuf::from("/nonexistent"),
        shell: PathBuf::from("/usr/bin/nologin"),
    }
}

#[test]
fn a_wrong_password_is_a_verdict_and_not_a_panic() {
    let mut boundary = PamBoundary::new("inox-auth");

    let verdict = boundary.authenticate("inox-no-such-account", &Secret::from_bytes(b"wrong"));

    assert!(
        matches!(verdict, Verdict::Rejected | Verdict::Unavailable(_)),
        "a machine with no such stack must not accept anybody: {verdict:?}"
    );
    assert_ne!(verdict, Verdict::Accepted);
}

#[test]
fn a_refused_attempt_leaves_the_boundary_usable_for_the_next_one() {
    let mut boundary = PamBoundary::new("inox-auth");

    let first = boundary.authenticate("inox-no-such-account", &Secret::from_bytes(b"one"));
    let second = boundary.authenticate("inox-no-such-account", &Secret::from_bytes(b"two"));

    assert!(!matches!(first, Verdict::Accepted));
    assert!(
        !matches!(second, Verdict::Accepted),
        "a second attempt on the same boundary must not inherit the first one's state"
    );
}

#[test]
fn the_credential_is_not_needed_after_the_authentication() {
    // The conversation clears its copy the moment `pam_authenticate` returns, and
    // nothing else in the boundary holds one: the session is opened in a different
    // process (ADR 0037 §决策 5), by a boundary that never sees a credential.
    let mut boundary = PamBoundary::new("inox-auth");
    let credential = Secret::from_bytes(b"hunter2");

    let _ = boundary.authenticate("inox-no-such-account", &credential);
    drop(credential);
}

#[test]
fn a_session_pam_will_not_open_is_an_error_and_not_a_panic() {
    // The session-only stack is not installed on a CI runner (and on a machine
    // where it is, an unprivileged test process cannot open a session for `nobody`).
    // Either way the answer is an error this crate can report, never a crash.
    let outcome = pam::open_session("inox-auth-session", &account());

    assert!(
        outcome.is_err(),
        "a session this process may not open must be refused, not reported as opened"
    );
}
