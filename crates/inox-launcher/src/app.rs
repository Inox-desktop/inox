//! The state machine: state, messages, and `update`.
//!
//! The shape is the one the technology stack fixes for every shell component and the interaction contract
//! repeats for interactions: a message goes in, the state changes, and the
//! interesting work comes back as a task. Nothing here draws, and nothing here
//! blocks: the two slow things a launcher does — reading the desktop files and
//! waiting for a launched process — both happen on other threads and come back
//! as messages ([`crate::runtime`]), which is what keeps `update` a pure state
//! transition.
//!
//! The Qt side reaches this module through exactly two doors, and both are
//! narrow (docs/qml-migration-contract.md §3): a [`Message`] parsed from the JSON
//! a QML control dispatched (`crate::cxxqt_object`), and the serial
//! [`focus_search`] bumps, which is how "the caret belongs in the search field"
//! survives without the toolkit handing the state a widget to poke.
//!
//! The launch decision itself is the part with the requirements in it. An
//! activation is keyed by [`State::selection_generation`], which changes
//! whenever the selection could have changed, so pressing Enter repeatedly
//! before the first press has a result produces exactly one launch (acceptance
//! D02); and the result that launch is `Requested` until evidence arrives
//! (`crate::activation`), so the launcher never claims a success it has not
//! seen.

use std::rc::Rc;

use inox_common::i18n::Localizer;
use inox_config::{LauncherConfig, Theme as Tokens};
use tracing::{debug, info, warn};

use crate::runtime::Task;

use crate::activation::{
    self, Activation, Activator, LaunchOutcome, LaunchRequest, Observation, READINESS_GRACE,
};
use crate::categories::Category;
use crate::contract::{Failure, FailureKind, ServiceState};
use crate::index::{self, AppIndex, IndexOptions};
use crate::placement::Placement;
use crate::search::{self, Hit, SearchEngine};
use crate::view_model::{self, Projection, ViewModel};

/// The widget id the search field is focused under.
///
/// Deliberately not shaped like a Fluent message id: `crate::catalog_tests` reads
/// every `launcher-…` literal in the sources as a message the code asks for, and
/// an interface identifier is not one.
pub const SEARCH_INPUT_ID: &str = "inox-launcher-search-field";

/// Which way the selection moves, in the grid's terms.
///
/// The grid is what the arrows step through: `Left`/`Right` move one tile and
/// `Up`/`Down` move a whole row, which is [`crate::theme::Grid::columns`] tiles —
/// the same number the projection draws the tiles with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// One tile towards the first.
    Left,
    /// One tile towards the last.
    Right,
    /// One row towards the top.
    Up,
    /// One row towards the bottom.
    Down,
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
    /// The binary entry point used to restore child signal masks before exec.
    pub launch_helper: Option<std::path::PathBuf>,
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
    /// Which category chip is filtering the browse.
    ///
    /// The chip only narrows the list a person is browsing; a query searches
    /// every application (see [`State::research`]), because narrowing a search by
    /// a category the user is not looking at is how results go missing.
    pub category: Category,
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
    /// How many times the launcher has asked for the caret in the search field.
    ///
    /// Focus is a QML-side widget state, so the request is a number the bridge
    /// publishes: QML moves the caret whenever it changes. Bumping it is what
    /// [`focus_search`] does, and what a test asserts instead of poking a widget.
    pub focus_serial: u64,
    /// What the view draws.
    ///
    /// Projected after every message rather than on every frame: the projection
    /// allocates the labels it localises, and a frame is drawn far more often
    /// than the state changes.
    pub model: ViewModel,
}

