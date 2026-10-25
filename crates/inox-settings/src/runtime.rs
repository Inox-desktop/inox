//! The window's runtime: what carries out what the update loop queues.
//!
//! [`crate::app::update`] performs no I/O. It leaves an [`OutgoingCommand`]
//! describing what has to happen
//! — write this file, ask this consumer, read the files again — and this module
//! is the only place those commands become syscalls and D-Bus calls. That is what
//! makes the state machine testable, and it is why there is no second
//! implementation of "write the file" anywhere in the binary.
//!
//! Three pieces replace what the widget toolkit used to provide:
//!
//! | used to be | is now |
//! |---|---|
//! | `iced::Task::perform` | [`Task::perform`] — a closure run on its own thread, its answer sent back as a [`Message`] |
//! | `iced::Subscription` (keyboard) | the QML tree reports the keys it does not consume through [`crate::view::UiAction::Key`] |
//! | `iced::Subscription` (timer) | the trial clock, advanced by [`UiRuntime::pump`] while a trial is open |
//!
//! A [`Task`] never blocks the window: the closure runs on a thread of its own and
//! its answer arrives on the queue [`UiRuntime::pump`] drains, which the QML tree
//! calls from its own timer. Nothing here is async, so the crate needs no
//! executor; the D-Bus calls keep the `zbus::block_on` shape they already had.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use inox_common::i18n::Localizer;
use inox_config::{Appearance, ConfigPaths, InoxConfig};
use tracing::{debug, error, warn};

use crate::app::{self, Message, OutgoingCommand, SaveRequest, State};
use crate::apply::Generation;
use crate::document::Documents;
use crate::draft::Field;
use crate::ui_host::{Applied, Host, Readiness};
use crate::view::{self, KeyName, UiAction, parse_action};

/// One unit of work the runtime has to carry out.
///
/// It is the migration's replacement for the toolkit's task type: the update loop
/// produces none today (its effects leave as [`OutgoingCommand`]s), and the
/// runtime produces [`Task::perform`] for each command it executes.
pub enum Task<T> {
    /// Nothing to do.
    None,
    /// Run `work` on a thread of its own and send its answer to the window.
    Perform(Box<dyn FnOnce() -> T + Send>),
    /// Several tasks.
    Batch(Vec<Task<T>>),
}

impl<T: Send + 'static> Task<T> {
    /// Nothing to do.
    pub fn none() -> Self {
        Self::None
    }

    /// Runs `work` off the window's thread and delivers its answer as a message.
    pub fn perform(work: impl FnOnce() -> T + Send + 'static) -> Self {
        Self::Perform(Box::new(work))
    }

    /// Several tasks, carried out in order.
    pub fn batch(tasks: impl IntoIterator<Item = Self>) -> Self {
        Self::Batch(tasks.into_iter().collect())
    }
}

/// The window's runtime: the state, the side effects, and the queue between them.
pub struct UiRuntime {
    paths: ConfigPaths,
    state: State,
    /// Which fields each in-flight request carries, for the consumers it asks.
    changes: BTreeMap<Generation, BTreeSet<Field>>,
    /// Answers from work that ran off this thread.
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    /// When the trial's countdown should be moved on next.
    tick: Option<Instant>,
    /// Whether the window asked to close.
    quit: bool,
}

impl UiRuntime {
    /// A window showing the configuration that was loaded.
    pub fn new(
        config: &InoxConfig,
        documents: Documents,
        paths: ConfigPaths,
        localizer: Localizer,
    ) -> Self {
        let (sender, receiver) = channel();

        Self {
            paths,
            state: State::new(
                config,
                documents,
                config.theme.clone(),
                std::rc::Rc::new(localizer),
            ),
            changes: BTreeMap::new(),
            sender,
            receiver,
            tick: None,
            quit: false,
        }
    }

    /// The window's state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Whether the window asked to close.
    pub fn quit_requested(&self) -> bool {
        self.quit
    }

    /// The projection, serialised for the QML tree.
    ///
    /// # Panics
    ///
    /// Never in practice: every value the projection holds is a string, a number
    /// or a boolean. A serialisation failure is a programming error, and a window
    /// that cannot describe itself has nothing to draw — it is reported as an
    /// empty object rather than as a panic, because a QObject's property getter
    /// may not abort the process.
    pub fn projection(&self) -> String {
        let projection = view::project(self.state.model(), self.state.tokens());

        match view::to_json(&projection) {
            Ok(json) => json,
            Err(error) => {
                error!(%error, "the window's projection could not be serialised");
                view::empty().to_owned()
            }
        }
    }

