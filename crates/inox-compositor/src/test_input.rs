//! Test-only input injection and frame dumps for a *running* compositor process.
//!
//! # This module is test-only
//!
//! Everything here exists so that end-to-end tests can drive a compositor that runs as
//! a separate process. In-process injection (see [`crate::state`]) cannot do that: the
//! M3 acceptance tests must focus windows, type into a real client and prove that a
//! frame really reached the render target of the running binary.
//!
//! The consequences are deliberate and narrow:
//!
//! - It is compiled only when the crate's `test-input` cargo feature is enabled, which
//!   is **off by default**; [`crate::runtime`] never binds a socket without it, and
//!   `--test-input` is rejected with a [`crate::config::ConfigError`] in default builds.
//! - It is not a Wayland global. It advertises nothing to clients and cannot be
//!   reached over the Wayland socket.
//! - The only capability it grants is the ability to send short commands over a unix
//!   socket whose path the operator chose, which is the file access the process
//!   already has; nothing here runs with more privilege than the compositor itself.
//! - The socket belongs to one compositor run and is created without a fixed public
//!   path: the caller passes one, and the tests put it inside a private
//!   `XDG_RUNTIME_DIR`.
//!
//! # Protocol
//!
//! One command per line, ASCII, terminated by `\n`. Arguments are separated by
//! whitespace, so a trailing `\r` (a CRLF client) is tolerated:
//!
//! ```text
//! key <evdev-keycode> press|release
//! motion <x> <y>
//! button <code> press|release
//! dump-frame <path>
//! ```
//!
//! Every command is answered with `ok\n` once it has been applied and the Wayland
//! display has been flushed, or with `err <message>\n` when it was rejected. The reply
//! is the synchronisation primitive the tests use: an `ok` means every client that
//! should have been told about the effect has been sent the event, so no test needs to
//! sleep and guess. A malformed line is answered with `err …` and never takes the
//! compositor or the listener down.
//!
//! `key` takes an **evdev** key code (`KEY_Q` is `16`) and converts it to the XKB code
//! the seat works with, exactly like a libinput event would.
//!
//! # Reader of this file
//!
//! The parser ([`parse_line`]) and the PPM encoder ([`to_ppm`]) are pure functions with
//! unit tests, and [`TestInputConnection`] holds every decision about a live connection
//! (parse, apply, answer, resynchronise) without owning the socket. Only
//! [`bind_listener`] and [`accept_connection`] touch the network at all, and they do
//! nothing but create non-blocking sockets; [`crate::runtime`] owns the calloop wiring.

use std::{
    io::{self, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
};

use smithay::{
    backend::input::{ButtonState, KeyState},
    input::keyboard::Keycode,
};
use tracing::debug;

use crate::render::RenderError;

/// Number of bytes one socket read may return.
const READ_CHUNK: usize = 4096;

/// Longest unterminated line the source buffers before it resynchronises.
///
/// The channel is request/response and the longest legitimate command is a path, so
/// this only ever trips on a client that sends garbage; it exists so that garbage cannot
/// grow the compositor's memory without bound.
const MAX_PENDING_BYTES: usize = 64 * 1024;

/// How often an acknowledgement is retried while the socket reports `WouldBlock`.
const MAX_WRITE_ATTEMPTS: usize = 32;

/// The offset from an evdev key code to the XKB key code the seat consumes.
const XKB_KEYCODE_OFFSET: u32 = 8;

/// Largest evdev key code the parser accepts (`KEY_MAX` in `input-event-codes.h`).
const MAX_EVDEV_KEYCODE: u32 = 0x2ff;

/// Largest evdev button code the parser accepts.
///
/// `BTN_TRIGGER_HAPPY40` is `0x2c7`, so this covers every code a real device can send.
const MAX_BUTTON_CODE: u32 = 0x2ff;

/// Bytes per pixel of the render target readback, which is RGBA8.
const RGBA_BYTES_PER_PIXEL: usize = 4;

/// A command from the test-input control socket.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Feed a key press or release into the seat.
    Key {
        /// The XKB key code handed to the seat.
        ///
        /// The parser has already applied the evdev-to-XKB offset, so this is what
        /// [`crate::state::InoxState::inject_key`] expects.
        keycode: Keycode,
        /// Whether the key went down or came up.
        state: KeyState,
    },
    /// Move the pointer to an absolute position in workspace coordinates.
    Motion {
        /// Horizontal position, in logical pixels.
        x: f64,
        /// Vertical position, in logical pixels.
        y: f64,
    },
    /// Press or release a pointer button at the current pointer position.
    Button {
        /// Evdev button code, for example `0x110` for the left mouse button.
        code: u32,
        /// Whether the button went down or came up.
        state: ButtonState,
    },
    /// Render a frame and write it to `path` as a binary PPM image.
    DumpFrame {
        /// File the frame is written to, replacing any previous contents.
        path: PathBuf,
    },
}

