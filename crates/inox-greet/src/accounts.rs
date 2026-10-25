//! The accounts the login screen offers, and the one it offers first.
//!
//! Since the screen does not let anyone type a user name, the identity it
//! authenticates has to come from somewhere, and that somewhere is the system's
//! own account database. This module is the whole of that: the list, the order it
//! is offered in, the one to preselect, and where the account a session was
//! started for is recorded so that the next login offers it first.
//!
//! # What is offered, and what is not
//!
//! An account is offered when it is a *login* account: a system account
//! (`uid` below [`FIRST_ORDINARY_UID`], and the `nobody` placeholder at
//! [`SYSTEM_UID`]) is not, an account whose shell refuses a session (`nologin`,
//! `false`) is not, and `root` is offered anyway. That last one is deliberate
//! rather than an oversight: `root` is a system account by uid and an account
//! somebody may genuinely need to log into, so the screen offers it — last, so
//! that it is never what a person gets by accident.
//!
//! The database is read through `getent passwd`, which is the same view of the
//! accounts the rest of the system has (name service switch and all, so an
//! account that lives in LDAP is offered too), and `/etc/passwd` directly when
//! `getent` cannot be run at all. Both are read, never written; nothing here
//! authenticates anybody, and nothing here decides anything about a password.
//!
//! # Where "used last" comes from
//!
//! The record is this component's own: [`LAST_USER_FILE`] under the Inox state
//! directory (`$XDG_STATE_HOME/inox/greet/last-user`, see [`record_file`]), written
//! when a session actually started. It is deliberately not an attempt to infer a
//! history from a login database such as `wtmp`: that would mean parsing a system
//! binary format to answer a question about *this greeter's* last login, and being
//! wrong about it costs a preselection, not a login.

use std::path::{Path, PathBuf};
use std::process::Command;

use inox_common::paths::XdgEnv;
use tracing::{debug, warn};

/// The directory below the Inox state directory that holds this component's state.
pub const GREET_DIR: &str = "greet";

/// The file the account used last is recorded in.
pub const LAST_USER_FILE: &str = "last-user";

/// `root`'s uid, which is offered although it is a system uid.
pub const ROOT_UID: u32 = 0;

/// The lowest uid an ordinary account may have.
pub const FIRST_ORDINARY_UID: u32 = 1000;

/// The uid of the `nobody` placeholder, and the exclusive end of the ordinary
/// range: an account at or above it is a system account.
pub const SYSTEM_UID: u32 = 65534;

/// The command that answers with the accounts the system actually resolves.
const GETENT: &str = "getent";

/// The account database to fall back to when [`GETENT`] cannot be run.
const PASSWD: &str = "/etc/passwd";

/// The shells that mean "this account cannot hold a session".
///
/// Matched as a suffix of the login shell, because the same shell lives in
/// different directories on different distributions (`/usr/sbin/nologin`,
/// `/sbin/nologin`, `/bin/false`).
const UNLOGINNABLE_SHELLS: [&str; 2] = ["nologin", "false"];

/// One account the login screen can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    name: String,
    uid: u32,
    full_name: Option<String>,
}

impl Account {
    /// An account with `name` and `uid`, drawn with `full_name` when it has one.
    pub fn new(name: impl Into<String>, uid: u32, full_name: Option<String>) -> Self {
        Self {
            name: name.into(),
            uid,
            full_name,
        }
    }

    /// The login name: what the authentication boundary is asked to check.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The account's numeric identifier, as the account database lists it.
    ///
    /// The ordering and the filtering are what the binary needs; the tests read the
    /// identifier itself to assert where an account came from.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn uid(&self) -> u32 {
        self.uid
    }

    /// The name to draw: the account's full name when the database carries one,
    /// and its login name otherwise.
    pub fn display(&self) -> &str {
        self.full_name.as_deref().unwrap_or(&self.name)
    }

    /// The one character drawn on the account's medallion.
    ///
    /// The first character of what is drawn, upper-cased — the same
    /// character-for-character answer in every locale, because a letter is not a
    /// sentence and there is nothing here for a translator to translate. An
    /// account with an empty name (which the database does not contain) draws
    /// nothing rather than a panic.
    pub fn initial(&self) -> String {
        let Some(first) = self.display().chars().next() else {
            return String::new();
        };

        first
            .to_uppercase()
            .next()
            .map_or_else(|| first.to_string(), String::from)
    }

    /// Whether this is `root`, which is offered last.
    pub const fn is_root(&self) -> bool {
        self.uid == ROOT_UID
    }
}

