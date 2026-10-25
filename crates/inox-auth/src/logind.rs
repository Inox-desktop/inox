//! logind: which VT a session goes on, and which session gets the display back.
//!
//! the boot login chain
//! The session leader's `pam_open_session` creates the logind session (through
//! `pam_systemd`), and `Manager.ActivateSession` is what puts it — and the display
//! — on a VT. Nothing here performs a VT ioctl, takes a DRM fd or owns a seat: the
//! only authority this module exercises is "ask logind, then report what it said".
//!
//! Two rules live here because they are the boundary's, not logind's:
//!
//! - **Where a session goes.** [`first_free_vt`] scans the configured range and
//!   takes the first VT that has no session on it and is not the VT of the session
//!   that is active now. The input is a [`Logind`] rather than a bus connection so
//!   that "skip the active session", "skip an occupied VT" and "the range can be
//!   exhausted" are unit tests.
//! - **What the display falls back to.** [`hand_back_at_startup`] is the self-heal
//!   the boot login chain
//!   ended" and "the display was handed back" leaves a seat with no active
//!   session, and the greeter's own session is then put back.
//!
//! Everything degrades rather than blocks. CI runners and development instances
//! have no system bus, and a session that cannot be placed on a VT is still a
//! session: the server logs a warning and starts it anyway ([`first_free_vt`]'s
//! error), while the hand-back logs and does nothing.

use std::fmt;
use std::sync::Mutex;

use thiserror::Error;
use zbus::Connection;
use zbus::OwnedObjectPath;
use zbus::Proxy;

/// The bus name logind owns.
pub const LOGIND_DESTINATION: &str = "org.freedesktop.login1";

/// The `Manager` object, which is where every call in this module starts.
pub const MANAGER_PATH: &str = "/org/freedesktop/login1";

/// The manager interface.
pub const MANAGER_INTERFACE: &str = "org.freedesktop.login1.Manager";

/// The seat interface, read for `ActiveSession`.
pub const SEAT_INTERFACE: &str = "org.freedesktop.login1.Seat";

/// The session interface, read for `Id` and `VT`.
pub const SESSION_INTERFACE: &str = "org.freedesktop.login1.Session";

/// The seat a session is placed on unless the deployment names another.
///
/// A single-seat machine calls it `seat0`; the name is a unit-level fact, which is
/// why it is configuration (`--seat`) rather than a constant read from a file
/// somewhere.
pub const DEFAULT_SEAT: &str = "seat0";

/// The lowest VT a user session is placed on by default.
///
/// VT 1 belongs to the greeter, and the rescue getty lives on VT 3.
pub const DEFAULT_VT_MIN: u32 = 2;

/// The highest VT a user session is placed on by default.
pub const DEFAULT_VT_MAX: u32 = 6;

/// What a logind query is about.
///
/// `Path` is what `GetSessionByVT`/`GetSessionByPID`/`ActiveSession` answer with;
/// the id is what `ActivateSession` wants, and the VT is what "do not put the
/// session on the display's VT" needs.
type ListedSession = (String, u32, String, String, OwnedObjectPath);

/// Why logind could not answer.
#[derive(Debug, Error)]
pub enum LogindError {
    /// The system bus, or logind on it, did not answer.
    #[error("logind is not reachable: {0}")]
    Bus(#[from] zbus::Error),
    /// There is no logind to ask at all.
    ///
    /// A development instance and a CI runner are both this case. It is a
    /// distinct variant from [`LogindError::Bus`] because it is not a failure of
    /// anything: the caller is expected to continue without VT arbitration.
    #[error("no logind is available")]
    NotAvailable,
}

/// A session logind knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownSession {
    /// The session id, as logind spells it. This is what `ActivateSession` takes.
    pub id: String,
    /// The VT the session is on, when logind would tell us (`VT` is `0` for a
    /// session that is not on a VT at all).
    pub vt: Option<u32>,
}

/// A session on the configured seat, as `Manager.ListSessions` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatedSession {
    /// The session id.
    pub id: String,
    /// The uid the session belongs to.
    pub uid: u32,
}