/// Why a line from the control socket was not accepted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The line carried no command at all.
    #[error("empty command")]
    Empty,

    /// The first token is not one of the four verbs.
    #[error("unknown command `{0}`; expected key, motion, button or dump-frame")]
    UnknownVerb(String),

    /// A required argument was not given.
    #[error("`{verb}` requires {expected}")]
    MissingArgument {
        /// The verb that was given.
        verb: &'static str,
        /// What the verb expects to be told.
        expected: &'static str,
    },

    /// An argument was not a number.
    #[error("`{token}` is not {expected}")]
    InvalidNumber {
        /// The rejected token.
        token: String,
        /// What the verb expects to be told.
        expected: &'static str,
    },

    /// An argument was numeric but outside the accepted range.
    #[error("`{token}` is outside the accepted range for {expected}")]
    OutOfRange {
        /// The rejected token.
        token: String,
        /// What the verb expects to be told.
        expected: &'static str,
    },

    /// The press/release argument was neither of the two accepted words.
    #[error("`{token}` is not a press/release state for `{verb}`")]
    InvalidState {
        /// The rejected token.
        token: String,
        /// The verb that was given.
        verb: &'static str,
    },

    /// More arguments were given than the command takes.
    #[error("unexpected argument `{token}`")]
    UnexpectedArgument {
        /// The first argument that has no place.
        token: String,
    },
}

/// Parse one line of the control protocol.
///
/// The trailing newline is optional and any whitespace around tokens is ignored, so a
/// client may send either `\n`-terminated lines or CRLF ones.
///
/// # Errors
///
/// Returns a [`ParseError`] naming the offending token for every malformed line; the
/// caller answers with `err <message>` and keeps serving.
pub fn parse_line(line: &str) -> Result<Command, ParseError> {
    let mut tokens = line.split_whitespace();
    let Some(verb) = tokens.next() else {
        return Err(ParseError::Empty);
    };

    let command = match verb {
        "key" => {
            let code = parse_u32(tokens.next(), "key", "<evdev-keycode>")?;
            check_range(code, MAX_EVDEV_KEYCODE, "an evdev key code")?;
            let state = parse_key_state(tokens.next())?;

            Command::Key {
                keycode: Keycode::new(code + XKB_KEYCODE_OFFSET),
                state,
            }
        }
        "motion" => {
            let x = parse_f64(tokens.next(), "motion", "<x>")?;
            let y = parse_f64(tokens.next(), "motion", "<y>")?;

            Command::Motion { x, y }
        }
        "button" => {
            let code = parse_u32(tokens.next(), "button", "<code>")?;
            check_range(code, MAX_BUTTON_CODE, "an evdev button code")?;
            let state = parse_button_state(tokens.next())?;

            Command::Button { code, state }
        }
        "dump-frame" => {
            let Some(path) = tokens.next() else {
                return Err(ParseError::MissingArgument {
                    verb: "dump-frame",
                    expected: "<path>",
                });
            };

            Command::DumpFrame {
                path: PathBuf::from(path),
            }
        }
        other => return Err(ParseError::UnknownVerb(other.to_owned())),
    };

    if let Some(extra) = tokens.next() {
        return Err(ParseError::UnexpectedArgument {
            token: extra.to_owned(),
        });
    }

    Ok(command)
}

/// Parse a whole number argument.
fn parse_u32(
    token: Option<&str>,
    verb: &'static str,
    expected: &'static str,
) -> Result<u32, ParseError> {
    let Some(token) = token else {
        return Err(ParseError::MissingArgument { verb, expected });
    };

    token.parse().map_err(|_| ParseError::InvalidNumber {
        token: token.to_owned(),
        expected,
    })
}

/// Parse a finite floating point argument.
fn parse_f64(
    token: Option<&str>,
    verb: &'static str,
    expected: &'static str,
) -> Result<f64, ParseError> {
    let Some(token) = token else {
        return Err(ParseError::MissingArgument { verb, expected });
    };

    let value: f64 = token.parse().map_err(|_| ParseError::InvalidNumber {
        token: token.to_owned(),
        expected,
    })?;

    if !value.is_finite() {
        return Err(ParseError::OutOfRange {
            token: token.to_owned(),
            expected,
        });
    }

    Ok(value)
}

