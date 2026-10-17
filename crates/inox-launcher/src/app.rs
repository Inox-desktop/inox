//! The Elm loop: state, messages, and `update`.
//!
//! The shape is the one ADR 0003 fixes for every shell component and ADR 0013
//! repeats for interactions: a message goes in, the state changes, and the
//! interesting work comes back as a task. Nothing here draws, and nothing here
//! blocks: the two slow things a launcher does — reading the desktop files and
//! waiting for a launched process — both happen on other threads and come back
//! as messages, which is what keeps `update` a pure state transition.
//!
//! The launch decision itself is the part with the requirements in it. An
//! activation is keyed by [`State::selection_generation`], which changes
//! whenever the selection could have changed, so pressing Enter repeatedly
//! before the first press has a result produces exactly one launch (acceptance
//! D02); and the result of that launch is `Requested` until evidence arrives
//! (`crate::activation`), so the launcher never claims a success it has not
//! seen.

use std::rc::Rc;

use iced::Task;
use iced::futures::StreamExt;
use iced::keyboard::Key;
use iced::keyboard::key::Named;
use inox_common::i18n::Localizer;
use inox_config::{LauncherConfig, Theme as Tokens};
use tracing::{debug, info, warn};

use crate::activation::{
    self, Activation, Activator, LaunchOutcome, LaunchRequest, Observation, READINESS_GRACE,
};
use crate::contract::{Failure, FailureKind, ServiceState};
use crate::index::{self, AppIndex, IndexOptions};
use crate::placement::Placement;
use crate::search::{self, Hit, SearchEngine};
use crate::view_model::{self, Projection, ViewModel};

/// The widget id the search field is focused under.
///
/// Deliberately not shaped like a Fluent message id: `tests/catalog.rs` reads
/// every `launcher-…` literal in the sources as a message the code asks for, and
/// an interface identifier is not one.
pub const SEARCH_INPUT_ID: &str = "inox-launcher-search-field";

/// Which way the selection moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Towards the first row.
    Previous,
    /// Towards the last row.
    Next,
}

/// Everything `main` resolves before the window exists.
///
/// Boot runs once per window and has to be a plain function, so the resolved
/// configuration is handed over by reference and the catalogs are shared: a
/// `Localizer` is neither cloneable nor shareable across threads, and rebuilding
/// it per window would re-parse catalogs that were already validated at startup.
/// An `Rc` is the right handle here — the launcher's state lives on the UI thread
/// and never crosses one.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The theme tokens every view reads.
    pub tokens: Tokens,
    /// The catalogs.
    pub localizer: Rc<Localizer>,
    /// `shell.toml`'s `[launcher]`.
    pub config: LauncherConfig,
    /// Where the overlay belongs, once a layer-shell host exists.
    pub placement: Placement,
}

/// The launcher's state.
#[derive(Debug)]
pub struct State {
    /// The theme tokens.
    pub tokens: Tokens,
    /// The catalogs.
    pub localizer: Rc<Localizer>,
    /// `shell.toml`'s `[launcher]`.
    pub config: LauncherConfig,
    /// Where the overlay belongs.
    pub placement: Placement,
    /// The application index, when it could be read.
    pub index: Option<AppIndex>,
    /// What the index service reports about itself.
    pub service: ServiceState,
    /// The newest scan generation.
    pub scan_generation: u64,
    /// The query and its results.
    pub engine: SearchEngine,
    /// The text in the search field.
    pub query: String,
    /// Which row is selected.
    pub selected: usize,
    /// The generation of the current selection.
    pub selection_generation: u64,
    /// Which selections have been activated.
    pub activator: Activator,
    /// The launch being reported on.
    pub activation: Option<Activation>,
    /// Whether the launcher has been asked to close.
    pub close_requested: bool,
    /// What the view draws.
    ///
    /// Projected after every message rather than on every frame: the projection
    /// allocates the labels it localises, and a frame is drawn far more often
    /// than the state changes.
    pub model: ViewModel,
}

impl State {
    /// The initial state, with the index already read.
    ///
    /// The first scan happens here rather than in a task: it runs before the
    /// window exists, so it blocks nothing, and the launcher's first frame is
    /// therefore never an empty list that fills in a moment later.
    pub fn new(prepared: &Prepared) -> Self {
        let mut state = Self::blank(prepared);
        state.read_index();
        state.research();
        state.refresh_model();
        state
    }

