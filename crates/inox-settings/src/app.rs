//! The settings application: state, messages and the Elm update loop.
//!
//! The shape is iced's, and ADR 0003 makes it mandatory: a [`State`], a
//! [`Message`] for everything that can happen, one [`update`] that is the only
//! place state changes, and a [`view()`] that only reads.
//!
//! **`update` performs no I/O.** Writing a file is a side effect, so it leaves
//! the loop as an [`OutgoingCommand`]: what to write, where, and which
//! [`Guard`] says the file has not changed since it was read. `main.rs` performs
//! those commands and hands the outcome back as [`Message::SaveSettled`], which
//! is why every rule below — "a save writes only the fields the user touched",
//! "the window may claim success only after a readback", "a stale answer
//! changes nothing" — is a state transition a test can drive without touching a
//! file, and why the binary needs no second implementation of any of them.
//!
//! The commands correspond one-to-one with what `main.rs` runs:
//!
//! | command | what the runtime does | answer |
//! |---|---|---|
//! | [`OutgoingCommand::Save`] | [`crate::persist::save`] | [`Message::SaveSettled`] |
//! | [`OutgoingCommand::Apply`] | ask the component to apply the generation | [`Message::ComponentApplied`] and friends |
//! | [`OutgoingCommand::Reload`] | read the files and the configuration again | [`Message::Reloaded`] |
//!
//! Nothing in this milestone can carry an [`OutgoingCommand::Apply`]: there is
//! no transport between the settings application and the components (ADR 0018),
//! so the runtime answers [`Message::ComponentUnreachable`] and the window
//! reports that the change is saved but not in effect — which is exactly the
//! distinction ADR 0014 asks it to show.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;

use iced::{Element, Subscription};
use inox_common::i18n::Localizer;
use inox_config::{InoxConfig, Theme as Tokens};
use tracing::debug;

use crate::apply::{Application, ApplyState, Component, Generation, Rejection};
use crate::document::{Documents, Edit, SettingFile};
use crate::draft::{Control, Draft, Field, Settings, Value, ValueKind};
use crate::pages::PageId;
use crate::persist::{Guard, SaveOutcome};
use crate::validate::{is_valid, validate};
use crate::view;
use crate::view_model::{Action, FocusTarget, SettingsInput, SettingsViewModel};

/// A write the runtime has been asked to perform.
///
/// The content is built by [`update`] — patching a document is pure string
/// work — and the guard is the digest of the bytes the window read, which is
/// what makes [`crate::persist::save`] refuse to overwrite a file somebody
/// edited in the meantime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRequest {
    /// The request this write belongs to.
    pub generation: Generation,
    /// Which file it writes.
    pub file: SettingFile,
    /// Where the file is.
    pub path: PathBuf,
    /// What to write.
    pub content: String,
    /// What the file held when it was read.
    pub guard: Guard,
    /// The key names this file's content updates.
    pub fields: Vec<Field>,
}

/// Something the runtime has to do for the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutgoingCommand {
    /// Write a file atomically, unless it changed since it was read.
    Save(Box<SaveRequest>),
    /// Ask one component to apply a generation and report what it finds.
    Apply {
        /// The request.
        generation: Generation,
        /// The component to ask.
        component: Component,
    },
    /// Read the configuration files again.
    Reload,
}

/// What a write left behind, kept until the runtime reports the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingWrite {
    generation: Generation,
    content: String,
}

/// Everything the settings window knows.
///
/// The localizer is shared rather than owned because iced asks for the initial
/// state through a factory it may call more than once, and a catalog cannot be
/// cloned: it is parsed once and read for the life of the process. A plain
/// [`Rc`] is enough — the window is one thread, and `FluentBundle` is neither
/// `Send` nor `Sync`.
pub struct State {
    localizer: Rc<Localizer>,
    tokens: Tokens,
    documents: Documents,
    draft: Draft,
    application: Application,
    page: PageId,
    focused: Option<FocusTarget>,
    pending: BTreeMap<SettingFile, PendingWrite>,
    notified: Option<Generation>,
    outgoing: VecDeque<OutgoingCommand>,
}

impl State {
    /// A window showing the configuration that was loaded.
    pub fn new(
        config: &InoxConfig,
        documents: Documents,
        tokens: Tokens,
        localizer: Rc<Localizer>,
    ) -> Self {
        Self {
            localizer,
            tokens,
            documents,
            draft: Draft::new(Settings::from_config(config)),
            application: Application::new(),
            page: PageId::ALL[0],
            focused: None,
            pending: BTreeMap::new(),
            notified: None,
            outgoing: VecDeque::new(),
        }
    }

    /// The localizer the window renders text with.
    pub fn localizer(&self) -> &Localizer {
        &self.localizer
    }

    /// The theme tokens the window draws with.
    pub fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    /// The draft.
    pub fn draft(&self) -> &Draft {
        &self.draft
    }

    /// How far the last request got.
    pub fn application(&self) -> &Application {
        &self.application
    }

    /// The files as they were read.
    pub fn documents(&self) -> &Documents {
        &self.documents
    }

    /// The open page.
    pub fn page(&self) -> PageId {
        self.page
    }

    /// The entry the keyboard is on.
    pub fn focused(&self) -> Option<FocusTarget> {
        self.focused
    }

    /// Takes the commands waiting for the runtime.
    ///
    /// Whatever is returned is the caller's to perform; an empty queue is the
    /// normal state of a window nobody has asked anything of.
    pub fn take_outgoing(&mut self) -> Vec<OutgoingCommand> {
        self.outgoing.drain(..).collect()
    }

    /// Projects the state into what the window draws.
    pub fn model(&self) -> SettingsViewModel {
        let mut errors = validate(self.draft.settings());
        errors.extend(
            self.draft
                .invalid_numbers()
                .map(|field| crate::validate::FieldError {
                    path: field.path(),
                    message: crate::validate::ValidationMessage::new(
                        "settings-error-integer",
                        vec![],
                    ),
                }),
        );

        let mut model = SettingsInput {
            page: self.page,
            draft: &self.draft,
            application: &self.application,
            errors: &errors,
            focused: self.focused,
        }
        .project(&self.localizer);
        if !self.pending.is_empty() && !self.application.state().is_busy() {
            let reason = self.localizer.message("settings-disabled-in-flight");
            for field in &mut model.fields {
                field.enabled = false;
                field.disabled_reason = Some(reason.clone());
                field.focused = false;
                field.reset_focused = false;
            }
            for action in &mut model.actions {
                action.enabled = false;
                action.focused = false;
                action.disabled_reason = Some(reason.clone());
            }
            model
                .focus_order
                .retain(|target| matches!(target, FocusTarget::Page(_)));
            model.focused = model
                .focused
                .filter(|target| model.focus_order.contains(target));
        }
        model
    }