/// The logind calls this boundary makes, as a seam.
///
/// The trait exists so the selection and hand-back rules are unit tests: the
/// system implementation is the only one that speaks D-Bus, and the tests replace
/// it exactly there (root AGENTS.md §6 allows stubbing a system boundary).
pub trait Logind: Send + Sync {
    /// The session that is active on the configured seat, if any.
    ///
    /// # Errors
    ///
    /// [`LogindError`] when the bus or logind is not reachable.
    fn active_session(&self) -> Result<Option<KnownSession>, LogindError>;

    /// The session on `vt`, if any.
    ///
    /// # Errors
    ///
    /// [`LogindError`] when the bus or logind is not reachable.
    fn session_on_vt(&self, vt: u32) -> Result<Option<KnownSession>, LogindError>;

    /// The session the process `pid` belongs to, if any.
    ///
    /// This is how a session leader learns the session `pam_open_session` just
    /// created for it without reading `XDG_SESSION_ID` out of its own environment
    /// (which edition 2024 makes impossible to write, and which the safe `pam`
    /// wrapper never sets — see `pam::open_session`).
    ///
    /// # Errors
    ///
    /// [`LogindError`] when the bus or logind is not reachable.
    fn session_of_pid(&self, pid: u32) -> Result<Option<KnownSession>, LogindError>;

    /// Every session on the configured seat.
    ///
    /// # Errors
    ///
    /// [`LogindError`] when the bus or logind is not reachable.
    fn sessions_on_seat(&self) -> Result<Vec<SeatedSession>, LogindError>;

    /// Makes `id` the active session on the seat it belongs to.
    ///
    /// # Errors
    ///
    /// [`LogindError`] when the bus or logind is not reachable, or when logind
    /// refuses (the session ended while the daemon was looking at it).
    fn activate(&self, id: &str) -> Result<(), LogindError>;
}

/// The interval of VTs a user session may be placed on.
///
/// Constructed through [`VtRange::new`] so that an inverted or zero-based range is
/// a start-up refusal rather than a login that silently goes to the wrong place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VtRange {
    min: u32,
    max: u32,
}

impl Default for VtRange {
    /// `2..=6`: the boot login chain
    fn default() -> Self {
        Self {
            min: DEFAULT_VT_MIN,
            max: DEFAULT_VT_MAX,
        }
    }
}

impl VtRange {
    /// The range `min..=max`.
    ///
    /// # Errors
    ///
    /// [`VtRangeError::Zero`] for a VT below 1 (there is no tty0) and
    /// [`VtRangeError::Empty`] for an inverted range.
    pub fn new(min: u32, max: u32) -> Result<Self, VtRangeError> {
        if min == 0 {
            return Err(VtRangeError::Zero);
        }
        if max < min {
            return Err(VtRangeError::Empty { min, max });
        }

        Ok(Self { min, max })
    }

    /// The lowest VT in the range.
    pub fn min(&self) -> u32 {
        self.min
    }

    /// The highest VT in the range.
    pub fn max(&self) -> u32 {
        self.max
    }

    /// Whether `vt` is inside the range.
    pub fn contains(&self, vt: u32) -> bool {
        (self.min..=self.max).contains(&vt)
    }

    /// The VTs in the range, ascending.
    pub fn iter(&self) -> std::ops::RangeInclusive<u32> {
        self.min..=self.max
    }
}

impl fmt::Display for VtRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}..={}", self.min, self.max)
    }
}

/// Why a VT range is not usable.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum VtRangeError {
    /// A VT is numbered from 1; `/dev/tty0` is the current VT, not a usable one.
    #[error("a VT is numbered from 1, not from 0")]
    Zero,
    /// The range names no VT at all.
    #[error("the VT range {min}..={max} is empty")]
    Empty {
        /// The lowest VT that was asked for.
        min: u32,
        /// The highest VT that was asked for.
        max: u32,
    },
}

/// The first VT in `range` that has no session on it, skipping `skip`.
///
/// `skip` is the VT of the session that is active now (the greeter's, in the
/// shipped deployment): putting the user session there would replace the display
/// the person is looking at instead of switching away from it.
///
/// # Errors
///
/// Propagates [`LogindError`] from the query. The caller decides what a missing
/// logind means; this function does not guess.
pub fn first_free_vt(
    range: VtRange,
    skip: &[u32],
    logind: &dyn Logind,
) -> Result<Option<u32>, LogindError> {
    for vt in range.iter() {
        if skip.contains(&vt) {
            continue;
        }
        if logind.session_on_vt(vt)?.is_none() {
            return Ok(Some(vt));
        }
    }

    Ok(None)
}

