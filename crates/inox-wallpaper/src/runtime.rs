//! The component's own event plumbing: work off the interaction loop, and an
//! explicit queue where the framework used to poll.
//!
//! The portable Elm shape (`Message`, `update`, `Task`) is not what a Qt host
//! needs, so this module supplies the two things the widget-toolkit runtime used
//! to supply for free:
//!
//! - **Work** is a closure that produces a [`Message`] —
//!   [`Task::perform`] — which this host runs on a thread of its own and posts
//!   back into an `mpsc` queue. Decoding a wallpaper is the only such job, and
//!   it has to stay off the interaction loop.
//! - **Events** are a queue the host drains when QML's timer asks it to
//!   ([`Runtime::pump`]). Configuration reloads arrive on the same queue, posted
//!   by the D-Bus / SIGHUP thread in [`crate::reload`].
//!
//! The runtime also owns the projection: [`Runtime::pump`] runs `update` for
//! every message, republishes the JSON document when anything changed, and
//! answers the readiness report the QML tree sends back. Keeping it Qt-free is
//! deliberate — everything except the cxx-qt property setter can be, and is,
//! unit-tested without a display.
//!
//! There is one runtime per process, on the thread that owns the window: the
//! state holds an `Rc<Localizer>` (a Fluent bundle is not shareable across
//! threads) and the interface is single-threaded. [`install`] hands it to the
//! QML element's constructor through a thread-local, which is the only way a
//! `Default`-constructed `QObject` can be given state the binary already built.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use tracing::{debug, warn};

use crate::app::{self, Message, State};
use crate::reload;
use crate::view::{self, Projection};

/// One unit of work the host still has to perform.
///
/// A task is a list of closures that each produce one value; a task with no
/// closures is [`Task::none`], which is what an action that needs no work
/// returns. The name is the vocabulary `app.rs` already uses — `update` returns
/// "what to do next", and the host decides where; the machinery is this crate's
/// own, with no framework behind it.
pub struct Task<T> {
    jobs: Vec<Work<T>>,
}

/// A closure that produces one message, safe to run on another thread.
type Work<T> = Box<dyn FnOnce() -> T + Send + 'static>;

impl<T: Send + 'static> Task<T> {
    /// A task with nothing to do.
    #[must_use]
    pub fn none() -> Self {
        Self { jobs: Vec::new() }
    }

    /// A task that finishes immediately with `value`.
    #[must_use]
    pub fn done(value: T) -> Self {
        Self {
            jobs: vec![Box::new(move || value)],
        }
    }

    /// A task that runs `work` off the interaction loop.
    #[must_use]
    pub fn perform(work: impl FnOnce() -> T + Send + 'static) -> Self {
        Self {
            jobs: vec![Box::new(work)],
        }
    }

    /// Every job of every task, in order.
    #[must_use]
    pub fn batch(tasks: impl IntoIterator<Item = Self>) -> Self {
        let mut jobs = Vec::new();
        for task in tasks {
            jobs.extend(task.jobs);
        }

        Self { jobs }
    }

    /// The same work, with each answer passed through `map`.
    #[must_use]
    pub fn map<U: Send + 'static>(self, map: impl Fn(T) -> U + Send + Sync + 'static) -> Task<U> {
        let map = Arc::new(map);
        let jobs = self
            .jobs
            .into_iter()
            .map(|work| {
                let map = Arc::clone(&map);

                Box::new(move || map(work())) as Work<U>
            })
            .collect();

        Task { jobs }
    }

    /// Whether this task has nothing to do.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// The jobs, moved out so the host can run them.
    #[must_use]
    pub fn into_jobs(self) -> Vec<Work<T>> {
        self.jobs
    }
}

impl<T: Send + 'static> Default for Task<T> {
    fn default() -> Self {
        Self::none()
    }
}

thread_local! {
    /// The runtime the QML element's constructor picks up.
    ///
    /// A thread-local rather than a `OnceLock` because the state is deliberately
    /// not `Send`/`Sync`: it holds the localized catalogs and the UI's message
    /// queues, and the window lives on exactly one thread.
    static INSTALLED: RefCell<Option<Runtime>> = const { RefCell::new(None) };
}

/// Makes `runtime` the one the QML element's constructor takes.
///
/// Returns `false` when a runtime was already installed: the binary installs
/// exactly one, and a second call is a programming error rather than something
/// to overwrite silently.
pub fn install(runtime: Runtime) -> bool {
    INSTALLED.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() {
            return false;
        }
        *slot = Some(runtime);

        true
    })
}

/// The installed runtime, taken out so the QML element can own it.
///
/// `None` when the binary installed none — the element then has no state machine
/// and renders nothing, which is what a test that instantiates it directly sees.
pub fn take() -> Option<Runtime> {
    INSTALLED.with(|slot| slot.borrow_mut().take())
}

