//! The projection from state to what the window shows.
//!
//! A view model exists here for the same reason it exists in every Inox shell
//! component (ADR 0013): the widget tree should be a transcription, not a place
//! where decisions are made. Everything a user can see is decided in
//! [`project`] — which strings are used, whether the retry control is enabled
//! and *why not* when it is not, which phase the component is in, and whether
//! anything is shown over the wallpaper at all — and the widget tree in
//! [`crate::view`] only draws it.
//!
//! The projection is pure and total: it reads the state and a catalog and
//! returns strings, so every state the component can be in — starting, showing,
//! empty, degraded, busy — has a tested answer instead of an untested branch
//! inside a `view` function (acceptance D05).
//!
//! # The shape of the surface
//!
//! The window is one status card, one problem card when there is a problem, and
//! a footer that says what kind of surface this is. The status card is always
//! there and always says the same four things in the same order: which output
//! ([`StatusCard::heading`]), what is on it now ([`StatusCard::summary`]), where
//! that came from ([`StatusCard::source`]) and how it was fitted
//! ([`StatusCard::scaling`]) — followed by the phase ([`StatusCard::phase`]) and,
//! when there is nothing configured, what would change that
//! ([`StatusCard::hint`], `docs/quality/ui-guidelines.md` §6).
//!
//! The phase is what keeps "in progress" and "in effect" apart (UI spec §5):
//! [`PhaseKind::Loading`] while a request is in flight, [`PhaseKind::Ready`] once
//! a frame is committed for the output, [`PhaseKind::Degraded`] when the selected
//! image is not on the output, and [`PhaseKind::Idle`] when nothing is
//! configured. It is omitted in one case only: while the first load runs the
//! notice card already says so, and the same sentence twice is noise.

use iced::widget::image::Handle;
use inox_common::i18n::{Arg, Localizer};
use inox_config::WallpaperMode;

use crate::app::State;
use crate::contract::{OutputStatus, Showing};
use crate::model::{PathKind, Unusable};
use crate::scaling::Placement;

/// Everything the window shows, ready to be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallpaperViewModel {
    /// The title of the preview window.
    pub title: String,
    /// What the previewed output is, and what it is drawing.
    pub card: StatusCard,
    /// What the user has to be told, when there is something.
    pub notice: Option<Notice>,
    /// The wallpaper itself, when there is one to draw.
    pub wallpaper: Option<Wallpaper>,
    /// The line at the foot of the preview explaining what it is.
    pub footer: Option<String>,
    /// The order the keyboard visits the interactive controls in.
    pub focus_order: Vec<Focus>,
}

/// What the previewed output is doing, one field per line of its card.
///
/// Every field is a finished string: the widget tree picks a type role and a
/// token colour for each one, and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusCard {
    /// Which output this card is about, e.g. "Output HDMI-A-1".
    pub heading: String,
    /// What is on the output right now. `None` until the output is known at all,
    /// which is the case while the first frame is still being prepared.
    pub summary: Option<String>,
    /// Where what is on the output came from, when it came from a file.
    pub source: Option<String>,
    /// How the image was fitted: the one part of the geometry a user can act on
    /// (`wallpaper.mode`), and so always on screen.
    pub scaling: String,
    /// The phase the component is in, when the card is the thing that says it.
    pub phase: Option<Phase>,
    /// What would change the empty state, when the state is empty.
    pub hint: Option<String>,
}

/// A phase, and the sentence it is drawn as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    /// Which phase it is; the widget tree colours the sentence by this.
    pub kind: PhaseKind,
    /// The sentence, in the session's language.
    pub text: String,
}

/// The phases the component is ever in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseKind {
    /// A request is in flight.
    Loading,
    /// A frame is committed for the output.
    Ready,
    /// The selected image is not on the output.
    Degraded,
    /// Nothing is configured; the theme background is the answer.
    Idle,
}

/// The image to draw and where its copies go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wallpaper {
    /// The pixels, in the renderer's own refcounted buffer.
    pub image: Handle,
    /// The destination rectangle of every copy.
    pub placement: Placement,
}

/// Something the user has to be told about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Whether this is a failure or a load that is still running. The two read
    /// differently and are coloured differently.
    pub kind: NoticeKind,
    /// What happened, in one line.
    pub heading: String,
    /// Which output and which file it is about, when it is about one.
    pub subject: Option<String>,
    /// Why it happened, when there is a reason to give.
    pub reason: Option<String>,
    /// What the user can do about it.
    pub remedy: Option<String>,
    /// The control the notice offers.
    pub action: Action,
}

