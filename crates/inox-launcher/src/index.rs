//! The application index: the XDG data directories, their precedence, and the
//! diagnostics of the files that could not be used.
//! The index is the launcher's authoritative state (the interaction contract: one authority per
//! The index is the launcher's authoritative state (one authority per
//! kind of state). It is built by walking, in order,
//!
//! 1. `$XDG_DATA_HOME/applications` (which is `~/.local/share/applications`),
//! 2. each directory of `$XDG_DATA_DIRS` in the order written, defaulting to
//!    `/usr/local/share:/usr/share`,
//!
//! and it is a *decision*, not a race, which of two files claiming the same
//! desktop file id wins: the first directory in that order wins, and the later
//! file is dropped. The same rule covers a file the entry gates reject — a
//! `Hidden=true` file in an earlier directory shadows the entry of the same id
//! in a later one, which is exactly what `Hidden` is for.
//!
//! Robustness is per file (acceptance D02): a file that cannot be read, does not
//! parse, or is unusable is recorded in [`AppIndex::skipped`] with its cause and
//! the scan continues. Nothing a single desktop file contains can empty the
//! launcher.
//!
//! Directory symlinks are not followed, so the walk always terminates;
//! symlinked *files* are read normally, which is how several distributions
//! install an application.

use std::collections::BTreeSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use inox_common::i18n::Locale;
use inox_common::paths::PathsError;
use tracing::{debug, info};

use crate::contract::{FailureKind, ServiceState};
use crate::desktop_entry::{
    self, DesktopAction, DesktopEntry, EntryFilter, EntryProblem, Localized,
};

/// One application, ready to be searched and launched.
///
/// The localized values are already resolved against the session's locale: the
/// index is rebuilt when the locale changes, and a search that had to resolve a
/// locale per keystroke would spend its budget on formatting instead of
/// matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedEntry {
    /// The desktop file id.
    pub id: String,
    /// The absolute path of the file, which `%k` expands to.
    pub path: PathBuf,
    /// The application's localized name.
    pub name: String,
    /// Its localized generic name.
    pub generic_name: Option<String>,
    /// Its localized comment.
    pub comment: Option<String>,
    /// Its icon name, as the desktop file wrote it.
    pub icon: Option<String>,
    /// The icon file the name resolved to, when the session has one.
    ///
    /// Resolved once, during the scan (see [`crate::icons`]): the lookup reads
    /// the icon theme directories, and doing it per frame would make the
    /// projection pay for the filesystem. `None` is a normal answer — the tile
    /// falls back to the entry's first letter.
    pub icon_path: Option<PathBuf>,
    /// Its command line, exactly as written.
    pub exec: String,
    /// Whether it needs a terminal.
    pub terminal: bool,
    /// The directory it needs to start in.
    pub working_dir: Option<String>,
    /// The window class it will use.
    pub startup_wm_class: Option<String>,
    /// Its categories.
    pub categories: Vec<String>,
    /// Its declared actions.
    pub actions: Vec<IndexedAction>,
    /// The `applications` directory it was read from.
    pub source: PathBuf,
}

impl IndexedEntry {
    /// The argv that starts this entry.
    ///
    /// The one place an entry becomes a command line: the view model asks it to
    /// find out whether a row can be launched at all, and the activation path
    /// asks it for the argv it hands to the process boundary. There is no
    /// second implementation to drift.
    ///
    /// # Errors
    ///
    /// Returns the [`FailureKind`] that makes the entry unlaunchable; see
    /// [`crate::exec::argv`].
    pub fn argv(&self, terminal_prefix: &[String]) -> Result<Vec<String>, FailureKind> {
        crate::exec::argv(&crate::exec::ExecContext {
            exec: &self.exec,
            name: &self.name,
            icon: self.icon.as_deref(),
            desktop_file: &self.path,
            files: &[],
            terminal: self.terminal,
            terminal_prefix,
        })
    }

    /// The executable named first on the command line, for diagnostics.
    ///
    /// This is the program name as the entry wrote it, before any terminal
    /// prefix is added.
    pub fn program(&self) -> &str {
        self.exec
            .split_whitespace()
            .next()
            .unwrap_or(self.exec.as_str())
    }
}

