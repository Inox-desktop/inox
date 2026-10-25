//! The difference between what is saved and what is in effect.
//!
//! the settings save semantics refuses to promise that changing the configuration changes what the
//! session is doing: the file is written by the settings application, and each
//! consumer applies the new values to its own devices on its own time. The
//! decision's fourth point states the consequence for the interface —
//! "设置显示 saved/applied/pending/rejected 的差别" — and its third that a stale
//! answer must never displace a newer request.
//!
//! [`Application`] is that state machine, and nothing in this crate claims
//! success without it:
//!
//! ```text
//!   Draft ──begin_save──▶ Saving { generation }
//!                              │
//!        every file written ───┴──▶ Pending { generation, outstanding }
//!                              │              │
//!        a write failed ───────┘              ├── every consumer confirms ─▶ Applied
//!                                            ├── a consumer refuses ──────▶ Rejected
//!                                            └── a consumer cannot be asked ─▶ SavedOnly
//! ```
//!
//! Two rules are load-bearing and tested rather than asserted:
//!
//! - **One request at a time.** [`Application::begin_save`] returns `None`
//!   while a save is in flight, so two Apply clicks cannot produce two
//!   generations racing for the same file.
//! - **A stale answer changes nothing.** Every answer carries the generation it
//!   belongs to, and one from an older request is dropped — the state a user
//!   sees can only ever come from the request that is current.
//!
//! Per-component results live beside the overall state
//! ([`Application::results`]) because the settings save semantics is explicit that there is no
//! global atomicity across components: the window has to be able to say "the
//! panel took it and the notification daemon refused it" rather than collapsing
//! both into one verdict.

use std::collections::{BTreeMap, BTreeSet};

use crate::document::SettingFile;
use crate::draft::Field;
use crate::persist::Digest;

/// One programme that has to be told about a changed configuration.
///
/// The set is the consumers of the files this application writes, not the L3
/// component list: `config.toml` carries the theme and the language, which
/// every component reads, while `shell.toml` and `notifications.toml` belong to
/// one consumer each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Component {
    /// The compositor: it reads the theme and the language.
    Compositor,
    /// The shell family: the panel, the launcher and the wallpaper.
    Shell,
    /// The notification daemon.
    Notifications,
}

impl Component {
    /// Every consumer, in the order they are listed.
    pub const ALL: [Self; 3] = [Self::Compositor, Self::Shell, Self::Notifications];

    /// The message id of the component's name, for a status line.
    pub const fn label_id(self) -> &'static str {
        match self {
            Self::Compositor => "settings-component-compositor",
            Self::Shell => "settings-component-shell",
            Self::Notifications => "settings-component-notifications",
        }
    }

    /// The components that have to be told about a change to `file`.
    ///
    /// The top-level file carries the theme and the language, and both are
    /// session-wide: every component loaded it, so every component is asked.
    pub fn consumers_of(file: SettingFile) -> &'static [Self] {
        match file {
            SettingFile::Compositor => &[Self::Compositor],
            SettingFile::Root => &Self::ALL,
            SettingFile::Shell => &[Self::Shell],
            SettingFile::Notifications => &[Self::Notifications],
        }
    }

    /// The components that have to be told about a set of changed files.
    pub fn for_files(files: &BTreeSet<SettingFile>) -> BTreeSet<Self> {
        files
            .iter()
            .flat_map(|file| Self::consumers_of(*file).iter().copied())
            .collect()
    }
}

/// Which save, and which round of asking, a state or an answer belongs to.
///
/// A monotonic counter: every save begins a new generation, so an answer that
/// carries an older number cannot be about the request that is current.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(u64);

