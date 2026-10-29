//! `mimeapps.list`: which application the desktop has been told to use.
//!
//! A file or a link is opened by the application the *desktop* was configured
//! with, and the file that records that is `mimeapps.list` (XDG mime-apps
//! specification). It holds three groups of the same shape — `[Default
//! Applications]`, `[Added Associations]`, `[Removed Associations]` — each
//! mapping a MIME type to a `;`-separated list of desktop file ids.
//!
//! Three rules decide the answer, and their relative order is the whole module:
//!
//! - **location beats order.** The specification lists where the files may live
//!   and in which order they are consulted. The first file that names a type
//!   answers for it, so a user's `~/.config/mimeapps.list` beats a
//!   distribution's `/usr/share/applications/mimeapps.list` even when the
//!   distribution's file names a different application.
//! - **defaults before associations.** Within one file, `[Default
//!   Applications]` is the choice the user made; `[Added Associations]` only
//!   means "this application can open this type" and is used when no default
//!   was recorded.
//! - **removals subtract.** An application listed under `[Removed Associations]`
//!   for a type is not offered for it, whichever file added the association.
//!   A *default* is not withdrawn by a removal: the specification gives the
//!   default group precedence, and a user who wrote both meant the default.
//!
//! What this module cannot decide is whether the application is installed. That
//! needs the desktop file, so [`MimeApps::candidates`] returns the whole ordered
//! list and the caller tries each id in turn.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The group holding the user's choice.
const DEFAULTS: &str = "[Default Applications]";

/// The group holding "this application can also open this type".
const ADDED: &str = "[Added Associations]";

/// The group holding associations that must not be used.
const REMOVED: &str = "[Removed Associations]";

/// The directories and desktop names the search order is built from.
///
/// A parameter struct rather than environment reads, for the reason
/// [`crate::desktop_entry::EntryFilter`] documents: a test cannot change its own
/// environment, but it can pass one in.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Search<'a> {
    /// `$XDG_CONFIG_HOME`, when it could be resolved.
    pub(crate) config_home: Option<&'a Path>,
    /// `$XDG_CONFIG_DIRS`, in search order.
    pub(crate) config_dirs: &'a [PathBuf],
    /// `$XDG_DATA_HOME`, when it could be resolved.
    pub(crate) data_home: Option<&'a Path>,
    /// `$XDG_DATA_DIRS`, in search order.
    pub(crate) data_dirs: &'a [PathBuf],
    /// The desktops the session reports, from `XDG_CURRENT_DESKTOP`.
    pub(crate) desktop_names: &'a [String],
}

/// The answers `mimeapps.list` gives, merged across every file that was read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MimeApps {
    /// The defaults, per MIME type, in the order the file wrote them.
    defaults: BTreeMap<String, Vec<String>>,
    /// The added associations, per MIME type, in the order the file wrote them.
    added: BTreeMap<String, Vec<String>>,
    /// The removed associations, per MIME type.
    removed: BTreeMap<String, Vec<String>>,
}

impl MimeApps {
    /// The applications that may open `mime`, most preferred first.
    ///
    /// Defaults come first, then the added associations that were not removed.
    /// Duplicates are dropped, so a type with one entry in both groups is
    /// offered once. An empty answer means nothing is registered for the type,
    /// which is a different failure from "the registered application is not
    /// installed" — that is for the caller to discover, because only it knows
    /// how to look.
    pub(crate) fn candidates(&self, mime: &str) -> Vec<String> {
        let mut candidates: Vec<String> = Vec::new();

        for id in self.defaults.get(mime).into_iter().flatten() {
            push_unique(&mut candidates, id);
        }
        for id in self.added.get(mime).into_iter().flatten() {
            if !self.is_removed(mime, id) {
                push_unique(&mut candidates, id);
            }
        }

        candidates
    }

    /// Whether `mime` was explicitly disassociated from `id`.
    pub(crate) fn is_removed(&self, mime: &str, id: &str) -> bool {
        self.removed
            .get(mime)
            .is_some_and(|ids| ids.iter().any(|listed| listed == id))
    }

