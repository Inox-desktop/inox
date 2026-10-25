//! The X11 connection-setup round trip behind the readiness probe.
//!
//! Acceptance item X01 (`docs/quality/acceptance.md` §5) requires readiness to
//! be proved by a *real* X11 round trip — socket existence is explicitly not
//! enough. A stale socket file, a listener that accepts connections and then
//! never answers, or a proxy that stalls after `accept(2)` all pass a
//! "can I connect?" check while no X11 client could ever come up.
//!
//! This module therefore speaks the first exchange of the X11 protocol — the
//! *connection setup* — directly over a [`UnixStream`]. It needs no X11 library
//! and no third-party crate, and every read is bounded so a hung server can
//! never hang the service.
//!
//! # Wire format
//!
//! The client's connection request, 12 bytes because this service sends no
//! authorization data (it relies on the socket's own permissions):
//!
//! ```text
//! offset size field
//! 0 1 byte-order byte, 0x6c = 'l' (least significant byte first)
//! 1 1 unused
//! 2 2 protocol major version (11), little endian
//! 4 2 protocol minor version (0), little endian
//! 6 2 length of the authorization protocol name (0)
//! 8 2 length of the authorization protocol data (0)
//! 10 2 unused
//! ```
//!
//! The server's reply header, 8 bytes, in the byte order the request declared:
//!
//! ```text
//! offset size field
//! 0 1 status: 0 failed, 1 success, 2 authenticate
//! 1 1 length of the failure reason (status 0); unused otherwise
//! 2 2 protocol major version (status 0 and 1 only), little endian
//! 4 2 protocol minor version (status 0 and 1 only), little endian
//! 6 2 length of the data that follows, in 4-byte units
//! ```
//!
//! For a successful setup the following data is the *setup body*: the first 32
//! bytes are fixed, then the vendor string (length declared at offset 16) is
//! followed by the pixmap formats and the screens.
//!
//! ```text
//! offset size field
//! 0 4 release number
//! 4 4 resource-id-base
//! 8 4 resource-id-mask
//! 12 4 motion-buffer-size
//! 16 2 length of the vendor string (0..=65535)
//! 18 2 maximum-request-length
//! 20 1 number of SCREENs
//! 21 1 number of pixmap FORMATs
//! 22 1 image byte order
//! 23 1 bitmap format bit order
//! 24 1 bitmap scanline unit
//! 25 1 bitmap scanline pad
//! 26 1 minimum keycode
//! 27 1 maximum keycode
//! 28 4 unused
//! 32 v vendor string, padded to a multiple of 4 bytes
//! ```
//!
//! For status 0 the following data is the failure reason; for status 2 it is
//! the data the server wants to authenticate with. Both are reported as text so
//! the failure is debuggable from the log.
//!
//! Everything that is not a well formed successful reply — a closed socket, a
//! short reply, a timeout, status 0 or 2, an unknown status byte, a bogus
//! length, trailing data, another protocol major version — is an error, and
//! callers treat any error as "not ready". Nothing here panics.

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Protocol major version requested by this service.
pub const PROTOCOL_MAJOR: u16 = 11;

/// Protocol minor version requested by this service.
pub const PROTOCOL_MINOR: u16 = 0;

/// Default budget for one whole connection-setup round trip.
///
/// The probe runs inside the calloop event loop, between two 100 ms polls, so
/// this bounds how long a hung server can stall signal handling. A real server
/// answers a setup request from the same machine in well under a millisecond,
/// and a probe that does time out is simply repeated on the next poll.
pub const SETUP_TIMEOUT: Duration = Duration::from_millis(500);

/// Length of the connection request prefix (no authorization data).
const REQUEST_LEN: usize = 12;

/// Length of the reply header that precedes the reply data.
const REPLY_HEADER_LEN: usize = 8;

/// Offset of the vendor length inside the setup body.
const VENDOR_LENGTH_OFFSET: usize = 16;

/// Offset of the vendor string inside the setup body.
const VENDOR_OFFSET: usize = 32;

/// The byte-order byte declaring the request to be little endian (`'l'`).
const BYTE_ORDER_LITTLE_ENDIAN: u8 = 0x6c;

/// Reply status reporting a successful setup.
const STATUS_SUCCESS: u8 = 1;

/// Reply status reporting that the server refused the connection.
const STATUS_FAILED: u8 = 0;