/// The application state, its queues, and the document QML reads.
#[derive(Debug)]
pub struct Runtime {
    state: State,
    sender: Sender<Message>,
    inbox: Receiver<Message>,
    reloads: Receiver<reload::Request>,
    projection: Projection,
    warned_image: bool,
}

impl Runtime {
    /// Starts the component with `state`, draining `reloads` for configuration
    /// changes, and queues the first frame.
    #[must_use]
    pub fn start(state: State, reloads: Receiver<reload::Request>) -> Self {
        let (sender, inbox) = mpsc::channel();
        let mut runtime = Self {
            state,
            sender,
            inbox,
            reloads,
            projection: Projection::empty(),
            warned_image: false,
        };

        let boot = app::boot(&mut runtime.state);
        runtime.submit(boot);
        runtime.refresh();

        runtime
    }

    /// The state, for tests and for the log line the binary prints at startup.
    #[must_use]
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The JSON document QML parses.
    #[must_use]
    pub fn model_json(&self) -> &str {
        self.projection.json()
    }

    /// Runs every message that arrived since the last pump.
    ///
    /// Called by QML's timer: it drains finished decodes and configuration
    /// reloads, runs `update` for each, hands any new work to a worker thread,
    /// and republishes the projection when anything changed. Returns whether it
    /// did, so the caller knows whether to touch the property.
    pub fn pump(&mut self) -> bool {
        let mut changed = false;

        while let Ok(message) = self.inbox.try_recv() {
            debug!("applying a finished wallpaper message");
            let task = app::update(&mut self.state, message);
            self.submit(task);
            changed = true;
        }

        while let Ok(request) = self.reloads.try_recv() {
            self.apply_reload(request);
            changed = true;
        }

        if changed {
            self.refresh();
        }

        changed
    }

    /// Applies one JSON action QML reported.
    ///
    /// Returns whether the action was understood, so the bridge can republish
    /// only when something happened. An unknown action is a programming error in
    /// the QML tree: it is logged and dropped, never a panic — a desktop that
    /// stops drawing because a control asked for the wrong thing would be a
    /// worse outcome than a control that does nothing.
    pub fn dispatch(&mut self, action: &str) -> bool {
        let Some(message) = view::parse_action(action) else {
            warn!(action, "ignoring an unknown wallpaper action");
            return false;
        };

        let task = app::update(&mut self.state, message);
        self.submit(task);
        self.refresh();

        true
    }

    /// Checks the report the QML tree sends once it has built itself.
    ///
    /// Three things have to agree before the surface counts as evidence: the
    /// window title the tree read (the projection crossed the bridge intact),
    /// the number of accessible items it built (the semantic tree exists, root
    /// AGENTS.md §7.4), and — when the model draws an image — that Qt actually
    /// decoded the data URL the projection carries.
    ///
    /// `false` while the first frame is still in flight, so the tree keeps
    /// asking instead of reporting a state it has not rendered.
    pub fn verify_report(&mut self, title: &str, accessible_items: i32, image_ready: bool) -> bool {
        if self.state.is_busy() {
            return false;
        }

        if title != self.projection.title() {
            warn!(
                expected = %self.projection.title(),
                reported = title,
                "the QML tree reported a title that does not match the projection"
            );

            return false;
        }

        if accessible_items != self.projection.accessible_items() {
            warn!(
                expected = self.projection.accessible_items(),
                reported = accessible_items,
                "the QML semantic tree does not match the projected accessible items"
            );

            return false;
        }

        if self.projection.has_wallpaper() && !image_ready {
            if !self.warned_image {
                warn!(
                    "the QML tree could not decode the projected wallpaper image; \
                     the window is drawing the theme background"
                );
                self.warned_image = true;
            }

            return false;
        }

        true
    }

    /// Runs the jobs of `task` on their own threads and posts the answers back.
    fn submit(&mut self, task: Task<Message>) {
        for work in task.into_jobs() {
            let sender = self.sender.clone();
            let spawned = std::thread::Builder::new()
                .name(String::from("wallpaper work"))
                .spawn(move || {
                    let message = work();
                    if sender.send(message).is_err() {
                        warn!("the wallpaper host stopped; dropping a finished message");
                    }
                });

            if let Err(error) = spawned {
                warn!(%error, "cannot start a worker thread; a wallpaper request will not finish");
            }
        }
    }

    /// Applies a configuration reload the D-Bus / SIGHUP thread prepared.
    fn apply_reload(&mut self, request: reload::Request) {
        let result = (|| {
            if !request.is_current() {
                return Err(String::from("configuration reload expired"));
            }
            let localizer = Rc::new(
                crate::i18n::localizer(request.config.locale.clone()).map_err(|e| e.to_string())?,
            );

            self.state
                .apply_configuration(
                    Arc::new(request.config.theme.clone()),
                    localizer,
                    request.resource.0.clone(),
                    request.resource.1.clone(),
                )
                .map_err(|e| format!("{e:?}"))
        })();

        request.finish(result);
    }