impl Generation {
    /// The generation with the given number.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The number, for a message or a log line.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The next generation.
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl std::fmt::Display for Generation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// Why a request did not end the way the user asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rejection {
    /// The file could not be written. The reason is the operating system's own
    /// words, shown beside a localized sentence rather than as one.
    Save {
        /// The file that could not be written.
        file: SettingFile,
        /// What went wrong.
        reason: String,
    },
    /// The file changed after it was read, so nothing was written over it.
    Conflict {
        /// The file that changed.
        file: SettingFile,
        /// What the file holds now.
        current_digest: Digest,
    },
    /// A consumer refused the configuration, naming the fields it objects to.
    Component {
        /// The consumer that refused.
        component: Component,
        /// Its own words.
        reason: String,
        /// The keys it objects to, when it names any.
        fields: Vec<Field>,
    },
    /// The configuration could not be read again.
    ///
    /// The window offers the reload as the way out of a conflict, so a reload
    /// that fails has to say so rather than silently leaving the window in the
    /// state it was trying to resolve.
    Reload {
        /// What went wrong.
        reason: String,
    },
    /// A consumer could not be asked at all.
    ///
    /// Kept apart from [`Rejection::Component`] because the two mean different
    /// things to the user: a refusal is a value the component rejected, while
    /// this is Inox being unable to deliver the request — which is the state
    /// the whole L3 milestone is in, because no transport between the settings
    /// application and the components exists yet.
    Unreachable {
        /// The consumer that could not be reached.
        component: Component,
        /// Why not.
        reason: String,
    },
}

/// What one consumer answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComponentState {
    /// The consumer has been asked and has not answered yet.
    Pending,
    /// The consumer reports the configuration in effect.
    Applied,
    /// The consumer refused it.
    Rejected {
        /// Its own words.
        reason: String,
        /// The keys it objects to.
        fields: Vec<Field>,
    },
    /// The consumer could not be asked.
    Unreachable {
        /// Why not.
        reason: String,
    },
}

impl ComponentState {
    /// Whether the consumer has answered.
    pub const fn is_settled(&self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// How far a request has got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyState {
    /// Nothing has been written.
    Draft,
    /// The files are being written.
    Saving {
        /// The request.
        generation: Generation,
    },
    /// The files are written and the consumers are being asked.
    Pending {
        /// The request.
        generation: Generation,
        /// The consumers that have not answered yet.
        outstanding: Vec<Component>,
    },
    /// Every consumer confirmed the request: the change is in effect.
    Applied {
        /// The request.
        generation: Generation,
    },
    /// The files are written, but at least one consumer could not be asked.
    SavedOnly {
        /// The request.
        generation: Generation,
        /// The consumers that could not be reached.
        unreachable: Vec<Component>,
    },
    /// The request did not take effect.
    Rejected {
        /// The request.
        generation: Generation,
        /// Why.
        reason: Rejection,
        /// The keys the failure names, when it names any.
        fields: Vec<Field>,
    },
}

impl ApplyState {
    /// The generation the state belongs to, if it belongs to one.
    pub const fn generation(&self) -> Option<Generation> {
        match self {
            Self::Draft => None,
            Self::Saving { generation }
            | Self::Pending { generation, .. }
            | Self::Applied { generation }
            | Self::SavedOnly { generation, .. }
            | Self::Rejected { generation, .. } => Some(*generation),
        }
    }

    /// Whether a save is on its way.
    pub const fn is_busy(&self) -> bool {
        matches!(self, Self::Saving { .. } | Self::Pending { .. })
    }

    /// Whether the file holds the change.
    ///
    /// True from the moment the last file is written, which is what
    /// [`ApplyState::Pending`] and everything after it mean — a consumer that
    /// has not answered has not un-saved the file.
    pub const fn is_saved(&self) -> bool {
        matches!(
            self,
            Self::Pending { .. }
                | Self::Applied { .. }
                | Self::SavedOnly { .. }
                | Self::Rejected {
                    reason: Rejection::Component { .. } | Rejection::Unreachable { .. },
                    ..
                }
        )
    }

    /// Whether the change is in effect everywhere it had to be.
    pub const fn is_applied(&self) -> bool {
        matches!(self, Self::Applied { .. })
    }

    /// Why the request did not take effect, if it did not.
    pub const fn rejection(&self) -> Option<&Rejection> {
        match self {
            Self::Rejected { reason, .. } => Some(reason),
            _ => None,
        }
    }

    /// The keys the current state is about, when it is about specific ones.
    pub fn fields(&self) -> &[Field] {
        match self {
            Self::Rejected { fields, .. } => fields,
            _ => &[],
        }
    }
}

/// The state machine behind the Apply action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    generation: Generation,
    state: ApplyState,
    files: BTreeSet<SettingFile>,
    written: BTreeSet<SettingFile>,
    targets: BTreeSet<Component>,
    results: BTreeMap<Component, ComponentState>,
}

impl Default for Application {
    fn default() -> Self {
        Self::new()
    }
}