/// Reply status asking the client to authenticate.
const STATUS_AUTHENTICATE: u8 = 2;

/// What a successful X11 connection setup reported about the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    vendor: String,
    protocol: (u16, u16),
}

impl Setup {
    /// The vendor string the server reported, for example `The X.Org Foundation`.
    pub fn vendor(&self) -> &str {
        &self.vendor
    }

    /// The protocol version the server reported, as `(major, minor)`.
    pub fn protocol_version(&self) -> (u16, u16) {
        self.protocol
    }
}

/// Errors from the X11 connection-setup round trip.
///
/// Every variant means "not ready": the caller logs it and keeps waiting for
/// the deadline (see [`crate::service`]).
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    /// The X11 socket could not be connected to.
    #[error("failed to connect to the X11 socket `{path}`: {source}")]
    Connect {
        /// The socket that was not connectable.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },

    /// The reply did not arrive within the budget.
    #[error("the X11 server did not finish the connection setup within {timeout:?}")]
    Timeout {
        /// The budget that was exceeded.
        timeout: Duration,
    },

    /// The peer closed the connection before the reply was complete.
    #[error("the X11 server closed the connection during the connection setup")]
    Closed,

    /// The server refused the connection setup (status 0).
    #[error("the X11 server refused the connection setup: {reason}")]
    Refused {
        /// The reason the server reported (empty when it sent none).
        reason: String,
    },

    /// The server asked for authentication (status 2).
    #[error("the X11 server requires authentication: {reason}")]
    Authentication {
        /// The data the server sent with the authentication request.
        reason: String,
    },

    /// The status byte was neither 0, 1 nor 2.
    #[error("the X11 server sent an unknown connection setup status {status:#04x}")]
    UnknownStatus {
        /// The unexpected status byte.
        status: u8,
    },

    /// The server speaks a protocol major version other than [`PROTOCOL_MAJOR`].
    #[error("the X11 server speaks protocol {major}.{minor}, expected {PROTOCOL_MAJOR}.x")]
    ProtocolVersion {
        /// The major version the server reported.
        major: u16,
        /// The minor version the server reported.
        minor: u16,
    },

    /// The setup reply is not structurally valid.
    #[error("the X11 setup reply is malformed: {reason}")]
    Malformed {
        /// What made the reply invalid.
        reason: &'static str,
    },

    /// The server sent more than the setup reply.
    #[error("the X11 server sent {bytes} unexpected bytes after the setup reply")]
    TrailingData {
        /// How many bytes were readable after the reply.
        bytes: usize,
    },

    /// The socket failed in a way that is neither a timeout nor a clean close.
    #[error("the X11 connection setup failed: {source}")]
    Io {
        /// The underlying I/O error.
        #[source]
        source: io::Error,
    },
}

/// Connect to the X11 socket at `path` and complete the connection setup.
///
/// # Errors
///
/// Returns [`HandshakeError::Connect`] when the socket cannot be connected to,
/// and any other [`HandshakeError`] when the round trip does not end in a well
/// formed successful setup reply.
pub fn probe(path: &Path, timeout: Duration) -> Result<Setup, HandshakeError> {
    let mut stream = UnixStream::connect(path).map_err(|source| HandshakeError::Connect {
        path: path.to_path_buf(),
        source,
    })?;

    setup(&mut stream, timeout)
}