    /// Whether nothing at all is recorded, per type or otherwise.
    pub(crate) fn is_empty(&self) -> bool {
        self.defaults.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }
}

/// The files that answer for this session, highest precedence first.
///
/// The order is the specification's, with the desktop-specific name repeated
/// for every desktop the session reports (a session may report several, and
/// `Inox` is the one this desktop sets):
///
/// 1. `$XDG_CONFIG_HOME/<desktop>-mimeapps.list`
/// 2. `$XDG_CONFIG_HOME/mimeapps.list`
/// 3. `<config dir>/<desktop>-mimeapps.list`, then `<config dir>/mimeapps.list`
/// 4. the same two under `$XDG_DATA_HOME/applications`
/// 5. the same two under every `$XDG_DATA_DIRS/applications`
/// 6. the deprecated `defaults.list` in the data directories, last
///
/// Files that do not exist are listed anyway: the caller reads them and skips
/// the ones it cannot open, which keeps the order in one place instead of
/// spreading existence checks through it.
pub(crate) fn files(search: &Search<'_>) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();

    let mut config_directories: Vec<PathBuf> = Vec::new();
    if let Some(home) = search.config_home {
        config_directories.push(home.to_path_buf());
    }
    config_directories.extend(search.config_dirs.iter().cloned());

    let mut data_directories: Vec<PathBuf> = Vec::new();
    if let Some(home) = search.data_home {
        data_directories.push(home.join("applications"));
    }
    for directory in search.data_dirs {
        data_directories.push(directory.join("applications"));
    }

    // Desktop-specific files first, in the order the desktops are reported.
    for name in search.desktop_names {
        let file = format!("{name}-mimeapps.list");
        for directory in &config_directories {
            files.push(directory.join(&file));
        }
        for directory in &data_directories {
            files.push(directory.join(&file));
        }
    }

    for directory in &config_directories {
        files.push(directory.join("mimeapps.list"));
    }
    for directory in &data_directories {
        files.push(directory.join("mimeapps.list"));
    }
    for directory in &data_directories {
        files.push(directory.join("defaults.list"));
    }

    deduplicate(files)
}

/// Reads every file of `files` that exists, keeping the first answer per type.
pub(crate) fn load(files: &[PathBuf]) -> MimeApps {
    let mut apps = MimeApps::default();

    for path in files {
        let source = match std::fs::read_to_string(path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                // A file that exists but cannot be read is a configuration
                // problem worth a line in the log, and not a reason to refuse
                // to open anything: the remaining files still answer.
                tracing::debug!(%error, file = %path.display(), "mimeapps.list could not be read");
                continue;
            }
        };
        merge(&mut apps, &parse(&source));
    }

    apps
}

/// Merges `later` into `earlier`, keeping the first answer for every type.
///
/// "First" is the point: `earlier` comes from a file higher in the search
/// order, so its answer is the one that wins, and a later file may only fill in
/// types that are still unanswered.
fn merge(earlier: &mut MimeApps, later: &MimeApps) {
    for (mime, ids) in &later.defaults {
        earlier
            .defaults
            .entry(mime.clone())
            .or_insert_with(|| ids.clone());
    }
    for (mime, ids) in &later.added {
        earlier
            .added
            .entry(mime.clone())
            .or_insert_with(|| ids.clone());
    }
    // Removals are a union: a removal subtracts wherever it was written.
    for (mime, ids) in &later.removed {
        let entry = earlier.removed.entry(mime.clone()).or_default();
        for id in ids {
            push_unique(entry, id);
        }
    }
}