    /// Applies one action the QML tree sent.
    pub fn dispatch(&mut self, action: &str) -> Applied {
        let Some(action) = parse_action(action) else {
            warn!(
                action,
                "the QML tree sent an action the window does not know"
            );
            return Applied::Unknown;
        };

        let Some(message) = self.message(action) else {
            debug!("the key belongs to the control that has the text cursor");
            return Applied::Unsupported;
        };

        self.update(message);
        Applied::Changed
    }

    /// Turns one parsed action into a message, or `None` when it is a key the
    /// window leaves to the control that has the text cursor.
    fn message(&self, action: UiAction) -> Option<Message> {
        Some(match action {
            UiAction::PageSelected(page) => Message::PageSelected(page),
            UiAction::FieldSet(field, value) => Message::FieldSet { field, value },
            UiAction::NumberEdited(field, text) => Message::NumberEdited { field, text },
            UiAction::FieldStepped(field, delta) => Message::FieldStepped { field, delta },
            UiAction::FieldReset(field) => Message::FieldReset(field),
            UiAction::CellEdited {
                field,
                row,
                column,
                text,
            } => Message::CellEdited {
                field,
                row,
                column,
                text,
            },
            UiAction::RowAdded(field) => Message::RowAdded(field),
            UiAction::RowRemoved { field, row } => Message::RowRemoved { field, row },
            UiAction::AllReset => Message::AllReset,
            UiAction::Apply => Message::Apply,
            UiAction::Cancel => Message::Cancel,
            UiAction::Reload => Message::Reload,
            UiAction::FocusNext => Message::FocusNext,
            UiAction::FocusPrevious => Message::FocusPrevious,
            UiAction::Activate => Message::Activate,
            UiAction::Back => Message::Back,
            UiAction::TrialConfirm => Message::TrialConfirm,
            UiAction::TrialRevert => Message::TrialRevert,
            UiAction::Key {
                key: KeyName::Other,
                ..
            } => return None,
            UiAction::Key { key, shift, repeat } => {
                return app::key_message(app::KeyPress {
                    name: key,
                    shift,
                    repeat,
                });
            }
        })
    }

    /// Runs one message through the update loop and carries out what it queued.
    pub fn update(&mut self, message: Message) {
        app::update(&mut self.state, message);

        let commands = self.state.take_outgoing();
        let tasks: Vec<Task<Message>> = commands.into_iter().map(|c| self.perform(c)).collect();

        for task in tasks {
            self.execute(task);
        }
    }

    /// Turns one queued command into the work that carries it out.
    fn perform(&mut self, command: OutgoingCommand) -> Task<Message> {
        match command {
            OutgoingCommand::Save(request) => {
                // The state machine serializes saves; keep only the current
                // generation, so a consumer is asked about the fields the
                // request it answers actually carried.
                self.changes
                    .retain(|generation, _| *generation == request.generation);
                self.changes
                    .entry(request.generation)
                    .or_default()
                    .extend(request.fields.iter().copied());
                let SaveRequest {
                    generation,
                    file,
                    path,
                    content,
                    guard,
                    ..
                } = *request;

                Task::perform(move || {
                    let outcome = crate::persist::save(&path, content.as_bytes(), Some(&guard));

                    Message::SaveSettled {
                        generation,
                        file,
                        outcome,
                    }
                })
            }
            OutgoingCommand::Restore(request) => {
                let SaveRequest {
                    generation,
                    file,
                    path,
                    content,
                    guard,
                    ..
                } = *request;

                // The same atomic write as a save, with its own answer: a restore
                // is the end of a display trial, not a request the components
                // have to confirm.
                Task::perform(move || {
                    let outcome = crate::persist::save(&path, content.as_bytes(), Some(&guard));

                    Message::DisplayRestored {
                        generation,
                        file,
                        outcome,
                    }
                })
            }
            OutgoingCommand::Apply {
                generation,
                component,
                trial,
            } => {
                let paths = self.paths.clone();
                let documents = self.state.documents().clone();
                let expected = self.state.draft().settings().clone();
                let fields = self
                    .changes
                    .get(&generation)
                    .or_else(|| self.changes.values().next())
                    .cloned()
                    .unwrap_or_default();

                Task::perform(move || {
                    crate::live::apply(
                        &paths, &documents, &expected, &fields, generation, component, trial,
                    )
                })
            }
            OutgoingCommand::ConfirmDisplay { generation } => {
                Task::perform(move || crate::live::confirm_display(generation))
            }
            OutgoingCommand::RollbackDisplay { generation } => {
                Task::perform(move || crate::live::rollback_display(generation))
            }
            OutgoingCommand::Reload => {
                let paths = self.paths.clone();

                Task::perform(move || {
                    let config = match InoxConfig::load(&paths, Appearance::System) {
                        Ok(config) => config,
                        Err(error) => {
                            return Message::ReloadFailed {
                                reason: error.to_string(),
                            };
                        }
                    };

                    match Documents::open(&config, &paths) {
                        Ok(documents) => Message::Reloaded {
                            config: Box::new(config),
                            documents: Box::new(documents),
                        },
                        Err(error) => {
                            error!(%error, "the configuration files could not be read again");

                            Message::ReloadFailed {
                                reason: error.to_string(),
                            }
                        }
                    }
                })
            }
        }
    }

