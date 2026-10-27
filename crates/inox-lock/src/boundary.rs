//! This screen's side of `auth-v1`.
//!
//! `inox-lock` is not the authentication boundary and must not become one: this
//! module speaks the contract in `inox-protocols/protocol/auth-v1.md` to the
//! privileged helper in `inox-auth`, and nothing here can check a password, open a
//! session or invent an answer. What it *can* do is refuse to keep talking, which
//! is why every failure below ends as a [`ServiceFailure`] or an
//! [`AuthFailure::Unavailable`] and every report the state acts on comes from a
//! reply the helper actually sent.
//!
//! Five decisions are worth reading before changing anything here.
//!
//! - **One request per connection.** The socket is opened, used and closed, and the
//!   helper closes it too. Nothing here holds a connection across a UI decision,
//!   so a cancelled attempt is a real cancellation on the helper's side rather than
//!   a decision this process keeps to itself.
//! - **The credential goes one way.** It arrives as an [`AuthAttempt`], which owns
//!   the [`crate::credential::Secret`]; the bytes are copied once into the wire's
//!   own zeroing buffer, encoded straight into the socket, and both buffers are
//!   cleared when they go out of scope. There is no path on which this module
//!   keeps a copy, prints one, or compares one.
//! - **The account is the uid this process runs as.** The lock screen runs inside
//!   the session, so the kernel already knows whose session it is: the name comes
//!   from the password database entry for `getuid()`, never from the environment,
//!   which the person at the keyboard controls. A name that cannot be resolved is a
//!   boundary this screen cannot use, not a guess.
//! - **The deadline is longer than the helper's.** The helper gives up on an
//!   attempt after 30 s; this client waits a little longer (35 s) so that the answer
//!   "the boundary failed" comes from the boundary rather than from a client that
//!   stopped listening a moment too early.
//! - **The request runs on its own thread.** The lock screen's event loop is a
//!   `calloop` loop serving the compositor; a blocking socket conversation inside it
//!   would stop the frames that keep the session covered. Each attempt therefore
//!   becomes one detached thread whose [`Report`] comes back through a `calloop`
//!   channel, and the loop is what decides what the report means.
//!
//! What this module does *not* do: it does not start a session
//! ([`inox_auth::wire::Request::StartSession`] belongs to the login screen), it does
//! not cancel an attempt on the helper's side (an abandoned attempt is refused by
//! the generation check in [`crate::app::update`]), and it never reports a success
//! the helper did not send.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use calloop::channel::Sender;
use inox_auth::wire::{self, Reply, Request};
use inox_common::secret::Secret as WireSecret;
use tracing::{debug, warn};

use crate::contract::{AuthAttempt, AuthFailure, AuthOutcome, Generation, ServiceFailure};

/// How long this client waits for the helper before it gives up.
///
/// The helper's own attempt deadline is 30 s, so this is that deadline plus the
/// time a refused connection needs to be noticed.
pub const DEADLINE: Duration = Duration::from_secs(35);

/// The socket the helper listens on unless the environment names another one.
pub const DEFAULT_SOCKET: &str = "/run/inox/auth.sock";

/// The variable that overrides [`DEFAULT_SOCKET`].
pub const SOCKET_VARIABLE: &str = "INOX_AUTH_SOCKET";

/// What one attempt ended in, on its way back to the event loop.
///
/// Every report names the generation it is about, because the state only obeys
/// reports about the attempt it still has in flight
/// ([`crate::contract::BoundaryState::is_current`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    /// The helper answered the attempt.
    Answered {
        /// The attempt the answer is about.
        generation: Generation,
        /// What the helper said.
        outcome: AuthOutcome,
    },
    /// The helper could not be asked, or could not answer.
    Unavailable {
        /// The attempt that went unanswered.
        generation: Generation,
        /// What is wrong with the boundary.
        failure: ServiceFailure,
    },
}

impl Report {
    /// Which attempt this report is about.
    pub const fn generation(self) -> Generation {
        match self {
            Self::Answered { generation, .. } | Self::Unavailable { generation, .. } => generation,
        }
    }
}

/// The socket the helper listens on, honouring [`SOCKET_VARIABLE`].
pub fn socket() -> PathBuf {
    match std::env::var_os(SOCKET_VARIABLE) {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => PathBuf::from(DEFAULT_SOCKET),
    }
}