    fn is_busy(&self) -> bool {
        !self.pending.is_empty() || self.application.state().is_busy()
    }

    /// The values the files hold, as the window last read them.
    pub fn base(&self) -> &Settings {
        self.draft.base()
    }

    /// Moves the window's own focus `delta` entries along.
    ///
    /// Only the entries the window draws are reachable, and the movement wraps,
    /// so a keyboard user can never be left on something that is not on screen
    /// (Q01).
    fn step_focus(&mut self, delta: isize) {
        let order = self.model().focus_order;

        let Some(last) = order.len().checked_sub(1) else {
            self.focused = None;
            return;
        };

        let current = self
            .focused
            .and_then(|target| order.iter().position(|candidate| *candidate == target));

        let next = match current {
            Some(index) => (index as isize + delta).rem_euclid(order.len() as isize) as usize,
            None if delta >= 0 => 0,
            None => last,
        };

        self.focused = Some(order[next]);
    }

    /// Activates whatever the keyboard is on, when the window still offers it.
    ///
    /// The walk is what the window draws as reachable ([`State::model`]), so a
    /// target that has left it is a control the window has just drawn as
    /// unusable — a field while a save is in flight, or an action that has
    /// nothing to do. Enter on one of those does nothing, exactly as a click on
    /// it does.
    fn activate(&mut self) {
        let Some(target) = self.focused else {
            debug!("nothing is focused, so there is nothing to activate");
            return;
        };

        if !self.model().focus_order.contains(&target) {
            debug!(
                ?target,
                "the control is not reachable; nothing was activated"
            );
            return;
        }

        match target {
            FocusTarget::Cell(_, _, _) => {}
            FocusTarget::RemoveRow(field, row) => {
                if let Value::Collection(mut rows) = self.draft.field(field)
                    && row < rows.len()
                {
                    rows.remove(row);
                    if let Err(error) = self.set(field, Value::Collection(rows)) {
                        debug!(%error, "cannot remove entry");
                    }
                    self.focused = Some(FocusTarget::Field(field));
                }
            }
            FocusTarget::Field(field) if field.is_collection() => {
                if let Value::Collection(mut rows) = self.draft.field(field) {
                    let row = rows.len();
                    rows.push(crate::collections::default_row(field));
                    if let Err(error) = self.set(field, Value::Collection(rows)) {
                        debug!(%error, "cannot add entry");
                    }
                    self.focused = Some(FocusTarget::Cell(field, row, 0));
                }
            }
            FocusTarget::Page(page) => self.page = page,
            FocusTarget::Field(field) => self.advance(field),
            FocusTarget::Reset(field) => {
                self.draft.reset_to_default(field);
            }
            FocusTarget::Action(Action::Apply) => self.apply(),
            FocusTarget::Action(Action::Cancel) => self.cancel(),
            FocusTarget::Action(Action::ResetAll) => self.draft.reset_all(),
            FocusTarget::Action(Action::Reload) => self.reload(),
        }
    }

    /// Throws the draft away, unless a save of it is on its way.
    ///
    /// A write the update loop has queued is already being performed by the
    /// runtime, so a Cancel that arrives now would not stop it: the file would
    /// keep the change the user withdrew, and the values the window takes the
    /// files to hold would disagree with what they actually hold. The window
    /// refuses the request instead of writing something the user did not ask for
    /// (D04: 取消不写盘). It cannot be reached from the window — Cancel names
    /// this as its reason and is not drawn as usable — and it is here so that the
    /// rule holds however the message arrives.
    fn cancel(&mut self) {
        if self.is_busy() {
            debug!("a save is in flight; the draft is not thrown away");
            return;
        }

        debug!("the draft was discarded; nothing was written");
        self.draft.discard();
    }

    /// Moves a field to its next value, whichever kind of control it has.
    ///
    /// A choice steps through its options, a switch flips, and a number steps
    /// along its range and wraps at the end — one key, every value, which is
    /// what makes the window usable without a pointer (Q01).
    fn advance(&mut self, field: Field) {
        let control = field.control();
        let value = self.draft.field(field);

        let next = match (value, control.kind) {
            (Value::Choice(current), ValueKind::Choice) => next_choice(&control, &current),
            (Value::Flag(current), ValueKind::Flag) => Some(Value::Flag(!current)),
            (Value::Number(current), ValueKind::Number) => control
                .range
                .map(|range| Value::Number(range.advance(current, 1))),
            (_, _) => None,
        };

        let Some(next) = next else {
            debug!(field = %field, "the field has no value to step to");
            return;
        };

        if let Err(error) = self.set(field, next) {
            debug!(%error, "the window produced a value of the wrong shape");
        }
    }

    /// Changes a field, logging a value of the wrong shape rather than losing it.
    fn set(&mut self, field: Field, value: Value) -> Result<bool, crate::draft::DraftError> {
        let changed = self.draft.set(field, value)?;
        debug!(field = %field, changed, "the draft was changed");

        Ok(changed)
    }