/// Complete the X11 connection setup on an already connected `stream`.
///
/// `timeout` bounds the whole round trip, not a single read: the socket's read
/// timeout is re-armed with whatever is left of the budget before every read,
/// so a server that trickles bytes cannot extend the wait.
///
/// # Errors
///
/// Returns a [`HandshakeError`] for every outcome other than a well formed
/// successful setup reply, including timeouts and a socket that is closed while
/// the reply is still incomplete.
pub fn setup(stream: &mut UnixStream, timeout: Duration) -> Result<Setup, HandshakeError> {
    // `Instant + Duration` panics on overflow and the budget is caller
    // provided, so an absurd one saturates to an already expired deadline
    // instead of panicking.
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);

    // The request is 12 bytes, far below any socket buffer, so this write
    // cannot block waiting for the peer to read.
    let request = connection_request();
    stream
        .write_all(&request)
        .map_err(|source| HandshakeError::Io { source })?;

    let mut header = [0u8; REPLY_HEADER_LEN];
    read_exact_within(stream, &mut header, deadline, timeout)?;

    let status = header[0];
    let reason_length = usize::from(header[1]);
    let protocol = (
        u16::from_le_bytes([header[2], header[3]]),
        u16::from_le_bytes([header[4], header[5]]),
    );
    let data_length = usize::from(u16::from_le_bytes([header[6], header[7]])) * 4;

    match status {
        STATUS_SUCCESS => {
            if protocol.0 != PROTOCOL_MAJOR {
                return Err(HandshakeError::ProtocolVersion {
                    major: protocol.0,
                    minor: protocol.1,
                });
            }

            let mut body = vec![0u8; data_length];
            read_exact_within(stream, &mut body, deadline, timeout)?;
            let vendor = vendor_of(&body)?;

            let trailing = readable_bytes(stream)?;
            if trailing > 0 {
                return Err(HandshakeError::TrailingData { bytes: trailing });
            }

            Ok(Setup { vendor, protocol })
        }
        STATUS_FAILED => {
            let mut data = vec![0u8; data_length];
            read_exact_within(stream, &mut data, deadline, timeout)?;

            // `data_length` is padded to 4 bytes while byte 1 carries the exact
            // reason length, and servers have shipped both spellings of the
            // latter; clamping keeps a too generous length from turning a
            // readable failure into a malformed reply.
            let used = reason_length.min(data.len());

            Err(HandshakeError::Refused {
                reason: text_of(&data[..used]),
            })
        }
        STATUS_AUTHENTICATE => {
            let mut data = vec![0u8; data_length];
            read_exact_within(stream, &mut data, deadline, timeout)?;

            Err(HandshakeError::Authentication {
                reason: text_of(&data),
            })
        }
        status => Err(HandshakeError::UnknownStatus { status }),
    }
}

/// Build the 12-byte connection request prefix.
fn connection_request() -> [u8; REQUEST_LEN] {
    let mut request = [0u8; REQUEST_LEN];

    // The byte-order byte describes the encoding of this request, so the fields
    // below are little endian on every host.
    request[0] = BYTE_ORDER_LITTLE_ENDIAN;
    request[2..4].copy_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
    request[4..6].copy_from_slice(&PROTOCOL_MINOR.to_le_bytes());

    // The authorization name and data lengths, and the final two padding bytes,
    // stay zero: this service never authenticates.
    request
}

/// Extract the vendor string from a setup body.
fn vendor_of(body: &[u8]) -> Result<String, HandshakeError> {
    // The fixed fields end where the vendor starts, so a body that does not
    // even reach the vendor offset is not a setup body at all.
    if body.len() < VENDOR_OFFSET {
        return Err(HandshakeError::Malformed {
            reason: "the setup body is shorter than the fixed setup fields",
        });
    }

    let vendor_length = usize::from(u16::from_le_bytes([
        body[VENDOR_LENGTH_OFFSET],
        body[VENDOR_LENGTH_OFFSET + 1],
    ]));
    let vendor_end = VENDOR_OFFSET + vendor_length;

    if vendor_end > body.len() {
        return Err(HandshakeError::Malformed {
            reason: "the declared vendor string does not fit in the setup body",
        });
    }

    Ok(text_of(&body[VENDOR_OFFSET..vendor_end]))
}

/// Render a NUL terminated, 4-byte padded protocol string.
fn text_of(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());

    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

/// Read exactly `buffer.len()` bytes, giving up at `deadline`.
fn read_exact_within(
    stream: &mut UnixStream,
    buffer: &mut [u8],
    deadline: Instant,
    timeout: Duration,
) -> Result<(), HandshakeError> {
    let mut filled = 0;

    while filled < buffer.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());

        if remaining.is_zero() {
            return Err(HandshakeError::Timeout { timeout });
        }

        stream
            .set_read_timeout(Some(remaining))
            .map_err(|source| HandshakeError::Io { source })?;

        match stream.read(&mut buffer[filled..]) {
            Ok(0) => return Err(HandshakeError::Closed),
            Ok(read) => filled += read,
            Err(error) if is_timeout(&error) => {
                return Err(HandshakeError::Timeout { timeout });
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => return Err(HandshakeError::Io { source }),
        }
    }

    Ok(())
}