/// Parses one file.
pub(crate) fn parse(source: &str) -> MimeApps {
    let mut apps = MimeApps::default();
    let mut group: Option<Group> = None;

    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') {
            group = Group::of(line);
            continue;
        }

        let Some(current) = group else {
            // A key outside any group belongs to no list.
            continue;
        };

        let Some((mime, value)) = line.split_once('=') else {
            continue;
        };
        let mime = mime.trim();
        // A MIME type has a type and a subtype; anything else (a typo, a
        // leftover key) cannot be a key of this file.
        if mime.is_empty() || !mime.contains('/') {
            continue;
        }

        let ids = split_ids(value);
        if ids.is_empty() {
            continue;
        }

        let map = match current {
            Group::Defaults => &mut apps.defaults,
            Group::Added => &mut apps.added,
            Group::Removed => &mut apps.removed,
        };
        // Within one file the first occurrence of a key wins, as it does across
        // files: a duplicated line is an editing accident, not a second choice.
        map.entry(mime.to_owned()).or_insert(ids);
    }

    apps
}

/// The group a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    /// `[Default Applications]`
    Defaults,
    /// `[Added Associations]`
    Added,
    /// `[Removed Associations]`
    Removed,
}

impl Group {
    /// The group `header` names, if it names one of the three.
    fn of(header: &str) -> Option<Self> {
        match header {
            DEFAULTS => Some(Self::Defaults),
            ADDED => Some(Self::Added),
            REMOVED => Some(Self::Removed),
            _ => None,
        }
    }
}

