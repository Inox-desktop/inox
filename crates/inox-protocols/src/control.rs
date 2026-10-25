//! `session-control-v1`: the one request a session accepts from its own desktop.
//!
//! The session supervisor owns one capability nobody else has: it is the only
//! process holding the session-lock token, so it is the only process that can
//! start the trusted lock client. Manual and idle-triggered locking therefore need
//! a way to *ask* it — this protocol is that way, and it is deliberately the
//! smallest possible one:
//!
//! ```text
//!   client ── connect ──▶ session control socket (0700 directory)
//!          ── "lock\n" ─▶  supervisor starts the lock service
//!          ◀── "ok\n"  ──  (dispatched, not "the session is locked")
//! ```
//!
//! Four decisions are worth reading before changing anything here.
//!
//! - **One verb, no arguments.** `lock` is the whole vocabulary. Anything that
//!   could name a command, an account, a display or a token would turn this socket
//!   into a way to make the session do something, and the point is that the worst a
//!   caller can do is ask to be locked out. Transport values (which program, which
//!   environment, which token) stay startup configuration.
//! - **A line, not a frame.** The request is one short line of ASCII, bounded by
//!   [`MAX_FRAME`], and a request that is longer, unterminated or not a known verb
//!   is refused rather than parsed leniently.
//! - **The reply is a dispatch receipt, not a lock confirmation.** `ok` means the
//!   supervisor asked the lock client to start. Whether the session is actually
//!   covered is the compositor's answer, reported on `inox-ipc`'s
//!   `session_lock_state`; a caller that treats `ok` as "locked" would be trusting
//!   the wrong half of the system.
//! - **Peer identity is checked by the server, not by this module.** The types and
//!   the framing live here so both sides agree; the socket's mode, directory
//!   ownership and `SO_PEERCRED` check belong to whoever binds it
//!   (`inox-session`), because it is the session's capability and not a library's.
//!
//! The normative text is `crates/inox-protocols/protocol/session-control-v1.md`.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// The longest request or reply this protocol accepts, in bytes.
pub const MAX_FRAME: usize = 64;

/// The directory under `XDG_RUNTIME_DIR` that holds the control socket.
pub const DIRECTORY: &str = "inox-session";

/// The socket's file name inside [`DIRECTORY`].
pub const SOCKET: &str = "control.sock";

/// The variable that overrides the computed socket path.
///
/// Used by tests and by a deployment that keeps its runtime directory somewhere
/// else; it never appears in the protocol itself.
pub const SOCKET_VARIABLE: &str = "INOX_SESSION_CONTROL";

/// Everything a caller may ask a session to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Start the trusted lock client, if it is not already running.
    Lock,
}

impl Request {
    /// The verb, as it travels on the wire.
    pub const fn verb(self) -> &'static str {
        match self {
            Self::Lock => "lock",
        }
    }
}

/// What the session did about a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// The request was accepted and the supervisor has asked the service to start.
    ///
    /// Not a confirmation that the session is covered: that comes from the
    /// compositor on `inox-ipc`.
    Dispatched,
    /// There is nothing to start: the session is not ready, or a lock request is
    /// already in flight.
    Busy,
    /// This session has no such service. A caller reading it should ask a newer
    /// session, not retry.
    Unknown,
}

impl Reply {
    /// The word, as it travels on the wire.
    pub const fn word(self) -> &'static str {
        match self {
            Self::Dispatched => "ok",
            Self::Busy => "busy",
            Self::Unknown => "unknown",
        }
    }
}

/// Why a request or reply could not be read.
///
/// Hand-written rather than derived: this crate sits on `wayland-rs` alone and
/// takes no error-handling dependency for one error type.
#[derive(Debug)]
pub enum ControlError {
    /// The socket failed.
    Io(io::Error),
    /// The line was not a verb this build knows.
    UnknownVerb,
    /// The line was too long, empty, or not terminated.
    Malformed,
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "control transport failed: {error}"),
            Self::UnknownVerb => f.write_str("not a control request this build understands"),
            Self::Malformed => f.write_str("malformed control frame"),
        }
    }
}

impl std::error::Error for ControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::UnknownVerb | Self::Malformed => None,
        }
    }
}

impl From<io::Error> for ControlError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The socket path a session under `runtime_dir` listens on.
pub fn socket_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join(DIRECTORY).join(SOCKET)
}