impl Application {
    /// An application with nothing written yet.
    pub fn new() -> Self {
        Self {
            generation: Generation::default(),
            state: ApplyState::Draft,
            files: BTreeSet::new(),
            written: BTreeSet::new(),
            targets: BTreeSet::new(),
            results: BTreeMap::new(),
        }
    }

    /// How far the current request has got.
    pub fn state(&self) -> &ApplyState {
        &self.state
    }

    /// The current request's generation.
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// What each consumer has answered about the current request.
    pub fn results(&self) -> &BTreeMap<Component, ComponentState> {
        &self.results
    }

    /// The consumers that have not answered yet.
    pub fn outstanding(&self) -> Vec<Component> {
        match &self.state {
            ApplyState::Pending { outstanding, .. } => outstanding.clone(),
            _ => Vec::new(),
        }
    }

    /// The files the current request has to write.
    pub fn files(&self) -> &BTreeSet<SettingFile> {
        &self.files
    }

    /// The consumers the current request has to ask.
    pub fn targets(&self) -> &BTreeSet<Component> {
        &self.targets
    }

    /// Whether saved preferences can be retried without writing any files.
    pub fn can_retry_apply(&self) -> bool {
        !self.state.is_busy()
            && !self.files.is_empty()
            && self.written.is_superset(&self.files)
            && self.results.values().any(|result| {
                matches!(
                    result,
                    ComponentState::Rejected { .. } | ComponentState::Unreachable { .. }
                )
            })
    }

    /// Begins a new confirmation round for already saved preferences.
    pub fn retry_apply(&mut self) -> Option<Generation> {
        if !self.can_retry_apply() {
            return None;
        }
        self.generation = self.generation.next();
        self.results = self
            .targets
            .iter()
            .map(|component| (*component, ComponentState::Pending))
            .collect();
        self.state = ApplyState::Pending {
            generation: self.generation,
            outstanding: self.targets.iter().copied().collect(),
        };
        Some(self.generation)
    }

    /// Forgets the current request without forgetting how many there have been.
    ///
    /// Used when the configuration is read again after a failure: the request
    /// that failed is no longer meaningful — its answers are about a file that
    /// is not what this window read — but the generation counter has to keep
    /// counting, or an answer from the old request could match a new one.
    pub fn reset(&mut self) {
        self.state = ApplyState::Draft;
        self.files.clear();
        self.written.clear();
        self.targets.clear();
        self.results.clear();
    }

    /// Begins a save of `files`, to be told to `targets`.
    ///
    /// Returns the new generation, or `None` when a save is already in flight:
    /// one request at a time, so two Apply clicks cannot produce two writes
    /// racing for the same file.
    pub fn begin_save(
        &mut self,
        files: impl IntoIterator<Item = SettingFile>,
        targets: impl IntoIterator<Item = Component>,
    ) -> Option<Generation> {
        if self.state.is_busy() {
            return None;
        }

        self.generation = self.generation.next();
        self.files = files.into_iter().collect();
        self.written.clear();
        self.targets = targets.into_iter().collect();
        self.results = self
            .targets
            .iter()
            .copied()
            .map(|component| (component, ComponentState::Pending))
            .collect();
        self.state = ApplyState::Saving {
            generation: self.generation,
        };

        Some(self.generation)
    }

    /// Records that one file was written.
    ///
    /// Returns whether the answer belonged to the current request. Once every
    /// file is written the request moves to [`ApplyState::Pending`], and
    /// [`Application::outstanding`] is what the caller asks next.
    pub fn file_written(&mut self, generation: Generation, file: SettingFile) -> bool {
        if !self.accepts(generation) {
            return false;
        }

        self.written.insert(file);

        if self.written.is_superset(&self.files) {
            let outstanding: Vec<Component> = self
                .results
                .iter()
                .filter(|(_, state)| !state.is_settled())
                .map(|(component, _)| *component)
                .collect();

            self.state = if outstanding.is_empty() {
                ApplyState::Applied { generation }
            } else {
                ApplyState::Pending {
                    generation,
                    outstanding,
                }
            };
        }

        true
    }

