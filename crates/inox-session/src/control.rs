//! The session's control socket: who may ask, and what an ask means.
//!
//! `inox-session` is the only process holding the session-lock token, so it is the
//! only process that can start the trusted lock client. Manual and idle-triggered
//! locking need a way to ask it, and `session-control-v1` is that way: one verb,
//! `lock`, on a private Unix socket under the session's own runtime directory.
//!
//! Four decisions are worth reading before changing anything here.
//!
//! - **The socket is the capability.** The directory is `0700` and owned by the
//!   session's user; the socket is `0600`. `SO_PEERCRED` is checked before a single
//!   byte is read, so a uid that is not ours gets no reply at all. That is why the
//!   protocol carries no token: reaching the socket already proves the caller is
//!   the session's own user, and a token here would only be one more secret to leak.
//! - **A stale socket is not a session.** A path that a previous session left behind
//!   would accept connections that go nowhere; the bind therefore probes the path
//!   first and refuses when somebody still answers on it.
//! - **One request, one reply, close.** There is no session state on this socket and
//!   nothing to keep open: a caller connects, writes a line, reads a line, and is
//!   done. The reply is a dispatch receipt (`ok`), never a claim that the session is
//!   covered — that answer belongs to the compositor's `session_lock_state`.
//! - **The read is bounded.** A client that opens the socket and says nothing must
//!   not hold this process: the stream gets a short read timeout and the line length
//!   is capped by the protocol.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use inox_protocols::control;
use rustix::net::sockopt::socket_peercred;
use tracing::{debug, warn};

/// How long a caller has to say its one line before the connection is dropped.
const READ_TIMEOUT: Duration = Duration::from_millis(250);

/// How long a caller is given to read the reply.
const WRITE_TIMEOUT: Duration = Duration::from_millis(250);

/// The listening half of `session-control-v1`.
pub struct Listener {
    path: PathBuf,
    directory: PathBuf,
    listener: UnixListener,
}

impl Listener {
    /// Binds the control socket at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be created or is not private, or
    /// when another session is still answering on the socket path — in which case
    /// binding would either steal a live session's socket or leave a dead one behind.
    pub fn bind(path: &Path) -> io::Result<Self> {
        let path = path.to_path_buf();

        let directory = path
            .parent()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "the control socket needs a parent directory",
                )
            })?
            .to_path_buf();

        prepare_directory(&directory)?;
        clear_stale_socket(&path)?;

        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        // The directory is already 0700; the socket itself is narrowed too, so the
        // permission that admits a caller is never wider than "this user".
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;

        debug!(path = %path.display(), "the session control socket is listening");

        Ok(Self {
            path,
            directory,
            listener,
        })
    }

    /// A second handle on the same listening socket, for the event loop.
    pub fn handle(&self) -> io::Result<UnixListener> {
        self.listener.try_clone()
    }

    /// Where the socket lives.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One pending connection, or `None` when none is waiting.
    pub fn accept(&self) -> io::Result<Option<UnixStream>> {
        match self.listener.accept() {
            Ok((stream, _)) => Ok(Some(stream)),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path)
            && error.kind() != io::ErrorKind::NotFound
        {
            warn!(%error, path = %self.path.display(), "failed to remove the control socket");
        }

        if let Err(error) = fs::remove_dir(&self.directory)
            && error.kind() != io::ErrorKind::NotFound
        {
            debug!(%error, path = %self.directory.display(), "failed to remove the control directory");
        }
    }
}

/// What one served connection asked for, or why it was dropped.
pub enum Served {
    /// A well-formed request from this user, which the supervisor must answer.
    Request(control::Request, UnixStream),
    /// A connection that was refused or could not be understood. No reply is sent:
    /// a caller that cannot speak the protocol is not owed a diagnostic.
    Dropped,
}

/// Reads one request from `stream`, after checking that the caller is this user.
pub fn serve(mut stream: UnixStream, uid: u32) -> Served {
    let peer = match socket_peercred(&stream) {
        Ok(peer) => peer,
        Err(error) => {
            debug!(%error, "failed to read the peer credentials of a control connection");

            return Served::Dropped;
        }
    };

    if peer.uid.as_raw() != uid {
        warn!(
            peer_uid = peer.uid.as_raw(),
            "refusing a control connection from another user"
        );

        return Served::Dropped;
    }

    if let Err(error) = stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(WRITE_TIMEOUT)))
    {
        debug!(%error, "failed to bound a control connection's timeouts");

        return Served::Dropped;
    }

    match control::read_request(&mut stream) {
        Ok(request) => Served::Request(request, stream),
        Err(error) => {
            debug!(%error, "a control connection did not send a request this build understands");

            Served::Dropped
        }
    }
}