impl State {
    /// Reconfigures the existing launcher while preserving its query and index.
    pub fn apply_configuration(&mut self, prepared: Prepared, index: Option<AppIndex>) {
        self.scan_generation += 1;
        if let Some(mut index) = index {
            index.set_generation(self.scan_generation);
            self.service = index.state();
            self.index = Some(index);
        }
        self.config = prepared.config;
        self.tokens = prepared.tokens;
        self.localizer = prepared.localizer;
        self.placement = prepared.placement;
        self.selection_generation += 1;
        self.selected = 0;
        self.research();
        self.refresh_model();
    }

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
            launch_helper: None,
            tokens: prepared.tokens.clone(),
            localizer: Rc::clone(&prepared.localizer),
            config: prepared.config.clone(),
            placement: prepared.placement.clone(),
            index: None,
            service: ServiceState::Loading { generation: 1 },
            scan_generation: 1,
            engine: SearchEngine::new(),
            query: String::new(),
            category: Category::All,
            selected: 0,
            selection_generation: 1,
            activator: Activator::new(),
            activation: None,
            close_requested: false,
            focus_serial: 0,
            model: empty_model(),
        }
    }

    /// A state over an index the caller supplies.
    ///
    /// [`State::new`] reads the XDG directories; this is the same state for a
    /// caller that already has an index — the projection tests in
    /// `crate::headless_tests`, which need a fixed list of applications to
    /// assert against, and any future preview or embedding of the launcher.
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
                category: self.category,
                activation: self.activation.as_ref(),
                label_budget: self.placement.label_chars,
            };
            view_model::project(&projection)
        };
        self.model = model;
    }

    /// Whether the launcher has been asked to close, for the caller that owns
    /// the cross-entry unwinding (the interaction contract: Esc closes one level at a time, and
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
    ///
    /// An empty query is the *browse*: every application, in name order, narrowed
    /// by the selected category chip. A query is a *search*: it runs over every
    /// application and the chip does not narrow it — a person who types "fire"
    /// while the Graphics chip happens to be selected is looking for Firefox, not
    /// for a graphics application whose name starts with "fire".
    ///
    /// The browse is not limited to `launcher.max_results`: that number says how
    /// many results a *search* may show, and a grid of applications exists to
    /// show the applications (the card scrolls when they do not fit).
    fn research(&mut self) {
        let Some(index) = self.index.as_ref() else {
            return;
        };

        let browsing = self.query.trim().is_empty();
        let limit = if browsing {
            index.len().max(1)
        } else {
            usize::from(self.config.max_results)
        };

        // A category the new index no longer has anything for cannot stay the
        // filter: an empty grid with a chip selected is a screen with no way out.
        if self.category != Category::All
            && !index
                .entries()
                .iter()
                .any(|entry| self.category.matches(&entry.categories))
        {
            self.category = Category::All;
        }

        let request =
            self.engine
                .request(&self.query, usize::from(self.config.max_query_chars), limit);
        let mut result = search::execute(index, &request);
        if browsing && self.category != Category::All {
            result.hits.retain(|hit| {
                index
                    .get(&hit.id)
                    .is_some_and(|entry| self.category.matches(&entry.categories))
            });
        }
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
    /// A category chip was chosen.
    CategorySelected(Category),
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
/// Pure, and tested on its own: QML reports the key it saw as one of the four
/// names below (`dispatch`'s `press` action), so "Enter activates and Escape
/// closes" stays a property of this function rather than of a running window.
///
/// Key repeat is ignored: holding Enter is a person leaning on a key, not a
/// request to launch the same application again.
pub fn key_binding(key: &str, repeat: bool) -> Option<Message> {
    if repeat {
        return None;
    }
    match key {
        "up" | "arrowup" => Some(Message::SelectionMoved(Step::Up)),
        "down" | "arrowdown" => Some(Message::SelectionMoved(Step::Down)),
        "left" | "arrowleft" => Some(Message::SelectionMoved(Step::Left)),
        "right" | "arrowright" => Some(Message::SelectionMoved(Step::Right)),
        "enter" | "return" => Some(Message::Activated),
        "escape" | "esc" => Some(Message::Dismissed),
        _ => None,
    }
}

/// Puts the caret in the search field.
///
/// The launcher is a thing you type into, so the field is focused before the
/// first frame: without this the window would open with nothing focused and the
/// first keystroke would go nowhere. Focus itself belongs to QML now, so what
/// this does is record the request as [`State::focus_serial`], which the bridge
/// publishes and the QML tree reacts to.
pub fn focus_search(state: &mut State) {
    state.focus_serial += 1;
    state.refresh_model();
}

/// The window title.
pub fn title(state: &State) -> String {
    state.localizer.message("launcher-window-title")
}

/// Applies one message.
pub fn update(state: &mut State, message: Message) -> Task<Message> {
    let task = handle(state, message);
    state.refresh_model();
    task
}

/// The view model before anything has been projected.
///
/// `State::blank` and the bridge's fallback projection both need a total value
/// here rather than an `Option` every renderer would have to unwrap.
pub(crate) fn empty_model() -> ViewModel {
    ViewModel {
        title: String::new(),
        placeholder: String::new(),
        query: String::new(),
        hint: String::new(),
        tiles: Vec::new(),
        chips: Vec::new(),
        heading: String::new(),
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
        Message::CategorySelected(category) => {
            if category == state.category {
                return Task::none();
            }
            state.category = category;
            // A new filter is a new list: the first tile is the selection, and
            // the next Enter is a new request.
            state.selected = 0;
            state.selection_generation += 1;
            state.research();
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
            Task::none()
        }
        Message::IndexRetried => retry_index(state),
        Message::IndexLoaded(generation, index) => {
            // the interaction contract, rule 4: a result that is not from the newest request
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
/// The ends are ends rather than a wrap-around: a grid that cycles makes "how
/// many applications are there" harder to answer, and Escape is always available.
/// `Up`/`Down` move by one grid row, and the row's width is the projected
/// [`crate::theme::Grid::columns`] — the window and the keyboard cannot disagree
/// about it because there is one value.
fn move_selection(state: &mut State, step: Step) {
    let rows = state
        .engine
        .current()
        .map(|result| result.hits.len())
        .unwrap_or(0);
    if rows == 0 {
        return;
    }

    let columns = state.placement.grid.columns.max(1);
    let last = rows - 1;
    let moved = match step {
        Step::Left => state.selected.saturating_sub(1),
        Step::Right => (state.selected + 1).min(last),
        Step::Up => state.selected.saturating_sub(columns),
        Step::Down => (state.selected + columns).min(last),
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

    probe(request, state.launch_helper.clone())
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

    probe(request, state.launch_helper.clone())
}

/// Reads the index off the update loop.
fn scan_task(options: IndexOptions, generation: u64) -> Task<Message> {
    Task::perform(move || index::scan(&options, generation))
        .map(move |index| Message::IndexLoaded(generation, index))
}

/// Starts a process and reports what became of it.
///
/// The blocking part runs on a thread of its own — never on the UI thread — and
/// the evidence comes back through the message queue [`crate::runtime`] owns,
/// which is the shape the "long tasks go to calloop's threads" has for a
/// Qt component: the Qt event loop stays in charge of the UI thread, and
/// `pump` turns the evidence into a state transition.
fn probe(request: LaunchRequest, helper: Option<std::path::PathBuf>) -> Task<Message> {
    let selection = request.selection;
    Task::attempt(
        move || {
            activation::spawn_and_probe_with_helper(
                &request.argv,
                READINESS_GRACE,
                helper.as_deref(),
            )
        },
        |error| Observation::SpawnFailed {
            detail: error.to_string(),
        },
    )
    .map(move |observation| Message::LaunchObserved(selection, observation))
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

    fn press(state: &mut State, key: &str) {
        let message = key_binding(key, false).expect("the key is bound");
        send(state, message);
    }

    #[test]
    fn live_configuration_preserves_query_and_index_and_reprojects_results() {
        let mut state = state();
        send(&mut state, Message::QueryChanged("example".into()));
        let mut config = prepared();
        config.config.max_results = 1;
        config.tokens = Tokens::builtin_light();
        state.apply_configuration(config.clone(), None);
        assert_eq!(state.query, "example");
        assert!(state.index.is_some());
        assert_eq!(state.config, config.config);
        assert_eq!(state.tokens, config.tokens);
        assert_eq!(state.model.tiles.len(), 1);
    }

    #[test]
    fn the_handled_keys_map_to_their_messages() {
        assert_eq!(
            key_binding("up", false),
            Some(Message::SelectionMoved(Step::Up))
        );
        assert_eq!(
            key_binding("arrowdown", false),
            Some(Message::SelectionMoved(Step::Down))
        );
        assert_eq!(
            key_binding("left", false),
            Some(Message::SelectionMoved(Step::Left))
        );
        assert_eq!(
            key_binding("arrowright", false),
            Some(Message::SelectionMoved(Step::Right))
        );
        assert_eq!(key_binding("enter", false), Some(Message::Activated));
        assert_eq!(key_binding("escape", false), Some(Message::Dismissed));
        assert_eq!(key_binding("a", false), None);
        assert_eq!(key_binding("tab", false), None);
        assert_eq!(key_binding("", false), None);
    }

    #[test]
    fn a_repeated_key_is_ignored() {
        for key in ["up", "down", "left", "right", "enter", "escape"] {
            assert_eq!(
                key_binding(key, true),
                None,
                "holding {key} is one press, not a stream of them",
            );
        }
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
    fn the_launcher_focuses_the_search_field_before_the_first_frame() {
        // `main` boots the state and then asks for the caret; the QML tree moves
        // it when the serial changes. A launcher that opens with nothing focused
        // cannot be typed into, so this is the startup contract.
        let mut state = state();
        assert_eq!(state.focus_serial, 0, "nothing has asked for focus yet");

        focus_search(&mut state);
        assert_eq!(state.focus_serial, 1);

        focus_search(&mut state);
        assert_eq!(state.focus_serial, 2, "every request is a new serial");
    }

    #[test]
    fn the_projection_names_the_field_the_caret_belongs_in() {
        let mut state = state();
        focus_search(&mut state);
        let projected = crate::view::project(&state);

        assert_eq!(
            projected["focus"]["field"].as_str(),
            Some(SEARCH_INPUT_ID),
            "the projection tells QML which control the caret belongs in",
        );
        assert_eq!(projected["focus"]["serial"].as_u64(), Some(1));
    }

    #[test]
    fn the_initial_state_lists_the_applications_by_name() {
        let state = state();
        assert_eq!(state.model.tiles.len(), 3);
        assert_eq!(state.model.tiles[0].label, "Example");
        assert_eq!(state.model.tiles[1].label, "htop");
        assert_eq!(state.model.tiles[2].label, "Second");
        assert_eq!(state.selected, 0);
        assert!(state.service.is_usable());
    }

    #[test]
    fn typing_then_enter_produces_exactly_one_launch() {
        let mut state = state();
        send(&mut state, Message::QueryChanged("ex".to_owned()));
        assert_eq!(state.model.tiles.len(), 1);
        assert_eq!(state.model.tiles[0].id, "example.desktop");

        let selection = state.selection_generation;
        press(&mut state, "enter");
        assert!(state.activator.is_tracked(selection));
        assert_eq!(state.activator.tracked(), 1);
        assert!(matches!(
            state.activation.as_ref().map(|it| &it.outcome),
            Some(LaunchOutcome::Requested)
        ));

        // The finger reaches Enter again before the first launch has a result.
        press(&mut state, "enter");
        press(&mut state, "enter");
        assert_eq!(
            state.activator.tracked(),
            1,
            "the same selection is one request, however often Enter is pressed",
        );
    }

    #[test]
    fn a_new_selection_is_a_new_launch() {
        let mut state = state();
        press(&mut state, "enter");
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

        press(&mut state, "up");
        assert_eq!(
            state.selection_generation, started,
            "the first tile is already selected",
        );

        press(&mut state, "right");
        assert_eq!(state.selection_generation, started + 1);

        // Down is a whole grid row; with three tiles it lands on the last one.
        press(&mut state, "down");
        assert_eq!(state.selected, state.model.tiles.len() - 1);
        let at_the_end = state.selection_generation;
        assert_eq!(at_the_end, started + 2);

        press(&mut state, "down");
        press(&mut state, "right");
        assert_eq!(
            state.selection_generation, at_the_end,
            "the last tile is a wall, so it is not a new selection",
        );
    }

    #[test]
    fn the_grid_arrows_step_by_one_tile_and_by_one_row() {
        // Three rows of applications on a grid whose columns come from the
        // placement: the keyboard and the window read the same number.
        let count = 30;
        let entries = (0..count)
            .map(|index| {
                AppIndex::test_entry(
                    &format!("app{index}.desktop"),
                    &format!("Application {index}"),
                    "true",
                )
            })
            .collect();
        let mut state = State::with_index(&prepared(), AppIndex::from_entries(entries, 1), 1);
        let columns = state.placement.grid.columns;
        assert!(
            columns >= 4 && columns < count,
            "the fixture grid is wider than a few columns: {columns}",
        );

        press(&mut state, "right");
        assert_eq!(state.selected, 1, "right is one tile");
        press(&mut state, "left");
        assert_eq!(state.selected, 0);
        press(&mut state, "left");
        assert_eq!(state.selected, 0, "the first tile is a wall");

        press(&mut state, "down");
        assert_eq!(state.selected, columns, "down is one grid row");
        press(&mut state, "down");
        assert_eq!(state.selected, columns * 2, "and another");
        press(&mut state, "up");
        assert_eq!(state.selected, columns);

        // Past the last row the selection clamps to the last tile.
        for _ in 0..4 {
            press(&mut state, "down");
        }
        assert_eq!(state.selected, count - 1);
        press(&mut state, "down");
        press(&mut state, "right");
        assert_eq!(
            state.selected,
            count - 1,
            "the last tile is a wall, not a wrap-around",
        );
    }

    #[test]
    fn a_category_chip_narrows_the_browse_but_not_a_search() {
        let mut gamed = AppIndex::test_entry("game.desktop", "Game", "true");
        gamed.categories = vec!["Game".to_owned()];
        let mut editor = AppIndex::test_entry("editor.desktop", "Editor", "true");
        editor.categories = vec!["Utility".to_owned(), "TextEditor".to_owned()];
        let mut state = State::with_index(
            &prepared(),
            AppIndex::from_entries(vec![gamed, editor], 1),
            1,
        );
        assert_eq!(state.model.tiles.len(), 2, "the browse shows everything");

        send(&mut state, Message::CategorySelected(Category::Game));
        assert_eq!(state.category, Category::Game);
        assert_eq!(state.model.tiles.len(), 1);
        assert_eq!(state.model.tiles[0].label, "Game");
        assert_eq!(
            state
                .model
                .chips
                .iter()
                .filter(|chip| chip.selected)
                .count(),
            1,
            "exactly one chip is the filter",
        );

        // A query searches every application: the chip does not hide the editor.
        send(&mut state, Message::QueryChanged("editor".to_owned()));
        assert_eq!(state.model.tiles.len(), 1);
        assert_eq!(state.model.tiles[0].label, "Editor");

        // Clearing the query brings the filter back.
        send(&mut state, Message::QueryCleared);
        assert_eq!(state.model.tiles.len(), 1);
        assert_eq!(state.model.tiles[0].label, "Game");

        // The same chip twice is not a new filter.
        let generation = state.selection_generation;
        send(&mut state, Message::CategorySelected(Category::Game));
        assert_eq!(state.selection_generation, generation);

        // Back to everything.
        send(&mut state, Message::CategorySelected(Category::All));
        assert_eq!(state.model.tiles.len(), 2);
    }

    #[test]
    fn a_category_the_index_no_longer_has_falls_back_to_all() {
        let mut game = AppIndex::test_entry("game.desktop", "Game", "true");
        game.categories = vec!["Game".to_owned()];
        let mut state = State::with_index(&prepared(), AppIndex::from_entries(vec![game], 1), 1);
        send(&mut state, Message::CategorySelected(Category::Game));
        assert_eq!(state.category, Category::Game);

        // A re-read finds no game at all: the filter cannot stay on an empty chip.
        let mut index = prepared();
        index.config.max_results = 8;
        send(
            &mut state,
            Message::IndexLoaded(
                1,
                AppIndex::from_entries(
                    vec![AppIndex::test_entry("editor.desktop", "Editor", "true")],
                    1,
                ),
            ),
        );
        assert_eq!(state.category, Category::All);
        assert_eq!(state.model.tiles.len(), 1);
    }

    #[test]
    fn the_selection_does_not_wrap_around() {
        let mut state = state();
        for _ in 0..5 {
            press(&mut state, "up");
            press(&mut state, "left");
        }
        assert_eq!(state.selected, 0);
        for _ in 0..9 {
            press(&mut state, "down");
            press(&mut state, "right");
        }
        assert_eq!(state.selected, state.model.tiles.len() - 1);
    }

    #[test]
    fn escape_asks_to_close() {
        let mut state = state();
        assert!(!state.close_requested());
        press(&mut state, "escape");
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
        assert_eq!(state.model.tiles.len(), 1);
        send(&mut state, Message::QueryCleared);
        assert!(state.query.is_empty());
        assert_eq!(state.model.tiles.len(), 3);

        let generation = state.selection_generation;
        send(&mut state, Message::QueryCleared);
        assert_eq!(state.selection_generation, generation, "nothing to clear");
    }

    #[test]
    fn a_query_with_no_matches_shows_the_no_match_banner() {
        let mut state = state();
        send(&mut state, Message::QueryChanged("zzzz".to_owned()));
        assert!(state.model.tiles.is_empty());
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
        press(&mut state, "enter");
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
        press(&mut state, "enter");
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
        assert_eq!(state.model.tiles.len(), 1);
        assert_eq!(state.model.tiles[0].label, "Other");
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
        assert_eq!(state.model.tiles[0].label, "Example");
        assert!(!state.model.tiles[0].truncated);
        assert!(state.placement.label_chars > 0);
    }
}