    /// Records that the request did not end the way the user asked.
    ///
    /// Returns whether the answer belonged to the current request. A rejection
    /// that names a consumer is only meaningful for a consumer this request is
    /// waiting for: one it never asked cannot refuse it, and one that already
    /// answered cannot refuse it twice.
    pub fn fail(&mut self, generation: Generation, reason: Rejection) -> bool {
        if !self.accepts_failure(generation) {
            return false;
        }

        let named = match &reason {
            Rejection::Component {
                component,
                reason,
                fields,
            } => Some((
                *component,
                ComponentState::Rejected {
                    reason: reason.clone(),
                    fields: fields.clone(),
                },
            )),
            Rejection::Unreachable { component, reason } => Some((
                *component,
                ComponentState::Unreachable {
                    reason: reason.clone(),
                },
            )),
            Rejection::Save { .. } | Rejection::Conflict { .. } | Rejection::Reload { .. } => None,
        };

        if let Some((component, state)) = named {
            if !self.settle(component) {
                return false;
            }

            self.results.insert(component, state);
            self.finish(generation);
            return true;
        }

        let fields = match &reason {
            Rejection::Component { fields, .. } => fields.clone(),
            _ => Vec::new(),
        };

        self.state = ApplyState::Rejected {
            generation,
            reason,
            fields,
        };

        true
    }

    /// Records that one consumer reports the configuration in effect.
    ///
    /// Returns whether the answer belonged to the current request. An answer
    /// from an older one is dropped: the state a user reads can only come from
    /// the request that is current.
    pub fn component_applied(&mut self, component: Component, generation: Generation) -> bool {
        if !self.accepts(generation) || !self.settle(component) {
            return false;
        }

        self.results.insert(component, ComponentState::Applied);
        self.finish(generation);

        true
    }

    /// Records that one consumer refused the configuration.
    ///
    /// A refusal is a rejection of the request, so it goes through
    /// [`Application::fail`]: the file stays written, and the window says both
    /// things.
    pub fn component_rejected(
        &mut self,
        component: Component,
        generation: Generation,
        reason: String,
        fields: Vec<Field>,
    ) -> bool {
        self.fail(
            generation,
            Rejection::Component {
                component,
                reason,
                fields,
            },
        )
    }

    /// Records that one consumer could not be asked.
    ///
    /// The file is still written, so the request does not become a rejection:
    /// it becomes [`ApplyState::SavedOnly`], which is exactly the difference
    /// the settings save semantics asks the window to show.
    pub fn component_unreachable(
        &mut self,
        component: Component,
        generation: Generation,
        reason: String,
    ) -> bool {
        if !self.accepts(generation) || !self.settle(component) {
            return false;
        }

        self.results.insert(
            component,
            ComponentState::Unreachable {
                reason: reason.clone(),
            },
        );
        self.finish(generation);

        true
    }

    /// Whether an answer belongs to a request that is still running.
    ///
    /// Once the request has settled — applied, saved only, or rejected — a
    /// later answer is about a state the user is no longer in, and is dropped.
    fn accepts(&self, generation: Generation) -> bool {
        self.accepts_failure(generation) && self.state.is_busy()
    }

    /// Whether a failure belongs to the current request.
    ///
    /// A failure may land on a request that is still running *or* refine one
    /// that already failed: a conflict a user tried to resolve with a reload
    /// that then failed turns into the reload's own failure.
    fn accepts_failure(&self, generation: Generation) -> bool {
        generation == self.generation && self.state.generation() == Some(generation)
    }

    /// Whether `component` is one the current request is waiting for.
    fn settle(&mut self, component: Component) -> bool {
        matches!(self.results.get(&component), Some(ComponentState::Pending))
    }