    /// Writes the changed fields and asks the components to apply them.
    fn apply(&mut self) {
        if self.is_busy() {
            return;
        }
        if self.draft.invalid_numbers().next().is_some() || !is_valid(self.draft.settings()) {
            debug!("the draft does not satisfy the schema; nothing was written");
            return;
        }

        let edits = match self.draft.edits() {
            Ok(edits) => edits,
            Err(error) => {
                debug!(%error, "the draft could not be turned into file edits");
                return;
            }
        };

        if edits.is_empty() {
            debug!("nothing has been changed, so there is nothing to write");
            return;
        }

        let files = self.draft.files();
        let targets = Component::for_files(&files);

        let Some(generation) = self.application.begin_save(files.clone(), targets) else {
            debug!("a save is already in flight; the second one was dropped");
            return;
        };

        let mut requests = Vec::new();
        for file in files {
            let Some(document) = self.documents.get(file) else {
                self.application.fail(
                    generation,
                    Rejection::Save {
                        file,
                        reason: format!("{}: document was not loaded", file.name()),
                    },
                );
                return;
            };
            let for_file: Vec<Edit> = edits
                .iter()
                .filter(|edit| edit.field().file() == file)
                .cloned()
                .collect();
            let content = document.patch(&for_file);
            if let Err(reason) = crate::validate::document(file, &content, &for_file) {
                self.application
                    .fail(generation, Rejection::Save { file, reason });
                return;
            }
            requests.push(SaveRequest {
                generation,
                file,
                path: document.path().to_path_buf(),
                content,
                guard: document.guard(),
                fields: for_file.iter().map(Edit::field).collect(),
            });
        }
        for request in requests {
            self.pending.insert(
                request.file,
                PendingWrite {
                    generation,
                    content: request.content.clone(),
                },
            );
            self.outgoing
                .push_back(OutgoingCommand::Save(Box::new(request)));
        }
    }

    /// Asks the runtime to read the configuration again.
    fn reload(&mut self) {
        debug!("asking the runtime to read the configuration again");
        self.outgoing.push_back(OutgoingCommand::Reload);
    }

    /// Asks every component that has not answered to apply the generation.
    ///
    /// Called once the request has moved to [`ApplyState::Pending`], which is
    /// the moment the file is known to hold the new values.
    fn notify(&mut self, generation: Generation) {
        if self.notified == Some(generation) {
            return;
        }

        if !matches!(self.application.state(), ApplyState::Pending { .. }) {
            return;
        }

        self.notified = Some(generation);

        for component in self.application.outstanding() {
            debug!(
                component = component.label_id(),
                generation = generation.get(),
                "asking a component to apply the new configuration"
            );
            self.outgoing.push_back(OutgoingCommand::Apply {
                generation,
                component,
            });
        }
    }

    /// Takes the loaded values as the new baseline for the fields whose file
    /// was just written.
    ///
    /// Only those fields: a file the request failed on still differs from what
    /// the window shows, and its fields stay changes so that a later Apply
    /// carries them (ADR 0014, decision 4: 每组件结果呈现，不承诺全局原子性).
    fn commit(&mut self, file: SettingFile) {
        let mut base = self.draft.base().clone();

        for field in Field::ALL.into_iter().filter(|field| field.file() == file) {
            let value = self.draft.field(field);

            if let Err(error) = field.set(&mut base, value) {
                debug!(%error, "a value could not be carried into the baseline");
            }
        }

        self.draft.rebase(base);
    }

    /// Records what a write left behind.
    fn settle_save(&mut self, generation: Generation, file: SettingFile, outcome: SaveOutcome) {
        if !self
            .pending
            .get(&file)
            .is_some_and(|write| write.generation == generation)
        {
            return;
        }
        let Some(write) = self.pending.remove(&file) else {
            return;
        };
        match outcome {
            SaveOutcome::Saved => {
                self.documents.install(file, write.content);
                self.commit(file);
                if self.application.file_written(generation, file) {
                    self.notify(generation);
                }
            }
            SaveOutcome::Conflicted { current_digest } => {
                self.application.fail(
                    generation,
                    Rejection::Conflict {
                        file,
                        current_digest,
                    },
                );
            }
            SaveOutcome::Failed { reason } => {
                self.application
                    .fail(generation, Rejection::Save { file, reason });
            }
        }
    }
}

/// The next option of a choice control, wrapping round.
///
/// A current value the control does not offer — a language tag written by hand
/// — steps to the first option rather than being left alone.
fn next_choice(control: &Control, current: &str) -> Option<Value> {
    let index = control
        .choices
        .iter()
        .position(|choice| choice.id == current)
        .map_or(0, |index| (index + 1) % control.choices.len());

    control
        .choices
        .get(index)
        .map(|choice| Value::Choice(choice.id.to_owned()))
}

/// Everything the settings window reacts to.
#[derive(Debug, Clone)]
pub enum Message {
    /// Add an entry to a structured list.
    RowAdded(Field),
    /// Remove one structured entry.
    RowRemoved {
        /// The collection field.
        field: Field,
        /// Row index in the current draft.
        row: usize,
    },
    /// Change one cell without reparsing other entries.
    CellEdited {
        /// The collection field.
        field: Field,
        /// Row index in the current draft.
        row: usize,
        /// Column index.
        column: usize,
        /// The exact text entered.
        text: String,
    },
    /// A numeric text input changed, including incomplete input.
    NumberEdited {
        /// The numeric setting being edited.
        field: Field,
        /// Its exact current text.
        text: String,
    },
    /// The user picked a page from the navigation list.
    PageSelected(PageId),
    /// The user activated one of a field's options.
    FieldSet {
        /// The field.
        field: Field,
        /// The value to write.
        value: Value,
    },
    /// The user stepped a number field.
    FieldStepped {
        /// The field.
        field: Field,
        /// How many steps, in either direction.
        delta: i64,
    },
    /// The user asked for one field's default back.
    FieldReset(Field),
    /// The user asked for every field's default back.
    AllReset,
    /// The user pressed Apply.
    Apply,
    /// The user pressed Cancel. Writes nothing at all.
    Cancel,
    /// The user asked for the files to be read again.
    Reload,
    /// The runtime reports what a write left behind.
    SaveSettled {
        /// The request the write belonged to.
        generation: Generation,
        /// The file.
        file: SettingFile,
        /// What happened.
        outcome: SaveOutcome,
    },
    /// A component reports the configuration in effect.
    ComponentApplied {
        /// The component.
        component: Component,
        /// The request it is answering.
        generation: Generation,
    },
    /// A component refused the configuration.
    ComponentRejected {
        /// The component.
        component: Component,
        /// The request it is answering.
        generation: Generation,
        /// Its own words.
        reason: String,
        /// The keys it objects to.
        fields: Vec<Field>,
    },
    /// A component could not be asked at all.
    ComponentUnreachable {
        /// The component.
        component: Component,
        /// The request.
        generation: Generation,
        /// Why not.
        reason: String,
    },
    /// The runtime could not read the configuration again.
    ReloadFailed {
        /// Why not.
        reason: String,
    },
    /// The runtime read the configuration and the files again.
    Reloaded {
        /// The configuration it read.
        config: Box<InoxConfig>,
        /// The files it read.
        documents: Box<Documents>,
    },
    /// The keyboard asked for the next entry.
    FocusNext,
    /// The keyboard asked for the previous entry.
    FocusPrevious,
    /// The keyboard activated the focused entry.
    Activate,
    /// The keyboard asked to unwind a level.
    Back,
}