/// Which of the two notices this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// A request that could not be honoured.
    Problem,
    /// A request that is still being worked on.
    Loading,
}

/// A control and whether it does anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// What the control is called.
    pub label: String,
    /// Whether pressing it does anything.
    pub enabled: bool,
    /// Why it does not, when it does not: a control that is disabled without a
    /// reason is a dead end (acceptance D05).
    pub reason: Option<String>,
}

/// Something the keyboard can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The retry control.
    Retry,
}

/// Projects the state into what the window shows.
#[must_use]
pub fn project(state: &State) -> WallpaperViewModel {
    let localizer = state.localizer();
    let output = &state.preview().output;
    let status = state.snapshot().output(output);

    let wallpaper = status.and_then(|status| match &status.showing {
        Showing::Image {
            image, placement, ..
        } => Some(Wallpaper {
            image: image.clone(),
            placement: placement.clone(),
        }),
        Showing::Background => None,
    });

    let notice = notice(state, status, wallpaper.is_some());
    let card = card(state, status, wallpaper.is_some(), notice.as_ref());
    let focus_order = match &notice {
        Some(notice) if notice.action.enabled => vec![Focus::Retry],
        _ => Vec::new(),
    };

    WallpaperViewModel {
        title: localizer.message("wallpaper-window-title"),
        card,
        notice,
        wallpaper,
        footer: Some(localizer.message("wallpaper-windowed-notice")),
        focus_order,
    }
}

/// The status card: the output, what is on it, where it came from, how it was
/// fitted, and the phase.
fn card(
    state: &State,
    status: Option<&OutputStatus>,
    drawn: bool,
    notice: Option<&Notice>,
) -> StatusCard {
    let localizer = state.localizer();
    let phase = phase(state, drawn, notice);

    StatusCard {
        heading: localizer.message_with(
            "wallpaper-output-heading",
            &[("output", Arg::from(state.preview().output.as_str()))],
        ),
        summary: status.map(|status| summary_line(localizer, status)),
        source: status.and_then(|status| source_line(localizer, status)),
        scaling: scaling_line(localizer, state.preview().mode),
        hint: match &phase {
            Some(phase) if phase.kind == PhaseKind::Idle => {
                Some(localizer.message("wallpaper-empty-remedy"))
            }
            _ => None,
        },
        phase,
    }
}

/// The phase the component is in, or `None` when a notice already says it.
///
/// The one case that is suppressed is the first load: the notice card says
/// "Loading the wallpaper…" and repeating it in the status card would be the
/// same sentence twice. Every other case keeps the phase, including a retry that
/// is running while the previous failure is still readable — "in progress" has
/// to be visible exactly when the user has just asked for something (UI spec
/// §5).
fn phase(state: &State, drawn: bool, notice: Option<&Notice>) -> Option<Phase> {
    let localizer = state.localizer();

    if notice.is_some_and(|notice| notice.kind == NoticeKind::Loading) {
        return None;
    }

    let kind = if state.is_busy() {
        PhaseKind::Loading
    } else if drawn {
        PhaseKind::Ready
    } else if !state.preview().selection.wants_image() && state.configuration_problem().is_none() {
        PhaseKind::Idle
    } else {
        PhaseKind::Degraded
    };

    Some(Phase {
        kind,
        text: phase_line(localizer, kind),
    })
}

/// The sentence for one phase, in the user's language.
#[must_use]
pub fn phase_line(localizer: &Localizer, kind: PhaseKind) -> String {
    localizer.message(match kind {
        PhaseKind::Loading => "wallpaper-phase-loading",
        PhaseKind::Ready => "wallpaper-phase-ready",
        PhaseKind::Degraded => "wallpaper-phase-degraded",
        PhaseKind::Idle => "wallpaper-phase-idle",
    })
}

