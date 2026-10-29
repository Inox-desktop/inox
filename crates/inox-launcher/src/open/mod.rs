//! Opening a file or a link with the application registered for it.
//!
//! This is the desktop's `xdg-open`: the thing an application calls when the
//! user clicks a link, and the thing that knows which editor opens a `.txt`. It
//! belongs in the launcher because it is the same job — finding an application
//! and starting it — with the target chosen by the file instead of by the user
//! typing. Everything it needs already exists here: [`crate::exec`] turns an
//! `Exec` value into an argv, [`crate::desktop_entry`] reads the entry, and this
//! module only adds the two answers the launcher never had to give: *what kind
//! of thing is this* ([`mime`]) and *which application did the user choose*
//! ([`mimeapps`]).
//!
//! ## What happens, in order
//!
//! 1. **Classify the argument.** A URI with a scheme is handled by whatever
//!    registered `x-scheme-handler/<scheme>`; a path is handled by whatever
//!    registered the MIME type its name implies. A `file:` URI is converted to
//!    the path it names, so the two spellings of the same file behave alike.
//! 2. **Ask `mimeapps.list` which application that is**, in the specification's
//!    file order, and get an ordered list of candidates.
//! 3. **Take the first candidate that is actually installed** — the desktop
//!    file is located in the data directories, parsed, and its `TryExec`
//!    honoured. A registration pointing at an application that has been
//!    removed falls through to the next candidate rather than failing.
//! 4. **Expand its `Exec` with the target**, through [`crate::exec::argv`], so
//!    `%f`/`%u`/`%U` receive the file or link as *one argument* and a name with
//!    spaces or quotes in it can never become a second one.
//! 5. **Start it** and report the process id.
//!
//! ## What this does not do
//!
//! - **No portal.** `org.freedesktop.portal.OpenURI` is the sandboxed path and
//!   it needs an `xdg-desktop-portal` backend, which this repository does not
//!   have (see `docs/book/src/architecture/components.md`). Everything here is
//!   the unsandboxed path: the caller's own environment, the caller's own
//!   permissions.
//! - **No magic-number sniffing.** The MIME type of a file comes from its name
//!   through the shared MIME database's glob rules; see [`mime`] for why.
//! - **No readiness verification.** [`crate::activation`] waits for evidence
//!   that a launched application is really up, because the launcher shows the
//!   user a status line. An opener cannot: `xdg-open`'s contract is that it
//!   returns as soon as the application was started, and a link clicked in a
//!   browser must not sit there for three quarters of a second. A spawn failure
//!   is still reported — as an exit code, which is what a caller checks.
//!
//! ## Exit codes
//!
//! | code | meaning |
//! |------|---------|
//! | 0 | an application was started |
//! | 1 | the command line was wrong |
//! | 2 | the target does not exist |
//! | 3 | no application can open this target (unknown type, no registration, or the registered one is not installed) |
//! | 4 | the application was found but could not be started |
//!
//! These are this binary's own, and the split that matters is 3 versus 4: a
//! caller that gets 3 may offer the user a choice of application, while 4 means
//! the choice was made and did not work.

pub(crate) mod mime;
pub(crate) mod mimeapps;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use inox_common::i18n::{Arg, Locale, Localizer};
use tracing::{debug, error, warn};

use crate::contract::{Failure, FailureKind};
use crate::desktop_entry::{self, EntryFilter};
use crate::exec::{self, ExecContext};
use crate::i18n as catalogs;
use crate::view_model;

/// An application was started.
pub(crate) const EXIT_OPENED: u8 = 0;
/// The command line could not be understood.
pub(crate) const EXIT_USAGE: u8 = 1;
/// The target is not there.
pub(crate) const EXIT_NOT_FOUND: u8 = 2;
/// Nothing can open the target.
pub(crate) const EXIT_NO_HANDLER: u8 = 3;
/// The application that should have opened it could not be started.
pub(crate) const EXIT_LAUNCH_FAILED: u8 = 4;

/// The name this binary answers to when it is installed as the opener.
///
/// A single binary serving two commands is the shape `--exec-application`
/// already established here, and it keeps the opener's logic in one place. A
/// packaging step that installs `inox-open` as a link to `inox-launcher` (or a
/// two-line wrapper) is therefore all the integration an `xdg-open` caller
/// needs; `--open` is the same entry point spelled out for scripts that would
/// rather not depend on the name.
const OPENER_NAME: &str = "inox-open";

