//! The idle daemon's side of `session-control-v1`.
//!
//! Idling is a request to lock, not a lock: this module asks the session to start
//! its trusted lock client (`inox_protocols::control`) and hands back exactly what
//! the session said. It has no way to confirm a lock — that answer belongs to the
//! compositor, which reports it on `inox-ipc`'s `session_lock_state` — and it has no
//! way to unlock anything.
//!
//! Two decisions are load-bearing.
//!
//! - **The request is bounded.** The control protocol is one line in and one line
//!   out on a local socket; the timeouts here are short enough that a session that
//!   has stopped answering cannot hold the idle loop (which is also the loop that
//!   must keep watching for activity).
//! - **A missing socket is not a failure of the machine.** `discover` returns
//!   `None` when there is no session to ask; the daemon then refuses the suspend and
//!   says so, rather than retrying forever against a path that will not appear.

use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use inox_protocols::control;

/// How long the session has to answer one request.
const REPLY_TIMEOUT: Duration = Duration::from_millis(250);

/// The socket a request goes to, when there is one.
pub struct LockRequester {
    socket: PathBuf,
}

impl LockRequester {
    /// Finds the session's control socket, if this session has one.
    pub fn discover() -> Option<Self> {
        let socket = match std::env::var_os(control::SOCKET_VARIABLE).filter(|v| !v.is_empty()) {
            Some(path) => PathBuf::from(path),
            None => {
                let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty())?;

                control::socket_path(PathBuf::from(runtime_dir).as_path())
            }
        };

        Some(Self { socket })
    }

    /// Asks the session to start its lock client.
    ///
    /// # Errors
    ///
    /// Returns the I/O error when the session cannot be reached or does not answer
    /// in time. A reply this build does not understand is an error too: it is not
    /// "busy" and it is certainly not "locked".
    pub fn request(&self) -> Result<control::Reply, io::Error> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
        stream.set_write_timeout(Some(REPLY_TIMEOUT))?;

        control::write_request(&mut stream, control::Request::Lock)?;

        control::read_reply(&mut stream).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::os::unix::net::UnixListener;

    use super::*;

    fn session(directory: &tempfile::TempDir, reply: &'static [u8]) -> LockRequester {
        let socket = directory.path().join("control.sock");
        let listener = UnixListener::bind(&socket).expect("the fake session listens");

        std::thread::spawn(move || {
            use std::io::Read as _;

            let (mut stream, _) = listener.accept().expect("the daemon connects");
            let mut line = Vec::new();
            let mut byte = [0u8; 1];

            while let Ok(read) = stream.read(&mut byte) {
                if read == 0 || byte[0] == b'\n' {
                    break;
                }

                line.push(byte[0]);
            }

            assert_eq!(line, b"lock", "the daemon asks for one thing");
            stream.write_all(reply).expect("the reply");
        });

        LockRequester { socket }
    }

    #[test]
    fn the_one_verb_is_sent_and_its_reply_comes_back() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let requester = session(&root, b"ok\n");

        assert_eq!(
            requester.request().expect("the session answers"),
            control::Reply::Dispatched
        );
    }

    #[test]
    fn a_busy_session_is_a_reply_rather_than_an_error() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let requester = session(&root, b"busy\n");

        assert_eq!(
            requester.request().expect("the session answers"),
            control::Reply::Busy
        );
    }

    #[test]
    fn an_unreadable_reply_is_an_error_and_never_a_dispatch() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let requester = session(&root, b"maybe\n");

        assert!(requester.request().is_err());
    }

    #[test]
    fn a_missing_session_is_an_error() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let requester = LockRequester {
            socket: root.path().join("nothing-here.sock"),
        };

        assert!(requester.request().is_err());
    }
}