/// Whether a helper is listening at `socket`.
///
/// Asked before the lock is taken, so the first frame is honest: a screen that
/// collected a password and only then discovered there is nobody to check it would
/// be a screen that wasted the one thing it must not waste.
pub fn is_available(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

/// The account this process authenticates as: the password-database name for the
/// uid the lock screen runs under.
///
/// Deliberately not `USER` or any other environment variable: the environment is
/// what the person at the keyboard can edit, and a lock screen that could be
/// pointed at another account would be a lock screen that unlocks somebody else's
/// session with somebody else's password.
pub fn session_account() -> Option<String> {
    let uid = rustix::process::getuid();
    let user = users::get_user_by_uid(uid.as_raw())?;
    let name = user.name().to_str()?;

    if !wire::valid_username(name) {
        warn!("the account this session runs as is not a name the contract accepts");

        return None;
    }

    Some(name.to_owned())
}

/// Asks the helper to check `attempt`'s credential for `username`.
///
/// The conversation happens on its own thread and its [`Report`] is delivered
/// through `reports`. Nothing is joined: a request that outlives the lock screen
/// sends its answer to a closed channel and ends, which is the right cost for "the
/// session was released while the boundary was thinking".
pub fn authenticate(
    socket: PathBuf,
    attempt: AuthAttempt,
    username: String,
    reports: &Sender<Report>,
) {
    let generation = attempt.generation();
    // Two handles: one for the thread, one kept here for the case where the thread
    // cannot be started at all.
    let delivery = reports.clone();
    let reports = reports.clone();

    let spawned = thread::Builder::new()
        .name("inox-lock-auth".to_owned())
        .spawn(move || {
            let report = ask(&socket, attempt, username, DEADLINE);

            if delivery.send(report).is_err() {
                debug!(%generation, "the lock screen is gone; the answer was dropped");
            }
        });

    if let Err(error) = spawned {
        // The attempt still has to be accounted for: the state is waiting for an
        // answer about it, and an answer that never arrives would leave the screen
        // saying "checking" for the rest of the lock.
        warn!(%error, "the authentication thread could not be started");
        let _ = reports.send(Report::Unavailable {
            generation,
            failure: ServiceFailure::Disconnected,
        });
    }
}

/// The conversation itself: connect, greet, ask, answer.
fn ask(socket: &Path, attempt: AuthAttempt, username: String, deadline: Duration) -> Report {
    let generation = attempt.generation();

    let request = Request::Authenticate {
        attempt: generation.get(),
        username,
        // One copy, into the wire's own zeroing buffer: `Request` owns the
        // credential it encodes, and the bytes leave this process only inside the
        // frame.
        credential: WireSecret::from_bytes(attempt.expose()),
    };

    match converse(socket, request, deadline) {
        Ok(Reply::Accepted { attempt }) if attempt == generation.get() => Report::Answered {
            generation,
            outcome: AuthOutcome::Success,
        },
        Ok(Reply::Rejected { attempt }) if attempt == generation.get() => Report::Answered {
            generation,
            outcome: AuthOutcome::Failure(AuthFailure::Rejected),
        },
        // The helper could not answer: busy, rate limited, PAM failed. That is a
        // failed attempt, not a successful one, and the session stays locked.
        Ok(Reply::Unavailable { attempt, reason }) if attempt == generation.get() => {
            debug!(?reason, "the boundary could not answer the attempt");

            // The four reasons are four different things to say to a person: a
            // busy boundary is not a rejected password, and an exhausted allowance
            // is not "the service is down".
            let failure = match reason {
                wire::UnavailableReason::NotProvided => AuthFailure::Unavailable,
                wire::UnavailableReason::Busy => AuthFailure::Busy,
                wire::UnavailableReason::RateLimited => AuthFailure::RateLimited,
                wire::UnavailableReason::Failed => AuthFailure::AuthenticatorFailed,
            };

            Report::Answered {
                generation,
                outcome: AuthOutcome::Failure(failure),
            }
        }
        Ok(other) => {
            // A reply about another attempt, a session start this client never
            // asked for, or a frame error: none of them is an answer to *this*
            // attempt, and none of them may be read as one.
            warn!(
                ?other,
                "the boundary answered with something this attempt cannot use"
            );

            Report::Unavailable {
                generation,
                failure: ServiceFailure::Disconnected,
            }
        }
        Err(error) if timed_out(&error) => {
            warn!(%error, "the boundary did not answer in time");

            Report::Unavailable {
                generation,
                failure: ServiceFailure::Timeout,
            }
        }
        Err(error) => {
            debug!(%error, "the boundary could not be reached");

            Report::Unavailable {
                generation,
                failure: ServiceFailure::Disconnected,
            }
        }
    }
}

/// One connection: handshake, one request, one answer.
fn converse(socket: &Path, request: Request, deadline: Duration) -> Result<Reply, io::Error> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(deadline))?;
    stream.set_write_timeout(Some(deadline))?;

    wire::write_request(
        &mut stream,
        &Request::Hello {
            version: wire::VERSION,
        },
    )
    .map_err(to_io)?;

    match wire::read_reply(&mut stream).map_err(to_io)? {
        Some(Reply::Welcome { version }) if version == wire::VERSION => {}
        Some(Reply::Welcome { version }) => {
            return Err(io::Error::other(format!(
                "the boundary speaks version {version}, this build speaks {}",
                wire::VERSION
            )));
        }
        other => {
            return Err(io::Error::other(format!(
                "the boundary did not greet this client: {other:?}"
            )));
        }
    }

    wire::write_request(&mut stream, &request).map_err(to_io)?;

    wire::read_reply(&mut stream)
        .map_err(to_io)?
        .ok_or_else(|| io::Error::other("the boundary closed the connection without answering"))
}

