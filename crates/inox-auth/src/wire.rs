//! The `auth-v1` wire: what the login screen says to the authentication
//! boundary, and what the boundary says back.
//!
//! The normative contract is `crates/inox-protocols/protocol/auth-v1.md`; this
//! module is its implementation, in the same shape ADR 0026 used for the
//! configuration contract (a contract document in `inox-protocols`, the types in
//! the crate that speaks it). It is a hand-written binary format rather than
//! `serde` + JSON for one reason that decides the whole design: **a credential
//! must never be parsed into a `String`**. A JSON parser allocates the password
//! as a growable, reallocating, `Debug`-printable `String` that no Inox type can
//! zeroize, which is exactly what ADR 0015 forbids. Here the credential bytes go
//! from a scratch buffer that is zeroed on every path into
//! [`Secret::from_bytes`], and the only thing that can print them is
//! [`Secret::expose`], whose one caller is the PAM conversation.
//!
//! Three properties the framing keeps:
//!
//! - **A frame is bounded before it is allocated.** The length prefix is checked
//!   against [`MAX_FRAME_BYTES`] before the buffer exists, so a peer cannot ask
//!   the root helper to allocate a gigabyte.
//! - **A frame is all-or-nothing.** Trailing bytes, a short payload and an
//!   unknown tag are errors, not something to resynchronise around: after a
//!   malformed frame the connection is closed by the server, and the client's
//!   only valid move is to try again.
//! - **The credential is bounded too.** [`MAX_CREDENTIAL_BYTES`] is a floor
//!   rather than a policy (ADR 0015's threat model is not "a password can be
//!   long"); it bounds what one connection can make the helper hold and hand to
//!   `libpam`.
//!
//! Values that are *not* credentials — the attempt identifier, the username, the
//! verdict — are ordinary data and are printable, which is what makes the
//! `Debug` rendering of a request useful in a log without ever being a leak.

use std::io::{self, Read, Write};

use inox_common::secret::Secret;
use thiserror::Error;

/// The contract's name, as `crates/inox-protocols/protocol/auth-v1.md` gives it.
pub const PROTOCOL: &str = "inox-auth-v1";

/// The version this implementation speaks.
pub const VERSION: u32 = 1;

/// The socket both sides use unless they are told otherwise.
///
/// It lives in the contract rather than in the server because both halves have to
/// agree on it and the login screen must be able to name it without linking the
/// privileged half. `crates/inox-protocols/protocol/auth-v1.md` names it too; the
/// deployment may move it, and the login screen reads `INOX_AUTH_SOCKET` for that.
pub const DEFAULT_SOCKET: &str = "/run/inox/auth.sock";

/// The largest frame either side will read or write, in bytes.
///
/// The bound is checked *before* the payload is allocated.
pub const MAX_FRAME_BYTES: usize = 4096;

/// The most bytes of username one request may carry.
///
/// The same floor the login screen puts on the identity it retains
/// (`inox_greet::state::MAX_IDENTITY_BYTES`), so the two cannot disagree about
/// what a submittable name is.
pub const MAX_USERNAME_BYTES: usize = 128;

/// The most credential bytes one request may carry.
pub const MAX_CREDENTIAL_BYTES: usize = 1024;

/// Frame tags, client to server.
mod request_tag {
    pub const HELLO: u8 = 1;
    pub const AUTHENTICATE: u8 = 2;
    pub const START_SESSION: u8 = 3;
    pub const CANCEL: u8 = 4;
}

/// Frame tags, server to client.
mod reply_tag {
    pub const WELCOME: u8 = 16;
    pub const ACCEPTED: u8 = 17;
    pub const REJECTED: u8 = 18;
    pub const UNAVAILABLE: u8 = 19;
    pub const SESSION_STARTED: u8 = 20;
    pub const ERROR: u8 = 21;
}