/// The VT to use when logind cannot answer, or `None` when `range` is all skipped.
///
/// This is the degradation the boot login chain's operator-facing rule needs: a machine
/// without logind (CI, a development instance) has no VT arbitration to lose, and
/// refusing to start the session would be a worse answer than starting it on the
/// first VT in the configured range.
pub fn first_free_vt_without_logind(range: VtRange, skip: &[u32]) -> Option<u32> {
    range.iter().find(|vt| !skip.contains(vt))
}

/// What [`hand_back_at_startup`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupHandBack {
    /// A session is already active on the seat; nothing was touched.
    Active,
    /// No session was active and the greeter's own session was activated.
    Activated,
    /// No session was active and no session of the greeter's uid exists on the
    /// seat: the display manager is not up yet, or its session is on another seat.
    NoGreeterSession,
    /// No session was active and more than one session of the greeter's uid
    /// exists: activating one of them would be a guess, so nothing was touched.
    Ambiguous,
    /// The greeter account is not known to this boundary, so its session cannot be
    /// recognised.
    UnknownGreeter,
}

/// Hands the display back to the greeter after a helper restart.
///
/// The failure this repairs is the boot login chain
/// helper died while a user session ran, the session ended, and because nobody was
/// left to call `ActivateSession` the seat has no active session. A freshly
/// restarted helper cannot know which session id it started, so it asks a question
/// it can answer: is anything active on the seat? If yes, there is nothing to
/// repair. If no, the greeter's own session — the one belonging to the greeter
/// account on this seat — is put back.
///
/// # Errors
///
/// Propagates [`LogindError`] from the queries, and from the activation when a
/// unique greeter session was found.
pub fn hand_back_at_startup(
    logind: &dyn Logind,
    greeter_uid: Option<u32>,
) -> Result<StartupHandBack, LogindError> {
    if logind.active_session()?.is_some() {
        return Ok(StartupHandBack::Active);
    }

    let Some(greeter_uid) = greeter_uid else {
        return Ok(StartupHandBack::UnknownGreeter);
    };

    let greeter_sessions: Vec<SeatedSession> = logind
        .sessions_on_seat()?
        .into_iter()
        .filter(|session| session.uid == greeter_uid)
        .collect();

    match greeter_sessions.as_slice() {
        [] => Ok(StartupHandBack::NoGreeterSession),
        [session] => {
            logind.activate(&session.id)?;
            Ok(StartupHandBack::Activated)
        }
        _ => Ok(StartupHandBack::Ambiguous),
    }
}

/// The logind client this boundary ships.
///
/// The connection is kept and reused: a login is a handful of calls, but they are
/// spread over the socket service and the session thread, and reconnecting to the
/// system bus for each of them would be work done for nobody. A failed call drops
/// the cached connection so the next attempt reconnects — a bus that restarted
/// (`systemctl restart dbus`) must not leave this process holding a dead socket
/// forever.
pub struct SystemLogind {
    seat: String,
    connection: Mutex<Option<Connection>>,
}

impl SystemLogind {
    /// A client for `seat`.
    pub fn new(seat: impl Into<String>) -> Self {
        Self {
            seat: seat.into(),
            connection: Mutex::new(None),
        }
    }

    /// The system bus connection, connecting once.
    fn connection(&self) -> Result<Connection, LogindError> {
        let mut cache = self.cache();
        if let Some(connection) = cache.as_ref() {
            return Ok(connection.clone());
        }

        let connection = zbus::block_on(Connection::system())?;
        *cache = Some(connection.clone());

        Ok(connection)
    }

    /// Forgets the cached connection after a failed call.
    fn forget(&self) {
        *self.cache() = None;
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Option<Connection>> {
        // A poisoned cache only means some other thread panicked while looking at
        // the connection; the connection itself is either usable or not, and both
        // are handled by the caller. Locking again is better than poisoning every
        // later login.
        self.connection
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Turns a bus result into this module's result, dropping the cached
    /// connection when the call failed.
    fn settle<T>(&self, result: zbus::Result<T>) -> Result<T, LogindError> {
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                self.forget();
                Err(LogindError::Bus(error))
            }
        }
    }
}