/// How many bytes are readable right now, without waiting for any.
fn readable_bytes(stream: &mut UnixStream) -> Result<usize, HandshakeError> {
    stream
        .set_nonblocking(true)
        .map_err(|source| HandshakeError::Io { source })?;

    let mut buffer = [0u8; 64];
    let outcome = stream.read(&mut buffer);

    // Restore the mode the caller expects before reporting anything, so a
    // failure here cannot leave the socket in a surprising state.
    stream
        .set_nonblocking(false)
        .map_err(|source| HandshakeError::Io { source })?;

    match outcome {
        // Zero means the peer closed the connection, which is not extra data.
        Ok(readable) => Ok(readable),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(0),
        Err(source) => Err(HandshakeError::Io { source }),
    }
}

/// Whether an I/O error is the read timeout elapsing.
fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{thread, time::Instant};

    /// Build a status-1 setup reply advertising `vendor`.
    ///
    /// The body is the 32 fixed bytes followed by the vendor string, padded to
    /// a multiple of four, exactly as the protocol describes it.
    fn success_reply(vendor: &str) -> Vec<u8> {
        let mut body = vec![0u8; VENDOR_OFFSET];
        body[0..4].copy_from_slice(&21u32.to_le_bytes());
        body[VENDOR_LENGTH_OFFSET..VENDOR_LENGTH_OFFSET + 2].copy_from_slice(
            &u16::try_from(vendor.len())
                .expect("short vendor")
                .to_le_bytes(),
        );
        body.extend_from_slice(vendor.as_bytes());

        while !body.len().is_multiple_of(4) {
            body.push(0);
        }

        let mut reply = vec![STATUS_SUCCESS, 0];
        reply.extend_from_slice(&PROTOCOL_MAJOR.to_le_bytes());
        reply.extend_from_slice(&PROTOCOL_MINOR.to_le_bytes());
        reply.extend_from_slice(
            &u16::try_from(body.len() / 4)
                .expect("body fits in 4-byte units")
                .to_le_bytes(),
        );
        reply.extend_from_slice(&body);
        reply
    }

    /// Build an 8-byte reply header with the given status and data length.
    fn header(status: u8, reason_length: u8, protocol: (u16, u16), units: u16) -> Vec<u8> {
        let mut header = vec![status, reason_length];
        header.extend_from_slice(&protocol.0.to_le_bytes());
        header.extend_from_slice(&protocol.1.to_le_bytes());
        header.extend_from_slice(&units.to_le_bytes());
        header
    }

    /// Build a status-0 setup reply carrying `reason`, padded to 4 bytes.
    fn failure_reply(reason: &str) -> Vec<u8> {
        let padded = reason.len().div_ceil(4) * 4;
        let mut reply = header(
            STATUS_FAILED,
            u8::try_from(reason.len()).expect("short reason"),
            (PROTOCOL_MAJOR, PROTOCOL_MINOR),
            u16::try_from(padded / 4).expect("short reason"),
        );
        reply.extend_from_slice(reason.as_bytes());
        reply.resize(REPLY_HEADER_LEN + padded, 0);
        reply
    }

    /// Run `setup` against a peer that reads the connection request, writes
    /// `reply` and then closes.
    ///
    /// Draining the request before closing is not cosmetic. A real server reads
    /// the request it answers, and the kernel resets a UNIX socket that is
    /// closed while its receive queue still holds unread data — the probe would
    /// then see `ECONNRESET` instead of the reply followed by a clean end of
    /// stream, which is a fixture artefact and not a protocol fact.
    fn setup_with_reply(reply: Vec<u8>, timeout: Duration) -> Result<Setup, HandshakeError> {
        let (mut client, mut server) = UnixStream::pair().expect("socket pair");

        let peer = thread::spawn(move || {
            // Bounded, so a client that never sends its request fails the test
            // instead of hanging it.
            if server.set_read_timeout(Some(REPLY_TIMEOUT)).is_err() {
                return;
            }

            let mut request = [0u8; REQUEST_LEN];
            if server.read_exact(&mut request).is_err() {
                return;
            }

            // A failed write only means the client gave up first.
            let _ = server.write_all(&reply);
        });

        let outcome = setup(&mut client, timeout);
        peer.join().expect("the peer thread panicked");

        outcome
    }

    /// Budget for the reply-driven tests: generous enough that a loaded CI
    /// machine cannot fail them, short enough that a genuine hang is caught.
    const REPLY_TIMEOUT: Duration = Duration::from_secs(2);

    /// Budget for the tests that assert the probe gives up on its own.
    const SILENCE_TIMEOUT: Duration = Duration::from_millis(200);

    #[test]
    fn the_connection_request_matches_the_x11_layout() {
        let request = connection_request();

        assert_eq!(request.len(), REQUEST_LEN);
        assert_eq!(request[0], 0x6c, "the request must declare little endian");
        assert_eq!(u16::from_le_bytes([request[2], request[3]]), PROTOCOL_MAJOR);
        assert_eq!(u16::from_le_bytes([request[4], request[5]]), PROTOCOL_MINOR);
        assert_eq!(
            u16::from_le_bytes([request[6], request[7]]),
            0,
            "no authorization protocol name"
        );
        assert_eq!(
            u16::from_le_bytes([request[8], request[9]]),
            0,
            "no authorization protocol data"
        );
        assert_eq!(
            [request[1], request[10], request[11]],
            [0, 0, 0],
            "padding must be zero"
        );
    }

    #[test]
    fn a_successful_setup_reports_the_vendor_string() {
        let setup = setup_with_reply(success_reply("The X.Org Foundation"), REPLY_TIMEOUT)
            .expect("a well formed setup reply must be accepted");

        assert_eq!(setup.vendor(), "The X.Org Foundation");
        assert_eq!(setup.protocol_version(), (PROTOCOL_MAJOR, PROTOCOL_MINOR));
    }

    #[test]
    fn a_padded_vendor_string_is_trimmed() {
        // Five bytes of vendor make the body one byte short of a multiple of
        // four, so the reply carries three NUL padding bytes after the string.
        let setup = setup_with_reply(success_reply("Inox "), REPLY_TIMEOUT).expect("valid reply");

        assert_eq!(setup.vendor(), "Inox");
    }

    #[test]
    fn a_setup_body_that_ends_right_after_a_short_vendor_is_accepted() {
        // 32 fixed bytes plus a one-byte vendor padded to four is the shortest
        // well formed setup body: a server reporting a short vendor must not be
        // rejected for not filling 40 bytes.
        let setup = setup_with_reply(success_reply("X"), REPLY_TIMEOUT).expect("valid reply");

        assert_eq!(setup.vendor(), "X");
    }

    #[test]
    fn a_refused_setup_reports_the_reason() {
        let outcome = setup_with_reply(failure_reply("Display is already in use"), REPLY_TIMEOUT);

        assert!(matches!(
            outcome,
            Err(HandshakeError::Refused { reason }) if reason == "Display is already in use"
        ));
    }

    #[test]
    fn an_authentication_request_is_reported() {
        let reason = "MIT-MAGIC-COOKIE-1";
        let padded = reason.len().div_ceil(4) * 4;
        let mut reply = header(
            STATUS_AUTHENTICATE,
            0,
            (0, 0),
            u16::try_from(padded / 4).expect("short reason"),
        );
        reply.extend_from_slice(reason.as_bytes());
        reply.resize(REPLY_HEADER_LEN + padded, 0);

        let outcome = setup_with_reply(reply, REPLY_TIMEOUT);

        assert!(matches!(
            outcome,
            Err(HandshakeError::Authentication { reason }) if reason == "MIT-MAGIC-COOKIE-1"
        ));
    }

    #[test]
    fn an_unknown_status_byte_is_rejected() {
        let outcome = setup_with_reply(header(0x42, 0, (PROTOCOL_MAJOR, 0), 0), REPLY_TIMEOUT);

        assert!(matches!(
            outcome,
            Err(HandshakeError::UnknownStatus { status: 0x42 })
        ));
    }

    #[test]
    fn another_protocol_major_version_is_rejected() {
        let mut reply = success_reply("Ancient X");
        reply[2..4].copy_from_slice(&10u16.to_le_bytes());

        let outcome = setup_with_reply(reply, REPLY_TIMEOUT);

        assert!(matches!(
            outcome,
            Err(HandshakeError::ProtocolVersion { major: 10, .. })
        ));
    }

    #[test]
    fn a_connection_closed_before_the_reply_is_rejected() {
        let (mut client, mut server) = UnixStream::pair().expect("socket pair");

        // The peer collects the request and then goes away, so the probe reads
        // a closed socket rather than failing to write its request.
        let peer = thread::spawn(move || {
            let mut request = [0u8; REQUEST_LEN];
            server.read_exact(&mut request).expect("read the request");
        });

        let outcome = setup(&mut client, REPLY_TIMEOUT);
        peer.join().expect("the peer thread panicked");

        assert!(matches!(outcome, Err(HandshakeError::Closed)));
    }

    #[test]
    fn a_truncated_reply_is_rejected() {
        let mut reply = success_reply("The X.Org Foundation");
        reply.truncate(reply.len() - 8);

        assert!(matches!(
            setup_with_reply(reply, REPLY_TIMEOUT),
            Err(HandshakeError::Closed) | Err(HandshakeError::Timeout { .. })
        ));
    }

    #[test]
    fn a_header_only_reply_is_rejected() {
        let outcome = setup_with_reply(
            header(STATUS_SUCCESS, 0, (PROTOCOL_MAJOR, 0), 2),
            REPLY_TIMEOUT,
        );

        assert!(matches!(
            outcome,
            Err(HandshakeError::Closed) | Err(HandshakeError::Timeout { .. })
        ));
    }

    #[test]
    fn a_setup_body_without_the_fixed_fields_is_rejected() {
        let mut reply = header(STATUS_SUCCESS, 0, (PROTOCOL_MAJOR, PROTOCOL_MINOR), 2);
        reply.extend_from_slice(&[0u8; 8]);

        let outcome = setup_with_reply(reply, REPLY_TIMEOUT);

        assert!(matches!(outcome, Err(HandshakeError::Malformed { .. })));
    }

    #[test]
    fn a_vendor_string_beyond_the_setup_body_is_rejected() {
        let mut reply = success_reply("The X.Org Foundation");
        // Claim far more vendor bytes than the body can hold.
        reply[REPLY_HEADER_LEN + VENDOR_LENGTH_OFFSET..REPLY_HEADER_LEN + VENDOR_LENGTH_OFFSET + 2]
            .copy_from_slice(&60000u16.to_le_bytes());

        let outcome = setup_with_reply(reply, REPLY_TIMEOUT);

        assert!(matches!(outcome, Err(HandshakeError::Malformed { .. })));
    }

    #[test]
    fn trailing_data_after_the_setup_reply_is_rejected() {
        let mut reply = success_reply("The X.Org Foundation");
        reply.extend_from_slice(b"not part of the setup");

        let outcome = setup_with_reply(reply, REPLY_TIMEOUT);

        assert!(matches!(outcome, Err(HandshakeError::TrailingData { .. })));
    }

    #[test]
    fn garbage_is_rejected() {
        let outcome = setup_with_reply(vec![0xff; 32], REPLY_TIMEOUT);

        assert!(outcome.is_err(), "garbage must never look like a setup");
    }

    #[test]
    fn a_silent_peer_times_out_and_does_not_hang_the_probe() {
        // The peer stays connected and never writes anything. It is dropped
        // only after the assertions, so the probe sees a hung server rather
        // than a reset connection.
        let (mut client, _silent_peer) = UnixStream::pair().expect("socket pair");
        let timeout = SILENCE_TIMEOUT;
        let started = Instant::now();
        let outcome = setup(&mut client, timeout);
        let elapsed = started.elapsed();

        assert!(matches!(outcome, Err(HandshakeError::Timeout { .. })));
        assert!(
            elapsed >= Duration::from_millis(100),
            "the probe must wait for the reply, waited only {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the read timeout must bound the wait, waited {elapsed:?}"
        );
    }

    #[test]
    fn a_peer_that_stops_mid_reply_times_out_instead_of_waiting_forever() {
        // This peer announces a body it never sends and stays connected, so the
        // probe has to give up on its own budget.
        let (mut client, mut silent_peer) = UnixStream::pair().expect("socket pair");
        silent_peer
            .write_all(&header(STATUS_SUCCESS, 0, (PROTOCOL_MAJOR, 0), 4000))
            .expect("write the header");

        let timeout = SILENCE_TIMEOUT;
        let started = Instant::now();
        let outcome = setup(&mut client, timeout);
        let elapsed = started.elapsed();

        assert!(matches!(outcome, Err(HandshakeError::Timeout { .. })));
        assert!(
            elapsed < Duration::from_secs(5),
            "a stalled body must be bounded by the budget, waited {elapsed:?}"
        );
    }

    #[test]
    fn probing_a_missing_socket_reports_a_connect_error() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("X7");

        let outcome = probe(&missing, REPLY_TIMEOUT);

        assert!(matches!(outcome, Err(HandshakeError::Connect { .. })));
    }
}