/// Why a request could not be read or written.
///
/// The variants are the closed set of things that can go wrong between two
/// processes of this workspace, so the server can turn one into a
/// [`Reply::Error`] without inventing a code at the call site.
#[derive(Debug, Error)]
pub enum WireError {
    /// The transport failed.
    #[error("the connection failed: {0}")]
    Io(#[from] io::Error),
    /// The peer announced a frame larger than [`MAX_FRAME_BYTES`].
    #[error("a frame of {length} bytes exceeds the {MAX_FRAME_BYTES}-byte bound")]
    TooLarge {
        /// The length the peer announced.
        length: usize,
    },
    /// The connection ended in the middle of a frame.
    #[error("the connection ended in the middle of a frame")]
    Truncated,
    /// The tag is not one this version defines.
    #[error("unknown frame tag {0}")]
    UnknownTag(u8),
    /// The version the contract does not define.
    #[error("the peer speaks version {0}, this build speaks {VERSION}")]
    UnsupportedVersion(u32),
    /// The frame carried bytes its tag does not define.
    #[error("malformed frame: {0}")]
    Malformed(&'static str),
    /// The username is empty, over-long, or not a portable account name.
    #[error("the username is not a portable account name")]
    InvalidUsername,
    /// The credential is longer than the boundary will carry in one request.
    #[error("the credential exceeds the {MAX_CREDENTIAL_BYTES}-byte bound")]
    CredentialTooLong,
}

impl WireError {
    /// The code the boundary answers with, when the connection is still usable.
    ///
    /// `None` means the frame was never understood well enough to answer *about*
    /// it (a truncated or oversized frame), so the server closes instead of
    /// guessing at a reply.
    pub fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::UnknownTag(_) | Self::Malformed(_) | Self::InvalidUsername => {
                Some(ErrorCode::Malformed)
            }
            Self::UnsupportedVersion(_) => Some(ErrorCode::UnsupportedVersion),
            Self::CredentialTooLong => Some(ErrorCode::Malformed),
            Self::TooLarge { .. } | Self::Truncated | Self::Io(_) => None,
        }
    }
}

/// The correlation identifier of one authentication attempt.
///
/// Chosen by the client, echoed by the server. It is not a capability: the model
/// on the client side only accepts an answer for the attempt it has in flight, so
/// an identifier guessed by someone else buys nothing (ADR 0015's rule that a
/// forged success is refused is a property of the model, not of this number).
pub type Attempt = u64;

/// What the login screen asks the boundary to do.
///
/// Not `PartialEq`: one variant holds a credential, and [`Secret`] is
/// deliberately not comparable (ADR 0015 rules out an equality oracle over
/// credential bytes). A test that wants to compare two requests compares the
/// fields it means to compare.
#[derive(Debug)]
pub enum Request {
    /// The first frame on every connection: which version this client speaks.
    Hello {
        /// The version the client speaks.
        version: u32,
    },
    /// Check these credentials for this account.
    ///
    /// `Debug` is derived and safe: [`Secret`] renders as `<redacted>`, which is
    /// the whole reason the credential is that type and not a `String`.
    Authenticate {
        /// The attempt the client will correlate the answer with.
        attempt: Attempt,
        /// The account name, already validated by the client for shape.
        username: String,
        /// What the user typed.
        credential: Secret,
    },
    /// Start the session for an attempt the boundary accepted.
    StartSession {
        /// The attempt that was accepted.
        attempt: Attempt,
    },
    /// Abandon the attempt; the boundary must collect nothing from it.
    Cancel {
        /// The attempt to abandon.
        attempt: Attempt,
    },
}

impl Request {
    /// The frame tag this request encodes as.
    pub const fn tag(&self) -> u8 {
        match self {
            Self::Hello { .. } => request_tag::HELLO,
            Self::Authenticate { .. } => request_tag::AUTHENTICATE,
            Self::StartSession { .. } => request_tag::START_SESSION,
            Self::Cancel { .. } => request_tag::CANCEL,
        }
    }

