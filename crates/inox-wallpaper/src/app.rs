//! The Elm application: state, messages, and what a message does to the state.
//!
//! [`update`] is a pure function over the state plus the tasks it returns. It
//! does no I/O and blocks on nothing: a request that needs the filesystem
//! returns a [`Task::perform`], which [`crate::runtime`] runs on a thread of its
//! own, and the answer comes back as another message (image decoding
//! must not block the interaction loop). That is also what keeps the whole state
//! testable: a message is data, so a test can feed one in and read the state out.
//!
//! The state itself is small because the interesting parts live elsewhere: what
//! is on screen is a [`Snapshot`] ([`crate::contract`]), getting something there
//! is [`crate::apply`], and what the widget tree shows is
//! [`crate::view_model`]'s projection of this state.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use inox_common::i18n::Localizer;
use inox_config::{Theme, WallpaperConfig, WallpaperMode};

use crate::apply::{self, Preparation};
use crate::contract::{CommandResult, Generation, ServiceState, Snapshot};
use crate::model::{OutputName, Selection, Size, Unusable};
use crate::runtime::Task;

/// The output name the preview uses when the configuration names none.
pub const PREVIEW_OUTPUT: &str = "preview";

/// The size the windowed preview lays out at.
///
/// 1280 × 720 is the size acceptance Q02 asks screenshots to be taken at, so the
/// preview doubles as the fixture that scenario is judged against.
pub const PREVIEW_SIZE: Size = Size::new(1280, 720);

/// What the preview draws, and with what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The output the preview stands in for.
    pub output: OutputName,
    /// The size that output is pretending to have.
    pub size: Size,
    /// How the image is fitted to it.
    pub mode: WallpaperMode,
    /// What the configuration asks that output to draw.
    pub selection: Selection,
    /// Where a bundled wallpaper is searched for, in order.
    ///
    /// Resolved once, from the session environment, and carried here so that
    /// loading a frame — which runs off the interaction loop — does not read the
    /// environment again.
    pub wallpaper_dirs: Arc<[PathBuf]>,
}

impl Preview {
    /// The preview for a configuration.
    ///
    /// With no Wayland connection there are no outputs to enumerate, so the
    /// preview takes the first output the configuration names — a `BTreeMap`, so
    /// "first" is deterministic — and otherwise a synthetic name that resolves
    /// through the wildcard and the shared image exactly as a real output would.
    #[must_use]
    pub fn from_config(
        config: &WallpaperConfig,
        config_dir: &Path,
        size: Size,
        wallpaper_dirs: Arc<[PathBuf]>,
    ) -> Self {
        let output = preview_output(config);

        Self {
            selection: Selection::from_config(config, &output, config_dir),
            output,
            size,
            mode: config.mode,
            wallpaper_dirs,
        }
    }
}

/// The output name a configuration is previewed as.
#[must_use]
pub fn preview_output(config: &WallpaperConfig) -> OutputName {
    config
        .per_output
        .keys()
        .next()
        .map(|name| OutputName::new(name.clone()))
        .unwrap_or_else(|| OutputName::new(PREVIEW_OUTPUT))
}

/// A load the host still has to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The generation this request belongs to.
    pub generation: Generation,
    /// The output it is for.
    pub output: OutputName,
    /// The image it is about, as the configuration resolved it.
    pub selection: Selection,
}

/// What the interface can ask the state to do.
///
/// [`Clone`] because the widget toolkit requires it of every message; see
/// [`Preparation`] for why a message that carries a frame is still never copied.
#[derive(Debug, Clone)]
pub enum Message {
    /// One output's request finished, successfully or not.
    ///
    /// Carries the prepared frame rather than the path it came from, which is
    /// what makes the message a value: the state does not have to redo the work
    /// to find out what happened.
    FramePrepared {
        /// The generation the finished request belonged to.
        generation: Generation,
        /// The outcome for that output.
        preparation: Preparation,
    },
    /// The user asked for the wallpaper to be loaded again.
    Reload,
}

