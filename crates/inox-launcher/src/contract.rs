//! The vocabulary of the launcher's service contract.
//! The vocabulary of the launcher's service contract.
//! the interaction contract asks every L3 component to distinguish four things that a launcher
//! is unusually prone to blurring:
//!
//! - **sent / accepted / applied / rejected / timeout** — a command's state at
//!   the *service*, not at the widget ([`CommandState`]);
//! - a **generation** carried by every request and result, so a stale result
//!   can be recognised and dropped instead of overwriting a newer view
//!   ([`crate::search`], [`ServiceState`]);
//! - a **readable failure**, with a cause that survives to the screen
//!   ([`Failure`], [`FailureKind`]) rather than an optimistic UI that assumes
//!   the launch worked;
//! - the three states the acceptance criteria name as design states —
//!   loading, empty, error — plus the retry action that has to come with the
//!   error (acceptance D05).
//!
//! Nothing here is Qt-specific and nothing here does I/O: the types are the
//! data the state machine moves between, which is what makes the transitions
//! testable without a compositor.

/// Where a command has reached, per the interaction contract, rule 2.
///
/// The distinction that matters is between [`CommandState::Sent`] and
/// [`CommandState::Applied`]: the launcher knows it handed a request to the
/// process boundary, and separately knows that the process is still alive
/// afterwards. Treating those two as one is exactly the optimistic UI the decision
/// rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommandState {
    /// Handed to the process boundary; nothing observed yet.
    Sent,
    /// The boundary acknowledged the request (a child process exists).
    Accepted,
    /// The effect was verified: the application is running.
    Applied,
    /// The service refused, or the effect could not be produced.
    Rejected,
    /// The bounded wait expired without evidence.
    TimedOut,
}

/// The application index's own state, as the component sees it.
///
/// This is the service half of the interaction contract's snapshot/event contract: the index is
/// the authoritative list of applications, and a UI that cannot read it says so
/// with a reason and a retry rather than spinning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    /// A scan is in flight; `generation` identifies it.
    Loading {
        /// The scan this state belongs to.
        generation: u64,
    },
    /// The scan finished and the index is usable.
    Ready {
        /// The scan this state belongs to.
        generation: u64,
        /// How many entries the index holds.
        entries: usize,
        /// How many files were skipped as unusable.
        skipped: usize,
        /// Where the index was read from, for the empty-state message.
        directories: Vec<std::path::PathBuf>,
    },
    /// The scan could not produce an index at all.
    Unavailable {
        /// The scan this state belongs to.
        generation: u64,
        /// What went wrong, in a form the UI can localise.
        failure: Failure,
    },
}

impl ServiceState {
    /// The generation this state belongs to.
    pub fn generation(&self) -> u64 {
        match self {
            Self::Loading { generation }
            | Self::Ready { generation, .. }
            | Self::Unavailable { generation, .. } => *generation,
        }
    }

    /// Whether the index can be searched right now.
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// Why an action could not be carried out.
///
/// Every variant is a *cause*, not a message: [`crate::view_model`] turns one
/// into a Fluent message, which is what keeps user-visible text out of the
/// domain model (root AGENTS.md §7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    /// The index scan failed. `detail` is the operating system's own words.
    IndexUnavailable {
        /// The underlying error, as text.
        detail: String,
    },
    /// The entry has no `Exec` key.
    NoCommand,
    /// The `Exec` value uses a field code the specification does not define.
    UnknownFieldCode {
        /// The offending code, as written after the `%`.
        code: char,
    },
    /// A multi-valued field code appeared inside a larger word.
    MisplacedFieldCode {
        /// The offending code, as written after the `%`.
        code: char,
    },
    /// The `Exec` value has an unbalanced double quote.
    UnterminatedQuote,
    /// The `Exec` value ends with a stray backslash.
    TrailingEscape,
    /// The `Exec` value ends with a `%` that introduces no field code.
    TrailingPercent,
    /// The entry asks for a terminal and `launcher.terminal` is empty.
    MissingTerminalPrefix,
    /// The process could not be created.
    SpawnFailed {
        /// The underlying error, as text.
        detail: String,
    },
    /// The process was created and exited before the readiness wait ended.
    ExitedBeforeReady {
        /// Exit status, or the error that reading it produced, as text.
        detail: String,
    },
    /// No evidence of progress within the bounded wait.
    NotReadyInTime {
        /// The bound that expired, in seconds.
        seconds: u64,
    },
}

/// A [`FailureKind`] together with the entry it happened to, if any.
///
/// The entry is carried separately from the kind so that the two failures a
/// launcher reports most often — "this entry is unusable" and "this entry could
/// not be started" — share one rendering path, and a failure that belongs to no
/// entry (a failed index scan) is representable without inventing a name for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// What went wrong.
    pub kind: FailureKind,
    /// The desktop entry's localized name, when the failure belongs to one.
    pub entry: Option<String>,
}

impl Failure {
    /// A failure that belongs to no particular entry.
    pub fn service(kind: FailureKind) -> Self {
        Self { kind, entry: None }
    }

    /// A failure that belongs to `entry`.
    pub fn for_entry(kind: FailureKind, entry: impl Into<String>) -> Self {
        Self {
            kind,
            entry: Some(entry.into()),
        }
    }
}