    /// The payload, without the length prefix.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::InvalidUsername`] or [`WireError::CredentialTooLong`]
    /// when a field is outside the bound the contract defines: the client is the
    /// one that knows what it typed, so an over-long field is a client error and
    /// is refused here rather than truncated somewhere in the middle.
    pub fn encode(&self) -> Result<Vec<u8>, WireError> {
        let mut payload = Vec::new();
        payload.push(self.tag());

        match self {
            Self::Hello { version } => payload.extend_from_slice(&version.to_be_bytes()),
            Self::Authenticate {
                attempt,
                username,
                credential,
            } => {
                if !valid_username(username) {
                    return Err(WireError::InvalidUsername);
                }
                if credential.byte_len() > MAX_CREDENTIAL_BYTES {
                    return Err(WireError::CredentialTooLong);
                }

                payload.extend_from_slice(&attempt.to_be_bytes());
                let name_length =
                    u16::try_from(username.len()).map_err(|_| WireError::InvalidUsername)?;
                payload.extend_from_slice(&name_length.to_be_bytes());
                payload.extend_from_slice(username.as_bytes());
                let secret_length = u16::try_from(credential.byte_len())
                    .map_err(|_| WireError::CredentialTooLong)?;
                payload.extend_from_slice(&secret_length.to_be_bytes());
                payload.extend_from_slice(credential.expose());
            }
            Self::StartSession { attempt } | Self::Cancel { attempt } => {
                payload.extend_from_slice(&attempt.to_be_bytes());
            }
        }

        Ok(payload)
    }
}

/// What the boundary answers.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    /// The handshake succeeded; the connection now speaks this version.
    Welcome {
        /// The version the boundary speaks.
        version: u32,
    },
    /// The credentials were accepted. The client may ask for the session.
    Accepted {
        /// The attempt this answers.
        attempt: Attempt,
    },
    /// The credentials were refused. Nothing was started and nothing will be.
    Rejected {
        /// The attempt this answers.
        attempt: Attempt,
    },
    /// The boundary cannot answer this attempt at all.
    Unavailable {
        /// The attempt this answers.
        attempt: Attempt,
        /// Why, without saying anything about *what* was typed.
        reason: UnavailableReason,
    },
    /// The session was launched (`started`) or would not start.
    SessionStarted {
        /// The attempt this answers.
        attempt: Attempt,
        /// Whether a session is now running.
        started: bool,
    },
    /// The frame was understood and refused.
    Error {
        /// What was wrong with it.
        code: ErrorCode,
    },
}

impl Reply {
    /// The frame tag this reply encodes as.
    pub const fn tag(&self) -> u8 {
        match self {
            Self::Welcome { .. } => reply_tag::WELCOME,
            Self::Accepted { .. } => reply_tag::ACCEPTED,
            Self::Rejected { .. } => reply_tag::REJECTED,
            Self::Unavailable { .. } => reply_tag::UNAVAILABLE,
            Self::SessionStarted { .. } => reply_tag::SESSION_STARTED,
            Self::Error { .. } => reply_tag::ERROR,
        }
    }

    /// The payload, without the length prefix.
    pub fn encode(&self) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.push(self.tag());

        match self {
            Self::Welcome { version } => payload.extend_from_slice(&version.to_be_bytes()),
            Self::Accepted { attempt } | Self::Rejected { attempt } => {
                payload.extend_from_slice(&attempt.to_be_bytes());
            }
            Self::Unavailable { attempt, reason } => {
                payload.extend_from_slice(&attempt.to_be_bytes());
                payload.push(reason.as_u8());
            }
            Self::SessionStarted { attempt, started } => {
                payload.extend_from_slice(&attempt.to_be_bytes());
                payload.push(u8::from(*started));
            }
            Self::Error { code } => payload.push(code.as_u8()),
        }