/// The arguments the opener was invoked with, if this is an opener invocation.
///
/// Two forms are accepted, and the name is checked first so that a link or
/// wrapper works without arguments of its own: `inox-open TARGET…`, and
/// `inox-launcher --open TARGET…`.
pub(crate) fn opener_arguments(
    program: Option<&OsStr>,
    arguments: &[OsString],
) -> Option<Vec<OsString>> {
    let named = program
        .map(Path::new)
        .and_then(Path::file_stem)
        .is_some_and(|stem| stem == OPENER_NAME);

    if named {
        return Some(arguments.to_vec());
    }

    match arguments.split_first() {
        Some((flag, rest)) if flag == OsStr::new("--open") => Some(rest.to_vec()),
        _ => None,
    }
}

/// The environment the opener reads, snapshotted so that it can be faked.
///
/// Every path the search depends on is a field rather than an environment read:
/// a test cannot set its own environment, and the order of the XDG directories
/// is the half of this module most worth testing.
#[derive(Debug, Clone)]
pub(crate) struct Environment {
    /// `$XDG_CONFIG_HOME`, when it resolves.
    pub(crate) config_home: Option<PathBuf>,
    /// `$XDG_CONFIG_DIRS`, in search order.
    pub(crate) config_dirs: Vec<PathBuf>,
    /// `$XDG_DATA_HOME`, when it resolves.
    pub(crate) data_home: Option<PathBuf>,
    /// `$XDG_DATA_DIRS`, in search order.
    pub(crate) data_dirs: Vec<PathBuf>,
    /// The desktops the session reports (`XDG_CURRENT_DESKTOP`, split on `:`).
    pub(crate) desktop_names: Vec<String>,
    /// The directories `TryExec` programs are looked for in (`PATH`).
    pub(crate) executables: Vec<PathBuf>,
    /// `shell.toml`'s `launcher.terminal`, for an entry with `Terminal=true`.
    pub(crate) terminal_prefix: Vec<String>,
    /// The locale an entry's name is resolved against.
    pub(crate) locale: Locale,
    /// The shared MIME database's glob rules.
    pub(crate) globs: mime::GlobTable,
}

impl Environment {
    /// Snapshots the session this process runs in.
    pub(crate) fn from_process(terminal_prefix: Vec<String>, locale: Locale) -> Self {
        let paths = inox_common::paths::XdgEnv::from_process();
        let filter = EntryFilter::from_env(&locale);
        let data_dirs = paths.data_dirs();

        Self {
            config_home: paths.config_home().ok(),
            config_dirs: paths.config_dirs(),
            data_home: paths.data_home().ok(),
            data_dirs: data_dirs.clone(),
            desktop_names: filter.desktop_names,
            executables: filter.executables,
            terminal_prefix,
            locale,
            globs: mime::load(&data_dirs),
        }
    }

    /// The roots the `applications` directories hang off, highest precedence
    /// first.
    fn data_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(home) = &self.data_home {
            roots.push(home.clone());
        }
        roots.extend(self.data_dirs.iter().cloned());
        roots
    }

    /// The rules an entry's own flags are judged by.
    ///
    /// Hidden entries are included on purpose: `NoDisplay=true` means "keep this
    /// out of the launcher", which is exactly what a handler such as
    /// `gs.desktop` wants, and refusing to open files with it would be wrong.
    fn filter(&self) -> EntryFilter {
        EntryFilter {
            locale: self.locale.clone(),
            desktop_names: self.desktop_names.clone(),
            include_hidden: true,
            executables: self.executables.clone(),
        }
    }

    /// The search order for `mimeapps.list`.
    fn search(&self) -> mimeapps::Search<'_> {
        mimeapps::Search {
            config_home: self.config_home.as_deref(),
            config_dirs: &self.config_dirs,
            data_home: self.data_home.as_deref(),
            data_dirs: &self.data_dirs,
            desktop_names: &self.desktop_names,
        }
    }
}

/// What the user asked to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A URI, handled by `x-scheme-handler/<scheme>`.
    Uri {
        /// The argument as it was written, which is what `%u` receives.
        raw: String,
        /// The scheme, lowercased.
        scheme: String,
    },
    /// A path on disk, handled by the application registered for its type.
    File {
        /// The path.
        path: PathBuf,
        /// Its MIME type.
        mime: String,
    },
}

impl Target {
    /// The MIME type whose registered application opens this target.
    fn mime_type(&self) -> String {
        match self {
            Self::Uri { scheme, .. } => format!("x-scheme-handler/{scheme}"),
            Self::File { mime, .. } => mime.clone(),
        }
    }

    /// The text passed to the application, for `%f`, `%u`, `%F` and `%U`.
    ///
    /// A path that is not valid UTF-8 loses the undecodable bytes here, for the
    /// same reason `%k` does in [`crate::exec`]: an argv element has to be a
    /// valid `String` in this process, and replacing the bytes keeps an
    /// otherwise fine file openable.
    fn argument(&self) -> String {
        match self {
            Self::Uri { raw, .. } => raw.clone(),
            Self::File { path, .. } => path.to_string_lossy().into_owned(),
        }
    }