/// Everything the component is doing, as one value.
#[derive(Debug, Clone)]
pub struct State {
    /// The tokens the interface draws with.
    theme: Arc<Theme>,
    /// The catalogs the interface reads its strings from.
    ///
    /// An [`Rc`] because a catalog is not shareable across threads — it holds a
    /// Fluent bundle — and the interface is single-threaded; the messages that
    /// are sent to other threads carry pixels and paths, never strings.
    localizer: Rc<Localizer>,
    /// The output the windowed preview stands in for.
    preview: Preview,
    /// What every output is showing.
    snapshot: Snapshot,
    /// The generation of the newest request.
    generation: Generation,
    /// A request the runtime has not picked up yet.
    queued: Option<Request>,
    /// A request that is being worked on, if one is.
    in_flight: Option<Generation>,
    /// What became of the last request that finished.
    last: Option<CommandResult>,
    /// Why the component as a whole is degraded, when it is: the configuration
    /// it was started with could not be read.
    configuration: Option<Unusable>,
    /// What the component as a whole is doing.
    service: ServiceState,
}

impl State {
    /// Commits an already prepared wallpaper and invalidates earlier image requests.
    ///
    /// # Errors
    /// A resource failure leaves the visible frame and all preferences unchanged.
    pub fn apply_configuration(
        &mut self,
        theme: Arc<Theme>,
        localizer: Rc<Localizer>,
        preview: Preview,
        preparation: Preparation,
    ) -> Result<(), crate::model::Unusable> {
        if let Preparation::Degraded { reason, .. } = &preparation {
            return Err(reason.clone());
        }
        self.generation = self.generation.next();
        self.queued = None;
        self.in_flight = None;
        self.theme = theme;
        self.localizer = localizer;
        self.preview = preview;
        self.configuration = None;
        self.accept(self.generation, preparation);
        Ok(())
    }

    /// A state that has not loaded anything yet.
    ///
    /// `configuration` is the reason the configuration could not be read, when
    /// it could not; the preview then has nothing to draw but the theme
    /// background, and says so instead of failing to start.
    #[must_use]
    pub fn new(
        theme: Arc<Theme>,
        localizer: Rc<Localizer>,
        preview: Preview,
        configuration: Option<Unusable>,
    ) -> Self {
        let service = match &configuration {
            Some(reason) => ServiceState::Degraded {
                reason: reason.clone(),
            },
            None => ServiceState::Starting,
        };

        let mut state = Self {
            theme,
            localizer,
            preview,
            snapshot: Snapshot::empty(),
            generation: Generation::FIRST,
            queued: None,
            in_flight: None,
            last: None,
            configuration,
            service,
        };
        state.start();

        state
    }

    /// The tokens the interface draws with.
    #[must_use]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The catalogs the interface reads its strings from.
    #[must_use]
    pub fn localizer(&self) -> &Localizer {
        &self.localizer
    }

    /// The output the preview stands in for, and what it draws it with.
    #[must_use]
    pub fn preview(&self) -> &Preview {
        &self.preview
    }