        payload
    }

    /// Reads one reply payload.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::UnknownTag`], [`WireError::Malformed`],
    /// [`WireError::UnsupportedVersion`] or [`WireError::Truncated`] for a frame
    /// this version does not define or does not fully contain.
    pub fn decode(payload: &[u8]) -> Result<Self, WireError> {
        let (&tag, rest) = payload.split_first().ok_or(WireError::Truncated)?;

        match tag {
            reply_tag::WELCOME => {
                let version = take_u32(rest)?;
                expect_end(&rest[4..])?;
                Ok(Self::Welcome { version })
            }
            reply_tag::ACCEPTED | reply_tag::REJECTED => {
                let attempt = take_u64(rest)?;
                expect_end(&rest[8..])?;
                Ok(if tag == reply_tag::ACCEPTED {
                    Self::Accepted { attempt }
                } else {
                    Self::Rejected { attempt }
                })
            }
            reply_tag::UNAVAILABLE => {
                let attempt = take_u64(rest)?;
                let reason = UnavailableReason::from_u8(*rest.get(8).ok_or(WireError::Truncated)?)?;
                expect_end(&rest[9..])?;
                Ok(Self::Unavailable { attempt, reason })
            }
            reply_tag::SESSION_STARTED => {
                let attempt = take_u64(rest)?;
                let started = match rest.get(8).ok_or(WireError::Truncated)? {
                    0 => false,
                    1 => true,
                    _ => return Err(WireError::Malformed("a started flag is a boolean")),
                };
                expect_end(&rest[9..])?;
                Ok(Self::SessionStarted { attempt, started })
            }
            reply_tag::ERROR => {
                let code = ErrorCode::from_u8(*rest.first().ok_or(WireError::Truncated)?)?;
                expect_end(&rest[1..])?;
                Ok(Self::Error { code })
            }
            other => Err(WireError::UnknownTag(other)),
        }
    }
}

/// Why the boundary could not answer an attempt.
///
/// The vocabulary is deliberately about the *boundary*, not about the account:
/// there is no "no such user" here, because an unknown account is answered
/// exactly like a wrong password (see [`Reply::Rejected`]). A login screen that
/// could say "that account does not exist" is an account-enumeration oracle for
/// anyone standing in front of the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableReason {
    /// No reviewed authentication transport is connected.
    NotProvided,
    /// The boundary is already busy with an attempt or a running session.
    Busy,
    /// Too many attempts in the rolling window.
    RateLimited,
    /// The authenticator itself failed (PAM returned an error).
    Failed,
}

impl UnavailableReason {
    /// The byte this reason encodes as.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::NotProvided => 0,
            Self::Busy => 1,
            Self::RateLimited => 2,
            Self::Failed => 3,
        }
    }

    /// The reason a byte encodes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Malformed`] for a byte this version does not define.
    pub const fn from_u8(byte: u8) -> Result<Self, WireError> {
        match byte {
            0 => Ok(Self::NotProvided),
            1 => Ok(Self::Busy),
            2 => Ok(Self::RateLimited),
            3 => Ok(Self::Failed),
            _ => Err(WireError::Malformed("unknown unavailable reason")),
        }
    }
}

/// What was wrong with a frame that was still answerable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The frame does not match its tag.
    Malformed,
    /// The client speaks a version this boundary does not implement.
    UnsupportedVersion,
    /// The peer is not a process this boundary serves.
    UnauthorizedPeer,
}

impl ErrorCode {
    /// The byte this code encodes as.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Malformed => 0,
            Self::UnsupportedVersion => 1,
            Self::UnauthorizedPeer => 2,
        }
    }

    /// The code a byte encodes.
    ///
    /// # Errors
    ///
    /// Returns [`WireError::Malformed`] for a byte this version does not define.
    pub const fn from_u8(byte: u8) -> Result<Self, WireError> {
        match byte {
            0 => Ok(Self::Malformed),
            1 => Ok(Self::UnsupportedVersion),
            2 => Ok(Self::UnauthorizedPeer),
            _ => Err(WireError::Malformed("unknown error code")),
        }
    }
}