/// One action of an entry, with its name already resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedAction {
    /// The action's id.
    pub id: String,
    /// Its localized name.
    pub name: String,
    /// Its command line.
    pub exec: String,
    /// Its icon name.
    pub icon: Option<String>,
}

/// A file that could not be used, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    /// The desktop file id the file would have had.
    pub id: String,
    /// The file, or directory, that was skipped.
    pub path: PathBuf,
    /// Why it was skipped.
    pub problem: EntryProblem,
}

/// Every application the session has, and every file that had to be skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIndex {
    entries: Vec<IndexedEntry>,
    skipped: Vec<SkippedEntry>,
    directories: Vec<PathBuf>,
    generation: u64,
}

impl AppIndex {
    /// Associates a prepared index with the live application's newest scan.
    pub(crate) fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    /// The applications, in the order the scan found them.
    ///
    /// The order is deterministic (directories in precedence order, files
    /// sorted within each directory) but it is not a ranking: ordering for
    /// display is [`crate::search`]'s job.
    pub fn entries(&self) -> &[IndexedEntry] {
        &self.entries
    }

    /// The files that had to be skipped.
    pub fn skipped(&self) -> &[SkippedEntry] {
        &self.skipped
    }

    /// The data directories the scan looked in, in precedence order.
    pub fn directories(&self) -> &[PathBuf] {
        &self.directories
    }

    /// The generation this index was built under.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The entry with this id, if the index holds one.
    pub fn get(&self, id: &str) -> Option<&IndexedEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// How many applications the index holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index holds no applications.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The service state this index reports.
    pub fn state(&self) -> ServiceState {
        ServiceState::Ready {
            generation: self.generation,
            entries: self.entries.len(),
            skipped: self.skipped.len(),
            directories: self.directories.clone(),
        }
    }
}

impl AppIndex {
    /// An index over entries the caller already has.
    ///
    /// The alternative is [`scan`], which walks the XDG directories; this is the
    /// constructor for a caller that has a list of applications in hand — a test
    /// harness today, and anything that caches or synthesises an index later —
    /// and wants the search and the projection over it without touching the
    /// filesystem.
    pub fn from_entries(entries: Vec<IndexedEntry>, generation: u64) -> Self {
        Self {
            entries,
            skipped: Vec::new(),
            directories: Vec::new(),
            generation,
        }
    }
}

#[cfg(test)]
impl AppIndex {
    /// An entry with only the fields the tests above care about.
    pub(crate) fn test_entry(id: &str, name: &str, exec: &str) -> IndexedEntry {
        IndexedEntry {
            id: id.to_owned(),
            path: std::path::PathBuf::from(format!("/usr/share/applications/{id}")),
            name: name.to_owned(),
            generic_name: None,
            comment: None,
            icon: None,
            icon_path: None,
            exec: exec.to_owned(),
            terminal: false,
            working_dir: None,
            startup_wm_class: None,
            categories: Vec::new(),
            actions: Vec::new(),
            source: std::path::PathBuf::from("/usr/share/applications"),
        }
    }
}

/// Where the index is read from, and which entries it keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOptions {
    /// The XDG *data* directories to scan, in precedence order; the entries are
    /// read from the `applications` subdirectory of each.
    pub directories: Vec<PathBuf>,
    /// The rules that decide whether an entry is listed.
    pub filter: EntryFilter,
    /// Where an entry's `Icon=` is looked up.
    ///
    /// Part of the options rather than of the entry so that a caller with no
    /// icon directories (the tests, a future index cache) still builds an index;
    /// [`IndexOptions::new`] leaves it empty.
    pub icons: crate::icons::IconSearch,
}

impl IndexOptions {
    /// Options over explicit directories and an explicit filter, without icons.
    pub fn new(directories: Vec<PathBuf>, filter: EntryFilter) -> Self {
        Self {
            directories: deduplicate(directories),
            filter,
            icons: crate::icons::IconSearch::none(),
        }
    }

    /// The same options, with an explicit icon search.
    pub fn with_icons(mut self, icons: crate::icons::IconSearch) -> Self {
        self.icons = icons;
        self
    }