/// The notice for the state, if it has one to make.
///
/// In order: a configuration that could not be read, an image that could not be
/// used, and a first load that is still running. The first two outrank the third
/// because the user can already read what is wrong while the retry they asked
/// for is running — which is also why the retry control is *disabled* rather
/// than gone in that case.
fn notice(state: &State, status: Option<&OutputStatus>, drawn: bool) -> Option<Notice> {
    let localizer = state.localizer();

    if let Some(problem) = state.configuration_problem() {
        return Some(Notice {
            kind: NoticeKind::Problem,
            heading: localizer.message_with(
                "wallpaper-config-error",
                &[("detail", Arg::from(problem.to_string()))],
            ),
            subject: None,
            reason: None,
            remedy: Some(localizer.message("wallpaper-config-remedy")),
            action: Action {
                label: localizer.message("wallpaper-action-retry"),
                enabled: false,
                reason: Some(localizer.message("wallpaper-config-read-once")),
            },
        });
    }

    if let Some(problem) = status.and_then(|status| status.problem.as_ref()) {
        let subject = localizer.message_with(
            "wallpaper-problem-subject",
            &[
                ("output", Arg::from(state.preview().output.as_str())),
                ("path", Arg::from(problem.path.clone().unwrap_or_default())),
            ],
        );

        return Some(Notice {
            kind: NoticeKind::Problem,
            heading: localizer.message("wallpaper-problem-heading"),
            subject: Some(subject),
            reason: Some(reason(localizer, &problem.reason)),
            remedy: Some(localizer.message("wallpaper-remedy")),
            action: action(state),
        });
    }

    if !drawn && state.is_busy() {
        let subject = localizer.message_with(
            "wallpaper-output-heading",
            &[("output", Arg::from(state.preview().output.as_str()))],
        );

        return Some(Notice {
            kind: NoticeKind::Loading,
            heading: localizer.message("wallpaper-status-loading"),
            subject: Some(subject),
            reason: None,
            remedy: None,
            action: action(state),
        });
    }

    None
}

/// The retry control, and why it is disabled when it is.
///
/// A running load is not a reason to disable it: asking again supersedes the
/// request in flight (see `State::queue_reload`), and a control that went dead
/// under the user's finger would be worse than one that redid the work. The two
/// cases that *do* disable it are the ones where pressing would do nothing at
/// all, and each says which it is.
fn action(state: &State) -> Action {
    let localizer = state.localizer();
    let label = localizer.message("wallpaper-action-retry");

    if !state.preview().selection.wants_image() {
        return Action {
            label,
            enabled: false,
            reason: Some(localizer.message("wallpaper-action-nothing")),
        };
    }

    Action {
        label,
        enabled: true,
        reason: None,
    }
}

/// The reason in the user's language.
///
/// One arm per variant of [`Unusable`], so that a reason added to the model
/// cannot reach the screen untranslated: the compiler asks for a message, and a
/// message with no translation fails the catalog test.
fn reason(localizer: &Localizer, reason: &Unusable) -> String {
    match reason {
        Unusable::Missing => localizer.message("wallpaper-reason-missing"),
        Unusable::NotAFile { kind } => localizer.message(match kind {
            PathKind::Directory => "wallpaper-reason-not-a-file-directory",
            PathKind::Other => "wallpaper-reason-not-a-file-other",
        }),
        Unusable::Unreadable { detail } => localizer.message_with(
            "wallpaper-reason-unreadable",
            &[("detail", Arg::from(detail.clone()))],
        ),
        Unusable::Undecodable { detail } => localizer.message_with(
            "wallpaper-reason-undecodable",
            &[("detail", Arg::from(detail.clone()))],
        ),
        Unusable::TooLarge { pixels, limit, .. } => localizer.message_with(
            "wallpaper-reason-too-large",
            &[
                ("pixels", Arg::from(*pixels as i64)),
                ("limit", Arg::from(*limit as i64)),
            ],
        ),
        Unusable::TooLongSide { size, limit } => localizer.message_with(
            "wallpaper-reason-too-long-side",
            &[
                ("width", Arg::from(size.width)),
                ("height", Arg::from(size.height)),
                ("limit", Arg::from(*limit)),
            ],
        ),
        Unusable::Degenerate { image, .. } => localizer.message_with(
            "wallpaper-reason-degenerate",
            &[
                ("width", Arg::from(image.width)),
                ("height", Arg::from(image.height)),
            ],
        ),
        Unusable::TooManyTiles { tile, tiles, limit } => localizer.message_with(
            "wallpaper-reason-too-many-tiles",
            &[
                ("width", Arg::from(tile.width)),
                ("height", Arg::from(tile.height)),
                ("tiles", Arg::from(*tiles as i64)),
                ("limit", Arg::from(*limit)),
            ],
        ),
    }
}

/// The line that says what the previewed output is drawing.
///
/// It is a function of its own because it is the one place that decides how "an
/// image, of this size" reads — and because the preview exists for a human to
/// look at, so what it is looking at belongs on screen and not only in the log.
/// Where the image came from is [`source_line`]'s line, not this one: a path is
/// metadata, and metadata is a `caption` of its own (UI spec §4).
#[must_use]
pub fn summary_line(localizer: &Localizer, status: &OutputStatus) -> String {
    match &status.showing {
        Showing::Image { size, .. } => localizer.message_with(
            "wallpaper-output-image",
            &[
                ("width", Arg::from(size.width)),
                ("height", Arg::from(size.height)),
            ],
        ),
        Showing::Background => localizer.message("wallpaper-output-background"),
    }
}