    /// The target as the user wrote it, for messages.
    fn describe(&self) -> String {
        self.argument()
    }
}

/// The application that will open the target, and the argv it will be started
/// with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Handler {
    /// The desktop file id the registration named.
    pub(crate) id: String,
    /// The desktop file that id resolved to.
    pub(crate) path: PathBuf,
    /// The entry's localized name.
    pub(crate) name: String,
    /// The argv, with the target already substituted.
    pub(crate) argv: Vec<String>,
}

/// A started application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Started {
    /// What was started and with which argv.
    pub(crate) handler: Handler,
    /// The process id the kernel gave it.
    pub(crate) pid: u32,
}

/// Why a target could not be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OpenError {
    /// The argument names nothing that exists.
    NotFound {
        /// The target, as the user wrote it.
        target: String,
    },
    /// No glob rule matched, so the type is unknown.
    UnknownType {
        /// The target, as the user wrote it.
        target: String,
    },
    /// The shared MIME database is not installed, so no type can be named.
    NoMimeDatabase {
        /// The directories that were searched.
        dirs: String,
    },
    /// Nothing is registered for the type.
    NoHandler {
        /// The MIME type.
        mime: String,
    },
    /// The registration names an application that is not installed.
    HandlerMissing {
        /// The MIME type.
        mime: String,
        /// The id that was named.
        id: String,
    },
    /// The application is installed but its entry cannot be used.
    UnusableEntry {
        /// The desktop file id.
        id: String,
        /// Why, in the entry reader's words.
        problem: String,
    },
    /// The entry's `Exec` cannot be expanded.
    Exec {
        /// The desktop file id.
        id: String,
        /// The field-code failure.
        failure: FailureKind,
    },
    /// The process could not be created.
    Spawn {
        /// The program that was asked for.
        program: String,
        /// The operating system's own words.
        detail: String,
    },
}

impl OpenError {
    /// The exit code this failure is reported with.
    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            Self::NotFound { .. } => EXIT_NOT_FOUND,
            Self::UnknownType { .. } | Self::NoMimeDatabase { .. } => EXIT_NO_HANDLER,
            Self::NoHandler { .. } | Self::HandlerMissing { .. } | Self::UnusableEntry { .. } => {
                EXIT_NO_HANDLER
            }
            Self::Exec { .. } | Self::Spawn { .. } => EXIT_LAUNCH_FAILED,
        }
    }

    /// The sentence the user reads, in their language.
    pub(crate) fn describe(&self, localizer: &Localizer) -> String {
        match self {
            Self::NotFound { target } => {
                localizer.message_with("launcher-open-not-found", &[("target", Arg::from(target))])
            }
            Self::UnknownType { target } => localizer.message_with(
                "launcher-open-unknown-type",
                &[("target", Arg::from(target))],
            ),
            Self::NoMimeDatabase { dirs } => localizer.message_with(
                "launcher-open-no-mime-database",
                &[("dirs", Arg::from(dirs))],
            ),
            Self::NoHandler { mime } => {
                localizer.message_with("launcher-open-no-handler", &[("type", Arg::from(mime))])
            }
            Self::HandlerMissing { mime, id } => localizer.message_with(
                "launcher-open-handler-missing",
                &[("type", Arg::from(mime)), ("id", Arg::from(id))],
            ),
            Self::UnusableEntry { id, problem } => localizer.message_with(
                "launcher-open-entry-unusable",
                &[("id", Arg::from(id)), ("reason", Arg::from(problem))],
            ),
            Self::Exec { id, failure } => {
                let reason = view_model::failure_message(
                    &Failure::for_entry(failure.clone(), id),
                    localizer,
                );
                localizer.message_with(
                    "launcher-open-exec-failed",
                    &[("id", Arg::from(id)), ("reason", Arg::from(reason))],
                )
            }
            Self::Spawn { program, detail } => localizer.message_with(
                "launcher-open-spawn-failed",
                &[
                    ("program", Arg::from(program)),
                    ("detail", Arg::from(detail)),
                ],
            ),
        }
    }
}

/// Opens `raw` with the application the session has registered for it.
///
/// # Errors
///
/// Returns the [`OpenError`] that stopped it: see the module documentation for
/// what each one means and which exit code it carries.
pub(crate) fn open(raw: &OsStr, environment: &Environment) -> Result<Started, OpenError> {
    let target = classify(raw, environment)?;
    debug!(
        target = %target.describe(),
        mime = %target.mime_type(),
        "resolved the target's type"
    );

    let handler = resolve(&target, environment)?;
    let pid = launch(&handler)?;

    Ok(Started { handler, pid })
}

