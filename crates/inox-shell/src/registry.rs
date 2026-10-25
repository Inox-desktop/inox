//! The entries a session knows about, and what the session knows about them.
//!
//! PRD §3.1 gives `inox-shell` a "plugin system" as its responsibility. ADR
//! 0013 deliberately defers the general case — a third-party plugin platform
//! needs its own ADR, because loading other people's code raises questions
//! about permissions, lifecycle and compatibility that this crate is not
//! allowed to answer by itself — and keeps the part that is actually in scope:
//! **the internal entry registry**. The entries are the four L3 binaries the
//! coordinator started with (the panel, the launcher, the notification daemon
//! and the settings application), plus the facts the session holds about each.
//!
//! ```text
//! panel inox-panel shell.toml required
//! launcher inox-launcher shell.toml required
//! notifications inox-notifications notifications.toml optional
//! settings inox-settings config.toml optional
//! ```
//!
//! What the registry is *for* is being the one place those facts are validated.
//! A name that no entry has, a name given twice and a configuration path that
//! points outside the configuration directory are all rejected here, at
//! construction, with an error that names the entry and the reason (the configuration model,
//! decision 11; acceptance C01). Everything downstream — starting an entry,
//! reporting it, restarting it after a failure — can then assume the names it
//! holds are real and the paths it holds are inside the directory the session
//! owns.
//!
//! Loading third-party plugins is **out of scope** pending its own ADR. This
//! module has no plugin discovery, no dynamic loading and no path search.

use std::collections::BTreeMap;
use std::path::PathBuf;

use inox_config::{ComponentFiles, ConfigError, ConfigPaths, paths};

use crate::contract::ServiceState;
use crate::diagnostic::Diagnostic;
use crate::i18n::id;
use crate::navigation::EntryId;

/// One entry as the session hands it to the registry.
///
/// Names are strings here, not [`EntryId`]s, because that is what a registry
/// entry, a configuration file or a future IPC request carries: turning one
/// into an [`EntryId`] is exactly the validation
/// [`EntryRegistry::resolve`] performs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySpec {
    /// The entry's name: `panel`, `launcher`, `notifications` or `settings`.
    pub name: String,
    /// The binary that runs it, for example `inox-panel`.
    pub binary: String,
    /// The configuration file it reads, relative to the configuration directory.
    pub config: PathBuf,
    /// Whether the session cannot run without it.
    pub required: bool,
}

impl EntrySpec {
    /// A spec for the entry called `name`.
    pub fn new(
        name: impl Into<String>,
        binary: impl Into<String>,
        config: impl Into<PathBuf>,
        required: bool,
    ) -> Self {
        Self {
            name: name.into(),
            binary: binary.into(),
            config: config.into(),
            required,
        }
    }
}

/// One entry the session knows about.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    id: EntryId,
    binary: String,
    config: PathBuf,
    required: bool,
    state: ServiceState,
}

impl Entry {
    /// Which entry this is.
    pub fn id(&self) -> EntryId {
        self.id
    }

    /// The binary that runs it.
    pub fn binary(&self) -> &str {
        &self.binary
    }

    /// The configuration file it reads, resolved inside the configuration
    /// directory.
    pub fn config(&self) -> &std::path::Path {
        &self.config
    }

    /// Whether the session cannot run without it.
    pub fn is_required(&self) -> bool {
        self.required
    }

    /// What the session knows about its state (acceptance D05).
    pub fn state(&self) -> &ServiceState {
        &self.state
    }
}

/// Why the registry could not be built.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The name is not one of the entries the shell knows.
    #[error("`{name}` is not an entry the Inox shell knows; the known entries are {known}")]
    UnknownEntry {
        /// The rejected name.
        name: String,
        /// The names that would have been accepted.
        known: String,
    },
    /// The same entry was registered twice.
    #[error("entry `{id}` is registered more than once")]
    DuplicateEntry {
        /// The entry given twice.
        id: EntryId,
    },
    /// The entry's configuration file is not inside the configuration directory.
    #[error(
        "entry `{id}` reads `{path}`, which is not a file inside the configuration directory: {source}"
    )]
    InvalidConfigPath {
        /// The entry whose path was rejected.
        id: EntryId,
        /// The rejected path, as it was given.
        path: PathBuf,
        /// Why it was rejected.
        #[source]
        source: ConfigError,
    },
    /// The registry has no such entry, so its state cannot be set.
    #[error("entry `{id}` is not registered")]
    NotRegistered {
        /// The entry that is missing.
        id: EntryId,
    },
}