/// Reject a code outside `1..=max`.
fn check_range(value: u32, max: u32, expected: &'static str) -> Result<(), ParseError> {
    if !(1..=max).contains(&value) {
        return Err(ParseError::OutOfRange {
            token: value.to_string(),
            expected,
        });
    }

    Ok(())
}

/// Parse the `press` / `release` argument of `key`.
fn parse_key_state(token: Option<&str>) -> Result<KeyState, ParseError> {
    match token {
        Some("press") => Ok(KeyState::Pressed),
        Some("release") => Ok(KeyState::Released),
        Some(other) => Err(ParseError::InvalidState {
            token: other.to_owned(),
            verb: "key",
        }),
        None => Err(ParseError::MissingArgument {
            verb: "key",
            expected: "press|release",
        }),
    }
}

/// Parse the `press` / `release` argument of `button`.
fn parse_button_state(token: Option<&str>) -> Result<ButtonState, ParseError> {
    match token {
        Some("press") => Ok(ButtonState::Pressed),
        Some("release") => Ok(ButtonState::Released),
        Some(other) => Err(ParseError::InvalidState {
            token: other.to_owned(),
            verb: "button",
        }),
        None => Err(ParseError::MissingArgument {
            verb: "button",
            expected: "press|release",
        }),
    }
}

/// Why a frame could not be encoded as a PPM image.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PpmError {
    /// The pixel buffer is shorter than the geometry requires.
    #[error("short frame: expected {expected} bytes, got {actual}")]
    ShortFrame {
        /// Number of bytes the geometry requires.
        expected: usize,
        /// Number of bytes the frame actually holds.
        actual: usize,
    },

    /// The geometry does not fit the header the format allows.
    #[error("the frame is too large to be encoded as a PPM image")]
    TooLarge,
}

/// Encode RGBA8 pixels as a binary PPM (`P6`) image.
///
/// PPM is the one image format a test can decode with no dependency at all, which keeps
/// the "the X11 window really was rendered" assertion free of extra crates. The alpha
/// channel is dropped, because PPM has none.
///
/// # Errors
///
/// Returns [`PpmError::ShortFrame`] when `rgba` is smaller than `width * height * 4`
/// bytes, and [`PpmError::TooLarge`] when the geometry overflows the header.
pub fn to_ppm(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, PpmError> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(PpmError::TooLarge)?;
    let expected = pixels
        .checked_mul(RGBA_BYTES_PER_PIXEL)
        .ok_or(PpmError::TooLarge)?;

    if rgba.len() < expected {
        return Err(PpmError::ShortFrame {
            expected,
            actual: rgba.len(),
        });
    }

    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    ppm.reserve(pixels * 3);

    for pixel in rgba[..expected].as_chunks::<RGBA_BYTES_PER_PIXEL>().0 {
        ppm.extend_from_slice(&pixel[..3]);
    }

    Ok(ppm)
}

/// Why a parsed command could not be applied to the running compositor.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// Rendering the frame a `dump-frame` command asked for failed.
    #[error("failed to render a frame: {0}")]
    Render(#[from] RenderError),

    /// The rendered frame could not be encoded as a PPM image.
    #[error("failed to encode the frame: {0}")]
    Ppm(#[from] PpmError),

    /// Writing the frame dump failed.
    #[error("failed to write {path}: {source}")]
    Dump {
        /// The file the frame should have been written to.
        path: String,
        /// The underlying write failure.
        source: io::Error,
    },

    /// The events the command produced could not be sent to the clients.
    #[error("failed to flush the wayland display: {0}")]
    Flush(#[source] io::Error),
}

/// What one service pass concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// The connection is healthy and can keep receiving commands.
    Open,
    /// The peer went away, or stopped reading its replies; the source must be removed.
    Closed,
}

/// Bind the control socket `--test-input` names and put it in non-blocking mode.
///
/// The caller chooses the path — nothing here decides where the socket lives.
///
/// # Errors
///
/// Returns the bind failure, or a failure to switch the listener to non-blocking mode,
/// so that [`crate::runtime`] can report it as a startup error.
pub fn bind_listener(path: &Path) -> io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    listener.set_nonblocking(true)?;

    Ok(listener)
}