/// Why the account database could not be read.
#[derive(Debug)]
pub enum AccountsError {
    /// `getent` is not installed, or could not be started.
    Getent(std::io::Error),

    /// `getent` ran and refused the request.
    GetentFailed {
        /// The status it exited with.
        status: i32,
    },

    /// The fallback database could not be read.
    Read {
        /// The path that could not be read.
        path: PathBuf,
        /// Why it could not be read.
        source: std::io::Error,
    },
}

impl std::fmt::Display for AccountsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Getent(error) => write!(f, "`getent passwd` could not be run: {error}"),
            Self::GetentFailed { status } => {
                write!(f, "`getent passwd` exited with status {status}")
            }
            Self::Read { path, source } => {
                write!(f, "cannot read {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for AccountsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Getent(error) => Some(error),
            Self::Read { source, .. } => Some(source),
            Self::GetentFailed { .. } => None,
        }
    }
}

/// The account list as the login screen needs it: what to offer, which one to
/// offer first, where to record the one a session was started for, and whether
/// the database could be read at all.
#[derive(Debug, Clone, Default)]
pub struct Accounts {
    /// The accounts to offer, in the order they are offered.
    pub accounts: Vec<Account>,
    /// The account recorded as used last, when the record named one.
    pub last_used: Option<String>,
    /// Where the account a session was started for is recorded; `None` records
    /// nothing, which is what a test — and a session with no state directory —
    /// takes.
    pub record: Option<PathBuf>,
    /// Whether the account database could not be read at all.
    ///
    /// An empty list and an unreadable database are two different things to the
    /// person at the keyboard, so the screen says which one it is.
    pub unreadable: bool,
}

impl Accounts {
    /// Reads the accounts the system resolves, remembering them at `record`.
    ///
    /// The record is read and the database is written to disk by the caller; this
    /// only reads. A database that cannot be read at all is not an error here: it
    /// becomes [`Accounts::unreadable`], because a login screen that refused to
    /// start would take the display away from a person who could otherwise log in
    /// with the account they know.
    pub fn from_system(record: Option<PathBuf>) -> Self {
        match enumerate() {
            Ok(accounts) => {
                let last_used = record.as_deref().and_then(last_used);
                debug!(accounts = accounts.len(), "the account database was read");

                Self {
                    accounts,
                    last_used,
                    record,
                    unreadable: false,
                }
            }
            Err(error) => {
                warn!(%error, "the account database could not be read");

                Self {
                    accounts: Vec::new(),
                    last_used: None,
                    record,
                    unreadable: true,
                }
            }
        }
    }

    /// The index of the account to offer first.
    ///
    /// The account used last when the system still has it, the first ordinary
    /// account otherwise, and `root` only when it is all there is.
    pub fn selected(&self) -> Option<usize> {
        default_selection(&self.accounts, self.last_used.as_deref())
    }

    /// Records `account` as the one a session was started for.
    ///
    /// A record that cannot be written is logged and otherwise ignored: the next
    /// login then offers the first account instead of this one, which is a worse
    /// preselection, not a failed login.
    pub fn record_started(&self, account: &str) {
        let Some(path) = self.record.as_deref() else {
            debug!("no state directory is configured; the account used last is not recorded");
            return;
        };

        if let Err(error) = remember(path, account) {
            warn!(%error, path = %path.display(), "the account used last could not be recorded");
        }
    }
}

/// Reads the system's accounts.
///
/// `getent passwd` first — it is the account database the rest of the system
/// resolves, name service switch included — and `/etc/passwd` when `getent`
/// cannot be run or answers with nothing.
///
/// # Errors
///
/// Returns [`AccountsError`] when neither database can be read.
pub fn enumerate() -> Result<Vec<Account>, AccountsError> {
    match getent() {
        Ok(database) if !database.trim().is_empty() => Ok(parse(&database)),
        Ok(_) => {
            debug!("`getent passwd` answered with nothing; reading {PASSWD}");
            read(PASSWD)
        }
        Err(error) => {
            debug!(%error, "`getent passwd` could not be run; reading {PASSWD}");
            read(PASSWD)
        }
    }
}