/// Whether `name` is an account name this contract carries.
///
/// The grammar is the portable one — what every account database in practice
/// accepts, and what the login screen's own field validates before it submits —
/// and it is deliberately narrow rather than "whatever the database accepts":
///
/// - it must start with a letter or `_`, so a name can never be read as an option
///   or a path fragment by anything that later sees it;
/// - the rest may be letters, digits, `_`, `-` and `.`;
/// - it is at most [`MAX_USERNAME_BYTES`] bytes, and never empty.
///
/// The last two rules are also why the client and the server can both check this
/// without disagreeing about which of them is authoritative: they are checking the
/// same sentence in the contract, not two different ideas of a name.
pub fn valid_username(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_USERNAME_BYTES {
        return false;
    }

    let mut bytes = name.bytes();

    matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

/// Reads one reply from `reader`.
///
/// # Errors
///
/// Propagates whatever [`read_frame`] and [`Reply::decode`] return.
pub fn read_reply<R: Read>(reader: &mut R) -> Result<Option<Reply>, WireError> {
    match read_frame(reader)? {
        Some(payload) => Reply::decode(&payload).map(Some),
        None => Ok(None),
    }
}

/// Writes one reply, flushed.
///
/// # Errors
///
/// Propagates the transport error.
pub fn write_reply<W: Write>(writer: &mut W, reply: &Reply) -> Result<(), WireError> {
    write_frame(writer, &reply.encode())
}

/// Reads one request from `reader`.
///
/// # Errors
///
/// Propagates whatever [`read_frame`] and [`decode_request`] return.
pub fn read_request<R: Read>(reader: &mut R) -> Result<Option<Request>, WireError> {
    match read_frame(reader)? {
        Some(payload) => decode_request(&payload).map(Some),
        None => Ok(None),
    }
}

/// Writes one request, flushed.
///
/// # Errors
///
/// Propagates the transport error and the bounds [`Request::encode`] enforces.
pub fn write_request<W: Write>(writer: &mut W, request: &Request) -> Result<(), WireError> {
    write_frame(writer, &request.encode()?)
}

/// Decodes one request payload.
///
/// # Errors
///
/// Returns [`WireError::UnknownTag`], [`WireError::Malformed`],
/// [`WireError::UnsupportedVersion`], [`WireError::InvalidUsername`] or
/// [`WireError::Truncated`] for a payload this version does not define.
pub fn decode_request(payload: &[u8]) -> Result<Request, WireError> {
    let (&tag, rest) = payload.split_first().ok_or(WireError::Truncated)?;

    match tag {
        request_tag::HELLO => {
            let version = take_u32(rest)?;
            expect_end(&rest[4..])?;
            if version != VERSION {
                return Err(WireError::UnsupportedVersion(version));
            }
            Ok(Request::Hello { version })
        }
        request_tag::AUTHENTICATE => {
            let attempt = take_u64(rest)?;
            let mut cursor = &rest[8..];

            let name_length = usize::from(take_u16(cursor)?);
            cursor = after(cursor, 2)?;
            if name_length == 0 || name_length > MAX_USERNAME_BYTES {
                return Err(WireError::InvalidUsername);
            }
            let name_bytes = cursor
                .get(..name_length)
                .ok_or(WireError::Truncated)?
                .to_vec();
            let username = String::from_utf8(name_bytes).map_err(|_| WireError::InvalidUsername)?;
            // Validated on the way in as well as on the way out: a frame that this
            // build would refuse to write is also a frame it refuses to act on.
            if !valid_username(&username) {
                return Err(WireError::InvalidUsername);
            }
            cursor = after(cursor, name_length)?;

            let secret_length = usize::from(take_u16(cursor)?);
            cursor = after(cursor, 2)?;
            if secret_length > MAX_CREDENTIAL_BYTES {
                return Err(WireError::CredentialTooLong);
            }
            // The bytes land in a scratch buffer that zeroes itself on every way
            // out of this scope, and are copied into the credential from there:
            // there is no `String`, no `Vec<u8>` that outlives the branch, and no
            // path on which the plaintext is merely forgotten.
            let scratch = zeroize::Zeroizing::new(
                cursor
                    .get(..secret_length)
                    .ok_or(WireError::Truncated)?
                    .to_vec(),
            );
            let credential = Secret::from_bytes(&scratch);
            cursor = after(cursor, secret_length)?;
            expect_end(cursor)?;

            Ok(Request::Authenticate {
                attempt,
                username,
                credential,
            })
        }
        request_tag::START_SESSION => {
            let attempt = take_u64(rest)?;
            expect_end(&rest[8..])?;
            Ok(Request::StartSession { attempt })
        }
        request_tag::CANCEL => {
            let attempt = take_u64(rest)?;
            expect_end(&rest[8..])?;
            Ok(Request::Cancel { attempt })
        }
        other => Err(WireError::UnknownTag(other)),
    }
}

/// Reads one length-prefixed frame, or `None` when the peer closed cleanly
/// between frames.
///
/// # Errors
///
/// Returns [`WireError::TooLarge`] once the announced length exceeds
/// [`MAX_FRAME_BYTES`] — before allocating it — and [`WireError::Truncated`] when
/// the connection ends inside a frame.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, WireError> {
    let mut header = [0u8; 4];
    let mut filled = 0;

    while filled < header.len() {
        match reader.read(&mut header[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => return Err(WireError::Truncated),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(WireError::Io(error)),
        }
    }

    let length = u32::from_be_bytes(header) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge { length });
    }

    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            WireError::Truncated
        } else {
            WireError::Io(error)
        }
    })?;

    Ok(Some(payload))
}