/// The line that says where what is on the output came from.
///
/// `None` when the output is drawing the theme background: the background has no
/// source to name, and a line saying so would be noise next to `summary_line`.
#[must_use]
pub fn source_line(localizer: &Localizer, status: &OutputStatus) -> Option<String> {
    match &status.showing {
        Showing::Image { path, .. } => Some(localizer.message_with(
            "wallpaper-output-source",
            &[("path", Arg::from(path.clone()))],
        )),
        Showing::Background => None,
    }
}

/// The line that says how the image is fitted to the output.
///
/// The mode is the part of the geometry a user can change, so its name is copy
/// from the catalog rather than the raw configuration value: the same
/// `wallpaper.mode` reads as `fill` in one language and `填充` in another
/// (root `AGENTS.md` §7.3).
#[must_use]
pub fn scaling_line(localizer: &Localizer, mode: WallpaperMode) -> String {
    localizer.message_with(
        "wallpaper-scaling",
        &[("mode", Arg::from(mode_name(localizer, mode)))],
    )
}

/// The name of one scaling mode, in the user's language.
fn mode_name(localizer: &Localizer, mode: WallpaperMode) -> String {
    localizer.message(match mode {
        WallpaperMode::Fill => "wallpaper-mode-fill",
        WallpaperMode::Fit => "wallpaper-mode-fit",
        WallpaperMode::Stretch => "wallpaper-mode-stretch",
        WallpaperMode::Center => "wallpaper-mode-center",
        WallpaperMode::Tile => "wallpaper-mode-tile",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Message, Preview, update};
    use crate::apply::{self};
    use crate::contract::{CommandResult, OutputStatus};
    use crate::i18n::MESSAGES;
    use crate::model::{OutputName, Selection, Size};
    use inox_common::i18n::Locale;
    use inox_config::{Theme, WallpaperMode};
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::Arc;

    const OUTPUT: Size = Size::new(1280, 720);

    fn localizer() -> Localizer {
        crate::i18n::localizer(Locale::default_locale()).expect("the shipped catalogs")
    }

    fn chinese() -> Localizer {
        crate::i18n::localizer(Locale::parse(crate::i18n::CHINESE_TAG).expect("a well-formed tag"))
            .expect("the shipped catalogs")
    }

    fn state(selection: Selection) -> State {
        State::new(
            Arc::new(Theme::builtin_dark()),
            Rc::new(localizer()),
            Preview {
                output: OutputName::from("HDMI-A-1"),
                size: OUTPUT,
                mode: WallpaperMode::Fill,
                selection,
                wallpaper_dirs: Arc::from(Vec::<PathBuf>::new()),
            },
            None,
        )
    }

    /// The same state, previewing `mode` instead of `fill`.
    fn state_with_mode(selection: Selection, mode: WallpaperMode) -> State {
        State::new(
            Arc::new(Theme::builtin_dark()),
            Rc::new(localizer()),
            Preview {
                output: OutputName::from("HDMI-A-1"),
                size: OUTPUT,
                mode,
                selection,
                wallpaper_dirs: Arc::from(Vec::<PathBuf>::new()),
            },
            None,
        )
    }

    /// A state that has been through both steps the runtime takes: a request, and
    /// its answer. The answer is produced by the real decoder from a real PNG.
    fn loaded(dir: &Path, width: u32, height: u32) -> State {
        let path = dir.join("wall.png");
        image::RgbaImage::from_pixel(width, height, image::Rgba([9, 8, 7, 255]))
            .save(&path)
            .expect("the fixture is written");

        let mut state = state(Selection::Image(path.clone()));
        let request = state.take_request().expect("a request");
        let preparation = apply::prepare(
            request.output,
            &Selection::Image(path),
            WallpaperMode::Fill,
            OUTPUT,
            &[],
        );
        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: request.generation,
                preparation,
            },
        );

        state
    }

    /// A state whose image could not be used, with the reason the caller names.
    fn failed(dir: &Path, reason: Unusable) -> State {
        let path = dir.join("wall.png");
        let mut state = state(Selection::Image(path.clone()));
        let request = state.take_request().expect("a request");
        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: request.generation,
                preparation: apply::Preparation::Degraded {
                    output: request.output,
                    path,
                    reason,
                },
            },
        );

        state
    }

    #[test]
    fn a_state_that_has_loaded_nothing_shows_the_theme_background_and_a_loading_notice() {
        let state = state(Selection::Image(PathBuf::from("/walls/wall.png")));
        let model = project(&state);

        assert_eq!(model.title, "Inox wallpaper");
        assert!(model.wallpaper.is_none(), "nothing to draw yet");
        assert_eq!(model.card.heading, "Output HDMI-A-1");
        assert_eq!(
            model.card.summary, None,
            "nothing has reached the output yet"
        );
        assert_eq!(model.card.scaling, "Scaling: fill");
        assert_eq!(
            model.card.phase, None,
            "the loading notice says it, so the card does not repeat it"
        );
        let notice = model.notice.expect("a notice");
        assert_eq!(notice.kind, NoticeKind::Loading);
        assert_eq!(notice.heading, "Loading the wallpaper…");
        assert_eq!(notice.subject.as_deref(), Some("Output HDMI-A-1"));
        assert!(
            notice.action.enabled,
            "asking again while the first load runs is allowed"
        );
        assert_eq!(notice.action.reason, None);
        assert_eq!(model.focus_order, vec![Focus::Retry]);
        assert!(model.footer.is_some());
    }

    #[test]
    fn a_state_that_was_asked_for_the_theme_background_shows_it_and_no_notice() {
        let state = state(Selection::ThemeBackground);
        let model = project(&state);

        assert!(model.wallpaper.is_none());
        assert!(
            model.notice.is_none(),
            "the theme background is a chosen state, not something to report"
        );
        assert_eq!(model.card.summary.as_deref(), Some("The theme background"));
        assert_eq!(model.card.source, None, "the background has no source");
        assert_eq!(
            model.card.phase.as_ref().map(|phase| phase.kind),
            Some(PhaseKind::Idle)
        );
        assert_eq!(
            model.card.phase.as_ref().map(|phase| phase.text.as_str()),
            Some("No image")
        );
        assert!(
            model.card.hint.is_some(),
            "the empty state says what would change it"
        );
        assert!(model.focus_order.is_empty());
    }

    #[test]
    fn an_output_showing_an_image_projects_its_pixels_and_geometry() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let model = project(&loaded(dir.path(), 640, 480));

        let wallpaper = model.wallpaper.expect("a wallpaper");

        assert!(
            wallpaper.placement.first().covers(OUTPUT),
            "a fill covers the output"
        );
        assert!(
            model
                .card
                .summary
                .as_deref()
                .is_some_and(|summary| summary.contains("640×480")),
            "the card says what it is showing: {:?}",
            model.card.summary
        );
        assert!(
            model
                .card
                .source
                .as_deref()
                .is_some_and(|source| source.contains("wall.png")),
            "the path is the card's own caption line: {:?}",
            model.card.source
        );
        assert_eq!(
            model.card.phase.as_ref().map(|phase| phase.kind),
            Some(PhaseKind::Ready),
            "a committed frame is in effect"
        );
        assert_eq!(model.card.hint, None, "nothing to fix");
        assert!(model.notice.is_none(), "a wallpaper with nothing to say");
        assert!(model.focus_order.is_empty(), "there is nothing to press");
    }

    #[test]
    fn a_missing_image_is_reported_with_its_output_its_path_and_a_remedy() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let model = project(&failed(dir.path(), Unusable::Missing));

        let notice = model.notice.expect("a notice");

        assert_eq!(notice.kind, NoticeKind::Problem);
        assert_eq!(notice.heading, "The selected wallpaper could not be used");
        assert!(
            notice
                .subject
                .as_deref()
                .is_some_and(|s| s.starts_with("HDMI-A-1: ")),
            "{:?}",
            notice.subject
        );
        assert_eq!(notice.reason.as_deref(), Some("the file does not exist"));
        assert!(notice.remedy.is_some());
        assert!(notice.action.enabled, "the user can ask again");
        assert_eq!(
            model.card.phase.as_ref().map(|phase| phase.kind),
            Some(PhaseKind::Degraded),
            "the card says the selected image is not applied"
        );
        assert_eq!(
            model.focus_order,
            vec![Focus::Retry],
            "the one control is reachable"
        );
        assert!(
            model.wallpaper.is_none(),
            "nothing was ever decoded for this output"
        );
    }

    #[test]
    fn every_reason_reads_as_a_sentence_in_the_user_language() {
        let localizer = localizer();
        let reasons = [
            Unusable::Missing,
            Unusable::NotAFile {
                kind: PathKind::Directory,
            },
            Unusable::NotAFile {
                kind: PathKind::Other,
            },
            Unusable::Unreadable {
                detail: "permission denied".to_owned(),
            },
            Unusable::Undecodable {
                detail: "unexpected end of file".to_owned(),
            },
            Unusable::TooLarge {
                size: Size::new(20_000, 20_000),
                pixels: 400_000_000,
                limit: 33_177_600,
            },
            Unusable::TooLongSide {
                size: Size::new(40_000, 8),
                limit: 32_768,
            },
            Unusable::Degenerate {
                image: Size::new(0, 10),
                output: OUTPUT,
            },
            Unusable::TooManyTiles {
                tile: Size::new(1, 1),
                tiles: 921_600,
                limit: 1024,
            },
        ];

        for reason in reasons {
            let rendered = super::reason(&localizer, &reason);

            assert!(!rendered.is_empty());
            assert!(
                !MESSAGES.contains(&rendered.as_str()),
                "`{rendered}` is still a message id"
            );
            assert!(
                !rendered.contains('$'),
                "`{rendered}` still has an unsubstituted placeholder"
            );
        }
    }

    #[test]
    fn every_phase_reads_as_a_sentence_in_the_user_language() {
        for kind in [
            PhaseKind::Loading,
            PhaseKind::Ready,
            PhaseKind::Degraded,
            PhaseKind::Idle,
        ] {
            for localizer in [localizer(), chinese()] {
                let rendered = phase_line(&localizer, kind);

                assert!(!rendered.is_empty(), "{kind:?}");
                assert!(
                    !MESSAGES.contains(&rendered.as_str()),
                    "`{rendered}` is still a message id"
                );
                assert!(
                    !rendered.contains('$'),
                    "`{rendered}` still has an unsubstituted placeholder"
                );
            }
        }
    }

    #[test]
    fn the_card_always_names_the_scaling_the_output_uses() {
        // The mode is the one part of the geometry a user can act on, so it has
        // to be readable rather than only computed.
        let localizer = localizer();
        let modes = [
            (WallpaperMode::Fill, "fill"),
            (WallpaperMode::Fit, "fit"),
            (WallpaperMode::Stretch, "stretch"),
            (WallpaperMode::Center, "center"),
            (WallpaperMode::Tile, "tile"),
        ];

        let mut seen: Vec<String> = Vec::new();
        for (mode, name) in modes {
            let line = scaling_line(&localizer, mode);

            assert_eq!(line, format!("Scaling: {name}"));
            assert!(!seen.contains(&line), "`{line}` is not a distinct mode");
            seen.push(line);

            let model = project(&state_with_mode(Selection::ThemeBackground, mode));
            assert_eq!(model.card.scaling, format!("Scaling: {name}"));
        }
    }

    #[test]
    fn the_scaling_mode_is_named_in_the_reader_language() {
        let chinese = chinese();

        assert_eq!(
            scaling_line(&chinese, WallpaperMode::Tile),
            "缩放：平铺",
            "the mode name is copy, not the raw configuration value"
        );
        assert_eq!(phase_line(&chinese, PhaseKind::Idle), "无图像");
    }

    #[test]
    fn the_word_for_a_path_that_is_not_a_file_comes_from_the_catalog() {
        // The defect this covers: the kind of path used to be a Rust literal
        // handed to the message as an argument, so a zh-CN reader of a directory
        // read "该路径是a directory，不是文件" — half a sentence in each language.
        let chinese = chinese();

        for (kind, id) in [
            (PathKind::Directory, "wallpaper-reason-not-a-file-directory"),
            (PathKind::Other, "wallpaper-reason-not-a-file-other"),
        ] {
            let rendered = super::reason(&chinese, &Unusable::NotAFile { kind });

            assert_eq!(rendered, chinese.message(id));
            assert!(
                !rendered.chars().any(|c| c.is_ascii_alphabetic()),
                "`{rendered}` still carries a word the code substituted in"
            );
        }
    }

    #[test]
    fn a_directory_selected_by_a_chinese_reader_is_reported_in_chinese() {
        // The reported defect end to end: the state, the real filesystem check,
        // the real preparation and the real projection. Before the fix this read
        // "该路径是a directory，不是文件".
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("walls");
        std::fs::create_dir(&path).expect("the fixture is created");

        let mut state = State::new(
            Arc::new(Theme::builtin_dark()),
            Rc::new(chinese()),
            Preview {
                output: OutputName::from("HDMI-A-1"),
                size: OUTPUT,
                mode: WallpaperMode::Fill,
                selection: Selection::Image(path.clone()),
                wallpaper_dirs: Arc::from(Vec::<PathBuf>::new()),
            },
            None,
        );
        let request = state.take_request().expect("a request");
        let preparation = apply::prepare(
            request.output,
            &Selection::Image(path),
            WallpaperMode::Fill,
            OUTPUT,
            &[],
        );
        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: request.generation,
                preparation,
            },
        );

        let notice = project(&state).notice.expect("a notice");

        assert_eq!(notice.reason.as_deref(), Some("该路径是目录，不是文件"));
    }

    #[test]
    fn a_reason_for_a_side_that_is_too_long_names_that_bound() {
        // The other half of the same principle, about numbers rather than words:
        // 40 000 × 8 is 320 000 pixels, so a reason that quotes the pixel bound
        // tells the user about a limit the image is nowhere near.
        let rendered = super::reason(
            &localizer(),
            &Unusable::TooLongSide {
                size: Size::new(40_000, 8),
                limit: 32_768,
            },
        );

        assert!(
            rendered.contains("40000×8") || rendered.contains("40,000×8"),
            "`{rendered}` should carry the size"
        );
        assert!(
            rendered.contains("32768") || rendered.contains("32,768"),
            "`{rendered}` should carry the side bound"
        );
        assert!(
            !rendered.contains("33177600") && !rendered.contains("33,177,600"),
            "`{rendered}` quotes the pixel bound the image does not exceed"
        );
    }

    #[test]
    fn a_reason_that_counts_something_shows_the_numbers() {
        let localizer = localizer();
        let rendered = super::reason(
            &localizer,
            &Unusable::TooLarge {
                size: Size::new(20_000, 20_000),
                pixels: 400_000_000,
                limit: 33_177_600,
            },
        );

        assert!(
            rendered.contains("400000000") || rendered.contains("400,000,000"),
            "`{rendered}` should carry the pixel count"
        );
        assert!(
            rendered.contains("33177600") || rendered.contains("33,177,600"),
            "`{rendered}` should carry the limit"
        );
    }

    #[test]
    fn a_configuration_that_could_not_be_read_disables_the_control_with_a_reason() {
        let state = State::new(
            Arc::new(Theme::builtin_dark()),
            Rc::new(localizer()),
            Preview {
                output: OutputName::from("HDMI-A-1"),
                size: OUTPUT,
                mode: WallpaperMode::Fill,
                selection: Selection::ThemeBackground,
                wallpaper_dirs: Arc::from(Vec::<PathBuf>::new()),
            },
            Some(Unusable::Unreadable {
                detail: "permission denied".to_owned(),
            }),
        );

        let model = project(&state);
        let notice = model.notice.expect("a notice");

        assert_eq!(notice.kind, NoticeKind::Problem);
        assert!(
            notice
                .heading
                .starts_with("The configuration could not be read"),
            "{:?}",
            notice.heading
        );
        assert!(notice.heading.contains("permission denied"));
        assert!(notice.remedy.is_some());
        assert!(!notice.action.enabled);
        assert_eq!(
            notice.action.reason.as_deref(),
            Some("The configuration is read once, when the component starts.")
        );
        assert_eq!(
            model.card.phase.as_ref().map(|phase| phase.kind),
            Some(PhaseKind::Degraded),
            "nothing is applied, and the reason is in the notice"
        );
        assert!(model.focus_order.is_empty(), "nothing to tab to");
    }

    #[test]
    fn a_reload_in_flight_leaves_the_wallpaper_up_and_says_nothing_new() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut state = loaded(dir.path(), 64, 64);
        let before = project(&state);

        let _ = update(&mut state, Message::Reload);
        assert!(state.is_busy());

        let after = project(&state);

        // Nothing on screen changes while a reload runs: no gap, and no second
        // notice about the load the user already sees the result of. What does
        // change is the phase, because "in progress" has to be visible.
        assert_eq!(before.wallpaper, after.wallpaper);
        assert!(after.wallpaper.is_some());
        assert!(after.notice.is_none());
        assert_eq!(
            before.card.phase.as_ref().map(|phase| phase.kind),
            Some(PhaseKind::Ready)
        );
        assert_eq!(
            after.card.phase.as_ref().map(|phase| phase.kind),
            Some(PhaseKind::Loading)
        );
    }

    #[test]
    fn a_retry_after_a_failure_shows_the_load_running_beside_the_problem() {
        // The user has just pressed "Try again" with a failure still on screen:
        // the failure stays readable and the load in progress is visible
        // underneath it (UI spec §5).
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut state = failed(dir.path(), Unusable::Missing);
        let _ = update(&mut state, Message::Reload);
        assert!(state.is_busy());

        let model = project(&state);

        assert_eq!(
            model.notice.as_ref().map(|notice| notice.kind),
            Some(NoticeKind::Problem),
            "what failed is still readable"
        );
        assert_eq!(
            model.card.phase.as_ref().map(|phase| phase.kind),
            Some(PhaseKind::Loading),
            "and so is the retry that is running"
        );
    }

    #[test]
    fn a_rejected_image_keeps_the_wallpaper_that_was_already_up() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let mut state = loaded(dir.path(), 64, 64);
        let before = project(&state).wallpaper.expect("a wallpaper");

        let _ = update(&mut state, Message::Reload);
        let reload = state.generation();
        let _ = update(
            &mut state,
            Message::FramePrepared {
                generation: reload,
                preparation: apply::Preparation::Degraded {
                    output: OutputName::from("HDMI-A-1"),
                    path: PathBuf::from("/walls/gone.png"),
                    reason: Unusable::Missing,
                },
            },
        );

        let model = project(&state);
        let after = model.wallpaper.expect("the old wallpaper is still up");

        assert_eq!(before, after, "nothing was cleared while reporting");
        let notice = model.notice.expect("a notice");
        assert_eq!(notice.reason.as_deref(), Some("the file does not exist"));
        assert!(notice.action.enabled, "and it can be retried");
        assert_eq!(
            model.card.phase.as_ref().map(|phase| phase.kind),
            Some(PhaseKind::Ready),
            "the frame the output is drawing is still the one in effect"
        );
        assert_eq!(
            state.last(),
            Some(&CommandResult::Rejected {
                reason: Unusable::Missing
            })
        );
    }

    #[test]
    fn the_status_line_describes_the_image_and_its_path() {
        let localizer = localizer();
        let dir = tempfile::tempdir().expect("a temporary directory");
        let state = loaded(dir.path(), 640, 480);
        let status = state
            .snapshot()
            .output(&OutputName::from("HDMI-A-1"))
            .expect("the output is known");
        let line = summary_line(&localizer, status);
        let source = source_line(&localizer, status).expect("an image has a source");

        assert!(line.contains("640×480"), "{line}");
        assert!(source.contains("wall.png"), "{source}");
        assert_eq!(
            summary_line(&localizer, &OutputStatus::background()),
            "The theme background"
        );
        assert_eq!(
            source_line(&localizer, &OutputStatus::background()),
            None,
            "the background has no path to name"
        );
    }

    #[test]
    fn a_notice_never_renders_a_message_id() {
        // Every heading, subject, reason and remedy the projection can produce
        // must be real text (acceptance Q03).
        let dir = tempfile::tempdir().expect("a temporary directory");
        let state = failed(
            dir.path(),
            Unusable::Undecodable {
                detail: "not a PNG".to_owned(),
            },
        );

        let model = project(&state);
        let notice = model.notice.expect("a notice");

        for part in [
            Some(notice.heading.as_str()),
            notice.subject.as_deref(),
            notice.reason.as_deref(),
            notice.remedy.as_deref(),
            Some(notice.action.label.as_str()),
        ]
        .into_iter()
        .flatten()
        {
            assert!(!MESSAGES.contains(&part), "`{part}` is still a message id");
            assert!(!part.is_empty());
        }
        assert_eq!(model.title, "Inox wallpaper");
        assert!(model.footer.is_some());
    }

    #[test]
    fn every_card_line_is_real_text_in_both_catalogs() {
        // The status card is on screen in every scenario, so every line of it
        // has to be copy rather than an id, in both languages.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let scenarios = [
            project(&loaded(dir.path(), 64, 64)),
            project(&state(Selection::ThemeBackground)),
            project(&state(Selection::Image(PathBuf::from("/walls/wall.png")))),
        ];

        for model in scenarios {
            for line in [
                Some(model.card.heading.as_str()),
                model.card.summary.as_deref(),
                model.card.source.as_deref(),
                Some(model.card.scaling.as_str()),
                model.card.phase.as_ref().map(|phase| phase.text.as_str()),
                model.card.hint.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                assert!(!MESSAGES.contains(&line), "`{line}` is still a message id");
                assert!(!line.is_empty());
            }
        }
    }

    #[test]
    fn the_same_state_projects_the_same_view_model() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let state = loaded(dir.path(), 64, 64);

        assert_eq!(project(&state), project(&state));
    }
}