/// The Elm update: the only place [`State`] changes.
pub fn update(state: &mut State, message: Message) -> iced::Task<Message> {
    if state.is_busy()
        && matches!(
            &message,
            Message::RowAdded(_)
                | Message::RowRemoved { .. }
                | Message::CellEdited { .. }
                | Message::NumberEdited { .. }
                | Message::FieldSet { .. }
                | Message::FieldStepped { .. }
                | Message::FieldReset(_)
                | Message::AllReset
                | Message::Reload
        )
    {
        return iced::Task::none();
    }
    let previous_page = state.page;
    let page_changed = matches!(&message, Message::PageSelected(_));
    let move_focus = matches!(
        &message,
        Message::FocusNext | Message::FocusPrevious | Message::Activate | Message::Back
    );

    match message {
        Message::RowAdded(field) => {
            if let Value::Collection(mut rows) = state.draft.field(field) {
                rows.push(crate::collections::default_row(field));
                if let Err(error) = state.set(field, Value::Collection(rows)) {
                    debug!(%error, "cannot add entry");
                }
            }
        }
        Message::RowRemoved { field, row } => {
            if let Value::Collection(mut rows) = state.draft.field(field)
                && row < rows.len()
            {
                rows.remove(row);
                if let Err(error) = state.set(field, Value::Collection(rows)) {
                    debug!(%error, "cannot remove entry");
                }
            }
        }
        Message::CellEdited {
            field,
            row,
            column,
            text,
        } => {
            if let Value::Collection(mut rows) = state.draft.field(field)
                && let Some(cell) = rows.get_mut(row).and_then(|cells| cells.get_mut(column))
            {
                *cell = text;
                state.focused = Some(FocusTarget::Cell(field, row, column));
                if let Err(error) = state.set(field, Value::Collection(rows)) {
                    debug!(%error, "cannot edit entry");
                }
            }
        }
        Message::NumberEdited { field, text } => {
            state.focused = Some(FocusTarget::Field(field));
            state.draft.edit_number(field, text);
        }
        Message::PageSelected(page) => {
            state.page = page;
            state.focused = None;
        }
        Message::FieldSet { field, value } => {
            if field.control().kind == ValueKind::Text {
                state.focused = Some(FocusTarget::Field(field));
            }
            if let Err(error) = state.set(field, value) {
                debug!(%error, "a value of the wrong shape was dropped");
            }
        }
        Message::FieldStepped { field, delta } => {
            let control = field.control();
            let Some(range) = control.range else {
                debug!(field = %field, "the field is not a number, so it does not step");
                return iced::Task::none();
            };

            let Value::Number(current) = state.draft.field(field) else {
                debug!(field = %field, "the field does not hold a number");
                return iced::Task::none();
            };

            if let Err(error) = state.set(
                field,
                Value::Number(
                    current
                        .saturating_add(range.step.saturating_mul(delta))
                        .clamp(range.min, range.max),
                ),
            ) {
                debug!(%error, "the stepped value was dropped");
            }
        }
        Message::FieldReset(field) => {
            state.draft.reset_to_default(field);
        }
        Message::AllReset => state.draft.reset_all(),
        Message::Apply => state.apply(),
        Message::Cancel => state.cancel(),
        Message::Reload => state.reload(),
        Message::SaveSettled {
            generation,
            file,
            outcome,
        } => state.settle_save(generation, file, outcome),
        Message::ComponentApplied {
            component,
            generation,
        } => {
            state.application.component_applied(component, generation);
        }
        Message::ComponentRejected {
            component,
            generation,
            reason,
            fields,
        } => {
            state
                .application
                .component_rejected(component, generation, reason, fields);
        }
        Message::ComponentUnreachable {
            component,
            generation,
            reason,
        } => {
            state
                .application
                .component_unreachable(component, generation, reason);
        }
        Message::Reloaded { config, documents } => {
            if state.is_busy() {
                return iced::Task::none();
            }
            match crate::i18n::load(&config.locale) {
                Ok(localizer) => state.localizer = Rc::new(localizer),
                Err(error) => {
                    debug!(%error, "cannot refresh locale");
                }
            }
            state.tokens = config.theme.clone();
            state.documents = *documents;
            state.draft.rebase(Settings::from_config(&config));
            state.application.reset();
            state.pending.clear();
            state.notified = None;
            debug!(
                modified = state.draft.is_modified(),
                "the configuration was read again"
            );
        }
        Message::ReloadFailed { reason } => {
            debug!(%reason, "the configuration could not be read again");
            state
                .application
                .fail(state.application.generation(), Rejection::Reload { reason });
        }
        Message::FocusNext => state.step_focus(1),
        Message::FocusPrevious => state.step_focus(-1),
        Message::Activate => state.activate(),
        Message::Back => {
            // Esc unwinds one level of the window: from a control back to the
            // page list, and from the page list to no focus at all. There is no
            // level above this one — closing an ordinary window is the
            // compositor's decision, not the window's (ADR 0013).
            state.focused = match state.focused {
                Some(FocusTarget::Page(_)) | None => None,
                Some(_) => Some(FocusTarget::Page(state.page)),
            };
        }
    }

    if page_changed || state.page != previous_page {
        return iced::widget::operation::snap_to(
            "settings-content",
            iced::widget::operation::RelativeOffset::START,
        );
    }
    if move_focus {
        return crate::focus::reveal(state.focused);
    }
    iced::Task::none()
}

/// Draws the settings window.
pub fn view(state: &State) -> Element<'_, Message> {
    view::settings(state.model(), &state.tokens)
}