    /// The options the session's environment describes.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError`] when `$XDG_DATA_HOME` cannot be determined, which
    /// is the one failure that leaves no index to build rather than an index of
    /// zero entries. Every other problem is per directory and is recorded as a
    /// diagnostic by [`scan`].
    pub fn from_env(locale: &Locale, include_hidden: bool) -> Result<Self, PathsError> {
        let mut directories = vec![inox_common::paths::data_home()?];
        directories.extend(data_directories());

        let mut filter = EntryFilter::from_env(locale);
        filter.include_hidden = include_hidden;

        Ok(Self::new(directories, filter).with_icons(crate::icons::IconSearch::from_env()))
    }
}

/// `$XDG_DATA_DIRS`, or the two directories the specification defaults to.
///
/// Shared with [`crate::icons`]: icons live under the same data directories as
/// the desktop files, and the two answers must not drift apart.
pub(crate) fn data_directories() -> Vec<PathBuf> {
    match std::env::var("XDG_DATA_DIRS") {
        Ok(value) if !value.trim().is_empty() => std::env::split_paths(&value).collect(),
        _ => vec![
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ],
    }
}

/// Drops repeated directories, keeping the first occurrence's position.
fn deduplicate(directories: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    directories
        .into_iter()
        .filter(|directory| seen.insert(directory.clone()))
        .collect()
}

/// Builds the index for `generation`.
///
/// The scan never fails as a whole: everything that went wrong is in
/// [`AppIndex::skipped`].
pub fn scan(options: &IndexOptions, generation: u64) -> AppIndex {
    let mut scanner = Scanner {
        filter: &options.filter,
        icons: &options.icons,
        seen: BTreeSet::new(),
        index: AppIndex {
            entries: Vec::new(),
            skipped: Vec::new(),
            directories: options.directories.clone(),
            generation,
        },
    };

    for directory in &options.directories {
        let root = directory.join("applications");
        scanner.walk(&root, &root);
    }

    info!(
        generation,
        entries = scanner.index.entries.len(),
        skipped = scanner.index.skipped.len(),
        directories = scanner.index.directories.len(),
        "application index built"
    );

    scanner.index
}

/// One scan's mutable state.
struct Scanner<'a> {
    filter: &'a EntryFilter,
    icons: &'a crate::icons::IconSearch,
    seen: BTreeSet<String>,
    index: AppIndex,
}

impl Scanner<'_> {
    /// Walks one `applications` directory.
    fn walk(&mut self, root: &Path, directory: &Path) {
        let listing = match std::fs::read_dir(directory) {
            Ok(listing) => listing,
            // A directory that does not exist is the normal state of
            // `~/.local/share/applications` on a fresh account.
            Err(error) if error.kind() == ErrorKind::NotFound => return,
            Err(error) => {
                self.record(
                    &relative_id(root, directory),
                    directory,
                    EntryProblem::Unreadable {
                        detail: error.to_string(),
                    },
                );
                return;
            }
        };

        let mut paths: Vec<PathBuf> = listing
            .filter_map(|item| match item {
                Ok(item) => Some(item.path()),
                Err(error) => {
                    debug!(%error, directory = %directory.display(), "unreadable directory entry");
                    None
                }
            })
            .collect();
        paths.sort();

        for path in paths {
            // `file_type` does not follow symlinks, so a directory symlink is
            // never entered and the walk terminates.
            let is_directory = std::fs::symlink_metadata(&path)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);

            if is_directory {
                self.walk(root, &path);
                continue;
            }

            if path
                .extension()
                .is_some_and(|extension| extension == "desktop")
            {
                self.load(root, &path);
            }
        }
    }

    /// Reads and parses one file, recording the result either way.
    fn load(&mut self, root: &Path, path: &Path) {
        let id = relative_id(root, path);

        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) => {
                self.record(
                    &id,
                    path,
                    EntryProblem::Unreadable {
                        detail: error.to_string(),
                    },
                );
                return;
            }
        };

        // The id is taken whether the entry is usable or not: the first file
        // claiming an id wins, which is what makes a later directory unable to
        // change the meaning of an earlier one.
        match desktop_entry::parse(&source, &id, path, self.filter) {
            Ok(entry) => {
                if !self.seen.insert(id.clone()) {
                    debug!(%id, path = %path.display(), "shadowed by an earlier directory");
                    return;
                }
                for line in &entry.malformed_lines {
                    debug!(%id, line, "skipped a malformed line");
                }
                self.index
                    .entries
                    .push(index_entry(entry, &self.filter.locale, self.icons));
            }
            Err(problem) => self.record(&id, path, problem),
        }
    }

    /// Records a file that could not be used.
    fn record(&mut self, id: &str, path: &Path, problem: EntryProblem) {
        self.seen.insert(id.to_owned());
        debug!(id, path = %path.display(), %problem, "skipping a desktop entry");
        self.index.skipped.push(SkippedEntry {
            id: id.to_owned(),
            path: path.to_path_buf(),
            problem,
        });
    }
}

