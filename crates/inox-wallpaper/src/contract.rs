//! The state contract of ADR 0013, as data.
//!
//! The contract asks every L3 component to model the same four things, so that a
//! component that is restarted, replaced or reconnected behaves like one that is
//! merely busy. Three of them have a shape here:
//!
//! - **A generation** ([`Generation`]) orders requests, so a result that belongs
//!   to a request a newer one has superseded cannot overwrite it — the rule
//!   acceptance C02 states as "a stale completion must not overwrite a newer
//!   request", and the reason [`State`](crate::app::State) keeps one.
//! - **A per-output result** ([`CommandResult`]) instead of one global success,
//!   so that "three outputs are up, the fourth fell back to the theme
//!   background" is expressible. A result is never optimistic: it is written
//!   when the frame is in place, not when the request was sent.
//! - **A service state** ([`ServiceState`]) that distinguishes *starting*,
//!   *ready* and *degraded with a reason*, which is what keeps the interface from
//!   spinning forever on a failure (acceptance D05).
//!
//! A snapshot ([`Snapshot`]) is the read-only half: what every output is showing
//! right now, with the generation it belongs to. It is the type a consumer reads
//! when it connects, before it subscribes to anything — and it is owned rather
//! than borrowed so that it can cross a process boundary unchanged once the
//! `inox-ipc` contract lands.
//!
//! There is deliberately no `sent`/`accepted` pair here. The wallpaper has no
//! queue between accepting a request and applying it — the same host call builds
//! the frame and swaps it in — so a state that could never be observed would be
//! a lie in the type system rather than a model of anything.

use std::collections::BTreeMap;

use crate::decode::FrameHandle;
use crate::model::{OutputName, Size, Unusable};
use crate::scaling::Placement;

/// How many times a request has been made.
///
/// Monotonic and never reused: a result that names a generation older than the
/// newest one is stale, and stale results are dropped rather than applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Generation(u64);

impl Generation {
    /// The first generation a session ever uses.
    pub const FIRST: Self = Self(1);

    /// The generation after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0 + 1)
    }

    /// The generation as a number, for logs and tests.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for Generation {
    fn default() -> Self {
        Self::FIRST
    }
}

/// What became of one output's request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResult {
    /// The frame is in place on that output.
    Applied,
    /// The output kept what it had, and the reason is the diagnosis.
    Rejected {
        /// Why the new frame could not be used.
        reason: Unusable,
    },
    /// The result belongs to a request a newer one has superseded, so it was
    /// dropped: applying it would show the user something they have already
    /// asked to replace.
    Stale {
        /// The generation the dropped result belonged to.
        generation: Generation,
        /// The generation that superseded it.
        current: Generation,
    },
}

impl CommandResult {
    /// Whether the output is showing the frame the request asked for.
    #[must_use]
    pub const fn is_applied(&self) -> bool {
        matches!(self, Self::Applied)
    }

    /// The reason this result is not an applied one, when it is not.
    #[must_use]
    pub const fn reason(&self) -> Option<&Unusable> {
        match self {
            Self::Rejected { reason } => Some(reason),
            Self::Applied | Self::Stale { .. } => None,
        }
    }
}

/// What the component as a whole is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceState {
    /// The first load has not produced a frame yet. The wallpaper itself is not
    /// visible in this state; nothing else is blocked by it.
    Starting,
    /// Every output that has a wallpaper is showing it. Outputs with nothing
    /// configured are showing the theme background, which is not a degradation.
    Ready,
    /// Something the user asked for could not be used. The session stays
    /// operable — the output falls back to the theme background — and the reason
    /// is readable, which is what keeps a failure from being a blank screen.
    Degraded {
        /// Why the component is not in its intended state.
        reason: Unusable,
    },
}

impl ServiceState {
    /// Whether the component finished what it set out to do.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// The reason this state is degraded, when it is.
    #[must_use]
    pub const fn reason(&self) -> Option<&Unusable> {
        match self {
            Self::Degraded { reason } => Some(reason),
            Self::Starting | Self::Ready => None,
        }
    }
}