/// Whether an I/O error is the deadline expiring rather than the socket failing.
fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// A wire error as an I/O error, for the one `Result` this module reports in.
fn to_io(error: wire::WireError) -> io::Error {
    match error {
        wire::WireError::Io(error) => error,
        other => io::Error::other(other),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::thread::{self, JoinHandle};

    use super::*;
    use crate::credential::Secret;

    fn attempt(generation: u64, typed: &str) -> AuthAttempt {
        let mut credential = Secret::default();

        for character in typed.chars() {
            credential.push(character);
        }

        AuthAttempt::new(Generation::new(generation), credential)
    }

    /// A helper that speaks one conversation and then ends.
    ///
    /// `answer` is what it says to the `Authenticate` frame; `None` means it says
    /// nothing at all and holds the connection until the client gives up.
    fn helper(
        root: &tempfile::TempDir,
        greet: Option<Reply>,
        answer: Option<Reply>,
    ) -> (PathBuf, JoinHandle<()>) {
        let path = root.path().join("auth.sock");
        let listener = UnixListener::bind(&path).expect("the test helper binds its socket");
        let path_clone = path.clone();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("the client connects");

            if let Some(greet) = greet {
                let _ = wire::read_request(&mut stream).expect("the hello frame");
                let _ = wire::write_reply(&mut stream, &greet);

                // A client that refused the greeting says nothing else, which is
                // itself the behaviour under test in one of these cases.
                if let Ok(Some(Request::Authenticate { .. })) = wire::read_request(&mut stream) {
                    if let Some(answer) = answer {
                        let _ = wire::write_reply(&mut stream, &answer);
                    } else {
                        // Hold the connection open, saying nothing, until the
                        // client gives up and closes its end.
                        let mut sink = Vec::new();
                        let _ = io::Read::read_to_end(&mut stream, &mut sink);
                    }
                }
            }
        });

        (path_clone, handle)
    }

    #[test]
    fn an_accepted_attempt_is_a_success_report() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let (path, helper) = helper(
            &root,
            Some(Reply::Welcome {
                version: wire::VERSION,
            }),
            Some(Reply::Accepted { attempt: 7 }),
        );

        let report = ask(
            &path,
            attempt(7, "correct horse"),
            "ada".to_owned(),
            DEADLINE,
        );

        assert_eq!(
            report,
            Report::Answered {
                generation: Generation::new(7),
                outcome: AuthOutcome::Success,
            }
        );
        assert_eq!(report.generation(), Generation::new(7));
        helper.join().expect("the test helper ends");
    }

    #[test]
    fn a_rejected_attempt_is_an_answered_failure() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let (path, helper) = helper(
            &root,
            Some(Reply::Welcome {
                version: wire::VERSION,
            }),
            Some(Reply::Rejected { attempt: 3 }),
        );

        let report = ask(&path, attempt(3, "wrong"), "ada".to_owned(), DEADLINE);

        assert_eq!(
            report,
            Report::Answered {
                generation: Generation::new(3),
                outcome: AuthOutcome::Failure(AuthFailure::Rejected),
            }
        );
        helper.join().expect("the test helper ends");
    }

    #[test]
    fn a_boundary_that_cannot_answer_is_a_failed_attempt_not_a_success() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let (path, helper) = helper(
            &root,
            Some(Reply::Welcome {
                version: wire::VERSION,
            }),
            Some(Reply::Unavailable {
                attempt: 5,
                reason: wire::UnavailableReason::RateLimited,
            }),
        );

        let report = ask(&path, attempt(5, "typed"), "ada".to_owned(), DEADLINE);

        assert_eq!(
            report,
            Report::Answered {
                generation: Generation::new(5),
                outcome: AuthOutcome::Failure(AuthFailure::RateLimited),
            },
            "a helper that could not check the password has not accepted it, and an \
             exhausted allowance is what it actually said"
        );
        helper.join().expect("the test helper ends");
    }

    #[test]
    fn an_answer_about_another_attempt_is_not_an_answer_to_this_one() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let (path, helper) = helper(
            &root,
            Some(Reply::Welcome {
                version: wire::VERSION,
            }),
            // A success, but for an attempt this client did not make: reading it as
            // this attempt's answer is exactly the replay the generation exists to
            // refuse.
            Some(Reply::Accepted { attempt: 99 }),
        );

        let report = ask(&path, attempt(4, "typed"), "ada".to_owned(), DEADLINE);

        assert_eq!(
            report,
            Report::Unavailable {
                generation: Generation::new(4),
                failure: ServiceFailure::Disconnected,
            }
        );
        helper.join().expect("the test helper ends");
    }

    #[test]
    fn a_silent_boundary_ends_in_a_timeout() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let (path, helper) = helper(
            &root,
            Some(Reply::Welcome {
                version: wire::VERSION,
            }),
            None,
        );

        let report = ask(
            &path,
            attempt(2, "typed"),
            "ada".to_owned(),
            Duration::from_millis(150),
        );

        assert_eq!(
            report,
            Report::Unavailable {
                generation: Generation::new(2),
                failure: ServiceFailure::Timeout,
            }
        );
        helper.join().expect("the test helper ends");
    }

    #[test]
    fn a_missing_socket_is_a_boundary_this_screen_cannot_use() {
        let root = tempfile::tempdir().expect("a temporary directory");

        let report = ask(
            &root.path().join("nothing-here.sock"),
            attempt(1, "typed"),
            "ada".to_owned(),
            DEADLINE,
        );

        assert_eq!(
            report,
            Report::Unavailable {
                generation: Generation::new(1),
                failure: ServiceFailure::Disconnected,
            }
        );
        assert!(!is_available(&root.path().join("nothing-here.sock")));
    }

    #[test]
    fn a_boundary_speaking_another_version_is_not_this_boundary() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let (path, helper) = helper(&root, Some(Reply::Welcome { version: 2 }), None);

        let report = ask(&path, attempt(1, "typed"), "ada".to_owned(), DEADLINE);

        assert_eq!(
            report,
            Report::Unavailable {
                generation: Generation::new(1),
                failure: ServiceFailure::Disconnected,
            }
        );
        helper.join().expect("the test helper ends");
    }

    #[test]
    fn the_account_is_the_one_the_process_uid_names() {
        let uid = rustix::process::getuid();
        let expected = users::get_user_by_uid(uid.as_raw())
            .and_then(|user| user.name().to_str().map(str::to_owned));

        assert_eq!(
            session_account(),
            expected.filter(|name| wire::valid_username(name)),
            "the account comes from the password database entry for this uid, not \
             from the environment"
        );
    }

    #[test]
    fn a_socket_is_available_once_a_helper_listens_on_it() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let path = root.path().join("auth.sock");
        let listener = UnixListener::bind(&path).expect("the test helper binds its socket");

        assert!(is_available(&path));

        drop(listener);
        // The listener is gone, and connecting now fails the way a stopped helper
        // fails: the screen must not offer a prompt it cannot check.
        let _ = UnixStream::connect(&path).is_err();
        assert!(!is_available(&path));
    }
}