    /// What every output is showing.
    #[must_use]
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }

    /// The generation of the newest request.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// What the component as a whole is doing.
    #[must_use]
    pub const fn service(&self) -> &ServiceState {
        &self.service
    }

    /// What became of the last request that finished.
    #[must_use]
    pub const fn last(&self) -> Option<&CommandResult> {
        self.last.as_ref()
    }

    /// Why the configuration could not be read, when it could not.
    #[must_use]
    pub const fn configuration_problem(&self) -> Option<&Unusable> {
        self.configuration.as_ref()
    }

    /// Whether a request is queued or being worked on.
    ///
    /// What is on screen stays on screen while this is true: a reload never
    /// blanks the output, which is the whole of the "no gap" rule.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.queued.is_some() || self.in_flight.is_some()
    }

    /// Forgets the request the runtime has taken, so it is performed once.
    ///
    /// Taking it twice returns `None`, which is how "one action, one request"
    /// holds even if a caller is careless (acceptance D05).
    pub fn take_request(&mut self) -> Option<Request> {
        let request = self.queued.take()?;
        self.in_flight = Some(request.generation);

        Some(request)
    }

    /// Records what the component starts out doing.
    ///
    /// A configured image — a path or a bundled wallpaper — is queued for the
    /// runtime to load. A theme background needs no work at all, so its output
    /// state is recorded here rather than left unknown: the preview then shows
    /// the theme background with the line that says so, instead of an empty
    /// window that could be mistaken for a failure.
    fn start(&mut self) {
        if self.preview.selection.wants_image() {
            self.queued = Some(Request {
                generation: self.generation,
                output: self.preview.output.clone(),
                selection: self.preview.selection.clone(),
            });

            return;
        }

        let output = self.preview.output.clone();
        self.last = Some(apply::settle(
            &mut self.snapshot,
            Preparation::Nothing { output },
        ));
        self.refresh_service();
    }

    /// Queues another load of the same selection.
    ///
    /// A reload always supersedes whatever is running: the generation moves on,
    /// so the older request's answer arrives as [`CommandResult::Stale`] and is
    /// dropped rather than shown. That is what makes asking twice safe — the
    /// user gets the newest answer, never an older one on top of it — and it is
    /// why a request in flight is not a reason to refuse.
    ///
    /// Nothing to draw is the one case that queues nothing: there is no image to
    /// load, and the interface says so instead of offering a control that would
    /// do nothing.
    fn queue_reload(&mut self) -> bool {
        if !self.preview.selection.wants_image() {
            return false;
        }

        self.generation = self.generation.next();
        self.queued = Some(Request {
            generation: self.generation,
            output: self.preview.output.clone(),
            selection: self.preview.selection.clone(),
        });

        true
    }

    /// Records the outcome of one request.
    fn accept(&mut self, generation: Generation, preparation: Preparation) {
        // A result from a request the user has already replaced is dropped
        // rather than applied: it would show them something they have asked to
        // move on from.
        if generation < self.generation {
            self.last = Some(CommandResult::Stale {
                generation,
                current: self.generation,
            });

            return;
        }

        self.snapshot.advance(generation);
        let result = apply::settle(&mut self.snapshot, preparation);
        self.in_flight = None;
        self.last = Some(result);
        self.refresh_service();
    }

    /// Recomputes what the component as a whole is doing.
    fn refresh_service(&mut self) {
        self.service = match (&self.configuration, &self.last) {
            (Some(reason), _) => ServiceState::Degraded {
                reason: reason.clone(),
            },
            (None, Some(CommandResult::Rejected { reason })) => ServiceState::Degraded {
                reason: reason.clone(),
            },
            _ => ServiceState::Ready,
        };
    }
}

/// The task that performs the request the state queued at startup, if any.
///
/// Called once, from the boot closure: the first frame is loaded off the
/// interaction loop like every other one, and the window opens as soon as the
/// frame is ready.
pub fn boot(state: &mut State) -> Task<Message> {
    match state.take_request() {
        Some(request) => load(
            request,
            state.preview.mode,
            state.preview.size,
            &state.preview.wallpaper_dirs,
        ),
        None => Task::none(),
    }
}

/// What a message does to the state.
pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::FramePrepared {
            generation,
            preparation,
        } => {
            state.accept(generation, preparation);

            Task::none()
        }
        Message::Reload => {
            if !state.queue_reload() {
                return Task::none();
            }

            match state.take_request() {
                Some(request) => load(
                    request,
                    state.preview.mode,
                    state.preview.size,
                    &state.preview.wallpaper_dirs,
                ),
                None => Task::none(),
            }
        }
    }
}