/// What an output is drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Showing {
    /// A decoded image, and where on the output it goes.
    Image {
        /// The file it was decoded from, so the interface can name it.
        path: String,
        /// The image's own size, before the placement scales it.
        size: Size,
        /// The pixels, in a refcounted frame handle.
        ///
        /// A [`FrameHandle`] rather than a byte slice because that is what draws
        /// a frame without copying it: it is built once, when the frame is
        /// applied, and cloned (a reference count) every time it is drawn.
        image: FrameHandle,
        /// The geometry the image is drawn with.
        placement: Placement,
    },
    /// The theme background. Either nothing was configured — the documented
    /// default — or the image that was asked for could not be used, which
    /// [`Problem`] says separately.
    Background,
}

impl Showing {
    /// The image on this output, if there is one.
    #[must_use]
    pub const fn image(&self) -> Option<&FrameHandle> {
        match self {
            Self::Image { image, .. } => Some(image),
            Self::Background => None,
        }
    }

    /// The size the image was decoded at, if there is one.
    #[must_use]
    pub const fn size(&self) -> Option<Size> {
        match self {
            Self::Image { size, .. } => Some(*size),
            Self::Background => None,
        }
    }

    /// The file the image on this output came from, if there is one.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        match self {
            Self::Image { path, .. } => Some(path),
            Self::Background => None,
        }
    }

    /// Whether this output is drawing the theme background.
    #[must_use]
    pub const fn is_background(&self) -> bool {
        matches!(self, Self::Background)
    }
}

/// A request that could not be honoured.
///
/// Kept beside what the output is drawing rather than replacing it, because the
/// two are independent: a wallpaper that is already up stays up while the
/// interface explains why the *new* one did not load.
/// Replacing a working frame with a blank one to report a failure would be the
/// worse outcome of the two, and it is the outcome this shape is chosen to make
/// impossible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The image that was asked for, when the request named one.
    pub path: Option<String>,
    /// Why it could not be used.
    pub reason: Unusable,
}

/// What one output is showing, and what went wrong the last time it was asked to
/// change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputStatus {
    /// The pixels on screen right now.
    pub showing: Showing,
    /// The last request this output did not honour, if there is one.
    pub problem: Option<Problem>,
}

impl OutputStatus {
    /// An output drawing the theme background with nothing to report.
    #[must_use]
    pub const fn background() -> Self {
        Self {
            showing: Showing::Background,
            problem: None,
        }
    }

    /// An output drawing the theme background for a reason worth reading.
    #[must_use]
    pub fn unavailable(path: Option<String>, reason: Unusable) -> Self {
        Self {
            showing: Showing::Background,
            problem: Some(Problem { path, reason }),
        }
    }

    /// The image on this output, if there is one.
    #[must_use]
    pub const fn image(&self) -> Option<&FrameHandle> {
        self.showing.image()
    }

    /// The size the image on this output was decoded at, if there is one.
    #[must_use]
    pub const fn size(&self) -> Option<Size> {
        self.showing.size()
    }

    /// Why this output is not showing what was asked for, if it is not.
    #[must_use]
    pub const fn reason(&self) -> Option<&Unusable> {
        match &self.problem {
            Some(problem) => Some(&problem.reason),
            None => None,
        }
    }

    /// Whether the last request for this output was not honoured.
    #[must_use]
    pub const fn is_degraded(&self) -> bool {
        self.problem.is_some()
    }
}

/// What every output is showing at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    generation: Generation,
    outputs: BTreeMap<OutputName, OutputStatus>,
}