/// The socket path a session uses: the override when it is set, otherwise the
/// conventional place under the session's runtime directory.
pub fn path_for(runtime_dir: &Path) -> PathBuf {
    std::env::var_os(control::SOCKET_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| control::socket_path(runtime_dir))
}

/// Creates the socket's directory as a private directory owned by this user.
fn prepare_directory(directory: &Path) -> io::Result<()> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "the control directory must be a private directory",
                ));
            }

            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(directory),
        Err(error) => Err(error),
    }
}

/// Removes a socket file only when nothing answers on it.
fn clear_stale_socket(path: &Path) -> io::Result<()> {
    match UnixStream::connect(path) {
        // Somebody is listening: that is a live session, and two supervisors must
        // never share the path.
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another session is already listening on the control socket",
        )),
        Err(_) => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn private_dir() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("a temporary directory");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700))
            .expect("private permissions");

        root
    }

    #[test]
    fn binding_creates_a_private_socket_and_dropping_removes_it() {
        let root = private_dir();
        let path = root.path().join("inox-session").join("control.sock");

        let listener = Listener::bind(&path).expect("the socket binds");
        assert!(path.exists(), "the socket exists while the session runs");
        assert_eq!(
            fs::metadata(root.path().join("inox-session"))
                .expect("the directory")
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "the directory admits this user alone"
        );
        assert_eq!(
            fs::metadata(&path)
                .expect("the socket")
                .permissions()
                .mode()
                & 0o777,
            0o600,
            "the socket admits this user alone"
        );

        drop(listener);

        assert!(!path.exists(), "the socket is removed with the session");
        assert!(
            !root.path().join("inox-session").exists(),
            "the directory goes with it"
        );
    }

    #[test]
    fn a_second_session_does_not_steal_a_live_socket() {
        let root = private_dir();
        let path = root.path().join("inox-session").join("control.sock");
        let _live = Listener::bind(&path).expect("the first session binds");

        let second = Listener::bind(&path);

        assert!(second.is_err(), "one session per control socket");
    }

    #[test]
    fn a_stale_socket_file_is_replaced() {
        let root = private_dir();
        let path = root.path().join("inox-session").join("control.sock");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path.parent().expect("a parent"))
            .expect("a private directory");
        fs::write(&path, b"").expect("a stale file");

        let listener = Listener::bind(&path).expect("a stale file is not a session");

        assert!(path.exists());
        drop(listener);
    }

    #[test]
    fn a_well_formed_request_from_this_user_is_served() {
        let (mut client, server) = UnixStream::pair().expect("a socket pair");
        let uid = rustix::process::getuid().as_raw();
        client.write_all(b"lock\n").expect("the request");

        let Served::Request(request, mut stream) = serve(server, uid) else {
            panic!("a well-formed request is served");
        };

        assert_eq!(request, control::Request::Lock);
        control::write_reply(&mut stream, control::Reply::Dispatched).expect("the reply");
        assert_eq!(
            control::read_reply(&mut client).expect("the reply reads"),
            control::Reply::Dispatched
        );
    }

    #[test]
    fn a_connection_that_says_nothing_is_dropped_without_a_reply() {
        let (client, server) = UnixStream::pair().expect("a socket pair");
        let uid = rustix::process::getuid().as_raw();
        drop(client);

        assert!(matches!(serve(server, uid), Served::Dropped));
    }

    #[test]
    fn a_caller_that_is_not_this_user_is_refused() {
        let (mut client, server) = UnixStream::pair().expect("a socket pair");
        client.write_all(b"lock\n").expect("the request");
        // A session whose uid is one that no test process runs as: nothing may be
        // read from the connection, let alone acted on.
        let other_uid = rustix::process::getuid().as_raw().wrapping_add(1);

        assert!(matches!(serve(server, other_uid), Served::Dropped));
    }
}