    /// The state before the index is read, which `main` never sees.
    fn blank(prepared: &Prepared) -> Self {
        Self {
            tokens: prepared.tokens.clone(),
            localizer: Rc::clone(&prepared.localizer),
            config: prepared.config.clone(),
            placement: prepared.placement.clone(),
            index: None,
            service: ServiceState::Loading { generation: 1 },
            scan_generation: 1,
            engine: SearchEngine::new(),
            query: String::new(),
            selected: 0,
            selection_generation: 1,
            activator: Activator::new(),
            activation: None,
            close_requested: false,
            model: empty_model(),
        }
    }

    /// A state over an index the caller supplies.
    ///
    /// [`State::new`] reads the XDG directories; this is the same state for a
    /// caller that already has an index — the render tests in `tests/render.rs`,
    /// which need a fixed list of applications to compare pixels against, and
    /// any future preview or embedding of the launcher.
    pub fn with_index(prepared: &Prepared, index: AppIndex, generation: u64) -> Self {
        let mut state = Self::blank(prepared);
        state.scan_generation = generation;
        state.service = index.state();
        state.index = Some(index);
        state.research();
        state.refresh_model();
        state
    }

    /// Rebuilds what the view draws from the current state.
    ///
    /// Called after every message that can change what is on screen, which is
    /// all of them; the projection is cheap enough that tracking which ones
    /// actually matter would cost more to maintain than it saves.
    pub fn refresh_model(&mut self) {
        let model = {
            let projection = Projection {
                localizer: &self.localizer,
                config: &self.config,
                index: self.index.as_ref(),
                service: &self.service,
                result: self.engine.current(),
                query: &self.query,
                selected: self.selected,
                activation: self.activation.as_ref(),
                label_budget: self.placement.label_chars,
            };
            view_model::project(&projection)
        };
        self.model = model;
    }

    /// Whether the launcher has been asked to close, for the caller that owns
    /// the cross-entry unwinding (ADR 0013: Esc closes one level at a time, and
    /// what is behind the launcher is `inox-shell`'s to restore).
    pub fn close_requested(&self) -> bool {
        self.close_requested
    }

    /// Reads the index synchronously, for startup.
    fn read_index(&mut self) {
        let locale = self.localizer.locale().clone();
        let generation = self.scan_generation;

        match IndexOptions::from_env(&locale, self.config.include_hidden) {
            Ok(options) => {
                let index = index::scan(&options, generation);
                self.service = index.state();
                self.index = Some(index);
            }
            Err(error) => {
                warn!(%error, "the application index could not be located");
                self.index = None;
                self.service = ServiceState::Unavailable {
                    generation,
                    failure: unavailable(&error.to_string()),
                };
            }
        }
    }

    /// Runs the current query and applies the result.
    fn research(&mut self) {
        let Some(index) = self.index.as_ref() else {
            return;
        };

        let request = self.engine.request(
            &self.query,
            usize::from(self.config.max_query_chars),
            usize::from(self.config.max_results),
        );
        let result = search::execute(index, &request);
        self.engine.apply(result);

        let rows = self
            .engine
            .current()
            .map(|result| result.hits.len())
            .unwrap_or(0);
        self.selected = self.selected.min(rows.saturating_sub(1));
    }

    /// The hit the selection points at.
    fn selected_hit(&self) -> Option<&Hit> {
        self.engine
            .current()
            .and_then(|result| result.hits.get(self.selected))
    }
}

/// A message the launcher acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The search field's text changed.
    QueryChanged(String),
    /// The clear button was pressed.
    QueryCleared,
    /// The selection moved.
    SelectionMoved(Step),
    /// A row was chosen with the pointer.
    RowChosen(String),
    /// Enter was pressed.
    Activated,
    /// Escape was pressed.
    Dismissed,
    /// The index should be read again.
    IndexRetried,
    /// A scan finished, under `generation`.
    IndexLoaded(u64, AppIndex),
    /// Evidence about a launch arrived.
    LaunchObserved(u64, Observation),
    /// A failed launch should be tried again.
    LaunchRetried,
}