/// Accept one pending connection, switching it to non-blocking mode.
///
/// Level-triggered: a listener with several queued connections is asked again on the
/// next loop iteration, which is what makes one-at-a-time serving the caller's decision
/// rather than this function's.
///
/// Returns `None` when nothing is pending. `Some(Err(_))` means a connection did arrive
/// but cannot be served with this loop — dropping it is the only option, since a blocking
/// socket would stall the compositor.
pub fn accept_connection(listener: &UnixListener) -> Option<io::Result<UnixStream>> {
    match listener.accept() {
        Ok((stream, _)) => Some(stream.set_nonblocking(true).map(|()| stream)),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => None,
        Err(error) => Some(Err(error)),
    }
}

/// The request half of one accepted test-input connection.
///
/// It holds only what the peer has sent that is not yet a complete line; the socket
/// itself stays owned by the calloop source that receives it. That split keeps every
/// decision in this module — parsing, applying, answering, resynchronising — testable
/// without a running loop.
#[derive(Debug, Default)]
pub struct TestInputConnection {
    /// Bytes received since the last newline.
    pending: Vec<u8>,
}

impl TestInputConnection {
    /// Read everything `stream` has to offer, apply every complete line, and answer it.
    ///
    /// The callback is handed one parsed command at a time; its result becomes the
    /// acknowledgement the client reads back (`ok` or `err <message>`), so returning
    /// `Ok` is the statement "this command took effect".
    pub fn service<F>(&mut self, stream: &UnixStream, mut callback: F) -> ConnectionState
    where
        F: FnMut(Command) -> Result<(), CommandError>,
    {
        let mut lines = Vec::new();
        let outcome = self.read_available(stream, &mut lines);

        let mut dropped = false;
        for line in &lines {
            if self.apply(stream, line, &mut callback) {
                dropped = true;
            }
        }

        match outcome {
            ReadOutcome::Progress if dropped => ConnectionState::Closed,
            ReadOutcome::Progress => ConnectionState::Open,
            ReadOutcome::LineTooLong => {
                // Resynchronise rather than tear the connection down: the next real
                // command after the oversized line is still served.
                debug!("test-input client sent an over-long line, dropping it");

                let reply = Reply::Error(format!("line longer than {MAX_PENDING_BYTES} bytes"));
                if self.reply(stream, &reply) {
                    ConnectionState::Closed
                } else {
                    ConnectionState::Open
                }
            }
            ReadOutcome::Closed => {
                debug!("test-input client disconnected");

                ConnectionState::Closed
            }
        }
    }