impl Logind for SystemLogind {
    fn active_session(&self) -> Result<Option<KnownSession>, LogindError> {
        let connection = self.connection()?;
        let result: zbus::Result<Option<KnownSession>> = zbus::block_on(async {
            let manager = manager(&connection).await?;
            let seat_path: OwnedObjectPath =
                manager.call("GetSeat", &(self.seat.as_str(),)).await?;
            let seat = Proxy::new(
                &connection,
                LOGIND_DESTINATION,
                seat_path.as_str(),
                SEAT_INTERFACE,
            )
            .await?;
            let (id, path): (String, OwnedObjectPath) = seat.get_property("ActiveSession").await?;

            if id.is_empty() {
                return Ok(None);
            }

            // The VT is only needed to avoid placing the new session on top of the
            // active one; a session that will not say which VT it is on does not
            // stop the query from being useful.
            let vt = session_vt(&connection, path.as_str()).await.ok().flatten();

            Ok(Some(KnownSession { id, vt }))
        });

        self.settle(result)
    }

    fn session_on_vt(&self, vt: u32) -> Result<Option<KnownSession>, LogindError> {
        let connection = self.connection()?;
        let result: zbus::Result<Option<KnownSession>> = zbus::block_on(async {
            let manager = manager(&connection).await?;
            let path: OwnedObjectPath = manager.call("GetSessionByVT", &(vt, false)).await?;

            if path.as_str() == "/" {
                return Ok(None);
            }

            let id = session_id(&connection, path.as_str()).await?;

            Ok(Some(KnownSession { id, vt: Some(vt) }))
        });

        self.settle(result)
    }

    fn session_of_pid(&self, pid: u32) -> Result<Option<KnownSession>, LogindError> {
        let connection = self.connection()?;
        let result: zbus::Result<Option<KnownSession>> = zbus::block_on(async {
            let manager = manager(&connection).await?;
            let path: OwnedObjectPath = manager.call("GetSessionByPID", &(pid,)).await?;

            if path.as_str() == "/" {
                return Ok(None);
            }

            let id = session_id(&connection, path.as_str()).await?;

            Ok(Some(KnownSession { id, vt: None }))
        });

        self.settle(result)
    }

    fn sessions_on_seat(&self) -> Result<Vec<SeatedSession>, LogindError> {
        let connection = self.connection()?;
        let result: zbus::Result<Vec<SeatedSession>> = zbus::block_on(async {
            let manager = manager(&connection).await?;
            let listed: Vec<ListedSession> = manager.call("ListSessions", &()).await?;

            Ok(listed
                .into_iter()
                .filter(|(_, _, _, seat, _)| *seat == self.seat)
                .map(|(id, uid, _, _, _)| SeatedSession { id, uid })
                .collect())
        });

        self.settle(result)
    }

    fn activate(&self, id: &str) -> Result<(), LogindError> {
        let connection = self.connection()?;
        let result: zbus::Result<()> = zbus::block_on(async {
            let manager = manager(&connection).await?;
            manager.call::<_, _, ()>("ActivateSession", &(id,)).await
        });

        self.settle(result)
    }
}

/// The `Manager` proxy, which every call starts from.
async fn manager(connection: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(
        connection,
        LOGIND_DESTINATION,
        MANAGER_PATH,
        MANAGER_INTERFACE,
    )
    .await
}

/// The `Id` of the session object at `path`.
async fn session_id(connection: &Connection, path: &str) -> zbus::Result<String> {
    let session = Proxy::new(connection, LOGIND_DESTINATION, path, SESSION_INTERFACE).await?;

    session.get_property("Id").await
}