/// The desktop ids of a `;`-separated value.
fn split_ids(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Appends `id` unless it is already listed.
fn push_unique(ids: &mut Vec<String>, id: &str) {
    if !ids.iter().any(|listed| listed == id) {
        ids.push(id.to_owned());
    }
}

/// Drops repeated paths, keeping the first occurrence's position.
fn deduplicate(files: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique: Vec<PathBuf> = Vec::with_capacity(files.len());
    for file in files {
        if !unique.contains(&file) {
            unique.push(file);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A search over `config` and `data`, with no desktop-specific file.
    fn search<'a>(config: &'a [PathBuf], data: &'a [PathBuf]) -> Search<'a> {
        Search {
            config_home: config.first().map(PathBuf::as_path),
            config_dirs: &config[1.min(config.len())..],
            data_home: data.first().map(PathBuf::as_path),
            data_dirs: &data[1.min(data.len())..],
            desktop_names: &[],
        }
    }

    fn paths(values: &[&str]) -> Vec<PathBuf> {
        values.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn a_default_is_the_first_candidate() {
        let apps = parse("[Default Applications]\ntext/plain=editor.desktop\n");
        assert_eq!(apps.candidates("text/plain"), ["editor.desktop"]);
    }

    #[test]
    fn an_association_is_used_only_when_there_is_no_default() {
        let apps = parse(
            "[Added Associations]\ntext/plain=viewer.desktop;editor.desktop;\
             \n[Default Applications]\ntext/plain=editor.desktop\n",
        );
        assert_eq!(
            apps.candidates("text/plain"),
            ["editor.desktop", "viewer.desktop"],
            "the default comes first and the association follows, deduplicated"
        );
    }

    #[test]
    fn a_removed_association_is_not_offered() {
        let apps = parse(
            "[Added Associations]\ntext/plain=a.desktop;b.desktop;\
             \n[Removed Associations]\ntext/plain=a.desktop\n",
        );
        assert_eq!(apps.candidates("text/plain"), ["b.desktop"]);
        assert!(apps.is_removed("text/plain", "a.desktop"));
    }

    #[test]
    fn a_removal_does_not_withdraw_a_default() {
        // The specification gives the default group precedence; a user who
        // wrote both meant the default to win.
        let apps = parse(
            "[Default Applications]\ntext/plain=a.desktop\n\
             [Removed Associations]\ntext/plain=a.desktop\n",
        );
        assert_eq!(apps.candidates("text/plain"), ["a.desktop"]);
    }

    #[test]
    fn a_later_file_only_fills_in_what_is_still_unanswered() {
        let mut earlier = parse("[Default Applications]\ntext/plain=user.desktop\n");
        let later =
            parse("[Default Applications]\ntext/plain=system.desktop\nimage/png=viewer.desktop\n");
        merge(&mut earlier, &later);

        assert_eq!(earlier.candidates("text/plain"), ["user.desktop"]);
        assert_eq!(earlier.candidates("image/png"), ["viewer.desktop"]);
    }

    #[test]
    fn removals_are_a_union_across_files() {
        let mut earlier = parse("[Added Associations]\ntext/plain=a.desktop;b.desktop\n");
        let later = parse("[Removed Associations]\ntext/plain=b.desktop\n");
        merge(&mut earlier, &later);

        assert_eq!(earlier.candidates("text/plain"), ["a.desktop"]);
    }

    #[test]
    fn the_user_file_is_consulted_before_the_system_file() {
        let files = files(&Search {
            config_home: Some(Path::new("/home/inox/.config")),
            config_dirs: &paths(&["/etc/xdg"]),
            data_home: Some(Path::new("/home/inox/.local/share")),
            data_dirs: &paths(&["/usr/share"]),
            desktop_names: &["Inox".to_owned()],
        });

        assert_eq!(
            files,
            vec![
                PathBuf::from("/home/inox/.config/Inox-mimeapps.list"),
                PathBuf::from("/etc/xdg/Inox-mimeapps.list"),
                PathBuf::from("/home/inox/.local/share/applications/Inox-mimeapps.list"),
                PathBuf::from("/usr/share/applications/Inox-mimeapps.list"),
                PathBuf::from("/home/inox/.config/mimeapps.list"),
                PathBuf::from("/etc/xdg/mimeapps.list"),
                PathBuf::from("/home/inox/.local/share/applications/mimeapps.list"),
                PathBuf::from("/usr/share/applications/mimeapps.list"),
                PathBuf::from("/home/inox/.local/share/applications/defaults.list"),
                PathBuf::from("/usr/share/applications/defaults.list"),
            ]
        );
    }

    #[test]
    fn the_deprecated_defaults_list_comes_last() {
        let files = files(&search(&[], &paths(&["/usr/share"])));
        let last = files.last().expect("the data directories contribute files");

        assert!(last.ends_with("applications/defaults.list"), "{last:?}");
    }

    #[test]
    fn a_repeated_directory_contributes_its_files_once() {
        let files = files(&Search {
            config_home: Some(Path::new("/etc/xdg")),
            config_dirs: &paths(&["/etc/xdg"]),
            data_home: None,
            data_dirs: &[],
            desktop_names: &[],
        });

        assert_eq!(files.len(), 1, "{files:?}");
    }

    #[test]
    fn comments_blank_lines_and_stray_keys_are_ignored() {
        let apps = parse(
            "# a comment\n\ntext/plain=outside-a-group.desktop\n\
             [Default Applications]\n\n   \n# note\ntext/plain=editor.desktop\n\
             not a key\n",
        );
        assert_eq!(apps.candidates("text/plain"), ["editor.desktop"]);
    }

    #[test]
    fn a_key_without_a_subtype_is_not_a_mime_type() {
        let apps = parse("[Default Applications]\nplain=editor.desktop\n");
        assert!(apps.is_empty());
    }

    #[test]
    fn an_empty_value_records_nothing() {
        let apps = parse("[Default Applications]\ntext/plain=\n");
        assert!(apps.is_empty());
    }

    #[test]
    fn an_unknown_group_is_ignored() {
        let apps = parse("[Some Other Group]\ntext/plain=editor.desktop\n");
        assert!(apps.is_empty());
    }

    #[test]
    fn a_type_nothing_names_has_no_candidates() {
        let apps = parse("[Default Applications]\ntext/plain=editor.desktop\n");
        assert!(apps.candidates("image/png").is_empty());
    }

    #[test]
    fn a_file_that_is_not_there_is_skipped_and_the_rest_still_answers() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let missing = directory.path().join("missing.list");
        let present = directory.path().join("mimeapps.list");
        std::fs::write(
            &present,
            "[Default Applications]\ntext/plain=editor.desktop\n",
        )
        .expect("writing the fixture");

        let apps = load(&[missing, present]);
        assert_eq!(apps.candidates("text/plain"), ["editor.desktop"]);
    }
}