/// Decides what `raw` names.
fn classify(raw: &OsStr, environment: &Environment) -> Result<Target, OpenError> {
    if environment.globs.is_empty() {
        // Without the database no file can be named, and every file would be
        // reported as "unknown type" — which would blame the file. Say what is
        // actually missing instead.
        let dirs = environment
            .data_dirs
            .iter()
            .map(|directory| directory.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(OpenError::NoMimeDatabase { dirs });
    }

    let text = raw.to_string_lossy();
    if let Some(scheme) = mime::scheme(&text).map(str::to_ascii_lowercase) {
        if scheme == "file" {
            let path = file_uri_path(&text).ok_or_else(|| OpenError::NotFound {
                target: text.clone().into_owned(),
            })?;
            return classify_path(path, environment);
        }
        return Ok(Target::Uri {
            raw: text.into_owned(),
            scheme,
        });
    }

    classify_path(PathBuf::from(raw), environment)
}

/// Names a path's type, and checks that it is there.
fn classify_path(path: PathBuf, environment: &Environment) -> Result<Target, OpenError> {
    let metadata = std::fs::metadata(&path).map_err(|_| OpenError::NotFound {
        target: path.to_string_lossy().into_owned(),
    })?;

    // A directory has no extension to match, and its type is fixed by the
    // specification rather than by the database.
    let mime = if metadata.is_dir() {
        "inode/directory".to_owned()
    } else {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        environment
            .globs
            .lookup(&name)
            .ok_or_else(|| OpenError::UnknownType {
                target: path.to_string_lossy().into_owned(),
            })?
            .to_owned()
    };

    Ok(Target::File { path, mime })
}

/// The path a `file:` URI names.
///
/// A URI naming another host is refused: reading it would mean a file transfer,
/// which is not what an opener does.
fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri
        .get(..5)
        .filter(|scheme| scheme.eq_ignore_ascii_case("file:"))
        .map(|_| &uri[5..])?;
    let path = rest.strip_prefix("//").unwrap_or(rest);

    if !path.starts_with('/') {
        return None;
    }
    Some(PathBuf::from(mime::percent_decode(path)))
}

/// Finds the application that opens `target` and what it will be started with.
fn resolve(target: &Target, environment: &Environment) -> Result<Handler, OpenError> {
    let mime = target.mime_type();
    let files = mimeapps::files(&environment.search());
    let apps = mimeapps::load(&files);
    let candidates = apps.candidates(&mime);

    if candidates.is_empty() {
        return Err(OpenError::NoHandler { mime });
    }

    let roots = environment.data_roots();
    let filter = environment.filter();
    // The first candidate that resolves wins. A candidate whose file is gone is
    // skipped, because a distribution that removed an application often leaves
    // the registration behind; only when nothing resolves is the failure
    // reported, and then with the reason the *first* candidate gave, which is
    // the one the user's configuration actually named.
    let mut first_failure: Option<OpenError> = None;

    for id in &candidates {
        let Some(path) = locate(&roots, id) else {
            debug!(id, "the registered application is not installed");
            first_failure.get_or_insert_with(|| OpenError::HandlerMissing {
                mime: mime.clone(),
                id: id.clone(),
            });
            continue;
        };

        match handler_from(&path, id, target, environment, &filter) {
            Ok(handler) => return Ok(handler),
            Err(error) => {
                let reason = error_display(&error);
                warn!(id, reason, "the registered application cannot be used");
                first_failure.get_or_insert(error);
            }
        }
    }

    Err(first_failure.unwrap_or(OpenError::NoHandler { mime }))
}

/// The path of the desktop file with `id`, in the first root that has it.
fn locate(roots: &[PathBuf], id: &str) -> Option<PathBuf> {
    for root in roots {
        let applications = root.join("applications");
        if let Some(path) = find(&applications, &applications, id) {
            return Some(path);
        }
    }
    None
}

/// Walks one `applications` directory looking for the file whose id is `id`.
///
/// The id is derived from the path (the path relative to `applications` with
/// `/` replaced by `-`), so the only faithful inverse is to derive it the same
/// way for every file. Guessing at dash positions would confuse
/// `applications/kde/foo.desktop` with `applications/kde-foo.desktop`.
fn find(root: &Path, directory: &Path, id: &str) -> Option<PathBuf> {
    let listing = std::fs::read_dir(directory).ok()?;
    let mut paths: Vec<PathBuf> = listing
        .filter_map(|item| item.ok().map(|item| item.path()))
        .collect();
    paths.sort();

    for path in paths {
        // `symlink_metadata` does not follow links, so a directory link is never
        // entered and the walk terminates.
        let is_directory = std::fs::symlink_metadata(&path)
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false);

        if is_directory {
            if let Some(found) = find(root, &path, id) {
                return Some(found);
            }
            continue;
        }

        if path
            .extension()
            .is_some_and(|extension| extension == "desktop")
            && relative_id(root, &path) == id
        {
            return Some(path);
        }
    }

    None
}

/// The desktop file id of `path` inside `root`; see [`locate`].
fn relative_id(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "-")
}

