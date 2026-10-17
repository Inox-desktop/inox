//! X11 display numbers and their UNIX sockets.
//!
//! The service only needs a small slice of X11 knowledge: how a display is
//! spelled (`:1`), where its socket lives (`<dir>/X1`) and whether that socket
//! completes a connection setup. Keeping it here keeps the supervision logic
//! free of X11 details and makes the readiness probe trivially testable.
//!
//! "Ready" is deliberately *not* "the socket file exists and accepts a
//! connection": acceptance item X01 requires a real X11 round trip, which lives
//! in [`crate::handshake`].

use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::handshake::{self, HandshakeError, Setup};

/// Default directory where Xwayland places its UNIX sockets.
pub const DEFAULT_SOCKET_DIR: &str = "/tmp/.X11-unix";

/// Errors from parsing an X11 display specification.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DisplayError {
    /// The display specification was empty.
    #[error("display specification must not be empty")]
    Empty,

    /// The display specification did not start with a `:`.
    #[error("display specification `{0}` must start with `:`")]
    MissingColon(String),

    /// The part after `:` was not a decimal number.
    #[error("display specification `{0}` has a non-numeric display number")]
    NotANumber(String),

    /// The display number was larger than [`Display::MAX`].
    #[error("display number {0} is out of range")]
    OutOfRange(u32),
}

/// An X11 display number such as `:1`.
///
/// The number is validated on the way in (see [`Display::parse`]), so a
/// `Display` can always be rendered as a well formed X11 display specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Display(u16);

impl Display {
    /// Largest accepted display number.
    pub const MAX: u16 = 9999;

    /// Build a display from a raw number.
    ///
    /// Numbers above [`Display::MAX`] are clamped to [`Display::MAX`] so that
    /// this constructor cannot panic.
    pub fn new(number: u16) -> Self {
        Self(number.min(Self::MAX))
    }

    /// The raw display number.
    pub fn number(self) -> u16 {
        self.0
    }

    /// Parse `":N"` (leading `:` required, `N` in `0..=MAX`).
    ///
    /// # Errors
    ///
    /// Returns a [`DisplayError`] for empty specifications, specifications that
    /// do not start with `:`, non-numeric numbers and numbers above
    /// [`Display::MAX`].
    pub fn parse(spec: &str) -> Result<Self, DisplayError> {
        if spec.is_empty() {
            return Err(DisplayError::Empty);
        }

        let digits = spec
            .strip_prefix(':')
            .ok_or_else(|| DisplayError::MissingColon(spec.to_owned()))?;

        let number: u32 = digits
            .parse()
            .map_err(|_| DisplayError::NotANumber(spec.to_owned()))?;

        let number = u16::try_from(number).map_err(|_| DisplayError::OutOfRange(number))?;

        if number > Self::MAX {
            return Err(DisplayError::OutOfRange(u32::from(number)));
        }

        Ok(Self(number))
    }

    /// `":N"`.
    pub fn name(self) -> String {
        format!(":{}", self.0)
    }

    /// Path to the display socket inside `dir` (`dir/X<N>`).
    pub fn socket_path_in(self, dir: &Path) -> PathBuf {
        dir.join(format!("X{}", self.0))
    }

    /// Connect to the display socket in `dir` and complete an X11 connection
    /// setup, bounded by `timeout`.
    ///
    /// This is the readiness check: it only succeeds when the X11 server behind
    /// the socket really answers a setup request, which is what acceptance item
    /// X01 requires (a socket that merely exists is not enough). Never panics.
    ///
    /// # Errors
    ///
    /// Returns [`HandshakeError::Connect`] when `<dir>/X<N>` cannot be connected
    /// to, and any other [`HandshakeError`] when the round trip does not end in
    /// a well formed successful setup reply.
    pub fn setup_in(self, dir: &Path, timeout: Duration) -> Result<Setup, HandshakeError> {
        handshake::probe(&self.socket_path_in(dir), timeout)
    }
}

impl fmt::Display for Display {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, ":{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::net::UnixListener;

    #[test]
    fn display_names_round_trip() {
        for (spec, number) in [(":0", 0), (":1", 1), (":42", 42), (":9999", 9999)] {
            let display = Display::parse(spec).expect("expected a valid display");

            assert_eq!(display.number(), number);
            assert_eq!(display.name(), spec);
            assert_eq!(display.to_string(), spec);
        }
    }

    #[test]
    fn raw_numbers_are_clamped_to_the_maximum() {
        assert_eq!(Display::new(7).number(), 7);
        assert_eq!(Display::new(u16::MAX).number(), Display::MAX);
    }

    #[test]
    fn invalid_display_specifications_are_rejected() {
        assert_eq!(Display::parse(""), Err(DisplayError::Empty));
        assert_eq!(
            Display::parse("1"),
            Err(DisplayError::MissingColon("1".to_owned()))
        );
        assert_eq!(
            Display::parse(":x"),
            Err(DisplayError::NotANumber(":x".to_owned()))
        );
        assert_eq!(
            Display::parse(":"),
            Err(DisplayError::NotANumber(":".to_owned()))
        );
        assert_eq!(
            Display::parse(":10000"),
            Err(DisplayError::OutOfRange(10_000))
        );
        assert_eq!(
            Display::parse(":65536"),
            Err(DisplayError::OutOfRange(65_536))
        );
        assert_eq!(
            Display::parse(":99999999999999999999"),
            Err(DisplayError::NotANumber(":99999999999999999999".to_owned()))
        );
    }

    #[test]
    fn socket_paths_use_the_x_prefix() {
        let dir = Path::new("/tmp/example");

        assert_eq!(
            Display::parse(":3").expect("valid").socket_path_in(dir),
            PathBuf::from("/tmp/example/X3")
        );
    }

    #[test]
    fn a_missing_socket_is_not_ready() {
        let dir = tempfile::tempdir().expect("temp dir");
        let display = Display::parse(":77").expect("valid");

        let outcome = display.setup_in(dir.path(), Duration::from_millis(200));

        assert!(matches!(outcome, Err(HandshakeError::Connect { .. })));
    }

    #[test]
    fn a_socket_that_never_answers_is_not_ready() {
        let dir = tempfile::tempdir().expect("temp dir");
        let display = Display::parse(":77").expect("valid");

        // The listener accepts the connection (into its backlog) but never
        // reads the request or writes a reply: exactly the case X01 says a
        // "socket exists" probe would wrongly call ready.
        let _listener = UnixListener::bind(display.socket_path_in(dir.path())).expect("bind");

        let outcome = display.setup_in(dir.path(), Duration::from_millis(200));

        assert!(matches!(outcome, Err(HandshakeError::Timeout { .. })));
    }
}