    /// Drain the socket and split what it held into complete lines.
    fn read_available(&mut self, stream: &UnixStream, lines: &mut Vec<Vec<u8>>) -> ReadOutcome {
        // `&UnixStream` is itself `Read`, which is what lets the socket stay owned by the
        // event loop source while this borrows it.
        let mut reader = stream;
        let mut chunk = [0u8; READ_CHUNK];
        let mut closed = false;

        loop {
            match reader.read(&mut chunk) {
                Ok(0) => {
                    closed = true;
                    break;
                }
                Ok(read) => {
                    self.pending.extend_from_slice(&chunk[..read]);

                    if read < chunk.len() {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // A read error and EOF mean the same thing here: the peer is gone. The
                // caller closes the connection either way, so the specific cause would
                // only be logged and never acted upon.
                Err(_) => {
                    closed = true;
                    break;
                }
            }
        }

        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            lines.push(self.pending.drain(..=end).collect());
        }

        if closed {
            return ReadOutcome::Closed;
        }

        if self.pending.len() > MAX_PENDING_BYTES {
            self.pending.clear();

            return ReadOutcome::LineTooLong;
        }

        ReadOutcome::Progress
    }

    /// Parse, apply and acknowledge one line.
    ///
    /// Returns whether the connection had to be dropped while answering.
    fn apply<F>(&mut self, stream: &UnixStream, line: &[u8], callback: &mut F) -> bool
    where
        F: FnMut(Command) -> Result<(), CommandError>,
    {
        let reply = match std::str::from_utf8(line) {
            Err(_) => Reply::Error("command is not valid UTF-8".to_owned()),
            Ok(text) => match parse_line(text) {
                Err(error) => Reply::Error(error.to_string()),
                Ok(command) => match callback(command) {
                    Ok(()) => Reply::Ok,
                    Err(error) => Reply::Error(error.to_string()),
                },
            },
        };

        self.reply(stream, &reply)
    }

    /// Answer the client that sent the last line.
    ///
    /// Returns whether the connection had to be dropped.
    fn reply(&mut self, stream: &UnixStream, reply: &Reply) -> bool {
        let text = match reply {
            Reply::Ok => "ok\n".to_owned(),
            Reply::Error(message) => format!("err {message}\n"),
        };

        match write_all_bounded(stream, text.as_bytes()) {
            Ok(()) => false,
            Err(error) => {
                // A client that stopped reading its replies cannot be served
                // meaningfully; closing the connection lets the next one take over.
                debug!(%error, "test-input acknowledgement was not delivered");

                true
            }
        }
    }
}

/// What one read pass produced.
enum ReadOutcome {
    /// The connection is still healthy; complete lines (if any) were collected.
    Progress,
    /// The peer went away; complete lines were still collected.
    Closed,
    /// The buffer grew past [`MAX_PENDING_BYTES`] without a newline and was dropped.
    LineTooLong,
}

/// The reply written back for one line.
enum Reply {
    /// The command was applied and the display flushed.
    Ok,
    /// The line was rejected or could not be applied.
    Error(String),
}

/// Write a short acknowledgement, retrying `WouldBlock` a bounded number of times.
///
/// An acknowledgement is a handful of bytes and the peer is expected to be reading, so a
/// socket that keeps refusing them means the peer stopped draining its replies; the
/// caller then closes the connection instead of blocking the compositor on it.
fn write_all_bounded(stream: &UnixStream, bytes: &[u8]) -> io::Result<()> {
    let mut writer = stream;
    let mut written = 0;
    let mut attempts = 0;

    while written < bytes.len() {
        match writer.write(&bytes[written..]) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),
            Ok(count) => {
                written += count;
                attempts = 0;
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && attempts < MAX_WRITE_ATTEMPTS =>
            {
                attempts += 1;
            }
            Err(error) => return Err(error),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Command {
        parse_line(line).unwrap_or_else(|error| panic!("`{line}` should parse: {error}"))
    }

    fn rejected(line: &str) -> ParseError {
        match parse_line(line) {
            Ok(command) => panic!("`{line}` should be rejected, got {command:?}"),
            Err(error) => error,
        }
    }

    #[test]
    fn key_codes_are_translated_from_evdev_to_xkb() {
        // `KEY_Q` is evdev 16, which the seat sees as XKB 24.
        assert_eq!(
            parsed("key 16 press"),
            Command::Key {
                keycode: Keycode::new(24),
                state: KeyState::Pressed,
            }
        );
        assert_eq!(
            parsed("key 16 release"),
            Command::Key {
                keycode: Keycode::new(24),
                state: KeyState::Released,
            }
        );
    }

    #[test]
    fn motion_and_buttons_parse_their_arguments() {
        assert_eq!(
            parsed("motion 12.5 -4"),
            Command::Motion { x: 12.5, y: -4.0 }
        );
        assert_eq!(
            parsed("button 272 press"),
            Command::Button {
                code: 0x110,
                state: ButtonState::Pressed,
            }
        );
        assert_eq!(
            parsed("button 272 release"),
            Command::Button {
                code: 0x110,
                state: ButtonState::Released,
            }
        );
    }

    #[test]
    fn dump_frame_carries_a_path() {
        assert_eq!(
            parsed("dump-frame /run/inox/frame.ppm"),
            Command::DumpFrame {
                path: PathBuf::from("/run/inox/frame.ppm"),
            }
        );
    }

    #[test]
    fn surrounding_whitespace_and_crlf_are_tolerated() {
        let expected = Command::Motion { x: 1.0, y: 2.0 };

        assert_eq!(parsed("  motion   1 2  "), expected);
        assert_eq!(parsed("motion 1 2\r\n"), expected);
        assert_eq!(parsed("\tmotion\t1\t2\r"), expected);
    }

    #[test]
    fn empty_lines_are_rejected() {
        assert_eq!(rejected(""), ParseError::Empty);
        assert_eq!(rejected("   \r\n"), ParseError::Empty);
    }

    #[test]
    fn unknown_verbs_are_rejected() {
        assert_eq!(
            rejected("teleport 1 2"),
            ParseError::UnknownVerb("teleport".to_owned())
        );
    }

    #[test]
    fn missing_arguments_are_rejected() {
        assert_eq!(
            rejected("key"),
            ParseError::MissingArgument {
                verb: "key",
                expected: "<evdev-keycode>",
            }
        );
        assert_eq!(
            rejected("key 16"),
            ParseError::MissingArgument {
                verb: "key",
                expected: "press|release",
            }
        );
        assert_eq!(
            rejected("motion 1"),
            ParseError::MissingArgument {
                verb: "motion",
                expected: "<y>",
            }
        );
        assert_eq!(
            rejected("button 272"),
            ParseError::MissingArgument {
                verb: "button",
                expected: "press|release",
            }
        );
        assert_eq!(
            rejected("dump-frame"),
            ParseError::MissingArgument {
                verb: "dump-frame",
                expected: "<path>",
            }
        );
    }

    #[test]
    fn non_numeric_arguments_are_rejected() {
        assert_eq!(
            rejected("key q press"),
            ParseError::InvalidNumber {
                token: "q".to_owned(),
                expected: "<evdev-keycode>",
            }
        );
        assert_eq!(
            rejected("button left press"),
            ParseError::InvalidNumber {
                token: "left".to_owned(),
                expected: "<code>",
            }
        );
        assert_eq!(
            rejected("motion 1.0.0 2"),
            ParseError::InvalidNumber {
                token: "1.0.0".to_owned(),
                expected: "<x>",
            }
        );
        assert_eq!(
            rejected("key -1 press"),
            ParseError::InvalidNumber {
                token: "-1".to_owned(),
                expected: "<evdev-keycode>",
            }
        );
    }

    #[test]
    fn out_of_range_arguments_are_rejected() {
        assert_eq!(
            rejected("key 0 press"),
            ParseError::OutOfRange {
                token: "0".to_owned(),
                expected: "an evdev key code",
            }
        );
        assert_eq!(
            rejected("key 768 press"),
            ParseError::OutOfRange {
                token: "768".to_owned(),
                expected: "an evdev key code",
            }
        );
        assert_eq!(
            rejected("button 0 press"),
            ParseError::OutOfRange {
                token: "0".to_owned(),
                expected: "an evdev button code",
            }
        );
        assert_eq!(
            rejected("motion nan 0"),
            ParseError::OutOfRange {
                token: "nan".to_owned(),
                expected: "<x>",
            }
        );
        assert_eq!(
            rejected("motion 0 inf"),
            ParseError::OutOfRange {
                token: "inf".to_owned(),
                expected: "<y>",
            }
        );

        // The extremes of the accepted ranges still parse.
        assert!(parse_line("key 1 press").is_ok());
        assert!(parse_line("key 767 press").is_ok());
    }

    #[test]
    fn a_bad_press_state_is_rejected() {
        assert_eq!(
            rejected("key 16 Press"),
            ParseError::InvalidState {
                token: "Press".to_owned(),
                verb: "key",
            }
        );
        assert_eq!(
            rejected("button 272 down"),
            ParseError::InvalidState {
                token: "down".to_owned(),
                verb: "button",
            }
        );
    }

    #[test]
    fn trailing_arguments_are_rejected() {
        assert_eq!(
            rejected("key 16 press now"),
            ParseError::UnexpectedArgument {
                token: "now".to_owned(),
            }
        );
        assert_eq!(
            rejected("dump-frame /tmp/a /tmp/b"),
            ParseError::UnexpectedArgument {
                token: "/tmp/b".to_owned(),
            }
        );
    }

    #[test]
    fn a_frame_is_encoded_as_a_binary_ppm() {
        // Two pixels: pure red, then the render target's clear colour.
        let frame = [255u8, 0, 0, 255, 17, 34, 51, 255];

        let ppm = to_ppm(&frame, 2, 1).expect("the frame should encode");

        assert_eq!(&ppm[..11], b"P6\n2 1\n255\n");
        assert_eq!(&ppm[11..], &[255u8, 0, 0, 17, 34, 51]);
    }

    #[test]
    fn a_frame_shorter_than_its_geometry_is_rejected() {
        assert_eq!(
            to_ppm(&[0u8; 7], 2, 1),
            Err(PpmError::ShortFrame {
                expected: 8,
                actual: 7,
            })
        );
    }

    #[test]
    fn a_frame_larger_than_its_geometry_keeps_only_the_first_pixels() {
        let ppm = to_ppm(&[1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], 2, 1)
            .expect("the frame should encode");

        assert_eq!(&ppm[11..], &[1u8, 2, 3, 5, 6, 7]);
    }
}