/// The message a key press maps to.
///
/// Pure, and tested on its own: the subscription below is a two-line adapter,
/// so "Enter activates and Escape closes" is a property of this function rather
/// than of a running window.
pub fn key_binding(key: &Key) -> Option<Message> {
    match key {
        Key::Named(Named::ArrowUp) => Some(Message::SelectionMoved(Step::Previous)),
        Key::Named(Named::ArrowDown) => Some(Message::SelectionMoved(Step::Next)),
        Key::Named(Named::Enter) => Some(Message::Activated),
        Key::Named(Named::Escape) => Some(Message::Dismissed),
        _ => None,
    }
}

/// The task that puts the caret in the search field.
///
/// `Operation` is Iced's only mechanism for reaching into a widget's state, and
/// `task::operate` is the only way to run one; iced 0.14 exposes both through
/// the `advanced` feature this crate enables for exactly this call.
pub fn focus_search() -> Task<Message> {
    iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::focus(
        iced::widget::Id::new(SEARCH_INPUT_ID),
    ))
}

/// The window title.
pub fn title(state: &State) -> String {
    state.localizer.message("launcher-window-title")
}

/// The window theme, from the theme tokens.
pub fn theme(state: &State) -> iced::Theme {
    crate::theme::iced_theme(&state.tokens)
}

/// The keys the launcher handles.
///
/// Key repeat is ignored: holding Enter is a person leaning on a key, not a
/// request to launch the same application again.
pub fn subscription(_state: &State) -> iced::Subscription<Message> {
    iced::keyboard::listen().filter_map(|event| match event {
        iced::keyboard::Event::KeyPressed {
            key, repeat: false, ..
        } => key_binding(&key),
        _ => None,
    })
}

/// Applies one message.
pub fn update(state: &mut State, message: Message) -> Task<Message> {
    let task = handle(state, message);
    state.refresh_model();
    task
}

/// The plan the update returns when it draws nothing, for `State::new` only.
fn empty_model() -> ViewModel {
    ViewModel {
        title: String::new(),
        placeholder: String::new(),
        query: String::new(),
        hint: String::new(),
        rows: Vec::new(),
        banner: None,
        notice: None,
        status: None,
    }
}

/// The message handler, without the projection step.
fn handle(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::QueryChanged(query) => {
            if query == state.query {
                return Task::none();
            }
            state.query = query;
            on_new_query(state);
            Task::none()
        }
        Message::QueryCleared => {
            if state.query.is_empty() {
                return Task::none();
            }
            state.query.clear();
            on_new_query(state);
            Task::none()
        }
        Message::SelectionMoved(step) => {
            move_selection(state, step);
            Task::none()
        }
        Message::RowChosen(id) => {
            // A click selects the row it landed on, and launching it is the
            // same decision Enter makes.
            if let Some(position) = state
                .engine
                .current()
                .and_then(|result| result.hits.iter().position(|hit| hit.id == id))
                && position != state.selected
            {
                state.selected = position;
                state.selection_generation += 1;
            }
            activate(state)
        }
        Message::Activated => activate(state),
        Message::Dismissed => {
            state.close_requested = true;
            info!("the launcher was dismissed");
            iced::exit()
        }
        Message::IndexRetried => retry_index(state),
        Message::IndexLoaded(generation, index) => {
            // ADR 0013 decision 4: a result that is not from the newest request
            // does not overwrite the view.
            if generation != state.scan_generation {
                debug!(
                    generation,
                    current = state.scan_generation,
                    "dropping an index the launcher has already moved on from"
                );
                return Task::none();
            }
            state.service = index.state();
            state.index = Some(index);
            state.research();
            Task::none()
        }
        Message::LaunchObserved(selection, observation) => {
            let Some(outcome) = state.activator.observe(selection, observation) else {
                debug!(selection, "evidence for an activation that is not tracked");
                return Task::none();
            };
            info!(selection, ?outcome, "launch observed");
            if let Some(activation) = state.activation.as_mut()
                && activation.selection == selection
            {
                activation.outcome = outcome;
            }
            Task::none()
        }
        Message::LaunchRetried => retry_launch(state),
    }
}

/// A new query invalidates the selection.
fn on_new_query(state: &mut State) {
    state.selected = 0;
    state.selection_generation += 1;
    state.research();
}