/// Writes one length-prefixed frame and flushes it.
///
/// # Errors
///
/// Returns [`WireError::TooLarge`] for a payload this contract cannot frame, and
/// propagates transport errors.
pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), WireError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge {
            length: payload.len(),
        });
    }

    let length = u32::try_from(payload.len()).map_err(|_| WireError::TooLarge {
        length: payload.len(),
    })?;

    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;

    Ok(())
}

/// The first four bytes of `bytes` as a big-endian `u32`.
fn take_u32(bytes: &[u8]) -> Result<u32, WireError> {
    let field = bytes.get(..4).ok_or(WireError::Truncated)?;
    let mut array = [0u8; 4];
    array.copy_from_slice(field);

    Ok(u32::from_be_bytes(array))
}

/// The first two bytes of `bytes` as a big-endian `u16`.
fn take_u16(bytes: &[u8]) -> Result<u16, WireError> {
    let field = bytes.get(..2).ok_or(WireError::Truncated)?;
    let mut array = [0u8; 2];
    array.copy_from_slice(field);

    Ok(u16::from_be_bytes(array))
}

/// The first eight bytes of `bytes` as a big-endian `u64`.
fn take_u64(bytes: &[u8]) -> Result<u64, WireError> {
    let field = bytes.get(..8).ok_or(WireError::Truncated)?;
    let mut array = [0u8; 8];
    array.copy_from_slice(field);

    Ok(u64::from_be_bytes(array))
}

/// `bytes` without its first `count` bytes.
fn after(bytes: &[u8], count: usize) -> Result<&[u8], WireError> {
    bytes.get(count..).ok_or(WireError::Truncated)
}