impl RegistryError {
    /// A user-readable explanation for the coordinator's log.
    pub fn describe(&self) -> Diagnostic {
        match self {
            RegistryError::UnknownEntry { name, known } => {
                Diagnostic::new(id::REGISTRY_UNKNOWN_ENTRY)
                    .with("name", name.clone())
                    .with("known", known.clone())
            }
            RegistryError::DuplicateEntry { id } => {
                Diagnostic::new(id::REGISTRY_DUPLICATE_ENTRY).with("entry", id.as_str())
            }
            RegistryError::InvalidConfigPath { id, path, source } => {
                Diagnostic::new(id::REGISTRY_INVALID_CONFIG_PATH)
                    .with("entry", id.as_str())
                    .with("path", path.display().to_string())
                    .with("detail", source.to_string())
            }
            RegistryError::NotRegistered { id } => {
                Diagnostic::new(id::REGISTRY_NOT_REGISTERED).with("entry", id.as_str())
            }
        }
    }
}

/// The entries a session knows about, with the state the session holds for them.
///
/// The registry does not start anything: it is the resolved description of what
/// the session is supposed to run, which is deliberately separate from running
/// it. It touches no files either — the
/// paths it validates and holds are resolved against the configuration
/// directory, not opened.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryRegistry {
    entries: BTreeMap<EntryId, Entry>,
}

impl EntryRegistry {
    /// Builds a registry from the entries a session knows about.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::UnknownEntry`] for a name no entry has,
    /// [`RegistryError::DuplicateEntry`] when one entry is listed twice, and
    /// [`RegistryError::InvalidConfigPath`] when an entry's configuration file
    /// is empty, absolute or escapes the configuration directory.
    pub fn resolve(paths: &ConfigPaths, specs: &[EntrySpec]) -> Result<Self, RegistryError> {
        let mut entries = BTreeMap::new();

        for spec in specs {
            let Some(id) = EntryId::parse(&spec.name) else {
                return Err(RegistryError::UnknownEntry {
                    name: spec.name.clone(),
                    known: EntryId::names(),
                });
            };
            if entries.contains_key(&id) {
                return Err(RegistryError::DuplicateEntry { id });
            }
            let config =
                paths
                    .file(&spec.config)
                    .map_err(|source| RegistryError::InvalidConfigPath {
                        id,
                        path: spec.config.clone(),
                        source,
                    })?;

            entries.insert(
                id,
                Entry {
                    id,
                    binary: spec.binary.clone(),
                    config,
                    required: spec.required,
                    state: ServiceState::Starting,
                },
            );
        }

        Ok(Self { entries })
    }

    /// The four entries a stock Inox session knows about.
    ///
    /// The configuration files are the ones `config.toml` points at, so a
    /// session that renamed them is described correctly; the settings
    /// application reads `config.toml` itself, because it edits the whole
    /// configuration rather than one component's section.
    ///
    /// # Errors
    ///
    /// Same as [`EntryRegistry::resolve`]; with these specs that is only
    /// [`RegistryError::InvalidConfigPath`] and only when `config.toml` gave a
    /// component path outside the configuration directory.
    pub fn standard(paths: &ConfigPaths, files: &ComponentFiles) -> Result<Self, RegistryError> {
        let specs = [
            EntrySpec::new("panel", "inox-panel", files.shell.clone(), true),
            EntrySpec::new("launcher", "inox-launcher", files.shell.clone(), true),
            EntrySpec::new(
                "notifications",
                "inox-notifications",
                files.notifications.clone(),
                false,
            ),
            EntrySpec::new("settings", "inox-settings", paths::ROOT_FILE, false),
        ];

        Self::resolve(paths, &specs)
    }

    /// The entry called `id`, if the session knows it.
    pub fn get(&self, id: EntryId) -> Option<&Entry> {
        self.entries.get(&id)
    }

    /// Every entry, in the order [`EntryId::ALL`] lists them.
    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    /// How many entries the session knows.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the session knows of no entries at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records what the session now knows about an entry's state (D05).
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::NotRegistered`] for an entry this registry does
    /// not hold.
    pub fn set_state(&mut self, id: EntryId, state: ServiceState) -> Result<(), RegistryError> {
        match self.entries.get_mut(&id) {
            Some(entry) => {
                entry.state = state;
                Ok(())
            }
            None => Err(RegistryError::NotRegistered { id }),
        }
    }
}