    /// Carries out one task.
    fn execute(&self, task: Task<Message>) {
        match task {
            Task::None => {}
            Task::Batch(tasks) => {
                for task in tasks {
                    self.execute(task);
                }
            }
            Task::Perform(work) => {
                let sender = self.sender.clone();

                // A thread of its own, so a write or a D-Bus round trip never
                // blocks the thread that renders: the answer arrives on the
                // queue `pump` drains.
                std::thread::spawn(move || {
                    let message = work();

                    if sender.send(message).is_err() {
                        debug!("the window is gone; its answer is dropped");
                    }
                });
            }
        }
    }

    /// Drains the answers that arrived, and moves the trial's clock on.
    ///
    /// The QML tree calls this from its own timer, which is why the display
    /// trial's countdown costs nothing when no trial is open: the tick only
    /// exists while one is.
    pub fn pump(&mut self, now: Instant) -> bool {
        let mut changed = false;

        while let Ok(message) = self.receiver.try_recv() {
            self.update(message);
            changed = true;
        }

        if self.state.trial_open() {
            let due = self.tick.is_none_or(|next| now >= next);

            if due {
                self.update(Message::TrialTick);
                self.tick = Some(now + crate::app::TRIAL_TICK);
                changed = true;
            }
        } else {
            self.tick = None;
        }

        changed
    }
}

impl Host for UiRuntime {
    fn model_json(&mut self) -> String {
        self.projection()
    }

    fn apply(&mut self, action: &str) -> Applied {
        self.dispatch(action)
    }

    fn pump(&mut self) -> bool {
        self.pump(Instant::now())
    }

    fn readiness(&mut self) -> Readiness {
        let model = self.state.model();

        Readiness {
            title: model.title,
            pages: i32::try_from(model.pages.len()).unwrap_or(i32::MAX),
            // The navigation list, one item per field, the actions, and the
            // status line: the items a loaded tree must have before its
            // semantic tree can be called complete (root `AGENTS.md` §7.4).
            accessible_items: i32::try_from(
                model.pages.len() + model.fields.len() + model.actions.len() + 2,
            )
            .unwrap_or(i32::MAX),
        }
    }

    fn quit(&mut self) {
        self.quit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::Field;
    use crate::pages::PageId;
    use inox_common::i18n::Locale;
    use std::time::Duration;

    fn runtime() -> (tempfile::TempDir, UiRuntime) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = ConfigPaths::new(directory.path());
        let config = InoxConfig::load(&paths, Appearance::System).expect("the configuration loads");
        let documents = Documents::open(&config, &paths).expect("the files are read");
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");

        let runtime = UiRuntime::new(&config, documents, paths, localizer);

        (directory, runtime)
    }

    #[test]
    fn the_projection_is_json_the_tree_can_parse() {
        let (_directory, runtime) = runtime();
        let json = runtime.projection();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert!(parsed["title"].is_string());
        assert_eq!(parsed["pages"].as_array().map(Vec::len), Some(10));
        assert!(parsed["theme"]["colors"]["window"].is_string());
    }