/// The VT of the session object at `path`, when it is on one.
async fn session_vt(connection: &Connection, path: &str) -> zbus::Result<Option<u32>> {
    let session = Proxy::new(connection, LOGIND_DESTINATION, path, SESSION_INTERFACE).await?;
    let vt: u32 = session.get_property("VT").await?;

    Ok((vt > 0).then_some(vt))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A logind whose answers the test writes down.
    #[derive(Default)]
    struct ScriptedLogind {
        active: Option<KnownSession>,
        taken: Vec<u32>,
        seat_sessions: Vec<SeatedSession>,
        activated: Mutex<Vec<String>>,
        queries: Mutex<Vec<u32>>,
        reachable: bool,
    }

    impl ScriptedLogind {
        fn reachable() -> Self {
            Self {
                reachable: true,
                ..Self::default()
            }
        }

        fn activated(&self) -> Vec<String> {
            self.activated.lock().expect("the test's own mutex").clone()
        }

        fn queried(&self) -> Vec<u32> {
            self.queries.lock().expect("the test's own mutex").clone()
        }
    }

    impl Logind for ScriptedLogind {
        fn active_session(&self) -> Result<Option<KnownSession>, LogindError> {
            if !self.reachable {
                return Err(LogindError::NotAvailable);
            }

            Ok(self.active.clone())
        }

        fn session_on_vt(&self, vt: u32) -> Result<Option<KnownSession>, LogindError> {
            if !self.reachable {
                return Err(LogindError::NotAvailable);
            }
            self.queries.lock().expect("the test's own mutex").push(vt);

            Ok(self.taken.contains(&vt).then(|| KnownSession {
                id: format!("session-on-{vt}"),
                vt: Some(vt),
            }))
        }

        fn session_of_pid(&self, _pid: u32) -> Result<Option<KnownSession>, LogindError> {
            if !self.reachable {
                return Err(LogindError::NotAvailable);
            }

            Ok(self.active.clone())
        }

        fn sessions_on_seat(&self) -> Result<Vec<SeatedSession>, LogindError> {
            if !self.reachable {
                return Err(LogindError::NotAvailable);
            }

            Ok(self.seat_sessions.clone())
        }

        fn activate(&self, id: &str) -> Result<(), LogindError> {
            if !self.reachable {
                return Err(LogindError::NotAvailable);
            }
            self.activated
                .lock()
                .expect("the test's own mutex")
                .push(id.to_owned());

            Ok(())
        }
    }

    fn range() -> VtRange {
        VtRange::new(2, 6).expect("2..=6 is a valid range")
    }

    #[test]
    fn the_default_range_is_the_one_adr_0037_names() {
        let range = VtRange::default();

        assert_eq!(range.min(), 2);
        assert_eq!(range.max(), 6);
        assert_eq!(range.to_string(), "2..=6");
        assert!(range.contains(2));
        assert!(range.contains(6), "the range is inclusive at both ends");
        assert!(
            !range.contains(1),
            "the greeter's own VT is not a candidate"
        );
        assert!(!range.contains(7));
    }

    #[test]
    fn a_range_that_names_no_vt_is_refused_rather_than_normalised() {
        assert_eq!(VtRange::new(0, 6), Err(VtRangeError::Zero));
        assert_eq!(
            VtRange::new(6, 2),
            Err(VtRangeError::Empty { min: 6, max: 2 })
        );
        assert!(VtRange::new(6, 6).is_ok(), "one VT is a range too");
    }

    #[test]
    fn the_first_free_vt_skips_the_active_sessions_vt() {
        let logind = ScriptedLogind::reachable();

        let chosen = first_free_vt(range(), &[2], &logind).expect("logind answers");

        assert_eq!(chosen, Some(3));
        assert_eq!(
            logind.queried(),
            vec![3],
            "the skipped VT is not even asked about"
        );
    }

    #[test]
    fn the_first_free_vt_skips_vts_that_already_have_a_session() {
        let logind = ScriptedLogind {
            taken: vec![2, 3],
            ..ScriptedLogind::reachable()
        };

        assert_eq!(
            first_free_vt(range(), &[], &logind).expect("logind answers"),
            Some(4)
        );
    }

    #[test]
    fn a_range_whose_vts_are_all_taken_is_exhausted_rather_than_wrapped() {
        let logind = ScriptedLogind {
            taken: vec![2, 3, 4, 5, 6],
            ..ScriptedLogind::reachable()
        };

        assert_eq!(
            first_free_vt(range(), &[], &logind).expect("logind answers"),
            None,
            "the session is refused rather than put on a busy VT"
        );
    }

    #[test]
    fn the_upper_bound_is_a_candidate_and_the_lower_one_is_not_below_the_range() {
        let logind = ScriptedLogind {
            taken: vec![2, 3, 4, 5],
            ..ScriptedLogind::reachable()
        };

        assert_eq!(
            first_free_vt(VtRange::new(2, 6).expect("valid"), &[], &logind).expect("answers"),
            Some(6)
        );
        assert_eq!(
            logind.queried(),
            vec![2, 3, 4, 5, 6],
            "nothing outside the range is asked about"
        );
    }

    #[test]
    fn a_logind_that_cannot_answer_is_an_error_the_caller_degrades_from() {
        let logind = ScriptedLogind::default();

        assert!(matches!(
            first_free_vt(range(), &[], &logind),
            Err(LogindError::NotAvailable)
        ));
    }

    #[test]
    fn without_logind_the_first_vt_that_is_not_skipped_is_used() {
        let range = VtRange::new(2, 4).expect("valid");

        assert_eq!(first_free_vt_without_logind(range, &[]), Some(2));
        assert_eq!(
            first_free_vt_without_logind(range, &[2, 3]),
            Some(4),
            "a skipped VT is still skipped without logind"
        );
        assert_eq!(
            first_free_vt_without_logind(range, &[2, 3, 4]),
            None,
            "the range can be exhausted even without logind"
        );
    }

    #[test]
    fn a_startup_with_an_active_session_touches_nothing() {
        let logind = ScriptedLogind {
            active: Some(KnownSession {
                id: "4".to_owned(),
                vt: Some(2),
            }),
            seat_sessions: vec![SeatedSession {
                id: "4".to_owned(),
                uid: 1000,
            }],
            ..ScriptedLogind::reachable()
        };

        assert_eq!(
            hand_back_at_startup(&logind, Some(967)).expect("logind answers"),
            StartupHandBack::Active
        );
        assert!(
            logind.activated().is_empty(),
            "an active session is not replaced by the greeter"
        );
    }

    #[test]
    fn a_seat_with_no_active_session_gets_its_greeter_session_back() {
        let logind = ScriptedLogind {
            seat_sessions: vec![SeatedSession {
                id: "7".to_owned(),
                uid: 967,
            }],
            ..ScriptedLogind::reachable()
        };

        assert_eq!(
            hand_back_at_startup(&logind, Some(967)).expect("logind answers"),
            StartupHandBack::Activated
        );
        assert_eq!(logind.activated(), vec!["7".to_owned()]);
    }

    #[test]
    fn a_seat_with_no_session_at_all_hands_nothing_back() {
        let logind = ScriptedLogind::reachable();

        assert_eq!(
            hand_back_at_startup(&logind, Some(967)).expect("logind answers"),
            StartupHandBack::NoGreeterSession
        );
        assert!(logind.activated().is_empty());
    }

    #[test]
    fn a_greeter_uid_that_cannot_be_recognised_hands_nothing_back() {
        let logind = ScriptedLogind {
            seat_sessions: vec![SeatedSession {
                id: "7".to_owned(),
                uid: 967,
            }],
            ..ScriptedLogind::reachable()
        };

        assert_eq!(
            hand_back_at_startup(&logind, None).expect("logind answers"),
            StartupHandBack::UnknownGreeter
        );
    }

    #[test]
    fn two_greeter_sessions_are_not_guessed_between() {
        let logind = ScriptedLogind {
            seat_sessions: vec![
                SeatedSession {
                    id: "7".to_owned(),
                    uid: 967,
                },
                SeatedSession {
                    id: "8".to_owned(),
                    uid: 967,
                },
            ],
            ..ScriptedLogind::reachable()
        };

        assert_eq!(
            hand_back_at_startup(&logind, Some(967)).expect("logind answers"),
            StartupHandBack::Ambiguous
        );
        assert!(logind.activated().is_empty());
    }

    #[test]
    fn the_shipped_names_are_the_contract_with_logind() {
        assert_eq!(LOGIND_DESTINATION, "org.freedesktop.login1");
        assert_eq!(MANAGER_PATH, "/org/freedesktop/login1");
        assert_eq!(MANAGER_INTERFACE, "org.freedesktop.login1.Manager");
        assert_eq!(SEAT_INTERFACE, "org.freedesktop.login1.Seat");
        assert_eq!(SESSION_INTERFACE, "org.freedesktop.login1.Session");
        assert_eq!(DEFAULT_SEAT, "seat0");
    }

    #[test]
    fn a_system_client_is_created_without_connecting() {
        // Constructing one must not connect: a development instance starts the
        // boundary long before it serves a login, and failure has to happen at the
        // point of use, where it can be logged and degraded from.
        let logind = SystemLogind::new(DEFAULT_SEAT);

        assert!(logind.cache().is_none());
        assert_eq!(logind.seat, "seat0");
    }
}