/// Runs `getent passwd` and returns what it wrote to stdout.
fn getent() -> Result<String, AccountsError> {
    let output = Command::new(GETENT)
        .arg("passwd")
        .output()
        .map_err(AccountsError::Getent)?;

    if !output.status.success() {
        return Err(AccountsError::GetentFailed {
            status: output.status.code().unwrap_or(-1),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Reads a `passwd`-format database from disk.
fn read(path: &str) -> Result<Vec<Account>, AccountsError> {
    let text = std::fs::read_to_string(path).map_err(|source| AccountsError::Read {
        path: PathBuf::from(path),
        source,
    })?;

    Ok(parse(&text))
}

/// Reads the accounts out of a `passwd(5)` database.
///
/// Pure, and the whole of what "which accounts are offered" means: the same
/// function reads the answer from `getent` and from `/etc/passwd`, so the two
/// paths cannot disagree about who may log in. Lines that are not seven
/// colon-separated fields with a numeric uid are skipped rather than guessed at,
/// a name that appears twice is offered once, the ordinary accounts come in uid
/// order, and `root` is appended last.
pub fn parse(database: &str) -> Vec<Account> {
    let mut ordinary = Vec::new();
    let mut root = None;

    for line in database.lines() {
        let Some(account) = account_from(line) else {
            continue;
        };

        if account.is_root() {
            root.get_or_insert(account);
            continue;
        }

        if !ordinary
            .iter()
            .any(|known: &Account| known.name == account.name)
        {
            ordinary.push(account);
        }
    }

    ordinary.sort_by(|one, other| {
        one.uid
            .cmp(&other.uid)
            .then_with(|| one.name.cmp(&other.name))
    });

    ordinary.extend(root);

    ordinary
}

/// One account from one `passwd` line, or `None` when the line is not an account
/// this screen offers.
fn account_from(line: &str) -> Option<Account> {
    let mut fields = line.split(':');

    let name = fields.next()?;
    fields.next()?; // The password placeholder; nothing here reads a password.
    let uid = fields.next()?.parse::<u32>().ok()?;
    fields.next()?; // The primary group.
    let gecos = fields.next()?;
    fields.next()?; // The home directory.
    let shell = fields.next()?;

    if name.is_empty() || !offered(uid, shell) {
        return None;
    }

    Some(Account::new(name, uid, full_name(gecos, name)))
}

/// Whether an account with this uid and shell is one this screen offers.
fn offered(uid: u32, shell: &str) -> bool {
    if uid == ROOT_UID {
        return true;
    }

    if !(FIRST_ORDINARY_UID..SYSTEM_UID).contains(&uid) {
        return false;
    }

    !UNLOGINNABLE_SHELLS
        .iter()
        .any(|refusing| shell.ends_with(refusing))
}

/// The full name out of a `passwd` GECOS field, when it says anything the login
/// name does not already say.
fn full_name(gecos: &str, name: &str) -> Option<String> {
    let full = gecos.split(',').next().unwrap_or_default().trim();

    if full.is_empty() || full == name {
        return None;
    }

    Some(full.to_owned())
}

/// Which account of `accounts` to offer first.
///
/// The recorded account when the system still has it, otherwise the first
/// ordinary account, otherwise `root` — and nothing at all when the database is
/// empty, because there is then nothing to offer.
pub fn default_selection(accounts: &[Account], last_used: Option<&str>) -> Option<usize> {
    if let Some(last) = last_used
        && let Some(index) = accounts.iter().position(|account| account.name == last)
    {
        return Some(index);
    }

    accounts
        .iter()
        .position(|account| !account.is_root())
        .or(if accounts.is_empty() { None } else { Some(0) })
}

/// The file this component records the account used last in.
///
/// `$XDG_STATE_HOME/inox/greet/last-user`, or `None` when no state directory can
/// be determined — in which case nothing is recorded and the first account is
/// offered first, rather than the component refusing to run.
pub fn record_file(env: &XdgEnv) -> Option<PathBuf> {
    env.inox_state_dir()
        .ok()
        .map(|dir| dir.join(GREET_DIR).join(LAST_USER_FILE))
}

/// The account recorded in `path`, when there is one.
///
/// Absent, unreadable, empty and whitespace-only are all "no record": a
/// preselection is not a thing worth refusing a login over.
pub fn last_used(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = text.trim();

    if name.is_empty() {
        return None;
    }

    Some(name.to_owned())
}

/// Records `account` as the one used last, creating the directory it lives in.
///
/// # Errors
///
/// Returns whatever the filesystem returned. The caller decides what to do about
/// it; this component only logs it.
pub fn remember(path: &Path, account: &str) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, format!("{account}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database with one account of every kind the filter has an opinion about.
    fn database() -> String {
        [
            "root:x:0:0:root:/root:/bin/bash",
            "daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin",
            "system:x:999:999::/var/lib/system:/bin/bash",
            "alice:x:1001:1001:Alice Doe:/home/alice:/bin/bash",
            "bob:x:1000:1000::/home/bob:/bin/bash",
            "service:x:1002:1002::/var/lib/service:/usr/sbin/nologin",
            "carl:x:1003:1003::/home/carl:/bin/false",
            "dora:x:1004:1004::/home/dora:/usr/bin/zsh",
            "nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin",
        ]
        .join("\n")
    }

    #[test]
    fn ordinary_accounts_come_from_the_documented_uid_range() {
        let names: Vec<String> = parse(&database())
            .into_iter()
            .map(|account| account.name)
            .collect();

        assert!(
            !names.contains(&"system".to_owned()),
            "uid 999 is a system account"
        );
        assert!(
            !names.contains(&"nobody".to_owned()),
            "uid 65534 is the placeholder"
        );
        assert!(names.contains(&"bob".to_owned()), "uid 1000 is an account");
        assert!(names.contains(&"alice".to_owned()));
    }

    #[test]
    fn shells_that_refuse_a_session_are_not_offered() {
        let names: Vec<String> = parse(&database())
            .into_iter()
            .map(|account| account.name)
            .collect();

        assert!(!names.contains(&"service".to_owned()), "nologin");
        assert!(!names.contains(&"carl".to_owned()), "false");
        assert!(
            !names.contains(&"daemon".to_owned()),
            "a system account with nologin"
        );
        assert!(names.contains(&"dora".to_owned()), "zsh is a login shell");
        assert!(
            names.contains(&"bob".to_owned()),
            "/bin/bash is a login shell"
        );
    }

    #[test]
    fn root_is_offered_and_offered_last() {
        let accounts = parse(&database());

        assert_eq!(
            accounts.last().map(Account::name),
            Some("root"),
            "root is a system uid and is still offered, at the end"
        );
        assert!(accounts.iter().any(Account::is_root));
        assert_eq!(
            accounts.iter().filter(|account| account.is_root()).count(),
            1
        );
    }

    #[test]
    fn the_ordinary_accounts_are_offered_in_uid_order() {
        let uids: Vec<u32> = parse(&database()).iter().map(Account::uid).collect();

        assert_eq!(uids, vec![1000, 1001, 1004, 0]);
    }

    #[test]
    fn a_name_that_appears_twice_is_offered_once() {
        // The same account resolved twice — two database lines for one name — is
        // one entry, and the first one wins.
        let accounts = parse(
            "alice:x:1000:1000::/home/alice:/bin/bash\nalice:x:1000:1000::/home/alice:/bin/bash\n",
        );

        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name(), "alice");
    }

    #[test]
    fn lines_that_are_not_accounts_are_skipped() {
        for line in [
            "",
            "alice",
            "alice:x:notanumber:1000::/home/alice:/bin/bash",
            "alice:x:1000:1000::/home/alice",
            ":x:1000:1000::/home/alice:/bin/bash",
            "# a comment",
        ] {
            assert!(parse(line).is_empty(), "`{line}` should not be an account");
        }
    }

    #[test]
    fn the_full_name_is_drawn_when_the_database_carries_one() {
        let database = "alice:x:1000:1000:Alice Doe,Room 1:/home/alice:/bin/bash\nbob:x:1001:1001::/home/bob:/bin/bash\ncarl:x:1002:1002:carl:/home/carl:/bin/bash\n";

        let accounts = parse(database);

        assert_eq!(accounts[0].display(), "Alice Doe");
        assert_eq!(
            accounts[1].display(),
            "bob",
            "an empty GECOS falls back to the name"
        );
        assert_eq!(
            accounts[2].display(),
            "carl",
            "a GECOS that repeats the login name is not a second name"
        );
    }

    #[test]
    fn the_medallion_shows_the_first_letter_of_what_is_drawn() {
        let database = "alice:x:1000:1000:alice doe:/home/alice:/bin/bash\nünter:x:1001:1001::/home/u:/bin/bash\n";

        let accounts = parse(database);

        assert_eq!(accounts[0].initial(), "A");
        assert_eq!(accounts[1].initial(), "Ü", "a letter is not an ASCII byte");
        assert_eq!(Account::new("", 1000, None).initial(), "");
    }

    #[test]
    fn the_account_used_last_is_offered_first() {
        let accounts = parse(&database());

        assert_eq!(
            default_selection(&accounts, Some("dora")),
            accounts.iter().position(|account| account.name == "dora")
        );
    }

    #[test]
    fn an_unknown_record_falls_back_to_the_first_ordinary_account() {
        let accounts = parse(&database());

        for last in [None, Some("nobody"), Some("")] {
            let selected = default_selection(&accounts, last).expect("an account");
            let account = &accounts[selected];

            assert_eq!(
                account.name, "bob",
                "the lowest ordinary uid is offered, not root ({last:?})"
            );
        }
    }

    #[test]
    fn root_is_the_fallback_only_when_it_is_all_there_is() {
        let root = parse("root:x:0:0:root:/root:/bin/bash\n");

        assert_eq!(default_selection(&root, None), Some(0));
        assert_eq!(default_selection(&[], Some("alice")), None);
        assert_eq!(
            default_selection(&[], None),
            None,
            "an empty database has nothing to offer"
        );
    }

    #[test]
    fn the_record_is_read_back_after_it_is_written() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("greet").join("last-user");

        assert_eq!(last_used(&path), None, "no record at all");

        remember(&path, "alice").expect("the record is written");
        assert_eq!(
            last_used(&path),
            Some("alice".to_owned()),
            "the directory was created on the way"
        );

        std::fs::write(&path, "   \n").expect("the record is rewritten");
        assert_eq!(last_used(&path), None, "a blank record names nobody");
    }

    #[test]
    fn the_record_lives_below_the_inox_state_directory() {
        let env = XdgEnv::from_pairs([("HOME", "/home/greeter"), ("XDG_STATE_HOME", "/state")]);

        assert_eq!(
            record_file(&env),
            Some(PathBuf::from("/state/inox/greet/last-user"))
        );
        assert_eq!(
            record_file(&XdgEnv::from_pairs([("HOME", "/home/greeter")])),
            Some(PathBuf::from(
                "/home/greeter/.local/state/inox/greet/last-user"
            ))
        );
        assert_eq!(
            record_file(&XdgEnv::default()),
            None,
            "without a home or a state directory there is nowhere to record"
        );
    }

    #[test]
    fn recording_a_started_session_writes_the_account_and_nothing_else() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("greet").join("last-user");
        let accounts = Accounts {
            accounts: parse(&database()),
            last_used: None,
            record: Some(path.clone()),
            unreadable: false,
        };

        assert_eq!(
            accounts.selected(),
            Some(0),
            "bob is the first ordinary account"
        );

        accounts.record_started("dora");

        assert_eq!(
            std::fs::read_to_string(&path).expect("the record"),
            "dora\n"
        );

        let recorded = Accounts {
            last_used: last_used(&path),
            ..accounts
        };
        assert_eq!(
            recorded
                .selected()
                .map(|index| recorded.accounts[index].name.clone()),
            Some("dora".to_owned())
        );
    }

    #[test]
    fn a_roster_with_nowhere_to_record_says_so_and_does_nothing() {
        let accounts = Accounts {
            accounts: parse(&database()),
            last_used: None,
            record: None,
            unreadable: false,
        };

        accounts.record_started("dora");
        assert_eq!(accounts.selected(), Some(0));
    }

    #[test]
    fn an_unreadable_database_offers_nothing_rather_than_refusing_to_run() {
        let roster = Accounts {
            accounts: Vec::new(),
            last_used: None,
            record: None,
            unreadable: true,
        };

        assert!(roster.accounts.is_empty());
        assert_eq!(roster.selected(), None);
        assert!(roster.unreadable);
    }

    #[test]
    fn whatever_this_system_offers_obeys_the_documented_filter() {
        // The one test that reads the database this machine really has, through
        // `getent` and its `/etc/passwd` fallback. It asserts properties rather
        // than a list, so it says the same thing on a laptop and in a container,
        // and a system that cannot be read at all is a case `parse` covers.
        let Ok(accounts) = enumerate() else {
            return;
        };

        for account in &accounts {
            assert!(!account.name().is_empty(), "an account with no name");
            assert!(
                account.is_root() || (FIRST_ORDINARY_UID..SYSTEM_UID).contains(&account.uid()),
                "`{}` is offered with uid {}",
                account.name(),
                account.uid()
            );
        }

        assert!(
            accounts.iter().filter(|account| account.is_root()).count() <= 1,
            "root is offered at most once"
        );

        if let Some(root) = accounts.iter().position(Account::is_root) {
            assert_eq!(
                root,
                accounts.len() - 1,
                "root is offered last: `{}`",
                accounts[root].name()
            );
        }
    }
}
