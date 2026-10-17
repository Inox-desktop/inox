//! Saving a configuration file so that a failure leaves the old one in place.
//!
//! ADR 0014 fixes the failure atomicity of every write to the Inox
//! configuration: a temporary file **inside the target directory**, with
//! restrictive permissions, written in full, flushed, renamed over the target,
//! and the directory synced afterwards. The file in use is never truncated, so
//! the window in which a reader can see half a document does not exist, and a
//! crash leaves either the old bytes or the new ones.
//!
//! Two further rules from the same decision live here rather than in the
//! application:
//!
//! - **An external edit is not overwritten.** [`save`] takes the [`Guard`]
//!   captured when the file was read and refuses to write when the file no
//!   longer has it, reporting [`SaveOutcome::Conflicted`] with the digest it
//!   found. This is ADR 0014's "检测外部编辑冲突并提示重读，不无条件覆盖手工修改".
//! - **`let _ =` on cleanup has a reason.** When a step fails, the temporary
//!   file is removed on a best-effort basis: the failure being reported is the
//!   one that matters, and a cleanup that fails as well must not replace it.
//!
//! [`Digest`] is the content check the guard is built from. It is FNV-1a, which
//! is a *change detector*, not a cryptographic hash, and deliberately so: it
//! answers "is this the same file this window read?", a question an accidental
//! edit or another tool answers, and it makes no claim against someone who can
//! already write the configuration directory — ADR 0014 states that same
//! boundary ("不宣称能隔离已经控制同一用户账号或配置目录的攻击者").

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{debug, warn};

/// FNV-1a offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Permission bits of a configuration file this component creates.
///
/// The configuration holds the user's own preferences and nothing else, so it
/// is created for its owner alone. A file that already exists keeps the mode it
/// has, so replacing it never changes who can read it.
const DEFAULT_MODE: u32 = 0o600;

/// The content of a file, as the value a [`Guard`] compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Digest {
    /// The file was not there.
    Absent,
    /// The FNV-1a hash of the file's bytes.
    Content(u64),
}

impl Digest {
    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self::Content(hash(bytes))
    }

    /// The digest of the file at `path`, or [`Digest::Absent`] when it is not
    /// there.
    ///
    /// # Errors
    ///
    /// Returns a readable reason when the file exists but cannot be read.
    pub fn read(path: &Path) -> Result<Self, String> {
        match fs::read(path) {
            Ok(bytes) => Ok(Self::of(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::Absent),
            Err(error) => Err(format!("cannot read `{}`: {error}", path.display())),
        }
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => formatter.write_str("absent"),
            Self::Content(hash) => write!(formatter, "{hash:016x}"),
        }
    }
}

/// What a file looked like when it was read.
///
/// The application carries one of these from the read to the write. It is what
/// turns "the user edited this file by hand while the settings window was open"
/// from a silent overwrite into a diagnosable conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Guard {
    digest: Digest,
}

impl Guard {
    /// Guards a file that had `digest` when it was read.
    pub const fn new(digest: Digest) -> Self {
        Self { digest }
    }

    /// The digest the file had.
    pub const fn digest(self) -> Digest {
        self.digest
    }
}

/// What became of a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The file now holds the new bytes.
    Saved,
    /// The file changed after it was read, so nothing was written. The digest
    /// is the one found on disk, for the message the user reads.
    Conflicted {
        /// What the file holds now.
        current_digest: Digest,
    },
    /// The write failed. The reason is the operating system's own words, shown
    /// as data beside a localized sentence rather than as the sentence itself.
    Failed {
        /// Why it failed.
        reason: String,
    },
}

impl SaveOutcome {
    /// Whether the file holds the new bytes.
    pub const fn is_saved(&self) -> bool {
        matches!(self, Self::Saved)
    }
}