/// Moves the selection, stopping at the ends.
///
/// The ends are ends rather than a wrap-around: a list that cycles makes "how
/// many results are there" harder to answer, and Escape is always available.
fn move_selection(state: &mut State, step: Step) {
    let rows = state
        .engine
        .current()
        .map(|result| result.hits.len())
        .unwrap_or(0);
    if rows == 0 {
        return;
    }

    let moved = match step {
        Step::Previous => state.selected.saturating_sub(1),
        Step::Next => (state.selected + 1).min(rows - 1),
    };

    if moved != state.selected {
        state.selected = moved;
        // The selection is what an activation is keyed by, so moving it makes
        // the next Enter a new request.
        state.selection_generation += 1;
    }
}

/// Launches the selected application, at most once per selection.
fn activate(state: &mut State) -> Task<Message> {
    // Everything needed is copied out of the state before anything is written
    // back, so that the borrow of the index ends here.
    let Some((id, name, argv)) = selected_command(state) else {
        return Task::none();
    };

    let argv = match argv {
        Ok(argv) => argv,
        Err(kind) => {
            // The row is disabled, and pressing Enter on it says why rather
            // than doing nothing at all.
            warn!(entry = %id, "refusing to launch an unusable entry");
            state.activation = Some(Activation {
                selection: state.selection_generation,
                entry: id,
                name: name.clone(),
                outcome: LaunchOutcome::Failed {
                    failure: Failure::for_entry(kind, name),
                },
            });
            return Task::none();
        }
    };

    let Some(request) = state
        .activator
        .request(state.selection_generation, &id, &name, argv)
    else {
        debug!(selection = state.selection_generation, "already activated");
        return Task::none();
    };

    info!(entry = %request.entry, argv = ?request.argv, "launching an application");
    state.activation = Some(Activation {
        selection: request.selection,
        entry: request.entry.clone(),
        name: request.name.clone(),
        outcome: LaunchOutcome::Requested,
    });

    probe(request)
}

/// The id, name and command line of the selected entry.
fn selected_command(state: &State) -> Option<(String, String, Result<Vec<String>, FailureKind>)> {
    let index = state.index.as_ref()?;
    let hit = state.selected_hit()?;
    let entry = index.get(&hit.id)?;
    Some((
        entry.id.clone(),
        entry.name.clone(),
        entry.argv(&state.config.terminal),
    ))
}

/// Reads the index again, under a new generation.
fn retry_index(state: &mut State) -> Task<Message> {
    let generation = state.scan_generation + 1;
    state.scan_generation = generation;
    let locale = state.localizer.locale().clone();

    match IndexOptions::from_env(&locale, state.config.include_hidden) {
        Ok(options) => {
            state.service = ServiceState::Loading { generation };
            scan_task(options, generation)
        }
        Err(error) => {
            warn!(%error, "the application index could not be located again");
            state.index = None;
            state.service = ServiceState::Unavailable {
                generation,
                failure: unavailable(&error.to_string()),
            };
            state.research();
            Task::none()
        }
    }
}

/// Launches the last failed activation again, under a new selection.
fn retry_launch(state: &mut State) -> Task<Message> {
    let Some(failed) = state
        .activation
        .as_ref()
        .filter(|activation| matches!(activation.outcome, LaunchOutcome::Failed { .. }))
        .map(|activation| activation.selection)
    else {
        return Task::none();
    };

    let selection = state.selection_generation + 1;
    state.selection_generation = selection;

    let Some(request) = state.activator.retry(selection, failed) else {
        debug!(failed, "a launch that cannot be retried");
        return Task::none();
    };

    info!(entry = %request.entry, argv = ?request.argv, "retrying a launch");
    state.activation = Some(Activation {
        selection: request.selection,
        entry: request.entry.clone(),
        name: request.name.clone(),
        outcome: LaunchOutcome::Requested,
    });

    probe(request)
}

/// Reads the index off the update loop.
fn scan_task(options: IndexOptions, generation: u64) -> Task<Message> {
    Task::perform(
        async move { index::scan(&options, generation) },
        move |index| Message::IndexLoaded(generation, index),
    )
}