/// The title of the window.
pub fn title(state: &State) -> String {
    state.localizer.message("settings-window-title")
}

/// The window's subscription: the keyboard, and nothing else.
///
/// The window performs no I/O of its own, so there is no subscription for a
/// service: what [`update`] queues is performed by the runtime in `main.rs`.
pub fn subscription(_state: &State) -> Subscription<Message> {
    iced::keyboard::listen().filter_map(key_message)
}

/// The message a key press means, if any.
///
/// Only the keys the window itself answers are named; everything else is left
/// for a widget that can use it, which is why the stream this maps sends only
/// events no widget consumed.
pub fn key_message(event: iced::keyboard::Event) -> Option<Message> {
    use iced::keyboard::{Event, Key, key};

    let Event::KeyPressed {
        key,
        modifiers,
        repeat,
        ..
    } = event
    else {
        return None;
    };

    if repeat && !matches!(key, Key::Named(key::Named::ArrowDown | key::Named::ArrowUp)) {
        return None;
    }

    match key.as_ref() {
        Key::Named(key::Named::Tab) if modifiers.shift() => Some(Message::FocusPrevious),
        Key::Named(key::Named::Tab | key::Named::ArrowDown) => Some(Message::FocusNext),
        Key::Named(key::Named::ArrowUp) => Some(Message::FocusPrevious),
        Key::Named(key::Named::Enter) => Some(Message::Activate),
        Key::Named(key::Named::Escape) => Some(Message::Back),
        _ => None,
    }
}