/// Writes `content` to `path` atomically, refusing to overwrite an external
/// edit.
///
/// When `guard` is given and the file no longer has that digest, nothing is
/// written and the current digest is reported. Everything else — a missing
/// directory, a read-only directory, a target that is not a file — is reported
/// as [`SaveOutcome::Failed`] with the operating system's reason.
pub fn save(path: &Path, content: &[u8], guard: Option<&Guard>) -> SaveOutcome {
    if let Some(guard) = guard {
        match Digest::read(path) {
            Ok(current) if current != guard.digest() => {
                debug!(
                    path = %path.display(),
                    expected = %guard.digest(),
                    found = %current,
                    "refusing to overwrite a file that changed since it was read"
                );

                return SaveOutcome::Conflicted {
                    current_digest: current,
                };
            }
            Ok(_) => {}
            Err(reason) => return SaveOutcome::Failed { reason },
        }
    }

    match replace(path, content) {
        Ok(()) => {
            debug!(
                path = %path.display(),
                bytes = content.len(),
                "wrote the configuration atomically"
            );

            SaveOutcome::Saved
        }
        Err(reason) => {
            warn!(path = %path.display(), %reason, "could not write the configuration");

            SaveOutcome::Failed { reason }
        }
    }
}

/// The write itself: temporary file, flush, rename, directory sync.
fn replace(path: &Path, content: &[u8]) -> Result<(), String> {
    let directory = parent_of(path);
    let temporary = temporary_path(directory, path);

    if let Err(reason) = write_temporary(&temporary, content, mode_of(path)) {
        // Best effort: the reason being returned is the one that matters, and
        // a cleanup that fails too must not replace it.
        let _ = fs::remove_file(&temporary);
        return Err(reason);
    }

    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("cannot replace `{}`: {error}", path.display()));
    }

    // The rename is atomic, but on its own it only promises that the *name*
    // points at the new inode: a crash right after it can still lose the
    // directory entry. Syncing the directory is what makes the new name
    // durable (ADR 0014: 不能把 rename 一步等同于所有文件系统的持久化保证).
    match fs::File::open(directory) {
        Ok(handle) => handle
            .sync_all()
            .map_err(|error| format!("cannot sync `{}`: {error}", directory.display())),
        Err(error) => Err(format!(
            "cannot open `{}` to sync it: {error}",
            directory.display()
        )),
    }
}

/// Writes the whole content to `temporary` and flushes it to the device.
fn write_temporary(temporary: &Path, content: &[u8], mode: u32) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(temporary)
        .map_err(|error| format!("cannot create `{}`: {error}", temporary.display()))?;

    file.write_all(content)
        .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;

    // The data has to reach the device before the rename publishes the name,
    // or a crash can leave the new name pointing at a partly written file.
    file.sync_all()
        .map_err(|error| format!("cannot flush `{}`: {error}", temporary.display()))
}

/// The directory a file lives in.
///
/// A path without a parent — `config.toml` — lives in the current directory,
/// which is where the temporary file has to go as well: a rename across
/// directories is not atomic.
fn parent_of(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// A name nothing else in the directory is using.
///
/// The temporary file sits beside its target — the rename has to stay inside
/// one directory — and starts with a dot so a directory listing does not show
/// it.
fn temporary_path(directory: &Path, target: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config.toml".to_owned());
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);

    directory.join(format!(
        ".{name}.inox-settings-{}-{stamp}-{counter}.tmp",
        std::process::id()
    ))
}

/// The mode a file should have: the one it has, or the restrictive default.
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;

    match fs::metadata(path) {
        Ok(metadata) => metadata.mode() & 0o777,
        Err(_) => DEFAULT_MODE,
    }
}