/// The desktop file id of `path` inside `root`.
///
/// The XDG specification defines it as the path relative to the `applications`
/// directory with `/` replaced by `-`, which is what makes an id unique across
/// directories and stable across a locale change.
fn relative_id(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "-")
}

/// Turns a parsed entry into an index entry, resolving its localized values and
/// its icon.
fn index_entry(
    entry: DesktopEntry,
    locale: &Locale,
    icons: &crate::icons::IconSearch,
) -> IndexedEntry {
    IndexedEntry {
        id: entry.id,
        path: entry.path.clone(),
        name: resolve(&entry.name, locale),
        generic_name: resolve_optional(&entry.generic_name, locale),
        comment: resolve_optional(&entry.comment, locale),
        icon_path: icons.resolve(entry.icon.as_deref()),
        icon: entry.icon,
        exec: entry.exec,
        terminal: entry.terminal,
        working_dir: entry.working_dir,
        startup_wm_class: entry.startup_wm_class,
        categories: entry.categories,
        actions: entry
            .actions
            .into_iter()
            .map(|action| index_action(action, locale))
            .collect(),
        source: entry
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
    }
}

/// Resolves one action's name.
fn index_action(action: DesktopAction, locale: &Locale) -> IndexedAction {
    IndexedAction {
        id: action.id,
        name: resolve(&action.name, locale),
        exec: action.exec,
        icon: action.icon,
    }
}

/// Resolves a localized value the entry is required to define.
fn resolve(value: &Localized, locale: &Locale) -> String {
    // `desktop_entry::parse` rejects an entry whose `Name` does not resolve, so
    // the empty string is unreachable rather than a value a caller has to
    // handle.
    value.resolve(locale).unwrap_or_default().to_owned()
}