/// Decodes and places one request, off the interaction loop.
fn load(
    request: Request,
    mode: WallpaperMode,
    size: Size,
    wallpaper_dirs: &[PathBuf],
) -> Task<Message> {
    let generation = request.generation;
    // The directories are copied into the task because it outlives this call;
    // they are a handful of paths, and the alternative is a reference into the
    // state that the runtime's thread would have to keep alive.
    let wallpaper_dirs = wallpaper_dirs.to_vec();

    Task::perform(move || {
        let preparation = apply::prepare(
            request.output,
            &request.selection,
            mode,
            size,
            &wallpaper_dirs,
        );

        Message::FramePrepared {
            generation,
            preparation,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::OutputStatus;
    use std::path::PathBuf;

    fn preview(selection: Selection) -> Preview {
        Preview {
            output: OutputName::from("HDMI-A-1"),
            size: PREVIEW_SIZE,
            mode: WallpaperMode::Fill,
            selection,
            wallpaper_dirs: Arc::from(Vec::<PathBuf>::new()),
        }
    }

    fn state(selection: Selection) -> State {
        State::new(
            Arc::new(Theme::builtin_dark()),
            Rc::new(
                crate::i18n::localizer(inox_common::i18n::Locale::default_locale())
                    .expect("the shipped catalogs"),
            ),
            preview(selection),
            None,
        )
    }

    /// A frame that really went through the decoder, so the state is fed the
    /// value the runtime would hand it.
    fn prepared(dir: &Path, output: &str, width: u32, height: u32) -> Preparation {
        let path = dir.join("wall.png");
        image::RgbaImage::from_pixel(width, height, image::Rgba([1, 2, 3, 255]))
            .save(&path)
            .expect("the fixture is written");

        apply::prepare(
            OutputName::from(output),
            &Selection::Image(path),
            WallpaperMode::Fill,
            PREVIEW_SIZE,
            &[],
        )
    }

    #[test]
    fn live_wallpaper_commit_invalidates_old_loads_and_failure_keeps_preferences() {
        let dir = tempfile::tempdir().expect("directory");
        let mut state = state(Selection::Image(dir.path().join("old.png")));
        let old = state.take_request().expect("initial image request");
        let next = Preview::from_config(
            &WallpaperConfig::default(),
            dir.path(),
            PREVIEW_SIZE,
            Arc::from(Vec::<PathBuf>::new()),
        );
        let preparation = prepared(dir.path(), next.output.as_str(), 8, 8);
        let localizer = Rc::clone(&state.localizer);
        state
            .apply_configuration(
                Arc::new(Theme::builtin_light()),
                localizer.clone(),
                next.clone(),
                preparation,
            )
            .expect("prepared image");
        let current = state.snapshot.clone();
        state.accept(old.generation, Preparation::Nothing { output: old.output });
        assert_eq!(
            state.snapshot, current,
            "a superseded decode cannot clear the new frame"
        );
        let error = Preparation::Degraded {
            output: next.output.clone(),
            path: dir.path().join("missing"),
            reason: Unusable::Missing,
        };
        assert_eq!(
            state.apply_configuration(
                Arc::new(Theme::builtin_dark()),
                localizer,
                next.clone(),
                error
            ),
            Err(Unusable::Missing)
        );
        assert_eq!(state.theme(), &Theme::builtin_light());
        assert_eq!(state.preview(), &next);
        assert_eq!(state.snapshot, current);
    }

    #[test]
    fn a_state_with_nothing_configured_is_ready_at_once_and_queues_nothing() {
        let mut state = state(Selection::ThemeBackground);

        assert!(!state.is_busy(), "nothing to load, so nothing is in flight");
        assert_eq!(
            state.service(),
            &ServiceState::Ready,
            "there was nothing to do, and it is done"
        );
        assert_eq!(state.generation(), Generation::FIRST);
        assert!(state.take_request().is_none());
        assert_eq!(
            state
                .snapshot()
                .output(&OutputName::from("HDMI-A-1"))
                .map(OutputStatus::is_degraded),
            Some(false),
            "the output is known, and drawing the theme background is not a failure"
        );
    }

    #[test]
    fn a_state_with_an_image_queues_exactly_one_request() {
        let state = state(Selection::Image(PathBuf::from("/walls/wall.png")));

        assert!(state.is_busy());
        assert_eq!(
            state.service(),
            &ServiceState::Starting,
            "nothing has been loaded yet"
        );
    }

    #[test]
    fn taking_the_request_twice_yields_it_once() {
        let mut state = state(Selection::Image(PathBuf::from("/walls/wall.png")));

        let request = state.take_request().expect("the first request");

        assert_eq!(request.output.as_str(), "HDMI-A-1");
        assert_eq!(request.generation, Generation::FIRST);
        assert!(state.take_request().is_none(), "no duplicate work");
        assert!(state.is_busy(), "still in flight until it answers");
    }

    #[test]
    fn a_finished_request_shows_a_frame_and_stops_being_busy() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut state = state(Selection::Image(dir.path().join("wall.png")));
        let request = state.take_request().expect("a request");

        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: request.generation,
                preparation: prepared(dir.path(), "HDMI-A-1", 640, 480),
            },
        );

        assert!(!state.is_busy());
        assert_eq!(state.service(), &ServiceState::Ready);
        assert_eq!(state.last(), Some(&CommandResult::Applied));
        assert_eq!(
            state
                .snapshot()
                .output(&OutputName::from("HDMI-A-1"))
                .and_then(OutputStatus::size),
            Some(Size::new(640, 480))
        );
    }

    #[test]
    fn a_rejected_request_is_degraded_rather_than_ready() {
        let mut state = state(Selection::Image(PathBuf::from("/walls/gone.png")));
        let request = state.take_request().expect("a request");

        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: request.generation,
                preparation: Preparation::Degraded {
                    output: OutputName::from("HDMI-A-1"),
                    path: PathBuf::from("/walls/gone.png"),
                    reason: Unusable::Missing,
                },
            },
        );

        assert_eq!(
            state.service(),
            &ServiceState::Degraded {
                reason: Unusable::Missing
            }
        );
        assert!(
            state.snapshot().is_empty() || {
                state
                    .snapshot()
                    .output(&OutputName::from("HDMI-A-1"))
                    .is_some_and(OutputStatus::is_degraded)
            }
        );
    }

    #[test]
    fn a_result_from_a_generation_the_user_has_moved_past_is_dropped() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut state = state(Selection::Image(dir.path().join("wall.png")));
        let stale = state.take_request().expect("the first request");

        // The user asks again before the first answer arrives. `update` takes
        // the request it queues, so the new one is already in flight.
        let _ = update(&mut state, Message::Reload);
        assert!(state.generation() > stale.generation);
        assert!(state.is_busy());

        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: stale.generation,
                preparation: prepared(dir.path(), "HDMI-A-1", 64, 64),
            },
        );

        assert_eq!(
            state.last(),
            Some(&CommandResult::Stale {
                generation: stale.generation,
                current: state.generation(),
            })
        );
        assert!(
            state.snapshot().is_empty(),
            "the superseded frame must not reach the output"
        );
        assert!(state.is_busy(), "the newer request is still running");
    }

    #[test]
    fn a_reload_bumps_the_generation_and_leaves_one_request_running() {
        let mut state = state(Selection::Image(PathBuf::from("/walls/wall.png")));
        let first = state.take_request().expect("the first request").generation;

        let _ = update(&mut state, Message::Reload);

        assert_eq!(state.generation(), first.next());
        assert!(
            state.take_request().is_none(),
            "the reload was taken by update, not left queued"
        );
        assert!(state.is_busy(), "and it is in flight");
    }

    #[test]
    fn a_reload_while_one_is_running_supersedes_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut state = state(Selection::Image(dir.path().join("wall.png")));
        let first = state.take_request().expect("the first request").generation;

        let _ = update(&mut state, Message::Reload);
        assert_eq!(state.generation(), first.next());

        // The superseded answer arrives late and is dropped.
        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: first,
                preparation: prepared(dir.path(), "HDMI-A-1", 64, 64),
            },
        );

        assert!(matches!(state.last(), Some(CommandResult::Stale { .. })));
        assert!(
            state.snapshot().is_empty(),
            "the superseded frame never reaches the output"
        );
    }

    #[test]
    fn a_reload_with_nothing_configured_queues_nothing() {
        let mut state = state(Selection::ThemeBackground);

        let _ = update(&mut state, Message::Reload);

        assert_eq!(state.generation(), Generation::FIRST);
        assert!(!state.is_busy());
    }

    #[test]
    fn a_configuration_that_could_not_be_read_is_degraded_from_the_start() {
        let state = State::new(
            Arc::new(Theme::builtin_dark()),
            Rc::new(
                crate::i18n::localizer(inox_common::i18n::Locale::default_locale())
                    .expect("the shipped catalogs"),
            ),
            preview(Selection::ThemeBackground),
            Some(Unusable::Unreadable {
                detail: "permission denied".to_owned(),
            }),
        );

        assert_eq!(
            state.configuration_problem(),
            Some(&Unusable::Unreadable {
                detail: "permission denied".to_owned()
            })
        );
        assert!(matches!(state.service(), ServiceState::Degraded { .. }));
        assert!(!state.is_busy());
    }

    #[test]
    fn a_later_success_moves_the_service_from_degraded_to_ready() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut state = state(Selection::Image(dir.path().join("wall.png")));
        let request = state.take_request().expect("a request");
        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: request.generation,
                preparation: Preparation::Degraded {
                    output: OutputName::from("HDMI-A-1"),
                    path: PathBuf::from("/walls/gone.png"),
                    reason: Unusable::Missing,
                },
            },
        );
        assert!(!state.service().is_ready());

        let _ = update(&mut state, Message::Reload);
        let reload = state.generation();
        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: reload,
                preparation: prepared(dir.path(), "HDMI-A-1", 320, 200),
            },
        );

        assert!(state.service().is_ready());
        assert_eq!(state.last(), Some(&CommandResult::Applied));
    }

    #[test]
    fn the_preview_takes_the_first_output_the_configuration_names() {
        let mut config = WallpaperConfig::default();
        config
            .per_output
            .insert("HDMI-A-1".to_owned(), PathBuf::from("wide.png"));
        config
            .per_output
            .insert("eDP-1".to_owned(), PathBuf::from("narrow.png"));

        assert_eq!(preview_output(&config).as_str(), "HDMI-A-1");
    }

    #[test]
    fn the_preview_of_a_configuration_without_outputs_is_a_synthetic_one() {
        let config = WallpaperConfig::default();

        assert_eq!(preview_output(&config).as_str(), PREVIEW_OUTPUT);
    }

    #[test]
    fn the_preview_resolves_its_selection_like_any_other_output() {
        let mut config = WallpaperConfig {
            image: Some(PathBuf::from("shared.png")),
            ..WallpaperConfig::default()
        };
        config
            .per_output
            .insert("HDMI-A-1".to_owned(), PathBuf::from("wide.png"));

        let preview = Preview::from_config(
            &config,
            Path::new("/cfg"),
            PREVIEW_SIZE,
            Arc::from(Vec::<PathBuf>::new()),
        );

        assert_eq!(preview.output.as_str(), "HDMI-A-1");
        assert_eq!(
            preview.selection,
            Selection::Image(PathBuf::from("/cfg/wide.png"))
        );
        assert_eq!(preview.size, PREVIEW_SIZE);
    }

    #[test]
    fn the_preview_size_is_the_one_the_screenshots_are_taken_at() {
        assert_eq!(PREVIEW_SIZE, Size::new(1280, 720));
    }

    #[test]
    fn a_configuration_that_names_no_image_previews_the_bundled_default() {
        let default = inox_config::wallpapers::default_bundled().expect("a default is shipped");
        let config = WallpaperConfig::default();

        let preview = Preview::from_config(
            &config,
            Path::new("/cfg"),
            PREVIEW_SIZE,
            Arc::from(vec![PathBuf::from("/usr/share/inox/wallpapers")]),
        );

        assert_eq!(
            preview.selection,
            Selection::Bundled {
                wallpaper: default,
                explicit: false,
            }
        );
        assert_eq!(
            preview.wallpaper_dirs.as_ref(),
            &[PathBuf::from("/usr/share/inox/wallpapers")]
        );
    }

    #[test]
    fn a_bundled_default_queues_a_request_like_any_other_image() {
        let default = inox_config::wallpapers::default_bundled().expect("a default is shipped");
        let mut state = state(Selection::Bundled {
            wallpaper: default,
            explicit: false,
        });

        assert!(state.is_busy(), "the bundled default is an image to load");
        let request = state.take_request().expect("a request");
        assert_eq!(
            request.selection,
            Selection::Bundled {
                wallpaper: default,
                explicit: false,
            }
        );
    }
}