/// Starts a process and reports what became of it.
///
/// The blocking part runs on a thread of its own — never on the update loop and
/// never on the executor's worker — and the evidence comes back through a
/// stream, which is the shape ADR 0004's "long tasks go to calloop's threads"
/// has for an Iced component (ADR 0018 keeps Iced's own executor for the same
/// reason).
fn probe(request: LaunchRequest) -> Task<Message> {
    let selection = request.selection;
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();

    let started = std::thread::Builder::new()
        .name("inox-launcher-probe".to_owned())
        .spawn(move || {
            let observation = activation::spawn_and_probe(&request.argv, READINESS_GRACE);
            if sender.unbounded_send(observation).is_err() {
                debug!("the launcher closed before the launch could be verified");
            }
        });

    match started {
        Ok(_handle) => Task::stream(
            receiver.map(move |observation| Message::LaunchObserved(selection, observation)),
        ),
        Err(error) => Task::done(Message::LaunchObserved(
            selection,
            Observation::SpawnFailed {
                detail: error.to_string(),
            },
        )),
    }
}

/// The failure that stands for "there is no index".
fn unavailable(detail: &str) -> Failure {
    Failure::service(FailureKind::IndexUnavailable {
        detail: detail.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::AppIndex;
    use crate::placement::LogicalSize;
    use inox_common::i18n::Locale;

    fn prepared() -> Prepared {
        let tokens = Tokens::builtin_dark();
        let config = LauncherConfig::default();
        let placement = Placement::overlay(&config, &tokens, LogicalSize::new(1920, 1080), None);
        Prepared {
            tokens,
            localizer: Rc::new(
                crate::i18n::localizer(&Locale::parse("en-US").expect("a tag"))
                    .expect("the catalogs load"),
            ),
            config,
            placement,
        }
    }

    /// An index holding two launchable applications and a terminal one that
    /// cannot be launched because no terminal prefix is configured.
    fn index() -> AppIndex {
        let mut terminal = AppIndex::test_entry("htop.desktop", "htop", "htop");
        terminal.terminal = true;
        AppIndex::from_entries(
            vec![
                AppIndex::test_entry("example.desktop", "Example", "true"),
                terminal,
                AppIndex::test_entry("second.desktop", "Second", "true"),
            ],
            1,
        )
    }

    fn state() -> State {
        State::with_index(&prepared(), index(), 1)
    }

    /// Applies a message, discarding the task: these tests assert on the state
    /// transition, which is where the at-most-once rule lives; the task is what
    /// a running window would hand to the executor.
    fn send(state: &mut State, message: Message) {
        drop(update(state, message));
    }

    fn press(state: &mut State, key: Named) {
        let message = key_binding(&Key::Named(key)).expect("the key is bound");
        send(state, message);
    }

    #[test]
    fn the_four_handled_keys_map_to_their_messages() {
        assert_eq!(
            key_binding(&Key::Named(Named::ArrowUp)),
            Some(Message::SelectionMoved(Step::Previous))
        );
        assert_eq!(
            key_binding(&Key::Named(Named::ArrowDown)),
            Some(Message::SelectionMoved(Step::Next))
        );
        assert_eq!(
            key_binding(&Key::Named(Named::Enter)),
            Some(Message::Activated)
        );
        assert_eq!(
            key_binding(&Key::Named(Named::Escape)),
            Some(Message::Dismissed)
        );
        assert_eq!(key_binding(&Key::Character("a".into())), None);
        assert_eq!(key_binding(&Key::Named(Named::Tab)), None);
    }

    #[test]
    fn the_window_title_comes_from_the_catalogs() {
        let state = state();
        assert_eq!(
            title(&state),
            state.localizer.message("launcher-window-title")
        );
    }

    #[test]
    fn the_window_theme_is_built_from_the_tokens() {
        let state = state();
        assert_eq!(theme(&state).palette().background, {
            crate::theme::colour(state.tokens.colors.background)
        });
    }

    #[test]
    fn the_initial_state_lists_the_applications_by_name() {
        let state = state();
        assert_eq!(state.model.rows.len(), 3);
        assert_eq!(state.model.rows[0].label, "Example");
        assert_eq!(state.model.rows[1].label, "htop");
        assert_eq!(state.model.rows[2].label, "Second");
        assert_eq!(state.selected, 0);
        assert!(state.service.is_usable());
    }

    #[test]
    fn typing_then_enter_produces_exactly_one_launch() {
        let mut state = state();
        send(&mut state, Message::QueryChanged("ex".to_owned()));
        assert_eq!(state.model.rows.len(), 1);
        assert_eq!(state.model.rows[0].id, "example.desktop");

        let selection = state.selection_generation;
        press(&mut state, Named::Enter);
        assert!(state.activator.is_tracked(selection));
        assert_eq!(state.activator.tracked(), 1);
        assert!(matches!(
            state.activation.as_ref().map(|it| &it.outcome),
            Some(LaunchOutcome::Requested)
        ));

        // The finger reaches Enter again before the first launch has a result.
        press(&mut state, Named::Enter);
        press(&mut state, Named::Enter);
        assert_eq!(
            state.activator.tracked(),
            1,
            "the same selection is one request, however often Enter is pressed",
        );
    }

    #[test]
    fn a_new_selection_is_a_new_launch() {
        let mut state = state();
        press(&mut state, Named::Enter);
        assert_eq!(state.activator.tracked(), 1);

        send(&mut state, Message::RowChosen("second.desktop".to_owned()));
        assert_eq!(state.activator.tracked(), 2);
        assert_eq!(
            state.activation.as_ref().map(|it| it.entry.as_str()),
            Some("second.desktop")
        );
    }

    #[test]
    fn moving_the_selection_changes_the_generation_only_when_it_moves() {
        let mut state = state();
        let started = state.selection_generation;

        press(&mut state, Named::ArrowUp);
        assert_eq!(
            state.selection_generation, started,
            "the first row is already selected",
        );

        press(&mut state, Named::ArrowDown);
        assert_eq!(state.selection_generation, started + 1);

        // Down to the last row, then one press too many.
        while state.selected < state.model.rows.len() - 1 {
            press(&mut state, Named::ArrowDown);
        }
        let at_the_end = state.selection_generation;
        assert_eq!(at_the_end, started + (state.model.rows.len() - 1) as u64);

        press(&mut state, Named::ArrowDown);
        assert_eq!(
            state.selection_generation, at_the_end,
            "the last row is a wall, so it is not a new selection",
        );
    }

    #[test]
    fn the_selection_does_not_wrap_around() {
        let mut state = state();
        for _ in 0..5 {
            press(&mut state, Named::ArrowUp);
        }
        assert_eq!(state.selected, 0);
        for _ in 0..9 {
            press(&mut state, Named::ArrowDown);
        }
        assert_eq!(state.selected, state.model.rows.len() - 1);
    }

    #[test]
    fn escape_asks_to_close() {
        let mut state = state();
        assert!(!state.close_requested());
        press(&mut state, Named::Escape);
        assert!(state.close_requested());
    }

    #[test]
    fn typing_the_same_query_again_changes_nothing() {
        let mut state = state();
        send(&mut state, Message::QueryChanged("ex".to_owned()));
        let generation = state.selection_generation;
        send(&mut state, Message::QueryChanged("ex".to_owned()));
        assert_eq!(state.selection_generation, generation);
    }

    #[test]
    fn clearing_the_query_restores_the_full_list() {
        let mut state = state();
        send(&mut state, Message::QueryChanged("ex".to_owned()));
        assert_eq!(state.model.rows.len(), 1);
        send(&mut state, Message::QueryCleared);
        assert!(state.query.is_empty());
        assert_eq!(state.model.rows.len(), 3);

        let generation = state.selection_generation;
        send(&mut state, Message::QueryCleared);
        assert_eq!(state.selection_generation, generation, "nothing to clear");
    }

    #[test]
    fn a_query_with_no_matches_shows_the_no_match_banner() {
        let mut state = state();
        send(&mut state, Message::QueryChanged("zzzz".to_owned()));
        assert!(state.model.rows.is_empty());
        assert_eq!(
            state.model.banner.as_ref().map(|it| it.title.as_str()),
            Some(
                state
                    .localizer
                    .message("launcher-state-nomatch-title")
                    .as_str()
            )
        );
    }

    #[test]
    fn choosing_a_row_with_the_pointer_launches_it() {
        let mut state = state();
        send(&mut state, Message::RowChosen("htop.desktop".to_owned()));
        // The row is disabled, so the launcher reports why instead of starting
        // something that cannot run.
        assert_eq!(state.selected, 1);
        assert_eq!(state.activator.tracked(), 0);
        let activation = state.activation.as_ref().expect("a reported failure");
        assert!(matches!(activation.outcome, LaunchOutcome::Failed { .. }));
        assert_eq!(
            activation.outcome.clone(),
            LaunchOutcome::Failed {
                failure: Failure::for_entry(FailureKind::MissingTerminalPrefix, "htop".to_owned()),
            }
        );
    }

    #[test]
    fn an_unusable_row_does_not_start_a_process() {
        let mut state = state();
        send(&mut state, Message::RowChosen("htop.desktop".to_owned()));
        assert_eq!(state.activator.tracked(), 0);
    }

    #[test]
    fn evidence_for_an_untracked_activation_is_ignored() {
        let mut state = state();
        send(
            &mut state,
            Message::LaunchObserved(99, Observation::Ready { pid: 1 }),
        );
        assert!(state.activation.is_none());
    }

    #[test]
    fn evidence_updates_the_reported_outcome() {
        let mut state = state();
        press(&mut state, Named::Enter);
        let selection = state.selection_generation;

        send(
            &mut state,
            Message::LaunchObserved(selection, Observation::Ready { pid: 4242 }),
        );
        assert_eq!(
            state.activation.as_ref().map(|it| it.outcome.clone()),
            Some(LaunchOutcome::Started { pid: 4242 })
        );

        // A late observation for a settled attempt does not undo the result.
        send(
            &mut state,
            Message::LaunchObserved(
                selection,
                Observation::SpawnFailed {
                    detail: "too late".to_owned(),
                },
            ),
        );
        assert_eq!(
            state.activation.as_ref().map(|it| it.outcome.clone()),
            Some(LaunchOutcome::Started { pid: 4242 })
        );
    }

    #[test]
    fn a_failed_launch_can_be_retried() {
        let mut state = state();
        press(&mut state, Named::Enter);
        let selection = state.selection_generation;
        send(
            &mut state,
            Message::LaunchObserved(
                selection,
                Observation::SpawnFailed {
                    detail: "not found".to_owned(),
                },
            ),
        );
        assert!(matches!(
            state.activation.as_ref().map(|it| &it.outcome),
            Some(LaunchOutcome::Failed { .. })
        ));
        assert!(
            state.model.can_retry_launch(),
            "the view has to offer the retry",
        );

        send(&mut state, Message::LaunchRetried);
        assert_eq!(state.activator.tracked(), 2);
        assert!(matches!(
            state.activation.as_ref().map(|it| &it.outcome),
            Some(LaunchOutcome::Requested)
        ));
    }

    #[test]
    fn a_retry_without_a_failure_does_nothing() {
        let mut state = state();
        send(&mut state, Message::LaunchRetried);
        assert_eq!(state.activator.tracked(), 0);
    }

    #[test]
    fn an_index_from_a_generation_the_launcher_has_moved_past_is_dropped() {
        let mut state = state();
        state.scan_generation = 5;
        let before = state.index.as_ref().map(|index| index.len());

        let mut replacement = AppIndex::test_entry("other.desktop", "Other", "other");
        replacement.generic_name = None;
        send(
            &mut state,
            Message::IndexLoaded(2, AppIndex::from_entries(vec![replacement], 2)),
        );

        assert_eq!(state.index.as_ref().map(|index| index.len()), before);
        assert_eq!(state.service.generation(), 1);
    }

    #[test]
    fn an_index_from_the_newest_generation_replaces_the_old_one() {
        let mut state = state();
        state.scan_generation = 2;
        send(
            &mut state,
            Message::IndexLoaded(
                2,
                AppIndex::from_entries(
                    vec![AppIndex::test_entry("other.desktop", "Other", "other")],
                    2,
                ),
            ),
        );

        assert_eq!(state.index.as_ref().map(|index| index.len()), Some(1));
        assert_eq!(state.service.generation(), 2);
        assert_eq!(state.model.rows.len(), 1);
        assert_eq!(state.model.rows[0].label, "Other");
    }

    #[test]
    fn the_service_state_is_reported_to_the_view() {
        let state = state();
        assert!(state.model.banner.is_none());
        assert!(matches!(
            state.service,
            ServiceState::Ready { entries: 3, .. }
        ));
    }

    #[test]
    fn the_placement_decides_how_much_of_a_name_fits() {
        let state = state();
        assert_eq!(state.model.rows[0].label, "Example");
        assert!(!state.model.rows[0].truncated);
        assert!(state.placement.label_chars > 0);
    }
}