/// Reads one entry and turns it into the argv that opens `target`.
fn handler_from(
    path: &Path,
    id: &str,
    target: &Target,
    environment: &Environment,
    filter: &EntryFilter,
) -> Result<Handler, OpenError> {
    let source = std::fs::read_to_string(path).map_err(|error| OpenError::UnusableEntry {
        id: id.to_owned(),
        problem: error.to_string(),
    })?;

    let entry = desktop_entry::parse(&source, id, path, filter).map_err(|problem| {
        OpenError::UnusableEntry {
            id: id.to_owned(),
            problem: problem.to_string(),
        }
    })?;

    let name = entry
        .name
        .resolve(&environment.locale)
        .unwrap_or(id)
        .to_owned();
    let argument = target.argument();
    let argv = exec::argv(&ExecContext {
        exec: &entry.exec,
        name: &name,
        icon: entry.icon.as_deref(),
        desktop_file: path,
        // The one difference from the launcher's own activations: the target is
        // passed to the entry, so `%f` and `%U` are not empty.
        files: std::slice::from_ref(&argument),
        terminal: entry.terminal,
        terminal_prefix: &environment.terminal_prefix,
    })
    .map_err(|failure| OpenError::Exec {
        id: id.to_owned(),
        failure,
    })?;

    Ok(Handler {
        id: id.to_owned(),
        path: path.to_path_buf(),
        name,
        argv,
    })
}

/// Starts `handler`'s argv.
///
/// The child's streams are not inherited: a GUI application holding the
/// opener's standard output would keep a pipe open that its caller is waiting
/// on. The child is deliberately not waited for — see the module documentation
/// — which leaves it to be reparented when this process exits.
fn launch(handler: &Handler) -> Result<u32, OpenError> {
    let (program, arguments) = handler.argv.split_first().ok_or_else(|| OpenError::Exec {
        id: handler.id.clone(),
        failure: FailureKind::NoCommand,
    })?;

    let child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| OpenError::Spawn {
            program: program.clone(),
            detail: error.to_string(),
        })?;

    Ok(child.id())
}

/// Runs the opener command line and returns its exit code.
///
/// The configuration is read for two things only — the terminal prefix a
/// `Terminal=true` handler needs, and the locale the messages are written in —
/// and a configuration that cannot be read is not fatal: the opener still
/// works, it just has no terminal prefix and speaks the environment's locale.
pub(crate) fn run_cli(arguments: &[OsString]) -> u8 {
    let config = configuration();
    let locale = config
        .as_ref()
        .map(|config| config.locale.clone())
        .unwrap_or_else(Locale::from_env);

    let localizer = match catalogs::localizer(&locale) {
        Ok(localizer) => localizer,
        // The catalogs are embedded and `crate::catalog_tests` keeps them
        // loadable, so this is a build-time mistake rather than something the
        // user can fix. It must still not panic in a program a link click runs.
        Err(error) => {
            error!(%error, "the launcher catalogs could not be loaded");
            return EXIT_LAUNCH_FAILED;
        }
    };

    let Some(raw) = arguments.first() else {
        error!("{}", localizer.message("launcher-open-usage"));
        return EXIT_USAGE;
    };

    if raw.to_string_lossy().starts_with('-') {
        // A flag this binary does not define; `--` would be a target, and a
        // target cannot start with a dash without being ambiguous.
        error!("{}", localizer.message("launcher-open-usage"));
        return EXIT_USAGE;
    }

    let terminal_prefix = config
        .as_ref()
        .map(|config| config.shell.launcher.terminal.clone())
        .unwrap_or_default();

    let environment = Environment::from_process(terminal_prefix, locale);
    match open(raw, &environment) {
        Ok(started) => {
            debug!(
                id = %started.handler.id,
                name = %started.handler.name,
                pid = started.pid,
                argv = ?started.handler.argv,
                "started the application that opens this target"
            );
            EXIT_OPENED
        }
        Err(failure) => {
            error!("{}", failure.describe(&localizer));
            failure.exit_code()
        }
    }
}

/// The session's configuration, when it can be read.
///
/// Nothing here needs it badly enough to fail over, so every step degrades to
/// "no configuration" rather than to an error the user cannot act on.
fn configuration() -> Option<inox_config::InoxConfig> {
    let paths = inox_config::ConfigPaths::discover().ok()?;
    inox_config::InoxConfig::load(&paths, inox_config::Appearance::System).ok()
}