/// Writes one request and its newline.
pub fn write_request(stream: &mut impl Write, request: Request) -> io::Result<()> {
    stream.write_all(request.verb().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

/// Writes one reply and its newline.
pub fn write_reply(stream: &mut impl Write, reply: Reply) -> io::Result<()> {
    stream.write_all(reply.word().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

/// Reads one request line.
///
/// # Errors
///
/// [`ControlError::Malformed`] for an empty, unterminated or over-long line, and
/// [`ControlError::UnknownVerb`] for a line that is not a verb of this protocol.
pub fn read_request(stream: &mut impl Read) -> Result<Request, ControlError> {
    match read_line(stream)? {
        line if line == Request::Lock.verb() => Ok(Request::Lock),
        _ => Err(ControlError::UnknownVerb),
    }
}

/// Reads one reply line.
///
/// # Errors
///
/// The same shapes as [`read_request`]: an unknown word is a reply this build
/// cannot act on, and is reported rather than guessed at.
pub fn read_reply(stream: &mut impl Read) -> Result<Reply, ControlError> {
    match read_line(stream)? {
        line if line == Reply::Dispatched.word() => Ok(Reply::Dispatched),
        line if line == Reply::Busy.word() => Ok(Reply::Busy),
        line if line == Reply::Unknown.word() => Ok(Reply::Unknown),
        _ => Err(ControlError::UnknownVerb),
    }
}

/// One newline-terminated line of at most [`MAX_FRAME`] bytes.
fn read_line(stream: &mut impl Read) -> Result<String, ControlError> {
    let mut bytes = Vec::new();

    for _ in 0..=MAX_FRAME {
        let mut byte = [0u8; 1];

        match stream.read(&mut byte) {
            Ok(0) => return Err(ControlError::Malformed),
            Ok(_) if byte[0] == b'\n' => {
                if bytes.is_empty() {
                    return Err(ControlError::Malformed);
                }

                return String::from_utf8(bytes).map_err(|_| ControlError::UnknownVerb);
            }
            Ok(_) => bytes.push(byte[0]),
            Err(error) => return Err(ControlError::Io(error)),
        }
    }

    Err(ControlError::Malformed)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    fn round_trip_request(request: Request) -> Request {
        let mut buffer = Vec::new();
        write_request(&mut buffer, request).expect("the request is writable");
        read_request(&mut Cursor::new(buffer)).expect("the request reads back")
    }

    fn round_trip_reply(reply: Reply) -> Reply {
        let mut buffer = Vec::new();
        write_reply(&mut buffer, reply).expect("the reply is writable");
        read_reply(&mut Cursor::new(buffer)).expect("the reply reads back")
    }

    #[test]
    fn the_one_verb_round_trips() {
        assert_eq!(round_trip_request(Request::Lock), Request::Lock);
    }

    #[test]
    fn every_reply_round_trips() {
        for reply in [Reply::Dispatched, Reply::Busy, Reply::Unknown] {
            assert_eq!(round_trip_reply(reply), reply);
        }
    }

    #[test]
    fn the_wire_words_are_what_the_document_says() {
        assert_eq!(Request::Lock.verb(), "lock");
        assert_eq!(Reply::Dispatched.word(), "ok");
        assert_eq!(Reply::Busy.word(), "busy");
        assert_eq!(Reply::Unknown.word(), "unknown");
    }

    #[test]
    fn an_unknown_verb_is_refused_rather_than_ignored() {
        assert!(matches!(
            read_request(&mut Cursor::new(b"unlock\n".to_vec())),
            Err(ControlError::UnknownVerb)
        ));
        assert!(matches!(
            read_reply(&mut Cursor::new(b"maybe\n".to_vec())),
            Err(ControlError::UnknownVerb)
        ));
    }

    #[test]
    fn a_line_without_its_newline_is_malformed() {
        assert!(matches!(
            read_request(&mut Cursor::new(b"lock".to_vec())),
            Err(ControlError::Malformed)
        ));
    }

    #[test]
    fn an_empty_line_is_malformed() {
        assert!(matches!(
            read_request(&mut Cursor::new(b"\n".to_vec())),
            Err(ControlError::Malformed)
        ));
    }

    #[test]
    fn an_over_long_line_is_refused_before_it_is_read_whole() {
        let mut line = vec![b'l'; MAX_FRAME + 8];
        line.push(b'\n');

        assert!(matches!(
            read_request(&mut Cursor::new(line)),
            Err(ControlError::Malformed)
        ));
    }

    #[test]
    fn the_socket_lives_where_the_document_says() {
        assert_eq!(
            socket_path(Path::new("/run/user/1000")),
            Path::new("/run/user/1000/inox-session/control.sock")
        );
    }
}