/// FNV-1a over `bytes`.
fn hash(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;

    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether this process is actually subject to the permission bits it sets.
    ///
    /// A CI container that runs as root ignores them, and a test that expects a
    /// write into a read-only directory to fail would then fail itself for a
    /// reason that says nothing about the code. The probe is the same operation
    /// the code performs, so the answer is about this process rather than about
    /// its user's name.
    fn permissions_are_enforced(directory: &Path) -> bool {
        let probe = directory.join(".probe");
        fs::write(&probe, b"").expect("a probe file is created");

        let mut permissions = fs::metadata(&probe)
            .expect("the probe has metadata")
            .permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o400);
        fs::set_permissions(&probe, permissions).expect("the probe is made read-only");

        let enforced = OpenOptions::new().write(true).open(&probe).is_err();
        let _ = fs::remove_file(&probe);

        enforced
    }

    fn temporary_files(directory: &Path) -> Vec<String> {
        fs::read_dir(directory)
            .expect("the directory is readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".inox-settings-"))
            .collect()
    }

    #[test]
    fn a_digest_depends_on_the_content_and_the_absence_of_a_file_does_not() {
        assert_eq!(Digest::of(b"a = 1\n"), Digest::of(b"a = 1\n"));
        assert_ne!(Digest::of(b"a = 1\n"), Digest::of(b"a = 2\n"));
        assert_ne!(Digest::of(b"a = 1\n"), Digest::of(b"a = 1"));
        assert_ne!(Digest::of(b""), Digest::Absent);
    }

    #[test]
    fn reading_a_digest_reports_a_missing_file_as_absent() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");

        assert_eq!(
            Digest::read(&path).expect("a missing file is not an error"),
            Digest::Absent
        );

        fs::write(&path, b"theme = \"inox-dark\"\n").expect("the file is written");
        assert_eq!(
            Digest::read(&path).expect("the file is readable"),
            Digest::of(b"theme = \"inox-dark\"\n")
        );
    }

    #[test]
    fn a_successful_save_writes_the_whole_file() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("shell.toml");

        let outcome = save(&path, b"[panel]\nheight = 40\n", None);

        assert_eq!(outcome, SaveOutcome::Saved);
        assert_eq!(
            fs::read(&path).expect("the file is there"),
            b"[panel]\nheight = 40\n"
        );
        assert!(
            temporary_files(directory.path()).is_empty(),
            "no temporary file is left behind"
        );
        assert!(
            temporary_files(directory.path()).is_empty(),
            "the guard's own temporary file is gone too"
        );
        assert_eq!(
            Digest::read(&path).expect("readable"),
            Digest::of(b"[panel]\nheight = 40\n")
        );
    }

    #[test]
    fn a_created_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::MetadataExt;

        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");

        assert_eq!(save(&path, b"", None), SaveOutcome::Saved);
        assert_eq!(
            fs::metadata(&path).expect("the file has metadata").mode() & 0o777,
            DEFAULT_MODE
        );
    }

    #[test]
    fn replacing_a_file_keeps_the_mode_it_already_had() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");
        fs::write(&path, b"old\n").expect("the file is written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("the mode is set");

        let guard = Guard::new(Digest::read(&path).expect("readable"));
        assert_eq!(save(&path, b"new\n", Some(&guard)), SaveOutcome::Saved);

        assert_eq!(fs::metadata(&path).expect("metadata").mode() & 0o777, 0o644);
        assert_eq!(fs::read(&path).expect("readable"), b"new\n");
    }

    #[test]
    fn an_existing_file_is_replaced_and_leaves_no_temporary_behind() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("notifications.toml");
        fs::write(&path, b"[policy]\ndo_not_disturb = false\n").expect("the file is written");

        let guard = Guard::new(Digest::read(&path).expect("readable"));
        assert_eq!(
            save(&path, b"[policy]\ndo_not_disturb = true\n", Some(&guard)),
            SaveOutcome::Saved
        );

        assert_eq!(
            fs::read(&path).expect("readable"),
            b"[policy]\ndo_not_disturb = true\n"
        );
        assert!(temporary_files(directory.path()).is_empty());
    }

    #[test]
    fn an_external_edit_is_a_conflict_not_an_overwrite() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");
        fs::write(&path, b"# what the window read\n").expect("the file is written");
        let guard = Guard::new(Digest::read(&path).expect("readable"));

        // The user edits the file in an editor while the window is open.
        fs::write(&path, b"# edited by hand\n").expect("the external edit");

        let outcome = save(&path, b"# what the window would write\n", Some(&guard));

        assert_eq!(
            outcome,
            SaveOutcome::Conflicted {
                current_digest: Digest::of(b"# edited by hand\n"),
            }
        );
        assert_eq!(
            fs::read(&path).expect("readable"),
            b"# edited by hand\n",
            "the manual edit survives untouched"
        );
    }

    #[test]
    fn deleting_the_file_out_from_under_the_window_is_a_conflict() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");
        fs::write(&path, b"# read\n").expect("the file is written");
        let guard = Guard::new(Digest::read(&path).expect("readable"));

        fs::remove_file(&path).expect("the external removal");

        assert_eq!(
            save(&path, b"# written\n", Some(&guard)),
            SaveOutcome::Conflicted {
                current_digest: Digest::Absent,
            }
        );
        assert!(
            !path.exists(),
            "nothing is created when the guard does not match"
        );
    }

    #[test]
    fn creating_a_file_the_guard_expected_to_be_there_is_a_conflict() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");
        let guard = Guard::new(Digest::Absent);

        fs::write(&path, b"# someone else made this\n").expect("the external write");

        assert_eq!(
            save(&path, b"# ours\n", Some(&guard)),
            SaveOutcome::Conflicted {
                current_digest: Digest::of(b"# someone else made this\n"),
            }
        );
    }

    #[test]
    fn a_guard_that_still_matches_lets_the_save_through() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");
        fs::write(&path, b"# read\n").expect("the file is written");

        let guard = Guard::new(Digest::read(&path).expect("readable"));
        assert_eq!(guard.digest(), Digest::of(b"# read\n"));
        assert_eq!(
            save(&path, b"# written\n", Some(&guard)),
            SaveOutcome::Saved
        );
    }

    #[test]
    fn a_read_only_directory_fails_and_leaves_the_original_bytes_intact() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");
        fs::write(&path, b"# the original\n").expect("the file is written");

        if !permissions_are_enforced(directory.path()) {
            // Running where the permission bits do not apply (a root container).
            // The behaviour under test cannot be produced here, so the case is
            // reported rather than asserted against.
            eprintln!("skipping: this process ignores the permission bits");
            return;
        }

        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o500))
            .expect("the directory is made read-only");

        let outcome = save(&path, b"# the new one\n", None);

        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
            .expect("the directory is writable again");

        match outcome {
            SaveOutcome::Failed { reason } => assert!(!reason.is_empty()),
            other => panic!("expected a failure, got {other:?}"),
        }
        assert_eq!(
            fs::read(&path).expect("readable"),
            b"# the original\n",
            "a failed save leaves the original bytes untouched"
        );
        assert!(temporary_files(directory.path()).is_empty());
    }

    #[test]
    fn a_target_that_cannot_be_replaced_fails_without_creating_anything() {
        // A directory in the target's place fails whatever the process is
        // allowed to do, so this covers the failure path unconditionally.
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");
        fs::create_dir(&path).expect("a directory in the file's place");

        let outcome = save(&path, b"# content\n", None);

        match outcome {
            SaveOutcome::Failed { reason } => {
                assert!(reason.contains("config.toml"), "reason `{reason}`");
            }
            other => panic!("expected a failure, got {other:?}"),
        }
        assert!(path.is_dir(), "the target is untouched");
        assert!(temporary_files(directory.path()).is_empty());
    }

    #[test]
    fn a_missing_directory_is_a_readable_failure() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("nowhere").join("config.toml");

        match save(&path, b"# content\n", None) {
            SaveOutcome::Failed { reason } => {
                assert!(reason.contains("nowhere"), "reason `{reason}`")
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn saving_without_a_guard_is_the_first_write() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("config.toml");

        assert_eq!(save(&path, b"# first\n", None), SaveOutcome::Saved);
        assert_eq!(fs::read(&path).expect("readable"), b"# first\n");
    }
}