    #[test]
    fn an_action_changes_the_window_and_a_stranger_does_not() {
        let (_directory, mut runtime) = runtime();
        let before = runtime.projection();

        assert_eq!(
            runtime.dispatch(&format!(
                "{{\"type\":\"field-set\",\"field\":\"{}\",\"value\":{{\"kind\":\"number\",\"value\":48}}}}",
                Field::PanelHeight.path()
            )),
            Applied::Changed
        );
        assert_ne!(before, runtime.projection(), "the change is projected");

        assert_eq!(
            runtime.dispatch("{\"type\":\"nonsense\"}"),
            Applied::Unknown
        );
        assert_eq!(runtime.dispatch("not json"), Applied::Unknown);
    }

    #[test]
    fn a_key_the_window_owns_moves_the_focus_and_a_letter_does_not() {
        let (_directory, mut runtime) = runtime();
        let before = runtime.projection();

        assert_eq!(
            runtime.dispatch("{\"type\":\"key\",\"key\":\"Tab\"}"),
            Applied::Changed
        );
        assert_ne!(before, runtime.projection());
        assert!(
            runtime.dispatch("{\"type\":\"key\",\"key\":\"x\"}") == Applied::Unsupported,
            "a letter belongs to the control that has the text cursor"
        );
    }

    #[test]
    fn applying_a_change_writes_the_file_and_reports_it() {
        let (directory, mut runtime) = runtime();
        runtime.dispatch(&format!(
            "{{\"type\":\"field-set\",\"field\":\"{}\",\"value\":{{\"kind\":\"number\",\"value\":48}}}}",
            Field::PanelHeight.path()
        ));
        runtime.dispatch("{\"type\":\"apply\"}");

        // The write happens on its own thread; the pump is what collects it.
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            runtime.pump(Instant::now());
            if runtime.state().application().state().is_saved() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        let written = std::fs::read_to_string(directory.path().join("shell.toml"))
            .expect("the file was written");
        assert!(written.contains("height = 48"), "{written}");
        assert!(
            runtime.state().application().state().is_saved(),
            "the status line follows what the write left behind"
        );
    }

    #[test]
    fn the_projection_follows_the_open_page() {
        let (_directory, mut runtime) = runtime();

        runtime.dispatch(&format!(
            "{{\"type\":\"page-selected\",\"page\":\"{}\"}}",
            PageId::Notifications.label_id()
        ));
        let parsed: serde_json::Value =
            serde_json::from_str(&runtime.projection()).expect("valid JSON");

        assert_eq!(parsed["page"]["id"], PageId::Notifications.label_id());
        assert_eq!(parsed["focus"]["page"], PageId::Notifications.label_id());
    }

    #[test]
    fn nothing_happens_while_no_work_is_in_flight() {
        let (_directory, mut runtime) = runtime();

        let height = runtime.state().draft().field(Field::PanelHeight);

        assert!(!runtime.pump(Instant::now()), "an idle window does nothing");
        assert_eq!(
            runtime.state().draft().field(Field::PanelHeight),
            height,
            "and changes nothing"
        );
    }

    #[test]
    fn a_task_runs_off_the_thread_and_answers_through_the_queue() {
        let (_directory, mut runtime) = runtime();
        let sender = runtime.sender.clone();

        let task = Task::perform(move || Message::FocusNext);
        runtime.execute(task);
        drop(sender);

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if runtime.pump(Instant::now()) {
                break;
            }
            assert!(Instant::now() < deadline, "the answer never arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_readiness_contract_counts_what_the_tree_must_have_drawn() {
        let (_directory, mut runtime) = runtime();
        let readiness = runtime.readiness();
        let model = runtime.state().model();

        assert_eq!(readiness.title, model.title);
        assert_eq!(readiness.pages, 10);
        assert!(
            readiness.accessible_items
                >= 10 + i32::try_from(model.fields.len()).expect("a small number"),
            "{readiness:?}"
        );
    }

    #[test]
    fn the_window_can_be_asked_to_close() {
        let (_directory, mut runtime) = runtime();

        assert!(!runtime.quit_requested());
        Host::quit(&mut runtime);
        assert!(runtime.quit_requested());
    }
}