    /// Moves the request on now that no consumer is outstanding.
    fn finish(&mut self, generation: Generation) {
        let outstanding: Vec<Component> = self
            .results
            .iter()
            .filter(|(_, state)| !state.is_settled())
            .map(|(component, _)| *component)
            .collect();

        self.state = if !outstanding.is_empty() {
            ApplyState::Pending {
                generation,
                outstanding,
            }
        } else {
            let unreachable: Vec<Component> = self
                .results
                .iter()
                .filter(|(_, state)| matches!(state, ComponentState::Unreachable { .. }))
                .map(|(component, _)| *component)
                .collect();

            if let Some((component, ComponentState::Rejected { reason, fields })) = self
                .results
                .iter()
                .find(|(_, result)| matches!(result, ComponentState::Rejected { .. }))
            {
                ApplyState::Rejected {
                    generation,
                    reason: Rejection::Component {
                        component: *component,
                        reason: reason.clone(),
                        fields: fields.clone(),
                    },
                    fields: fields.clone(),
                }
            } else if unreachable.is_empty() {
                ApplyState::Applied { generation }
            } else {
                ApplyState::SavedOnly {
                    generation,
                    unreachable,
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_only() -> BTreeSet<SettingFile> {
        BTreeSet::from([SettingFile::Shell])
    }

    fn started() -> (Application, Generation) {
        let mut application = Application::new();
        let files = shell_only();
        let targets = Component::for_files(&files);
        let generation = application
            .begin_save(files, targets)
            .expect("nothing is in flight");

        (application, generation)
    }

    fn saved() -> (Application, Generation) {
        let (mut application, generation) = started();
        assert!(application.file_written(generation, SettingFile::Shell));

        (application, generation)
    }

    #[test]
    fn a_fresh_application_has_saved_nothing() {
        let application = Application::new();

        assert_eq!(application.state(), &ApplyState::Draft);
        assert!(!application.state().is_busy());
        assert!(!application.state().is_saved());
        assert!(!application.state().is_applied());
        assert_eq!(application.state().generation(), None);
    }

    #[test]
    fn a_second_save_cannot_start_while_one_is_in_flight() {
        let (mut application, generation) = started();
        let files = shell_only();

        assert_eq!(
            application.begin_save(files.clone(), Component::for_files(&files)),
            None,
            "the first request is still being written"
        );
        assert_eq!(application.generation(), generation);

        application.file_written(generation, SettingFile::Shell);

        assert!(
            application
                .begin_save(files.clone(), Component::for_files(&files))
                .is_none(),
            "the consumers have not answered yet"
        );

        application.component_applied(Component::Shell, generation);
        assert!(
            application
                .begin_save(files.clone(), Component::for_files(&files))
                .is_some(),
            "the previous request is finished"
        );
    }

    #[test]
    fn every_generation_is_a_new_number() {
        let mut application = Application::new();
        let files = shell_only();

        let first = application
            .begin_save(files.clone(), Component::for_files(&files))
            .expect("nothing is in flight");
        application.file_written(first, SettingFile::Shell);
        application.component_applied(Component::Shell, first);

        let second = application
            .begin_save(files.clone(), Component::for_files(&files))
            .expect("nothing is in flight");

        assert!(second > first, "{second} is not after {first}");
    }

    #[test]
    fn the_request_moves_on_only_when_every_file_is_written() {
        let mut application = Application::new();
        let files = BTreeSet::from([SettingFile::Shell, SettingFile::Notifications]);
        let targets = Component::for_files(&files);
        let generation = application
            .begin_save(files, targets)
            .expect("nothing is in flight");

        assert!(matches!(application.state(), ApplyState::Saving { .. }));
        assert!(!application.state().is_saved());

        application.file_written(generation, SettingFile::Shell);
        assert!(
            matches!(application.state(), ApplyState::Saving { .. }),
            "one file is still to be written"
        );

        application.file_written(generation, SettingFile::Notifications);
        assert!(matches!(application.state(), ApplyState::Pending { .. }));
        assert!(application.state().is_saved());
        assert!(!application.state().is_applied());
    }

    #[test]
    fn the_consumers_are_asked_once_every_file_is_written() {
        let (application, _) = saved();

        assert_eq!(application.outstanding(), vec![Component::Shell]);
        assert_eq!(
            application.results().get(&Component::Shell),
            Some(&ComponentState::Pending)
        );
    }

    #[test]
    fn a_change_to_the_top_level_file_asks_every_component() {
        let files = BTreeSet::from([SettingFile::Root]);

        assert_eq!(
            Component::for_files(&files),
            BTreeSet::from([
                Component::Compositor,
                Component::Shell,
                Component::Notifications
            ])
        );
    }

    #[test]
    fn each_component_file_asks_its_own_consumer() {
        assert_eq!(
            Component::for_files(&BTreeSet::from([SettingFile::Shell])),
            BTreeSet::from([Component::Shell])
        );
        assert_eq!(
            Component::for_files(&BTreeSet::from([SettingFile::Notifications])),
            BTreeSet::from([Component::Notifications])
        );
    }

    #[test]
    fn the_change_is_in_effect_only_once_every_consumer_has_confirmed_it() {
        let (mut application, generation) = saved();

        assert!(application.component_applied(Component::Shell, generation));

        assert_eq!(application.state(), &ApplyState::Applied { generation });
        assert!(application.state().is_saved());
        assert!(application.state().is_applied());
        assert_eq!(application.outstanding(), Vec::new());
    }

    #[test]
    fn a_stale_confirmation_changes_nothing() {
        let (mut application, generation) = saved();
        let stale = Generation::new(generation.get() + 41);

        assert!(
            !application.component_applied(Component::Shell, stale),
            "an older request"
        );
        assert!(
            !application.component_applied(Component::Shell, Generation::default()),
            "the request before the first one"
        );
        assert!(matches!(application.state(), ApplyState::Pending { .. }));
        assert_eq!(application.outstanding(), vec![Component::Shell]);
    }

    #[test]
    fn a_stale_refusal_changes_nothing() {
        let (mut application, generation) = saved();

        assert!(!application.fail(
            Generation::new(generation.get() + 7),
            Rejection::Save {
                file: SettingFile::Shell,
                reason: "no space left on device".to_owned(),
            }
        ));
        assert!(matches!(application.state(), ApplyState::Pending { .. }));
        assert_eq!(application.state().rejection(), None);
    }

    #[test]
    fn a_consumer_that_answers_twice_is_heard_once() {
        let (mut application, generation) = saved();

        assert!(application.component_applied(Component::Shell, generation));
        assert!(
            !application.component_applied(Component::Shell, generation),
            "the consumer was already settled"
        );
        assert_eq!(application.state(), &ApplyState::Applied { generation });
    }

    #[test]
    fn a_consumer_that_was_never_asked_changes_nothing() {
        let (mut application, generation) = started();

        assert!(
            !application.component_applied(Component::Compositor, generation),
            "a consumer this request does not target"
        );
        assert!(!application.fail(
            generation,
            Rejection::Component {
                component: Component::Compositor,
                reason: "not my file".to_owned(),
                fields: Vec::new(),
            }
        ));
        assert!(matches!(application.state(), ApplyState::Saving { .. }));
    }

    #[test]
    fn a_write_that_fails_leaves_the_request_rejected() {
        let (mut application, generation) = started();

        assert!(application.fail(
            generation,
            Rejection::Save {
                file: SettingFile::Shell,
                reason: "read-only file system".to_owned(),
            }
        ));

        assert!(!application.state().is_saved(), "nothing was written");
        match application.state() {
            ApplyState::Rejected { reason, .. } => match reason {
                Rejection::Save { file, reason } => {
                    assert_eq!(*file, SettingFile::Shell);
                    assert!(reason.contains("read-only"));
                }
                other => panic!("expected a save failure, got {other:?}"),
            },
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[test]
    fn an_external_edit_leaves_the_request_rejected_with_the_digest_that_was_found() {
        let (mut application, generation) = started();
        let digest = Digest::of(b"# edited by hand\n");

        assert!(application.fail(
            generation,
            Rejection::Conflict {
                file: SettingFile::Shell,
                current_digest: digest,
            }
        ));

        assert_eq!(
            application.state().rejection(),
            Some(&Rejection::Conflict {
                file: SettingFile::Shell,
                current_digest: digest,
            })
        );
        assert!(!application.state().is_saved());
    }

    #[test]
    fn a_consumer_that_refuses_names_itself_and_the_fields() {
        let (mut application, generation) = saved();

        assert!(application.component_rejected(
            Component::Shell,
            generation,
            "the compositor has no such output".to_owned(),
            vec![Field::PanelHeight],
        ));

        assert!(application.state().is_saved(), "the file was written");
        assert!(!application.state().is_applied());
        assert_eq!(application.state().fields(), &[Field::PanelHeight]);
        assert_eq!(
            application.results().get(&Component::Shell),
            Some(&ComponentState::Rejected {
                reason: "the compositor has no such output".to_owned(),
                fields: vec![Field::PanelHeight],
            })
        );
    }

    #[test]
    fn a_consumer_that_cannot_be_reached_leaves_the_change_saved_but_not_in_effect() {
        let (mut application, generation) = saved();

        assert!(application.component_unreachable(
            Component::Shell,
            generation,
            "no transport to the shell is implemented in this milestone".to_owned(),
        ));

        assert!(application.state().is_saved());
        assert!(!application.state().is_applied());
        assert_eq!(
            application.state(),
            &ApplyState::SavedOnly {
                generation,
                unreachable: vec![Component::Shell],
            }
        );
    }

    #[test]
    fn one_consumer_answering_does_not_speak_for_another() {
        let mut application = Application::new();
        let files = BTreeSet::from([SettingFile::Root]);
        let targets = Component::for_files(&files);
        let generation = application
            .begin_save(files, targets)
            .expect("nothing is in flight");
        application.file_written(generation, SettingFile::Root);

        assert!(application.component_applied(Component::Compositor, generation));

        assert_eq!(
            application.outstanding(),
            vec![Component::Shell, Component::Notifications]
        );
        assert!(matches!(application.state(), ApplyState::Pending { .. }));

        assert!(application.component_applied(Component::Shell, generation));
        assert!(matches!(application.state(), ApplyState::Pending { .. }));

        assert!(application.component_applied(Component::Notifications, generation));
        assert_eq!(application.state(), &ApplyState::Applied { generation });
    }

    #[test]
    fn a_refusal_waits_for_other_consumers_and_preserves_all_results() {
        let mut application = Application::new();
        let files = BTreeSet::from([SettingFile::Root]);
        let targets = Component::for_files(&files);
        let generation = application
            .begin_save(files, targets)
            .expect("nothing is in flight");
        application.file_written(generation, SettingFile::Root);

        assert!(application.component_rejected(
            Component::Notifications,
            generation,
            "the daemon is not running".to_owned(),
            Vec::new(),
        ));
        assert!(application.state().is_busy());
        assert!(application.component_applied(Component::Compositor, generation));
        assert!(application.component_unreachable(
            Component::Shell,
            generation,
            "not running".into()
        ));
        assert!(matches!(application.state(), ApplyState::Rejected { .. }));
        assert_eq!(
            application.results().get(&Component::Compositor),
            Some(&ComponentState::Applied)
        );
        let retry = application
            .retry_apply()
            .expect("saved preferences are retryable");
        assert!(retry > generation);
        assert!(!application.component_applied(Component::Shell, generation));
        for component in Component::ALL {
            assert!(application.component_applied(component, retry));
        }
        assert_eq!(
            application.state(),
            &ApplyState::Applied { generation: retry }
        );
        assert!(!application.can_retry_apply());
    }

    #[test]
    fn a_rejected_request_can_be_asked_for_again() {
        let (mut application, generation) = started();
        application.fail(
            generation,
            Rejection::Save {
                file: SettingFile::Shell,
                reason: "read-only file system".to_owned(),
            },
        );

        let files = shell_only();
        let next = application
            .begin_save(files.clone(), Component::for_files(&files))
            .expect("a rejected request is not in flight");

        assert!(next > generation);
        assert_eq!(
            application.state(),
            &ApplyState::Saving { generation: next }
        );
    }

    #[test]
    fn a_confirmed_request_can_be_asked_for_again() {
        let (mut application, generation) = saved();
        application.component_applied(Component::Shell, generation);

        let files = shell_only();
        let next = application
            .begin_save(files.clone(), Component::for_files(&files))
            .expect("a finished request is not in flight");

        assert!(next > generation);
    }

    #[test]
    fn a_component_is_named_in_the_users_language() {
        for component in Component::ALL {
            assert!(
                component.label_id().starts_with("settings-component-"),
                "{component:?}"
            );
        }
    }

    #[test]
    fn resetting_forgets_the_request_but_not_the_numbering() {
        let (mut application, generation) = started();
        application.file_written(generation, SettingFile::Shell);

        application.reset();

        assert_eq!(application.state(), &ApplyState::Draft);
        assert!(application.files().is_empty());
        assert!(application.targets().is_empty());
        assert!(application.results().is_empty());
        assert_eq!(application.outstanding(), Vec::new());

        let files = shell_only();
        let next = application
            .begin_save(files.clone(), Component::for_files(&files))
            .expect("nothing is in flight");

        assert!(
            next > generation,
            "an old answer can never match a new request"
        );
    }

    #[test]
    fn a_reload_that_failed_can_be_reported_against_the_request() {
        let (mut application, generation) = started();

        assert!(application.fail(
            generation,
            Rejection::Reload {
                reason: "cannot parse `/cfg/shell.toml`: expected `=`".to_owned(),
            }
        ));

        match application.state().rejection() {
            Some(Rejection::Reload { reason }) => assert!(reason.contains("expected")),
            other => panic!("expected a reload failure, got {other:?}"),
        }
    }

    #[test]
    fn a_generation_is_printed_as_its_number() {
        assert_eq!(Generation::new(7).to_string(), "7");
        assert_eq!(Generation::default(), Generation::new(0));
    }
}