/// Renders an error for a log line, without a localizer.
fn error_display(error: &OpenError) -> String {
    match error {
        OpenError::NotFound { target } => format!("{target}: not found"),
        OpenError::UnknownType { target } => format!("{target}: unknown type"),
        OpenError::NoMimeDatabase { dirs } => format!("no MIME database in {dirs}"),
        OpenError::NoHandler { mime } => format!("no handler for {mime}"),
        OpenError::HandlerMissing { mime, id } => format!("{mime}: {id} is not installed"),
        OpenError::UnusableEntry { id, problem } => format!("{id}: {problem}"),
        OpenError::Exec { id, failure } => format!("{id}: {failure:?}"),
        OpenError::Spawn { program, detail } => format!("{program}: {detail}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;

    /// A MIME database small enough to reason about, in the real format.
    const GLOBS: &str = "50:text/plain:*.txt\n50:image/png:*.png\n50:text/markdown:*.md\n";

    /// A temporary XDG tree: a user data directory, a system one, and the
    /// configuration directory the user's `mimeapps.list` lives in.
    struct Fixture {
        directory: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                directory: tempfile::tempdir().expect("a temporary directory"),
            }
        }

        fn root(&self) -> &Path {
            self.directory.path()
        }

        /// Writes `source` at `relative`, creating the directories above it.
        fn write(&self, relative: &str, source: &str) -> PathBuf {
            let path = self.root().join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("creating the fixture directories");
            }
            std::fs::write(&path, source).expect("writing the fixture");
            path
        }

        /// An empty file at `relative`.
        fn touch(&self, relative: &str) -> PathBuf {
            self.write(relative, "")
        }

        fn environment(&self) -> Environment {
            Environment {
                config_home: Some(self.root().join("config")),
                config_dirs: Vec::new(),
                data_home: Some(self.root().join("data")),
                data_dirs: vec![self.root().join("system")],
                desktop_names: vec!["Inox".to_owned()],
                executables: vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")],
                terminal_prefix: vec!["test-terminal".to_owned(), "-e".to_owned()],
                locale: Locale::parse("en-US").expect("a well-formed tag"),
                globs: mime::parse(GLOBS, true),
            }
        }
    }

    /// A desktop entry that runs `exec` and does nothing else.
    fn application(exec: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName=Test Application\nExec={exec}\n")
    }

    /// A user `mimeapps.list` that names `id` as the default for `mime`.
    fn registration(mime: &str, id: &str) -> String {
        format!("[Default Applications]\n{mime}={id}\n")
    }

    #[test]
    fn a_text_file_is_opened_by_its_default_application() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/editor.desktop",
            &application("/bin/editor %f"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "editor.desktop"),
        );
        let file = fixture.touch("data/notes.txt");

        let started =
            classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let handler = resolve(&started, &fixture.environment()).expect("a handler is registered");

        assert_eq!(handler.id, "editor.desktop");
        assert_eq!(handler.name, "Test Application");
        assert_eq!(
            handler.argv,
            [
                String::from("/bin/editor"),
                file.to_string_lossy().into_owned()
            ]
        );
    }

    #[test]
    fn a_link_is_opened_by_its_scheme_handler() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/browser.desktop",
            &application("/bin/browser %u"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("x-scheme-handler/https", "browser.desktop"),
        );

        let target = classify(
            OsStr::new("https://example.org/page?q=1"),
            &fixture.environment(),
        )
        .expect("a URI needs no file");
        let handler = resolve(&target, &fixture.environment()).expect("a handler is registered");

        assert_eq!(
            handler.argv,
            ["/bin/browser", "https://example.org/page?q=1"],
            "the link is passed as written"
        );
    }

    #[test]
    fn a_registration_that_names_an_installed_application_is_preferred_to_the_next() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/editor.desktop",
            &application("/bin/editor %f"),
        );
        fixture.write(
            "data/applications/viewer.desktop",
            &application("/bin/viewer %f"),
        );
        fixture.write(
            "config/mimeapps.list",
            "[Default Applications]\ntext/plain=gone.desktop\n\
             [Added Associations]\ntext/plain=viewer.desktop;editor.desktop;\n",
        );
        let file = fixture.touch("data/notes.txt");

        let target = classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let handler = resolve(&target, &fixture.environment()).expect("the second candidate works");

        assert_eq!(
            handler.id, "viewer.desktop",
            "the association stands in for the registration that is not installed"
        );
    }

    #[test]
    fn a_type_nothing_is_registered_for_is_a_handler_failure_not_a_crash() {
        let fixture = Fixture::new();
        let file = fixture.touch("data/picture.png");

        let target = classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let error = resolve(&target, &fixture.environment()).expect_err("nothing opens PNG here");

        assert_eq!(
            error,
            OpenError::NoHandler {
                mime: "image/png".to_owned()
            }
        );
        assert_eq!(error.exit_code(), EXIT_NO_HANDLER);
    }

    #[test]
    fn a_file_whose_name_matches_no_rule_has_no_type() {
        let fixture = Fixture::new();
        let file = fixture.touch("data/mystery.zzz");

        let error =
            classify(file.as_os_str(), &fixture.environment()).expect_err("no rule matches");

        assert!(matches!(error, OpenError::UnknownType { .. }), "{error:?}");
        assert_eq!(error.exit_code(), EXIT_NO_HANDLER);
    }

    #[test]
    fn a_missing_database_is_reported_as_a_missing_database() {
        let fixture = Fixture::new();
        let file = fixture.touch("data/notes.txt");
        let mut environment = fixture.environment();
        environment.globs = mime::GlobTable::default();

        let error = classify(file.as_os_str(), &environment).expect_err("there is no database");

        assert!(
            matches!(error, OpenError::NoMimeDatabase { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_target_that_is_not_there_is_not_found() {
        let fixture = Fixture::new();
        let missing = fixture.root().join("data/nothing.txt");

        let error =
            classify(missing.as_os_str(), &fixture.environment()).expect_err("nothing there");

        assert!(matches!(error, OpenError::NotFound { .. }), "{error:?}");
        assert_eq!(error.exit_code(), EXIT_NOT_FOUND);
    }

    #[test]
    fn a_directory_opens_the_file_manager_whatever_its_name() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/files.desktop",
            &application("/bin/files %f"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("inode/directory", "files.desktop"),
        );
        let directory = fixture.root().join("data/some-folder");
        std::fs::create_dir_all(&directory).expect("creating the directory");

        let target = classify(directory.as_os_str(), &fixture.environment()).expect("a directory");
        assert_eq!(target.mime_type(), "inode/directory");

        let handler = resolve(&target, &fixture.environment()).expect("a handler is registered");
        assert_eq!(handler.id, "files.desktop");
    }

    #[test]
    fn a_terminal_application_is_started_through_the_configured_prefix() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/console.desktop",
            "[Desktop Entry]\nType=Application\nName=Console\nExec=/bin/less %f\nTerminal=true\n",
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/markdown", "console.desktop"),
        );
        let file = fixture.touch("data/notes.md");

        let target = classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let handler = resolve(&target, &fixture.environment()).expect("a handler is registered");

        assert_eq!(
            handler.argv,
            [
                "test-terminal",
                "-e",
                "/bin/less",
                file.to_string_lossy().as_ref()
            ]
        );
    }

    #[test]
    fn a_nested_entry_is_found_by_the_id_its_path_gives_it() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/kde/foo.desktop",
            &application("/bin/foo %f"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "kde-foo.desktop"),
        );
        let file = fixture.touch("data/notes.txt");

        let target = classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let handler = resolve(&target, &fixture.environment()).expect("the nested entry is found");

        assert_eq!(handler.id, "kde-foo.desktop");
        assert!(handler.path.ends_with("applications/kde/foo.desktop"));
    }

    #[test]
    fn the_user_registration_beats_the_system_one() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/mine.desktop",
            &application("/bin/mine %f"),
        );
        fixture.write(
            "system/applications/theirs.desktop",
            &application("/bin/theirs %f"),
        );
        fixture.write(
            "system/applications/mimeapps.list",
            &registration("text/plain", "theirs.desktop"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "mine.desktop"),
        );
        let file = fixture.touch("data/notes.txt");

        let target = classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let handler = resolve(&target, &fixture.environment()).expect("a handler is registered");

        assert_eq!(handler.id, "mine.desktop");
    }

    #[test]
    fn a_name_with_spaces_and_quotes_stays_one_argument() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/editor.desktop",
            &application("/bin/editor --new %u"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "editor.desktop"),
        );
        let file = fixture.touch("data/a \"quoted\" name.txt");

        let target = classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let handler = resolve(&target, &fixture.environment()).expect("a handler is registered");

        assert_eq!(handler.argv.len(), 3, "{:?}", handler.argv);
        assert_eq!(handler.argv[2], file.to_string_lossy());
    }

    #[test]
    fn a_file_uri_is_opened_as_the_path_it_names() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/editor.desktop",
            &application("/bin/editor %f"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "editor.desktop"),
        );
        let file = fixture.touch("data/a b.txt");
        let uri = format!("file://{}", file.to_string_lossy().replace(' ', "%20"));

        let target = classify(OsStr::new(&uri), &fixture.environment()).expect("a file URI");
        assert_eq!(
            target,
            Target::File {
                path: file.clone(),
                mime: "text/plain".to_owned()
            }
        );
        let handler = resolve(&target, &fixture.environment()).expect("a handler is registered");
        assert_eq!(handler.argv[1], file.to_string_lossy());
    }

    #[test]
    fn a_file_uri_naming_another_host_is_refused() {
        let fixture = Fixture::new();
        let error = classify(
            OsStr::new("file://elsewhere/tmp/a.txt"),
            &fixture.environment(),
        )
        .expect_err("a remote file is not a local path");

        assert!(matches!(error, OpenError::NotFound { .. }), "{error:?}");
    }

    #[test]
    fn an_entry_with_an_unreadable_exec_reports_the_field_code_problem() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/broken.desktop",
            &application("/bin/editor %q"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "broken.desktop"),
        );
        let file = fixture.touch("data/notes.txt");

        let target = classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let error = resolve(&target, &fixture.environment()).expect_err("the Exec is invalid");

        assert_eq!(
            error,
            OpenError::Exec {
                id: "broken.desktop".to_owned(),
                failure: FailureKind::UnknownFieldCode { code: 'q' }
            }
        );
        assert_eq!(error.exit_code(), EXIT_LAUNCH_FAILED);
    }

    #[test]
    fn an_entry_that_is_not_type_application_is_unusable() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/link.desktop",
            "[Desktop Entry]\nType=Link\nName=A Link\nExec=/bin/editor\n",
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "link.desktop"),
        );
        let file = fixture.touch("data/notes.txt");

        let target = classify(file.as_os_str(), &fixture.environment()).expect("the type is known");
        let error = resolve(&target, &fixture.environment()).expect_err("only applications open");

        assert!(
            matches!(error, OpenError::UnusableEntry { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn opening_starts_the_application_and_reports_its_pid() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/true.desktop",
            &application("/bin/true %f"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "true.desktop"),
        );
        let file = fixture.touch("data/notes.txt");

        let started = open(file.as_os_str(), &fixture.environment()).expect("the process starts");

        assert!(started.pid > 0);
        assert_eq!(started.handler.id, "true.desktop");
    }

    #[test]
    fn an_application_that_cannot_be_started_is_reported_with_its_name() {
        let fixture = Fixture::new();
        fixture.write(
            "data/applications/ghost.desktop",
            &application("/nonexistent/inox-test-program %f"),
        );
        fixture.write(
            "config/mimeapps.list",
            &registration("text/plain", "ghost.desktop"),
        );
        let file = fixture.touch("data/notes.txt");

        let error = open(file.as_os_str(), &fixture.environment()).expect_err("the spawn fails");

        match &error {
            OpenError::Spawn { program, detail } => {
                assert_eq!(program, "/nonexistent/inox-test-program");
                assert!(!detail.is_empty());
            }
            other => panic!("expected a spawn failure, got {other:?}"),
        }
        assert_eq!(error.exit_code(), EXIT_LAUNCH_FAILED);
    }

    #[test]
    fn the_opener_answers_to_its_own_name_and_to_the_open_flag() {
        let one = vec![OsString::from("/tmp/a.txt")];

        assert_eq!(
            opener_arguments(Some(OsStr::new("/usr/bin/inox-open")), &one),
            Some(one.clone())
        );
        assert_eq!(
            opener_arguments(
                Some(OsStr::new("/usr/bin/inox-launcher")),
                &[OsString::from("--open"), OsString::from("/tmp/a.txt")]
            ),
            Some(one.clone())
        );
        assert_eq!(
            opener_arguments(Some(OsStr::new("/usr/bin/inox-launcher")), &one),
            None,
            "the launcher's own window is the default"
        );
    }

    #[test]
    fn every_failure_carries_a_distinct_sentence() {
        let localizer = catalogs::localizer(&Locale::parse("en-US").expect("a tag"))
            .expect("the catalogs load");

        let failures = [
            OpenError::NotFound {
                target: "/tmp/a.txt".to_owned(),
            },
            OpenError::UnknownType {
                target: "/tmp/a.zzz".to_owned(),
            },
            OpenError::NoMimeDatabase {
                dirs: "/usr/share".to_owned(),
            },
            OpenError::NoHandler {
                mime: "image/png".to_owned(),
            },
            OpenError::HandlerMissing {
                mime: "image/png".to_owned(),
                id: "viewer.desktop".to_owned(),
            },
            OpenError::UnusableEntry {
                id: "link.desktop".to_owned(),
                problem: "the entry has no Exec command".to_owned(),
            },
            OpenError::Exec {
                id: "broken.desktop".to_owned(),
                failure: FailureKind::NoCommand,
            },
            OpenError::Spawn {
                program: "/bin/ghost".to_owned(),
                detail: "No such file or directory (os error 2)".to_owned(),
            },
        ];

        let mut sentences: Vec<String> = Vec::new();
        for failure in &failures {
            let sentence = failure.describe(&localizer);
            assert!(!sentence.is_empty(), "{failure:?}");
            // An untranslated message renders as its own id, which has no spaces;
            // a sentence does. Written this way so that the assertion does not
            // itself look like a catalog id to `catalog_tests`.
            assert!(
                sentence.contains(' '),
                "{sentence} is an untranslated message id"
            );
            assert!(!sentences.contains(&sentence), "{sentence} is repeated");
            sentences.push(sentence);
        }
    }
}