impl Snapshot {
    /// A snapshot with nothing on any output: what a component looks like before
    /// its first frame, and what it looks like when no output is known.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            generation: Generation::FIRST,
            outputs: BTreeMap::new(),
        }
    }

    /// The generation this snapshot belongs to.
    #[must_use]
    pub const fn generation(&self) -> Generation {
        self.generation
    }

    /// Moves the snapshot to a newer generation.
    ///
    /// Only forward: a snapshot that has been through generation 4 cannot be
    /// talked back into 3 by a late answer.
    pub fn advance(&mut self, generation: Generation) {
        if generation > self.generation {
            self.generation = generation;
        }
    }

    /// Every output and what it is showing, in output-name order.
    #[must_use]
    pub fn outputs(&self) -> &BTreeMap<OutputName, OutputStatus> {
        &self.outputs
    }

    /// What one output is showing, if it is known at all.
    #[must_use]
    pub fn output(&self, output: &OutputName) -> Option<&OutputStatus> {
        self.outputs.get(output)
    }

    /// Records what an output is showing.
    ///
    /// The previous value is dropped *after* the new one is in the map, which is
    /// what makes the swap atomic from the point of view of anyone holding a
    /// reference to the snapshot: the frame they are drawing stays alive until
    /// the replacement is in place, so there is no moment without a frame.
    pub fn set(&mut self, output: OutputName, status: OutputStatus) {
        self.outputs.insert(output, status);
    }

    /// How many outputs are known.
    #[must_use]
    pub fn len(&self) -> usize {
        self.outputs.len()
    }

    /// Whether no output is known.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    /// Forgets an output that no longer exists, for example an unplugged one.
    ///
    /// Only the output's own entry is removed: a display being unplugged says
    /// nothing about the others (acceptance O02).
    pub fn forget(&mut self, output: &OutputName) -> Option<OutputStatus> {
        self.outputs.remove(output)
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Size;

    /// A decoded-looking image: the pixels are built the way
    /// [`crate::apply`] builds them, so that equality of two handles means
    /// identity of the frame they came from.
    fn an_image(output: &str, width: u32, height: u32) -> (OutputName, OutputStatus) {
        let name = OutputName::from(output);
        let status = OutputStatus {
            showing: Showing::Image {
                path: "/walls/wall.png".to_owned(),
                size: Size::new(width, height),
                image: crate::decode::Frame::new(
                    width,
                    height,
                    vec![0; (width * height * 4) as usize],
                )
                .expect("a well-formed frame")
                .into_handle(),
                placement: crate::scaling::place(
                    inox_config::WallpaperMode::Fill,
                    Size::new(width, height),
                    Size::new(1920, 1080),
                )
                .expect("a placement"),
            },
            problem: None,
        };

        (name, status)
    }

    #[test]
    fn the_first_generation_is_the_one_a_session_starts_at() {
        assert_eq!(Generation::default(), Generation::FIRST);
        assert_eq!(Generation::FIRST.get(), 1);
    }

    #[test]
    fn generations_only_ever_increase() {
        let first = Generation::FIRST;

        assert_eq!(first.next(), Generation(first.get() + 1));
        assert!(first.next() > first);
        assert!(Generation::FIRST < Generation::FIRST.next().next());
    }

    #[test]
    fn a_snapshot_starts_empty_at_the_first_generation() {
        let snapshot = Snapshot::empty();

        assert!(snapshot.is_empty());
        assert_eq!(snapshot.len(), 0);
        assert_eq!(snapshot.generation(), Generation::FIRST);
        assert_eq!(snapshot.output(&OutputName::from("HDMI-A-1")), None);
        assert_eq!(Snapshot::default(), snapshot);
    }

    #[test]
    fn a_snapshot_advances_forward_and_never_back() {
        let mut snapshot = Snapshot::empty();
        let fourth = Generation::FIRST.next().next().next();

        snapshot.advance(fourth);
        assert_eq!(snapshot.generation(), fourth);

        // A late answer from an older request must not move it back.
        snapshot.advance(Generation::FIRST);
        assert_eq!(snapshot.generation(), fourth);
        snapshot.advance(fourth);
        assert_eq!(snapshot.generation(), fourth);
    }

    #[test]
    fn an_output_keeps_its_frame_when_another_one_changes() {
        let (first, first_status) = an_image("HDMI-A-1", 640, 360);
        let (second, second_status) = an_image("DP-1", 800, 600);

        let mut snapshot = Snapshot::empty();
        snapshot.set(first.clone(), first_status.clone());
        snapshot.set(second.clone(), second_status.clone());

        let held = snapshot
            .output(&first)
            .and_then(OutputStatus::image)
            .cloned()
            .expect("the first output has an image");

        // Replacing one output must leave the other byte-for-byte alone: the
        // same handle, not an equal one (acceptance D05 and O02).
        snapshot.set(
            second.clone(),
            OutputStatus::unavailable(Some("/walls/gone.png".to_owned()), Unusable::Missing),
        );

        let after = snapshot
            .output(&first)
            .and_then(OutputStatus::image)
            .cloned()
            .expect("the first output still has an image");

        assert_eq!(held, after, "the frame must not be rebuilt");
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.output(&second).expect("the output").is_degraded());
    }

    #[test]
    fn the_previous_frame_outlives_the_swap() {
        let (output, status) = an_image("HDMI-A-1", 64, 64);
        let mut snapshot = Snapshot::empty();
        snapshot.set(output.clone(), status);

        let held = snapshot
            .output(&output)
            .and_then(OutputStatus::image)
            .cloned()
            .expect("an image");

        let (_, replacement) = an_image("HDMI-A-1", 128, 128);
        snapshot.set(output.clone(), replacement);

        // Whoever is drawing the old frame still can: it was not released when
        // the new one went in. The handle is the whole of the frame's pixels —
        // there is no second buffer behind it — so holding it kept the frame
        // alive across the swap, which is why no frame is ever half-drawn.
        assert_eq!(
            snapshot.output(&output).and_then(OutputStatus::size),
            Some(Size::new(128, 128)),
            "the new frame is in place"
        );
        assert_ne!(
            held,
            *snapshot
                .output(&output)
                .expect("known")
                .image()
                .expect("an image"),
            "the frame that was replaced is a different one, and is still ours"
        );
    }

    #[test]
    fn forgetting_one_output_leaves_the_others() {
        let (first, first_status) = an_image("HDMI-A-1", 64, 64);
        let (second, second_status) = an_image("DP-1", 64, 64);

        let mut snapshot = Snapshot::empty();
        snapshot.set(first.clone(), first_status);
        snapshot.set(second.clone(), second_status);

        assert!(snapshot.forget(&first).is_some());
        assert_eq!(snapshot.len(), 1);
        assert!(snapshot.output(&first).is_none());
        assert!(snapshot.output(&second).is_some());
        assert!(
            snapshot.forget(&first).is_none(),
            "forgetting twice is not an error"
        );
    }

    #[test]
    fn an_applied_result_is_the_only_one_that_means_success() {
        let applied = CommandResult::Applied;
        assert!(applied.is_applied());
        assert_eq!(applied.reason(), None);

        let rejected = CommandResult::Rejected {
            reason: Unusable::Missing,
        };
        assert!(!rejected.is_applied());
        assert_eq!(rejected.reason(), Some(&Unusable::Missing));

        let stale = CommandResult::Stale {
            generation: Generation::FIRST,
            current: Generation::FIRST.next(),
        };
        assert!(!stale.is_applied());
        assert_eq!(
            stale.reason(),
            None,
            "stale is not a rejection of the image"
        );
    }

    #[test]
    fn a_service_is_starting_ready_or_degraded_with_a_reason() {
        assert!(!ServiceState::Starting.is_ready());
        assert_eq!(ServiceState::Starting.reason(), None);

        assert!(ServiceState::Ready.is_ready());
        assert_eq!(ServiceState::Ready.reason(), None);

        let degraded = ServiceState::Degraded {
            reason: Unusable::Missing,
        };
        assert!(!degraded.is_ready());
        assert_eq!(degraded.reason(), Some(&Unusable::Missing));
    }

    #[test]
    fn a_background_without_a_reason_is_a_default_and_not_a_degradation() {
        let background = OutputStatus::background();

        assert!(!background.is_degraded());
        assert_eq!(background.image(), None);
        assert_eq!(background.size(), None);
        assert_eq!(background.reason(), None);
        assert!(background.showing.is_background());
    }

    #[test]
    fn a_failure_is_reported_beside_what_is_on_screen_not_instead_of_it() {
        // The wallpaper that is up stays up; the diagnosis is a second fact.
        let (_, mut status) = an_image("HDMI-A-1", 64, 64);
        let before = status.image().cloned().expect("an image");

        status.problem = Some(Problem {
            path: Some("/walls/gone.png".to_owned()),
            reason: Unusable::Missing,
        });

        assert_eq!(status.image(), Some(&before), "the frame is still there");
        assert!(status.is_degraded());
        assert_eq!(status.reason(), Some(&Unusable::Missing));
        assert_eq!(status.showing.path(), Some("/walls/wall.png"));
    }

    #[test]
    fn an_image_status_reports_its_frame_and_no_reason() {
        let (_, status) = an_image("HDMI-A-1", 320, 200);

        assert!(!status.is_degraded());
        assert_eq!(status.reason(), None);
        assert_eq!(status.size(), Some(Size::new(320, 200)));
        assert!(status.image().is_some());
    }

    #[test]
    fn the_snapshot_reads_back_in_output_name_order() {
        let mut snapshot = Snapshot::empty();
        for name in ["eDP-1", "HDMI-A-1", "DP-2"] {
            let (output, status) = an_image(name, 64, 64);
            snapshot.set(output, status);
        }

        let names: Vec<&str> = snapshot.outputs().keys().map(OutputName::as_str).collect();

        assert_eq!(names, ["DP-2", "HDMI-A-1", "eDP-1"]);
    }
}
