//! The login screen's side of `auth-v1`.
//!
//! The login screen is not the authentication boundary and must not become one:
//! this module speaks the contract, and nothing here can check a password, open a
//! session or invent an answer. What it *can* do is refuse to keep talking, which
//! is why every failure below ends as [`Report::Unreachable`] and every report the
//! model acts on comes from a reply the boundary actually sent.
//!
//! Four decisions are worth reading before changing anything here.
//!
//! - **One request per connection.** The socket is opened, used and closed, and the
//!   server closes it too. Nothing in this crate holds a connection across a UI
//!   decision, which is what makes "cancel" mean something: the accepted attempt is
//!   continued by a *later* connection, when the person has actually asked for the
//!   session (ADR 0029).
//! - **The credential goes one way.** It arrives as a [`Secret`] and is moved into
//!   the encoder here; there is no path on which this module keeps a copy, prints
//!   one, or compares one.
//! - **The deadline is longer than the boundary's.** The server gives up on an
//!   attempt after 30 s; this client waits a little longer (35 s) so that the
//!   answer "the boundary failed" comes from the boundary rather than from a client
//!   that stopped listening a moment too early.
//! - **Blocking I/O in an `async fn` is deliberate.** These run on iced's thread
//!   pool through [`iced::Task::perform`], one attempt at a time, and the work is a
//!   connect, two writes and two reads with a timeout on each. Making it a real
//!   reactor would mean another runtime in the login screen to do the same thing.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use inox_auth::wire::{self, Reply, Request};
use inox_common::secret::Secret;
use tracing::{debug, warn};

/// How long the login screen waits for the boundary.
pub const DEADLINE: Duration = Duration::from_secs(35);

/// What the boundary did about one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    /// The credentials were accepted; the session may be asked for.
    Accepted,
    /// The credentials were not accepted.
    Rejected,
    /// The boundary could not answer, and said so.
    Unavailable,
    /// There was no boundary to ask, or it spoke something this build does not.
    Unreachable,
    /// The session was started (`true`) or would not start.
    SessionStarted(bool),
}

/// Whether a boundary is listening at `socket`.
///
/// Used before the window opens: a login screen that could collect an identity and
/// a password and only then discover there is nobody to check them would be a
/// screen that wastes the one thing it must not waste.
pub fn is_available(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

/// Asks the boundary to check `credential` for `username`.
pub async fn authenticate(
    socket: PathBuf,
    attempt: u64,
    username: String,
    credential: Secret,
) -> Report {
    let request = Request::Authenticate {
        attempt,
        username,
        credential,
    };

    match converse(&socket, request) {
        Ok(Reply::Accepted { .. }) => Report::Accepted,
        Ok(Reply::Rejected { .. }) => Report::Rejected,
        Ok(Reply::Unavailable { reason, .. }) => {
            debug!(?reason, "the boundary could not answer the attempt");
            Report::Unavailable
        }
        Ok(other) => {
            warn!(
                ?other,
                "the boundary answered an attempt with something else"
            );
            Report::Unreachable
        }
        Err(error) => {
            debug!(%error, "the boundary could not be reached");
            Report::Unreachable
        }
    }
}

/// Asks the boundary to start the session for an attempt it accepted.
pub async fn start_session(socket: PathBuf, attempt: u64) -> Report {
    match converse(&socket, Request::StartSession { attempt }) {
        Ok(Reply::SessionStarted { started, .. }) => Report::SessionStarted(started),
        Ok(other) => {
            warn!(
                ?other,
                "the boundary answered a session start with something else"
            );
            Report::Unreachable
        }
        Err(error) => {
            debug!(%error, "the boundary could not be reached to start the session");
            Report::Unreachable
        }
    }
}

/// Tells the boundary to abandon an attempt.
///
/// The answer — there is none — is not something the screen waits for: the attempt
/// is already over as far as the person is concerned, and a cancellation that never
/// arrives simply expires on the boundary's side.
pub async fn cancel(socket: PathBuf, attempt: u64) {
    // The boundary does not answer a cancellation, so the conversation stops after
    // the write. A failure is logged rather than surfaced: cancelling is what the
    // user asked for, and there is nothing left for them to do about it.
    if let Err(error) = converse(&socket, Request::Cancel { attempt }) {
        debug!(%error, "the boundary could not be told about the cancellation");
    }
}

/// One connection: handshake, one request, one answer.
fn converse(socket: &Path, request: Request) -> Result<Reply, io::Error> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(DEADLINE))?;
    stream.set_write_timeout(Some(DEADLINE))?;

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

/// A wire error as an I/O error, for the one `Result` this module reports in.
fn to_io(error: wire::WireError) -> io::Error {
    match error {
        wire::WireError::Io(error) => error,
        other => io::Error::other(other),
    }
}