/// Nothing after the fields this tag defines.
///
/// A frame with trailing bytes is a frame from a version that means something
/// else by it, and reading it as if it did not would be guessing.
fn expect_end(bytes: &[u8]) -> Result<(), WireError> {
    if bytes.is_empty() {
        Ok(())
    } else {
        Err(WireError::Malformed("trailing bytes after the fields"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn secret(value: &str) -> Secret {
        Secret::from_bytes(value.as_bytes())
    }

    fn round_trip_request(request: Request) {
        let mut encoded = Vec::new();
        write_request(&mut encoded, &request).expect("the request encodes");

        let mut reader = Cursor::new(encoded);
        let decoded = read_request(&mut reader)
            .expect("the request decodes")
            .expect("a frame");

        match (request, decoded) {
            (
                Request::Authenticate {
                    attempt: sent,
                    username: sent_name,
                    credential,
                },
                Request::Authenticate {
                    attempt,
                    username,
                    credential: decoded_credential,
                },
            ) => {
                assert_eq!(sent, attempt);
                assert_eq!(sent_name, username);
                assert_eq!(credential.expose(), decoded_credential.expose());
            }
            (Request::Hello { version: sent }, Request::Hello { version }) => {
                assert_eq!(sent, version);
            }
            (Request::StartSession { attempt: sent }, Request::StartSession { attempt }) => {
                assert_eq!(sent, attempt);
            }
            (Request::Cancel { attempt: sent }, Request::Cancel { attempt }) => {
                assert_eq!(sent, attempt);
            }
            (sent, decoded) => panic!("a {sent:?} came back as a {decoded:?}"),
        }
    }

    fn round_trip_reply(reply: Reply) {
        let mut encoded = Vec::new();
        write_reply(&mut encoded, &reply).expect("the reply encodes");

        let mut reader = Cursor::new(encoded);
        let decoded = read_reply(&mut reader)
            .expect("the reply decodes")
            .expect("a frame");

        assert_eq!(reply, decoded);
    }

    #[test]
    fn every_request_round_trips() {
        round_trip_request(Request::Hello { version: VERSION });
        round_trip_request(Request::Authenticate {
            attempt: 7,
            username: "alice-1.test".to_owned(),
            credential: secret("correct horse battery staple"),
        });
        round_trip_request(Request::StartSession { attempt: 7 });
        round_trip_request(Request::Cancel { attempt: 7 });
    }

    #[test]
    fn every_reply_round_trips() {
        round_trip_reply(Reply::Welcome { version: VERSION });
        round_trip_reply(Reply::Accepted { attempt: 1 });
        round_trip_reply(Reply::Rejected { attempt: 1 });
        for reason in [
            UnavailableReason::NotProvided,
            UnavailableReason::Busy,
            UnavailableReason::RateLimited,
            UnavailableReason::Failed,
        ] {
            round_trip_reply(Reply::Unavailable { attempt: 1, reason });
        }
        round_trip_reply(Reply::SessionStarted {
            attempt: 1,
            started: true,
        });
        round_trip_reply(Reply::SessionStarted {
            attempt: 1,
            started: false,
        });
        round_trip_reply(Reply::Error {
            code: ErrorCode::UnauthorizedPeer,
        });
    }

    #[test]
    fn a_credential_is_never_printable_through_a_request() {
        let request = Request::Authenticate {
            attempt: 1,
            username: "alice".to_owned(),
            credential: secret("hunter2"),
        };
        let rendered = format!("{request:?}");

        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
        assert!(
            rendered.contains("alice"),
            "the username is not a credential and stays readable: {rendered}"
        );
    }

    #[test]
    fn the_credential_survives_bytes_that_are_not_utf8() {
        let credential = Secret::from_bytes(&[0xff, 0x00, 0x41, 0x80]);
        let request = Request::Authenticate {
            attempt: 1,
            username: "alice".to_owned(),
            credential,
        };

        let mut encoded = Vec::new();
        write_request(&mut encoded, &request).expect("the request encodes");
        let decoded = read_request(&mut Cursor::new(encoded))
            .expect("the request decodes")
            .expect("a frame");

        let Request::Authenticate { credential, .. } = decoded else {
            panic!("an authenticate request");
        };
        assert_eq!(credential.expose(), &[0xff, 0x00, 0x41, 0x80]);
    }

    #[test]
    fn a_clean_close_between_frames_is_not_an_error() {
        let mut reader = Cursor::new(Vec::new());

        assert!(read_frame(&mut reader).expect("no frame").is_none());
        assert!(read_reply(&mut reader).expect("no reply").is_none());
    }

    #[test]
    fn a_connection_that_ends_inside_a_frame_is_truncated() {
        // A length prefix promising more than is there.
        let mut reader = Cursor::new(vec![0, 0, 1, 0, 7]);
        assert!(matches!(read_frame(&mut reader), Err(WireError::Truncated)));

        // Half a header.
        let mut reader = Cursor::new(vec![0, 0]);
        assert!(matches!(read_frame(&mut reader), Err(WireError::Truncated)));
    }

    #[test]
    fn an_oversized_frame_is_refused_before_it_is_allocated() {
        let announced = u32::try_from(MAX_FRAME_BYTES + 1).expect("fits");
        let mut reader = Cursor::new(announced.to_be_bytes().to_vec());

        match read_frame(&mut reader) {
            Err(WireError::TooLarge { length }) => assert_eq!(length, MAX_FRAME_BYTES + 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_tags_trailing_bytes_and_truncated_fields_are_malformed() {
        assert!(matches!(
            decode_request(&[99]),
            Err(WireError::UnknownTag(99))
        ));
        assert!(matches!(
            Reply::decode(&[99]),
            Err(WireError::UnknownTag(99))
        ));

        // A well-formed Hello with one byte too many.
        let mut hello = Request::Hello { version: VERSION }
            .encode()
            .expect("encodes");
        hello.push(0);
        assert!(matches!(
            decode_request(&hello),
            Err(WireError::Malformed(_))
        ));

        // A Cancel whose identifier is only half present.
        assert!(matches!(
            decode_request(&[request_tag::CANCEL, 0, 0, 0]),
            Err(WireError::Truncated)
        ));

        // A username length that runs past the frame.
        let payload = [
            request_tag::AUTHENTICATE,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            1,
            0,
            9,
            b'a',
        ];
        assert!(matches!(
            decode_request(&payload),
            Err(WireError::Truncated)
        ));
    }

    #[test]
    fn another_protocol_version_is_refused_by_name() {
        let payload = [request_tag::HELLO, 0, 0, 0, 2];

        match decode_request(&payload) {
            Err(WireError::UnsupportedVersion(2)) => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(
            WireError::UnsupportedVersion(2).code(),
            Some(ErrorCode::UnsupportedVersion)
        );
    }

    #[test]
    fn an_over_long_or_malformed_username_is_refused() {
        for name in [
            String::new(),
            "-alice".to_owned(),
            ".alice".to_owned(),
            "0alice".to_owned(),
            "alice bob".to_owned(),
            "alice\n".to_owned(),
            "../root".to_owned(),
            "a".repeat(MAX_USERNAME_BYTES + 1),
        ] {
            let request = Request::Authenticate {
                attempt: 1,
                username: name.clone(),
                credential: secret("x"),
            };
            assert!(
                matches!(request.encode(), Err(WireError::InvalidUsername)),
                "`{name}` should not be a submittable account name"
            );
            assert!(!valid_username(&name), "`{name}` should not validate");
        }

        for name in ["alice", "alice-1.test", "_alice", "Alice"] {
            assert!(valid_username(name), "`{name}` is a portable account name");
        }
    }

    #[test]
    fn an_over_long_credential_is_refused_rather_than_truncated() {
        let request = Request::Authenticate {
            attempt: 1,
            username: "alice".to_owned(),
            credential: Secret::from_bytes(&vec![b'x'; MAX_CREDENTIAL_BYTES + 1]),
        };

        assert!(matches!(
            request.encode(),
            Err(WireError::CredentialTooLong)
        ));
    }

    #[test]
    fn the_bounds_are_the_protocols_and_not_a_call_sites() {
        assert_eq!(PROTOCOL, "inox-auth-v1");
        assert_eq!(VERSION, 1);
        assert_eq!(MAX_USERNAME_BYTES, 128);
        const {
            assert!(
                MAX_FRAME_BYTES < 64 * 1024,
                "a frame bound that needs a lot of memory to reach is not a bound"
            )
        };
    }
}