    /// Republishes the projection from the current state.
    fn refresh(&mut self) {
        self.projection = view::project(&self.state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Preview;
    use crate::model::{Selection, Size, Unusable};
    use inox_config::{Theme, WallpaperConfig};
    use std::path::PathBuf;

    fn state(selection: Selection, configuration: Option<Unusable>) -> State {
        let preview = Preview::from_config(
            &WallpaperConfig::default(),
            std::path::Path::new("/config"),
            Size::new(1280, 720),
            Arc::from(Vec::<PathBuf>::new()),
        );
        let preview = Preview {
            selection,
            ..preview
        };

        State::new(
            Arc::new(Theme::builtin_dark()),
            Rc::new(
                crate::i18n::localizer(inox_common::i18n::Locale::default_locale())
                    .expect("the shipped catalogs"),
            ),
            preview,
            configuration,
        )
    }

    fn runtime(state: State) -> (Runtime, Sender<reload::Request>) {
        let (sender, receiver) = mpsc::channel();

        (Runtime::start(state, receiver), sender)
    }

    #[test]
    fn a_task_maps_and_batches_its_work() {
        let task = Task::batch([Task::done(1u8), Task::perform(|| 2u8)])
            .map(|value| u16::from(value) * 10);
        let answers: Vec<u16> = task.into_jobs().into_iter().map(|work| work()).collect();

        assert_eq!(answers, [10, 20]);
        assert!(Task::<u8>::none().is_empty());
        assert!(!Task::done(0u8).is_empty());
    }

    #[test]
    fn the_projection_is_ready_before_anything_is_pumped() {
        let (runtime, _reloads) = runtime(state(Selection::ThemeBackground, None));
        let document: serde_json::Value =
            serde_json::from_str(runtime.model_json()).expect("the projection is JSON");

        assert_eq!(document["title"], "Inox wallpaper");
        assert!(document["wallpaper"].is_null());
        assert_eq!(
            runtime.state().generation(),
            crate::contract::Generation::FIRST
        );
    }

    #[test]
    fn pumping_without_messages_changes_nothing() {
        let (mut runtime, _reloads) = runtime(state(Selection::ThemeBackground, None));
        let before = String::from(runtime.model_json());

        assert!(!runtime.pump());
        assert_eq!(runtime.model_json(), before);
    }

    #[test]
    fn a_reload_action_with_nothing_configured_keeps_the_empty_state() {
        let (mut runtime, _reloads) = runtime(state(Selection::ThemeBackground, None));

        assert!(runtime.dispatch(r#"{"type":"reload"}"#));
        assert!(!runtime.pump());

        let document: serde_json::Value =
            serde_json::from_str(runtime.model_json()).expect("the projection is JSON");
        assert!(document["wallpaper"].is_null());
        // The phase is one of the card's lines, tagged with the state it reports:
        // the document has no second copy of it under `card`.
        assert!(
            document["card"]["lines"]
                .as_array()
                .is_some_and(|lines| lines.iter().any(|line| line["kind"] == "idle")),
            "a reload with nothing configured stays in the empty state: {}",
            document["card"]["lines"]
        );
    }

    #[test]
    fn an_unknown_action_is_ignored_rather_than_applied() {
        let (mut runtime, _reloads) = runtime(state(Selection::ThemeBackground, None));

        assert!(!runtime.dispatch(r#"{"type":"self-destruct"}"#));
        assert!(!runtime.dispatch("not json"));
    }

    #[test]
    fn a_readiness_report_has_to_agree_with_the_projection() {
        let (mut runtime, _reloads) = runtime(state(Selection::ThemeBackground, None));
        let title = String::from("Inox wallpaper");
        let items = runtime.projection.accessible_items();

        assert!(!runtime.verify_report("Something else", items, true));
        assert!(!runtime.verify_report(&title, items + 1, true));
        assert!(runtime.verify_report(&title, items, true));
    }

    #[test]
    fn a_report_is_not_evidence_while_the_first_frame_is_in_flight() {
        let (mut runtime, _reloads) = runtime(state(
            Selection::Image(PathBuf::from("/walls/wall.png")),
            None,
        ));
        let title = String::from(runtime.projection.title());
        let items = runtime.projection.accessible_items();

        assert!(runtime.state().is_busy());
        assert!(
            !runtime.verify_report(&title, items, true),
            "a tree that reports before the wallpaper is drawn is not evidence"
        );
    }

    #[test]
    fn installing_a_second_runtime_is_refused() {
        let (first, _reloads) = runtime(state(Selection::ThemeBackground, None));
        assert!(install(first));

        let (second, _reloads) = runtime(state(Selection::ThemeBackground, None));
        assert!(!install(second), "the binary installs exactly one runtime");

        let taken = take().expect("the installed runtime");
        assert!(taken.model_json().contains("Inox wallpaper"));
        assert!(take().is_none(), "a runtime is taken once");
    }
}