/// Resolves a localized value the entry may omit.
fn resolve_optional(value: &Localized, locale: &Locale) -> Option<String> {
    value.resolve(locale).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;
    use std::fs;

    fn filter() -> EntryFilter {
        EntryFilter {
            locale: Locale::parse("en-US").expect("a well-formed tag"),
            desktop_names: vec!["Inox".to_owned()],
            include_hidden: false,
            executables: Vec::new(),
        }
    }

    /// An XDG *data* directory with an empty `applications` under it.
    fn data_directory(root: &Path, name: &str) -> PathBuf {
        let directory = root.join(name);
        fs::create_dir_all(directory.join("applications")).expect("creating a data directory");
        directory
    }

    /// The `applications` directory of a data directory.
    fn applications(data: &Path) -> PathBuf {
        data.join("applications")
    }

    fn write(directory: &Path, relative: &str, source: &str) {
        let path = directory.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("creating a directory");
        }
        fs::write(path, source).expect("writing a desktop entry");
    }

    fn entry(name: &str, exec: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\n{extra}")
    }

    fn scan_of(directories: Vec<PathBuf>) -> AppIndex {
        scan(&IndexOptions::new(directories, filter()), 1)
    }

    #[test]
    fn entries_are_found_in_every_directory() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        let system = data_directory(root.path(), "system");
        write(
            &applications(&user),
            "one.desktop",
            &entry("One", "one", ""),
        );
        write(
            &applications(&system),
            "two.desktop",
            &entry("Two", "two", ""),
        );

        let index = scan_of(vec![user, system]);
        assert_eq!(index.len(), 2);
        assert!(index.get("one.desktop").is_some());
        assert!(index.get("two.desktop").is_some());
    }

    #[test]
    fn an_earlier_directory_wins_a_duplicate_id() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        let system = data_directory(root.path(), "system");
        write(
            &applications(&user),
            "same.desktop",
            &entry("User copy", "user-copy", ""),
        );
        write(
            &applications(&system),
            "same.desktop",
            &entry("System copy", "system-copy", ""),
        );

        let index = scan_of(vec![user.clone(), system.clone()]);
        assert_eq!(index.len(), 1);
        let entry = index.get("same.desktop").expect("the id is present");
        assert_eq!(entry.name, "User copy");
        assert_eq!(entry.source, applications(&user));

        // The other order is the other decision, and it is the same rule.
        let index = scan_of(vec![system, user]);
        assert_eq!(
            index.get("same.desktop").expect("the id").name,
            "System copy"
        );
    }

    #[test]
    fn a_hidden_entry_in_an_earlier_directory_shadows_a_later_one() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        let system = data_directory(root.path(), "system");
        write(
            &applications(&user),
            "same.desktop",
            &entry("Gone", "gone", "Hidden=true\n"),
        );
        write(
            &applications(&system),
            "same.desktop",
            &entry("Still here", "still", ""),
        );

        let index = scan_of(vec![user, system]);
        assert!(index.is_empty(), "Hidden=true means the entry is withdrawn");
        assert_eq!(index.skipped().len(), 1);
        assert_eq!(index.skipped()[0].problem, EntryProblem::Hidden);
    }

    #[test]
    fn include_hidden_lists_them_and_they_still_shadow() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        let system = data_directory(root.path(), "system");
        write(
            &applications(&user),
            "same.desktop",
            &entry("Hidden", "hidden", "NoDisplay=true\n"),
        );
        write(
            &applications(&system),
            "same.desktop",
            &entry("System", "system", ""),
        );

        let mut filter = filter();
        filter.include_hidden = true;
        let index = scan(&IndexOptions::new(vec![user, system], filter), 1);

        assert_eq!(index.len(), 1);
        assert_eq!(index.get("same.desktop").expect("the id").name, "Hidden");
    }

    #[test]
    fn an_id_carries_a_subdirectory() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        write(
            &applications(&user),
            "kde/thing.desktop",
            &entry("Thing", "thing", ""),
        );

        let index = scan_of(vec![user]);
        assert!(
            index.get("kde-thing.desktop").is_some(),
            "the id is the path relative to applications, with / replaced by -",
        );
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let index = scan_of(vec![root.path().join("absent")]);
        assert!(index.is_empty());
        assert!(index.skipped().is_empty());
    }

    #[test]
    fn a_file_that_is_not_a_desktop_entry_is_ignored() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        write(&applications(&user), "notes.txt", "not an entry");
        write(
            &applications(&user),
            "thing.desktop",
            &entry("Thing", "thing", ""),
        );

        let index = scan_of(vec![user]);
        assert_eq!(index.len(), 1);
        assert!(index.skipped().is_empty());
    }

    #[test]
    fn one_bad_file_does_not_stop_the_scan() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        // Three ways a file can be unusable, and one that is fine.
        write(
            &applications(&user),
            "a-broken.desktop",
            "[Desktop Entry]\nType=Application\n",
        );
        fs::write(
            applications(&user).join("b-binary.desktop"),
            [0xff, 0xfe, 0x00],
        )
        .expect("writing bytes");
        write(
            &applications(&user),
            "c-hidden.desktop",
            &entry("Hidden", "hidden", "Hidden=true\n"),
        );
        write(
            &applications(&user),
            "d-good.desktop",
            &entry("Good", "good", ""),
        );

        let index = scan_of(vec![user.clone()]);
        assert_eq!(index.len(), 1, "the good entry is still indexed");
        assert!(index.get("d-good.desktop").is_some());

        let problems: Vec<&EntryProblem> = index
            .skipped()
            .iter()
            .map(|skipped| &skipped.problem)
            .collect();
        assert_eq!(problems.len(), 3);
        assert!(problems.contains(&&EntryProblem::MissingName));
        assert!(problems.contains(&&EntryProblem::Hidden));
        assert!(
            problems
                .iter()
                .any(|problem| matches!(problem, EntryProblem::Unreadable { .. })),
        );
    }

    #[test]
    fn an_entry_with_a_blank_name_is_skipped_and_reported() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        // The exact shape that used to reach the row list: a `Name` key that is
        // present but empty. It must be a diagnostic, not a nameless row.
        write(
            &applications(&user),
            "blank.desktop",
            "[Desktop Entry]\nType=Application\nName=\nExec=blank-app\n",
        );
        write(
            &applications(&user),
            "good.desktop",
            &entry("Good", "good", ""),
        );

        let index = scan_of(vec![user]);
        assert_eq!(index.len(), 1, "only the good entry is indexed");
        assert!(index.get("good.desktop").is_some());
        assert!(index.get("blank.desktop").is_none());
        assert!(index.skipped().iter().any(|skipped| {
            skipped.id == "blank.desktop" && skipped.problem == EntryProblem::MissingName
        }));
    }

    #[test]
    fn a_file_that_is_not_utf8_is_skipped_with_a_reason() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        fs::write(
            applications(&user).join("binary.desktop"),
            [0xff, 0xfe, 0x00],
        )
        .expect("writing bytes");

        let index = scan_of(vec![user]);
        assert!(index.is_empty());
        assert!(matches!(
            index.skipped()[0].problem,
            EntryProblem::Unreadable { .. }
        ));
    }

    #[test]
    fn the_scan_order_is_deterministic() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        for name in ["c", "a", "b"] {
            write(
                &applications(&user),
                &format!("{name}.desktop"),
                &entry(name, name, ""),
            );
        }

        let first = scan_of(vec![user.clone()]);
        let second = scan_of(vec![user]);
        let ids: Vec<&str> = first.entries().iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["a.desktop", "b.desktop", "c.desktop"]);
        assert_eq!(first, second, "two scans of one tree are the same index");
    }

    #[test]
    fn repeated_directories_are_scanned_once() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        write(
            &applications(&user),
            "one.desktop",
            &entry("One", "one", ""),
        );

        let options = IndexOptions::new(vec![user.clone(), user], filter());
        assert_eq!(options.directories.len(), 1);
    }

    #[test]
    fn localized_names_are_resolved_against_the_filter_locale() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        write(
            &applications(&user),
            "files.desktop",
            "[Desktop Entry]\nType=Application\nName=Files\nName[zh_CN]=文件\nExec=files\n",
        );

        let mut filter = filter();
        filter.locale = Locale::parse("zh-CN").expect("a tag");
        let index = scan(&IndexOptions::new(vec![user], filter), 7);

        assert_eq!(index.get("files.desktop").expect("the id").name, "文件");
        assert_eq!(index.generation(), 7);
    }

    #[test]
    fn the_service_state_reports_what_the_scan_found() {
        let root = tempfile::tempdir().expect("a temporary directory");
        let user = data_directory(root.path(), "user");
        write(
            &applications(&user),
            "good.desktop",
            &entry("Good", "good", ""),
        );
        write(
            &applications(&user),
            "bad.desktop",
            "[Desktop Entry]\nType=Application\n",
        );

        let index = scan(&IndexOptions::new(vec![user.clone()], filter()), 3);
        assert_eq!(
            index.state(),
            ServiceState::Ready {
                generation: 3,
                entries: 1,
                skipped: 1,
                directories: vec![user],
            }
        );
        assert!(index.state().is_usable());
        assert_eq!(index.state().generation(), 3);
    }

    #[test]
    fn the_loading_and_unavailable_states_are_usable_only_when_ready() {
        assert!(!ServiceState::Loading { generation: 1 }.is_usable());
        assert!(
            !ServiceState::Unavailable {
                generation: 1,
                failure: crate::contract::Failure::service(
                    crate::contract::FailureKind::IndexUnavailable {
                        detail: "no HOME".to_owned()
                    },
                ),
            }
            .is_usable()
        );
    }

    #[test]
    fn an_entry_reports_the_program_it_names() {
        let entry = IndexedEntry {
            icon_path: None,
            id: "x.desktop".to_owned(),
            path: PathBuf::from("/usr/share/applications/x.desktop"),
            name: "X".to_owned(),
            generic_name: None,
            comment: None,
            icon: None,
            exec: "/usr/bin/x --flag".to_owned(),
            terminal: false,
            working_dir: None,
            startup_wm_class: None,
            categories: Vec::new(),
            actions: Vec::new(),
            source: PathBuf::from("/usr/share/applications"),
        };
        assert_eq!(entry.program(), "/usr/bin/x");
        assert_eq!(
            entry.argv(&[]).expect("a valid command line"),
            ["/usr/bin/x", "--flag"]
        );
    }
}