/// Whether the window is holding changes that have not been written.
pub fn is_modified(state: &State) -> bool {
    state.draft.is_modified()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Documents;
    use inox_common::i18n::{Locale, compare_catalogs};
    use inox_config::{Appearance, ConfigPaths};

    /// A window over an empty configuration directory.
    ///
    /// The three files do not exist, which is the first-run case the schema is
    /// designed for, and `Documents` holds them as empty documents a save
    /// creates.
    fn state() -> (tempfile::TempDir, State) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = ConfigPaths::new(directory.path());
        let config = InoxConfig::load(&paths, Appearance::System).expect("the defaults load");

        let documents = Documents::open(&config, &paths).expect("the files are read");
        let localizer =
            Rc::new(crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse"));

        let state = State::new(&config, documents, Tokens::builtin_dark(), localizer);

        (directory, state)
    }

    fn send(state: &mut State, message: Message) {
        let _ = update(state, message);
    }

    /// A window with one change, and the write it queued for it.
    fn changed() -> (tempfile::TempDir, State) {
        let (directory, mut state) = state();
        send(&mut state, Message::PageSelected(PageId::Panel));
        send(
            &mut state,
            Message::FieldSet {
                field: Field::PanelHeight,
                value: Value::Number(48),
            },
        );

        (directory, state)
    }

    fn applied(state: &mut State) -> Vec<OutgoingCommand> {
        send(state, Message::Apply);
        state.take_outgoing()
    }

    /// Performs the writes a save queued, the way `main.rs` does.
    fn write(state: &mut State, commands: &[OutgoingCommand]) -> Generation {
        let mut generation = Generation::default();

        for command in commands {
            if let OutgoingCommand::Save(request) = command {
                generation = request.generation;
                let outcome = crate::persist::save(
                    &request.path,
                    request.content.as_bytes(),
                    Some(&request.guard),
                );
                send(
                    state,
                    Message::SaveSettled {
                        generation: request.generation,
                        file: request.file,
                        outcome,
                    },
                );
            }
        }

        generation
    }

    #[test]
    fn a_new_window_shows_the_loaded_values_and_has_nothing_to_save() {
        let (_directory, state) = state();

        assert_eq!(state.page(), PageId::ALL[0]);
        assert_eq!(state.focused(), None);
        assert!(!state.draft().is_modified());

        let model = state.model();
        assert_eq!(model.status.kind, crate::view_model::StatusKind::Unchanged);
        assert!(!model.action(Action::Apply).expect("Apply").enabled);
    }

    #[test]
    fn a_new_window_has_nothing_for_the_runtime_to_do() {
        let (_directory, mut state) = state();

        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn selecting_a_page_opens_it() {
        let (_directory, mut state) = changed();

        send(&mut state, Message::PageSelected(PageId::Notifications));

        assert_eq!(state.page(), PageId::Notifications);
        assert_eq!(
            state.model().fields.len(),
            PageId::Notifications.fields().len()
        );
    }

    #[test]
    fn a_field_changed_from_a_page_lands_in_the_draft() {
        let (_directory, mut state) = state();

        send(
            &mut state,
            Message::FieldSet {
                field: Field::PanelAutohide,
                value: Value::Flag(true),
            },
        );

        assert_eq!(state.draft().changed(), vec![Field::PanelAutohide]);
        assert!(state.draft().settings().panel_autohide);
    }

    #[test]
    fn a_number_is_stepped_by_its_own_range() {
        let (_directory, mut state) = state();

        send(
            &mut state,
            Message::FieldStepped {
                field: Field::PanelHeight,
                delta: 1,
            },
        );
        send(
            &mut state,
            Message::FieldStepped {
                field: Field::PanelHeight,
                delta: -1,
            },
        );
        send(
            &mut state,
            Message::FieldStepped {
                field: Field::PanelHeight,
                delta: -1,
            },
        );

        // The default is 32; up, down, down leaves it one below where it began.
        assert_eq!(state.draft().settings().panel_height, 31);

        // Pointer stepping clamps at the bounds; keyboard activation cycles separately.
        send(
            &mut state,
            Message::FieldSet {
                field: Field::PanelHeight,
                value: Value::Number(8),
            },
        );
        send(
            &mut state,
            Message::FieldStepped {
                field: Field::PanelHeight,
                delta: -1,
            },
        );
        assert_eq!(state.draft().settings().panel_height, 8);
    }

    #[test]
    fn a_switch_is_activated_by_the_keyboard() {
        let (_directory, mut state) = state();
        send(&mut state, Message::PageSelected(PageId::Panel));

        for _ in 0..PageId::ALL.len() + 1 {
            send(&mut state, Message::FocusNext);
        }

        // The walk is the four page entries, then the open page's fields, so the
        // step after the last page lands on the panel's first field.
        assert_eq!(
            state.focused(),
            Some(FocusTarget::Field(Field::PanelPosition))
        );

        send(&mut state, Message::Activate);
        assert_eq!(
            state.draft().field(Field::PanelPosition),
            Value::Choice("top".to_owned())
        );
    }

    #[test]
    fn the_keyboard_walks_the_window_without_leaving_it() {
        let (_directory, mut state) = state();
        let order = state.model().focus_order;

        // The first step lands on the first entry, so one full lap ends on the
        // last one and the next step comes back round to the start.
        for _ in 0..order.len() {
            send(&mut state, Message::FocusNext);
        }
        assert_eq!(state.focused(), Some(order[order.len() - 1]));

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focused(), Some(order[0]), "the walk wraps round");

        send(&mut state, Message::FocusPrevious);
        assert_eq!(
            state.focused(),
            Some(order[order.len() - 1]),
            "and back the other way"
        );
    }

    #[test]
    fn the_keyboard_can_apply_the_change() {
        let (_directory, mut state) = changed();

        // Walk to the Apply action, which is the last three entries.
        let order = state.model().focus_order;
        let apply = order
            .iter()
            .position(|target| *target == FocusTarget::Action(Action::Apply))
            .expect("Apply is reachable");
        while state.focused() != Some(FocusTarget::Action(Action::Apply)) {
            send(&mut state, Message::FocusNext);
        }
        assert_eq!(order[apply], FocusTarget::Action(Action::Apply));

        let commands = {
            send(&mut state, Message::Activate);
            state.take_outgoing()
        };

        assert_eq!(commands.len(), 1, "the keyboard wrote the file");
        assert!(matches!(commands[0], OutgoingCommand::Save(_)));
    }

    #[test]
    fn the_keyboard_can_cancel_the_change() {
        let (_directory, mut state) = changed();

        while state.focused() != Some(FocusTarget::Action(Action::Cancel)) {
            send(&mut state, Message::FocusNext);
        }

        send(&mut state, Message::Activate);

        assert!(!state.draft().is_modified());
        assert!(state.take_outgoing().is_empty(), "Cancel writes nothing");
    }

    #[test]
    fn esc_unwinds_one_level_and_stops() {
        let (_directory, mut state) = changed();
        send(&mut state, Message::FocusNext);

        assert_eq!(state.focused(), Some(FocusTarget::Page(PageId::Appearance)));

        send(&mut state, Message::Back);
        assert_eq!(
            state.focused(),
            None,
            "the page list is the outermost level"
        );

        for _ in 0..PageId::ALL.len() + 1 {
            send(&mut state, Message::FocusNext);
        }
        assert_eq!(
            state.focused(),
            Some(FocusTarget::Field(Field::PanelPosition)),
            "the panel page's first field"
        );

        send(&mut state, Message::Back);
        assert_eq!(
            state.focused(),
            Some(FocusTarget::Page(PageId::Panel)),
            "Esc comes back to the page the control is on"
        );
    }

    #[test]
    fn cancelling_throws_the_draft_away_and_writes_nothing() {
        let (_directory, mut state) = changed();

        send(&mut state, Message::Cancel);

        assert!(!state.draft().is_modified());
        assert_eq!(state.draft().settings(), state.base());
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn cancelling_while_a_write_is_in_flight_does_not_take_the_change_back() {
        let (directory, mut state) = changed();

        // The write is on its way: the update loop has queued it and the runtime
        // is performing it. A Cancel here would leave the file holding a change
        // the user withdrew, and leave the window taking the file to hold
        // something it does not hold (acceptance D04: 取消不写盘).
        let commands = applied(&mut state);
        assert_eq!(commands.len(), 1, "the shell file was queued");
        assert!(state.application().state().is_busy());

        send(&mut state, Message::Cancel);

        assert!(
            state.draft().is_modified(),
            "the change the write is carrying is still the window's"
        );
        assert_eq!(state.draft().settings().panel_height, 48);
        assert!(state.take_outgoing().is_empty(), "Cancel queues nothing");

        // And what lands is what the window is showing.
        write(&mut state, &commands);

        let written = std::fs::read_to_string(directory.path().join("shell.toml"))
            .expect("the file is written");
        assert_eq!(written, "[panel]\nheight = 48\n");
        assert_eq!(
            state.base().panel_height,
            48,
            "the values the window takes the file to hold are the file's"
        );
        assert_eq!(state.draft().settings().panel_height, 48);
        assert!(!state.draft().is_modified());
    }

    #[test]
    fn the_keyboard_cannot_activate_a_control_the_window_has_disabled() {
        let (_directory, mut state) = changed();

        // The keyboard is on Cancel; the pointer starts the write. Cancel is not
        // on offer any more, so Enter has to do nothing rather than throw away a
        // draft the write is already carrying.
        while state.focused() != Some(FocusTarget::Action(Action::Cancel)) {
            send(&mut state, Message::FocusNext);
        }

        send(&mut state, Message::Apply);
        assert!(state.application().state().is_busy());

        let cancel = state.model().action(Action::Cancel).cloned();
        let cancel = cancel.expect("Cancel is on offer");
        assert!(!cancel.enabled, "the window draws it as unusable");

        send(&mut state, Message::Activate);

        assert!(
            state.draft().is_modified(),
            "the draft survived a keystroke on a control that cannot be used"
        );
        assert_eq!(state.draft().settings().panel_height, 48);
    }

    #[test]
    fn the_keyboard_cannot_change_a_field_while_a_save_is_in_flight() {
        let (_directory, mut state) = changed();

        while state.focused() != Some(FocusTarget::Field(Field::PanelHeight)) {
            send(&mut state, Message::FocusNext);
        }

        send(&mut state, Message::Apply);

        assert!(
            !state
                .model()
                .focus_order
                .contains(&FocusTarget::Field(Field::PanelHeight)),
            "a disabled field is not an entry of the walk"
        );

        send(&mut state, Message::Activate);

        assert_eq!(
            state.draft().settings().panel_height,
            48,
            "the value did not step on a field the window has disabled"
        );
    }

    #[test]
    fn resetting_everything_turns_the_file_back_into_the_schema_defaults() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        std::fs::write(
            directory.path().join("shell.toml"),
            "[panel]\nheight = 64\nautohide = true\n",
        )
        .expect("the file is written");

        let paths = ConfigPaths::new(directory.path());
        let config = InoxConfig::load(&paths, Appearance::System).expect("the file loads");
        let documents = Documents::open(&config, &paths).expect("the files are read");
        let localizer =
            Rc::new(crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse"));
        let mut state = State::new(&config, documents, Tokens::builtin_dark(), localizer);

        assert_eq!(state.draft().settings().panel_height, 64);

        send(&mut state, Message::AllReset);

        assert_eq!(
            state.draft().settings().panel_height,
            32,
            "the schema's default, not the file's value"
        );
        assert_eq!(
            state.draft().changed(),
            vec![Field::PanelHeight, Field::PanelAutohide],
            "only the fields it changed are rewritten"
        );
    }

    #[test]
    fn applying_a_change_queues_one_write_per_changed_file() {
        let (_directory, mut state) = state();
        send(
            &mut state,
            Message::FieldSet {
                field: Field::PanelHeight,
                value: Value::Number(48),
            },
        );
        send(
            &mut state,
            Message::FieldSet {
                field: Field::NotificationsDoNotDisturb,
                value: Value::Flag(true),
            },
        );
        send(
            &mut state,
            Message::FieldSet {
                field: Field::ThemeAppearance,
                value: Value::Choice("light".to_owned()),
            },
        );

        let commands = applied(&mut state);
        let files: Vec<SettingFile> = commands
            .iter()
            .filter_map(|command| match command {
                OutgoingCommand::Save(request) => Some(request.file),
                _ => None,
            })
            .collect();

        assert_eq!(
            files,
            vec![
                SettingFile::Root,
                SettingFile::Shell,
                SettingFile::Notifications
            ],
            "one write per changed file, in the order the files are listed"
        );

        for command in &commands {
            if let OutgoingCommand::Save(request) = command {
                assert_eq!(request.generation, Generation::new(1));
                assert!(!request.fields.is_empty(), "every write names its keys");
                assert!(!request.content.is_empty());
            }
        }
    }

    #[test]
    fn a_write_carries_the_guard_of_the_document_it_was_read_from() {
        let (_directory, mut state) = changed();
        let commands = applied(&mut state);
        let request = commands.iter().find_map(|command| match command {
            OutgoingCommand::Save(request) => Some(request.clone()),
            _ => None,
        });

        let request = request.expect("the shell file is written");
        let document = state
            .documents()
            .get(SettingFile::Shell)
            .expect("the shell document");

        assert_eq!(request.guard, document.guard());
        assert_eq!(request.path, document.path());
        assert_eq!(request.fields, vec![Field::PanelHeight]);
    }

    #[test]
    fn a_second_apply_while_a_write_is_in_flight_is_dropped() {
        let (_directory, mut state) = changed();
        let _ = applied(&mut state);

        send(&mut state, Message::Apply);

        assert!(
            state.take_outgoing().is_empty(),
            "one request at a time, so two writes cannot race for the file"
        );
    }

    #[test]
    fn applying_an_invalid_draft_writes_nothing() {
        let (_directory, mut state) = state();

        // No click can produce this; a draft can hold it, and the validator is
        // what stops it reaching the file.
        let out_of_range = Settings {
            panel_height: 9_000,
            ..Settings::defaults()
        };
        state.draft = Draft::new(out_of_range);

        send(&mut state, Message::Apply);

        assert!(state.take_outgoing().is_empty());
        assert!(!state.application().state().is_saved());
    }

    #[test]
    fn applying_with_nothing_changed_writes_nothing() {
        let (_directory, mut state) = state();

        send(&mut state, Message::Apply);

        assert!(state.take_outgoing().is_empty());
        assert_eq!(state.application().state(), &ApplyState::Draft);
    }

    #[test]
    fn a_successful_write_asks_every_consumer_and_waits() {
        let (_directory, mut state) = changed();
        let commands = applied(&mut state);
        let generation = write(&mut state, &commands);

        assert_eq!(
            state.application().state(),
            &ApplyState::Pending {
                generation,
                outstanding: vec![Component::Shell],
            }
        );

        let asked: Vec<Component> = state
            .take_outgoing()
            .into_iter()
            .filter_map(|command| match command {
                OutgoingCommand::Apply { component, .. } => Some(component),
                _ => None,
            })
            .collect();
        assert_eq!(asked, vec![Component::Shell]);
        assert!(
            !state.draft().is_modified(),
            "the file now holds the change"
        );
    }

    #[test]
    fn the_change_is_in_effect_only_after_the_readback() {
        let (_directory, mut state) = changed();
        let commands = applied(&mut state);
        let generation = write(&mut state, &commands);
        state.take_outgoing();

        send(
            &mut state,
            Message::ComponentApplied {
                component: Component::Shell,
                generation,
            },
        );

        assert_eq!(
            state.application().state(),
            &ApplyState::Applied { generation }
        );
        assert_eq!(
            state.model().status.kind,
            crate::view_model::StatusKind::Applied
        );
    }

    #[test]
    fn a_stale_readback_does_not_take_the_window_back() {
        let (_directory, mut state) = changed();
        let commands = applied(&mut state);
        let generation = write(&mut state, &commands);
        send(
            &mut state,
            Message::ComponentApplied {
                component: Component::Shell,
                generation,
            },
        );

        send(
            &mut state,
            Message::ComponentRejected {
                component: Component::Shell,
                generation: Generation::new(generation.get() + 9),
                reason: "an older request's answer".to_owned(),
                fields: Vec::new(),
            },
        );

        assert_eq!(
            state.application().state(),
            &ApplyState::Applied { generation }
        );
    }

    #[test]
    fn a_write_the_window_could_not_finish_is_reported_and_the_draft_kept() {
        let (_directory, mut state) = changed();
        let commands = applied(&mut state);
        let generation = match &commands[0] {
            OutgoingCommand::Save(request) => request.generation,
            other => panic!("expected a write, got {other:?}"),
        };

        send(
            &mut state,
            Message::SaveSettled {
                generation,
                file: SettingFile::Shell,
                outcome: SaveOutcome::Failed {
                    reason: "read-only file system".to_owned(),
                },
            },
        );

        assert!(matches!(
            state.application().state(),
            ApplyState::Rejected { .. }
        ));
        assert!(
            state.draft().is_modified(),
            "the change is still the user's"
        );
        assert!(state.take_outgoing().is_empty());
    }

    #[test]
    fn an_external_edit_offers_a_reload_and_keeps_the_change() {
        let (_directory, mut state) = changed();
        let commands = applied(&mut state);
        let generation = match &commands[0] {
            OutgoingCommand::Save(request) => request.generation,
            other => panic!("expected a write, got {other:?}"),
        };

        send(
            &mut state,
            Message::SaveSettled {
                generation,
                file: SettingFile::Shell,
                outcome: SaveOutcome::Conflicted {
                    current_digest: crate::persist::Digest::of(b"# edited by hand\n"),
                },
            },
        );

        let model = state.model();
        assert_eq!(model.status.kind, crate::view_model::StatusKind::Rejected);
        assert!(
            model.action(Action::Reload).is_some(),
            "the way out is offered"
        );

        send(&mut state, Message::Reload);
        assert!(matches!(
            state.take_outgoing().as_slice(),
            [OutgoingCommand::Reload]
        ));
    }

    #[test]
    fn a_reload_takes_the_file_but_keeps_what_the_user_changed() {
        let (directory, mut state) = changed();
        let paths = ConfigPaths::new(directory.path());
        let config = InoxConfig::load(&paths, Appearance::System).expect("the defaults load");

        let documents = Documents::open(&config, &paths).expect("the files are read");

        send(
            &mut state,
            Message::Reloaded {
                config: Box::new(config),
                documents: Box::new(documents),
            },
        );

        assert_eq!(
            state.draft().field(Field::PanelHeight),
            Value::Number(48),
            "the user's change survives"
        );
        assert!(state.draft().is_modified());
        assert_eq!(state.application().state(), &ApplyState::Draft);
    }

    #[test]
    fn a_component_that_cannot_be_asked_leaves_the_change_saved() {
        let (_directory, mut state) = changed();
        let commands = applied(&mut state);
        let generation = write(&mut state, &commands);
        state.take_outgoing();

        send(
            &mut state,
            Message::ComponentUnreachable {
                component: Component::Shell,
                generation,
                reason: "no transport is implemented in this milestone".to_owned(),
            },
        );

        assert!(state.application().state().is_saved());
        assert_eq!(
            state.model().status.kind,
            crate::view_model::StatusKind::SavedOnly
        );
    }

    #[test]
    fn a_component_that_refuses_keeps_the_draft_a_change() {
        let (_directory, mut state) = changed();
        let commands = applied(&mut state);
        let generation = write(&mut state, &commands);
        state.take_outgoing();

        send(
            &mut state,
            Message::ComponentRejected {
                component: Component::Shell,
                generation,
                reason: "no output is connected".to_owned(),
                fields: vec![Field::PanelHeight],
            },
        );

        assert!(state.application().state().is_saved());
        assert!(!state.application().state().is_applied());
        assert_eq!(
            state.model().status.kind,
            crate::view_model::StatusKind::Rejected
        );
    }

    #[test]
    fn a_key_the_window_answers_becomes_a_message() {
        use iced::keyboard::{Event, Key, Modifiers, key};

        let pressed = |key: Key, modifiers: Modifiers| Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Code(key::Code::Escape),
            location: iced::keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        };

        assert!(matches!(
            key_message(pressed(Key::Named(key::Named::Tab), Modifiers::default())),
            Some(Message::FocusNext)
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(key::Named::Tab), Modifiers::SHIFT)),
            Some(Message::FocusPrevious)
        ));
        assert!(matches!(
            key_message(pressed(
                Key::Named(key::Named::Escape),
                Modifiers::default()
            )),
            Some(Message::Back)
        ));
        assert!(matches!(
            key_message(pressed(Key::Named(key::Named::Enter), Modifiers::default())),
            Some(Message::Activate)
        ));
        assert!(key_message(pressed(Key::Character("x".into()), Modifiers::default())).is_none());
        assert!(key_message(Event::ModifiersChanged(Modifiers::default())).is_none());
    }

    #[test]
    fn a_key_held_down_does_not_walk_the_window_faster() {
        use iced::keyboard::{Event, Key, Modifiers, key};

        let held = Event::KeyPressed {
            key: Key::Named(key::Named::Tab),
            modified_key: Key::Named(key::Named::Tab),
            physical_key: key::Physical::Code(key::Code::Escape),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::default(),
            text: None,
            repeat: true,
        };

        assert!(key_message(held).is_none());

        let arrow = Event::KeyPressed {
            key: Key::Named(key::Named::ArrowDown),
            modified_key: Key::Named(key::Named::ArrowDown),
            physical_key: key::Physical::Code(key::Code::Escape),
            location: iced::keyboard::Location::Standard,
            modifiers: Modifiers::default(),
            text: None,
            repeat: true,
        };
        assert!(
            matches!(key_message(arrow), Some(Message::FocusNext)),
            "an arrow is meant to repeat"
        );
    }

    #[test]
    fn the_window_is_modified_only_while_it_holds_changes() {
        let (_directory, mut state) = state();
        assert!(!is_modified(&state));

        send(
            &mut state,
            Message::FieldSet {
                field: Field::PanelHeight,
                value: Value::Number(48),
            },
        );
        assert!(is_modified(&state));

        send(&mut state, Message::Cancel);
        assert!(!is_modified(&state));
    }

    #[test]
    fn the_title_and_the_model_agree_on_the_window_name() {
        let (_directory, state) = state();

        assert_eq!(title(&state), state.model().title);
    }

    #[test]
    fn the_catalogs_of_the_window_are_in_step() {
        // A second copy of the check `tests/catalog.rs` makes, for the case
        // where only the unit tests are run.
        let difference =
            compare_catalogs(crate::i18n::EN_US, crate::i18n::ZH_CN).expect("both catalogs parse");

        assert!(difference.is_clean(), "{difference:?}");
    }
}
