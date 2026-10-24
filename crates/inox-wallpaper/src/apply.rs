//! Getting a frame onto an output without a gap.
//!
//! The property this module exists for is one sentence long: **the new frame is
//! complete before the previous one is released**. It is achieved by splitting
//! the work in two —
//!
//! 1. [`prepare`] does everything that can fail or take time: resolve the path,
//!    check it, decode it under the memory bound, compute the geometry. It
//!    produces a [`Prepared`] frame or a diagnosis, and it never touches what is
//!    on screen.
//! 2. [`settle`] does the only thing left: one map entry. The old frame is
//!    dropped *after* the new one is in place, so no reference to it ever
//!    observes a missing frame, and there is no moment in which the output would
//!    be blank.
//!
//! Both halves are per output. One output failing leaves the others exactly as
//! they were — the same pixels, not an equal copy — and a failing output keeps
//! *its own* previous frame, with the diagnosis recorded beside it
//! ([`OutputStatus`]) rather than in place of it. That is the model of ADR 0013's
//! snapshot/apply contract, and it is what acceptance D05 asks for when it says
//! a degraded resource must remain operable.

use std::path::PathBuf;

use iced::widget::image::Handle;
use inox_config::WallpaperMode;

use crate::contract::{CommandResult, OutputStatus, Problem, Showing, Snapshot};
use crate::decode::{self, Frame};
use crate::model::{OutputName, Selection, Size, Unusable};
use crate::scaling::{self, Placement};
use crate::source::{self, OutputSource};

/// A frame that is decoded and placed, and therefore ready to be swapped in.
///
/// Nothing about this value can fail: by the time one exists, the file was read,
/// the pixels were decoded within the bound, and the geometry was computed. That
/// is the whole reason it is a distinct type from a path and a mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    output: OutputName,
    path: PathBuf,
    frame: Frame,
    placement: Placement,
}

impl Prepared {
    /// The output this frame belongs to.
    #[must_use]
    pub fn output(&self) -> &OutputName {
        &self.output
    }

    /// The file the pixels were decoded from.
    #[must_use]
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// The pixels, still in the decoder's buffer.
    ///
    /// [`settle`] moves this buffer into the renderer's own handle; nothing
    /// copies it, which is why the type is not `Arc`.
    #[must_use]
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// Where the frame goes on the output.
    #[must_use]
    pub fn placement(&self) -> &Placement {
        &self.placement
    }
}

/// The result of preparing one output.
///
/// [`Clone`] because the widget toolkit requires every message to be cloneable,
/// and this is one. Nothing in the component clones a message: a prepared frame
/// is moved into [`settle`], which moves its buffer into the renderer's handle,
/// so no pixel buffer is ever copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preparation {
    /// A complete frame, ready to be swapped in.
    Ready(Box<Prepared>),
    /// Nothing was configured for this output: the theme background is the
    /// answer, and there is nothing to report.
    Nothing {
        /// The output.
        output: OutputName,
    },
    /// Something was configured for this output and cannot be used. The output
    /// degrades to the theme background *if it has nothing better on screen*,
    /// and the user is told.
    Degraded {
        /// The output.
        output: OutputName,
        /// The image that was asked for.
        path: PathBuf,
        /// Why it cannot be used.
        reason: Unusable,
    },
}

/// Decodes and places the image `selection` names, for an output of `size`.
///
/// This is the I/O half: it reads the filesystem and allocates the pixel buffer,
/// so it belongs on the host's worker rather than in
/// [`update`](crate::app::update). Anything it cannot do comes back as a
/// [`Preparation::Degraded`] with a reason, never as an error the caller has to
/// turn into one — the degradation *is* the designed outcome.
///
/// `wallpaper_dirs` are the directories a bundled wallpaper is searched for, in
/// order; a selection that names a file ignores them.
#[must_use]
pub fn prepare(
    output: OutputName,
    selection: &Selection,
    mode: WallpaperMode,
    size: Size,
    wallpaper_dirs: &[PathBuf],
) -> Preparation {
    let path = match source::resolve(selection, wallpaper_dirs) {
        OutputSource::Image(path) => path,
        OutputSource::ThemeBackground => return Preparation::Nothing { output },
        OutputSource::Unavailable { path, reason } => {
            return Preparation::Degraded {
                output,
                path,
                reason,
            };
        }
    };

    let frame = match decode::decode(&path) {
        Ok(frame) => frame,
        Err(error) => {
            return Preparation::Degraded {
                output,
                path,
                reason: error.into(),
            };
        }
    };

    let placement = match scaling::plan(mode, frame.size(), size) {
        Ok(placement) => placement,
        Err(reason) => {
            return Preparation::Degraded {
                output,
                path,
                reason,
            };
        }
    };

    Preparation::Ready(Box::new(Prepared {
        output,
        path,
        frame,
        placement,
    }))
}

/// Records the result of [`prepare`] for one output.
///
/// The result is the output's fate, and it is not always "applied": an image the
/// user selected that could not be used is a *rejected* request even though the
/// output continues to work, and it is reported as such so that a caller which
/// counts successes is not lied to.
#[must_use]
pub fn settle(snapshot: &mut Snapshot, preparation: Preparation) -> CommandResult {
    match preparation {
        Preparation::Ready(prepared) => {
            let Prepared {
                output,
                path,
                frame,
                placement,
            } = *prepared;
            let image_size = frame.size();

            // The decoder's buffer *is* the renderer's buffer: the handle
            // takes ownership of it, so a frame that has been placed is never
            // copied and never exists twice.
            let image = Handle::from_rgba(frame.width(), frame.height(), frame.into_rgba());

            snapshot.set(
                output,
                OutputStatus {
                    showing: Showing::Image {
                        path: path.to_string_lossy().into_owned(),
                        size: image_size,
                        image,
                        placement,
                    },
                    problem: None,
                },
            );

            CommandResult::Applied
        }
        Preparation::Nothing { output } => {
            snapshot.set(output, OutputStatus::background());

            CommandResult::Applied
        }
        Preparation::Degraded {
            output,
            path,
            reason,
        } => {
            let path = path.to_string_lossy().into_owned();
            let previous = snapshot.output(&output);

            // A frame that is already up stays up: reporting a failure by
            // blanking the screen would be a second failure.
            let showing = match previous {
                Some(status) => status.showing.clone(),
                None => Showing::Background,
            };

            snapshot.set(
                output,
                OutputStatus {
                    showing,
                    problem: Some(Problem {
                        path: Some(path),
                        reason: reason.clone(),
                    }),
                },
            );

            CommandResult::Rejected { reason }
        }
    }
}

/// Forgets an output, which is what unplugging it means.
///
/// Returns whether anything was forgotten, so a caller can tell "the output is
/// gone" from "it was never here" without asking twice.
pub fn removed(snapshot: &mut Snapshot, output: &OutputName) -> bool {
    snapshot.forget(output).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// [`prepare`] with no wallpaper search directories: every fixture here is a
    /// path, and only a bundled wallpaper is searched for.
    fn prepare(
        output: OutputName,
        selection: &Selection,
        mode: WallpaperMode,
        size: Size,
    ) -> Preparation {
        super::prepare(output, selection, mode, size, &[])
    }

    const OUTPUT: Size = Size::new(1920, 1080);
    const MODE: WallpaperMode = WallpaperMode::Fill;

    fn an_output_with_image(output: &str, width: u32, height: u32) -> (OutputName, OutputStatus) {
        let name = OutputName::from(output);
        let placement =
            scaling::place(MODE, Size::new(width, height), OUTPUT).expect("a placement");
        let status = OutputStatus {
            showing: Showing::Image {
                path: "/walls/wall.png".to_owned(),
                size: Size::new(width, height),
                image: Handle::from_rgba(width, height, vec![0; (width * height * 4) as usize]),
                placement,
            },
            problem: None,
        };

        (name, status)
    }

    fn a_png(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.join(name);
        image::RgbaImage::from_pixel(width, height, image::Rgba([9, 8, 7, 255]))
            .save(&path)
            .expect("the fixture is written");
        path
    }

    fn selected(path: PathBuf) -> Selection {
        Selection::Image(path)
    }

    #[test]
    fn a_decoded_image_is_prepared_complete() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = a_png(dir.path(), "wall.png", 640, 480);

        let preparation = prepare(
            OutputName::from("HDMI-A-1"),
            &selected(path.clone()),
            MODE,
            OUTPUT,
        );

        let Preparation::Ready(prepared) = preparation else {
            panic!("expected a prepared frame, got {preparation:?}");
        };

        assert_eq!(prepared.output().as_str(), "HDMI-A-1");
        assert_eq!(prepared.path(), &path);
        assert_eq!(prepared.frame().size(), Size::new(640, 480));
        assert!(prepared.placement().first().covers(OUTPUT));
    }

    #[test]
    fn a_missing_file_is_prepared_as_a_degradation_that_names_the_path() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("absent.png");

        let preparation = prepare(
            OutputName::from("HDMI-A-1"),
            &selected(path.clone()),
            MODE,
            OUTPUT,
        );

        assert_eq!(
            preparation,
            Preparation::Degraded {
                output: OutputName::from("HDMI-A-1"),
                path,
                reason: Unusable::Missing,
            }
        );
    }

    #[test]
    fn a_truncated_file_is_prepared_as_a_degradation_and_not_as_a_panic() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let whole = a_png(dir.path(), "whole.png", 64, 64);
        let bytes = fs::read(&whole).expect("the fixture");
        let cut = dir.path().join("cut.png");
        fs::write(&cut, &bytes[..bytes.len() / 2]).expect("the fixture");

        match prepare(OutputName::from("eDP-1"), &selected(cut), MODE, OUTPUT) {
            Preparation::Degraded { reason, .. } => {
                assert!(matches!(reason, Unusable::Undecodable { .. }), "{reason:?}");
            }
            other => panic!("expected a degradation, got {other:?}"),
        }
    }

    #[test]
    fn nothing_configured_is_prepared_as_the_theme_background_and_is_not_a_failure() {
        let preparation = prepare(
            OutputName::from("eDP-1"),
            &Selection::ThemeBackground,
            MODE,
            OUTPUT,
        );

        assert_eq!(
            preparation,
            Preparation::Nothing {
                output: OutputName::from("eDP-1")
            }
        );
    }

    #[test]
    fn a_degenerate_geometry_is_prepared_as_a_degradation() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = a_png(dir.path(), "wall.png", 64, 64);

        match prepare(
            OutputName::from("eDP-1"),
            &selected(path),
            MODE,
            Size::new(0, 1080),
        ) {
            Preparation::Degraded { reason, .. } => {
                assert!(matches!(reason, Unusable::Degenerate { .. }))
            }
            other => panic!("expected a degradation, got {other:?}"),
        }
    }

    #[test]
    fn settling_a_prepared_frame_puts_it_on_its_output() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = a_png(dir.path(), "wall.png", 640, 480);
        let mut snapshot = Snapshot::empty();

        let result = settle(
            &mut snapshot,
            prepare(OutputName::from("HDMI-A-1"), &selected(path), MODE, OUTPUT),
        );

        assert_eq!(result, CommandResult::Applied);
        let status = snapshot
            .output(&OutputName::from("HDMI-A-1"))
            .expect("the output is known");

        assert!(!status.is_degraded());
        assert_eq!(status.size(), Some(Size::new(640, 480)));
        assert!(
            status
                .showing
                .path()
                .is_some_and(|p| p.ends_with("wall.png"))
        );
        assert!(status.image().is_some(), "the frame is ready to draw");
    }

    #[test]
    fn settling_nothing_configured_is_applied_as_the_theme_background() {
        let mut snapshot = Snapshot::empty();

        let result = settle(
            &mut snapshot,
            Preparation::Nothing {
                output: OutputName::from("eDP-1"),
            },
        );

        assert_eq!(result, CommandResult::Applied);
        let status = snapshot.output(&OutputName::from("eDP-1")).expect("known");
        assert!(status.showing.is_background());
        assert!(!status.is_degraded(), "a default is not a failure");
    }

    #[test]
    fn a_failing_output_leaves_the_other_outputs_frames_exactly_as_they_were() {
        let mut snapshot = Snapshot::empty();
        let (healthy, healthy_status) = an_output_with_image("HDMI-A-1", 800, 600);
        let (fragile, fragile_status) = an_output_with_image("DP-1", 400, 300);
        snapshot.set(healthy.clone(), healthy_status);
        snapshot.set(fragile.clone(), fragile_status);

        let held = snapshot
            .output(&healthy)
            .and_then(OutputStatus::image)
            .cloned()
            .expect("an image");

        let result = settle(
            &mut snapshot,
            Preparation::Degraded {
                output: fragile.clone(),
                path: PathBuf::from("/walls/gone.png"),
                reason: Unusable::Missing,
            },
        );

        assert_eq!(
            result,
            CommandResult::Rejected {
                reason: Unusable::Missing,
            }
        );

        let after = snapshot
            .output(&healthy)
            .and_then(OutputStatus::image)
            .cloned()
            .expect("the frame is still there");

        assert_eq!(held, after, "another output must not be touched");
        assert!(!snapshot.output(&healthy).expect("known").is_degraded());
    }

    #[test]
    fn a_failing_output_keeps_its_own_previous_frame() {
        let mut snapshot = Snapshot::empty();
        let (fragile, fragile_status) = an_output_with_image("DP-1", 400, 300);
        snapshot.set(fragile.clone(), fragile_status);

        let held = snapshot
            .output(&fragile)
            .and_then(OutputStatus::image)
            .cloned()
            .expect("an image");

        let result = settle(
            &mut snapshot,
            Preparation::Degraded {
                output: fragile.clone(),
                path: PathBuf::from("/walls/gone.png"),
                reason: Unusable::Missing,
            },
        );
        assert!(!result.is_applied(), "the request was not honoured");

        let status = snapshot.output(&fragile).expect("known");
        let after = status.image().cloned().expect("the old frame is still up");

        assert_eq!(held, after, "no gap: the frame stays");
        assert!(status.is_degraded());
        assert_eq!(status.reason(), Some(&Unusable::Missing));
    }

    #[test]
    fn a_failing_output_that_has_no_frame_falls_back_to_the_theme_background() {
        let mut snapshot = Snapshot::empty();
        let output = OutputName::from("HDMI-A-1");

        assert!(
            !settle(
                &mut snapshot,
                Preparation::Degraded {
                    output: output.clone(),
                    path: PathBuf::from("/walls/gone.png"),
                    reason: Unusable::Missing,
                },
            )
            .is_applied()
        );

        let status = snapshot.output(&output).expect("known");
        assert!(status.showing.is_background());
        assert_eq!(status.image(), None);
        assert!(status.is_degraded());
        assert_eq!(
            status.problem.as_ref().and_then(|p| p.path.as_deref()),
            Some("/walls/gone.png"),
            "the diagnosis names the path the user selected"
        );
    }

    #[test]
    fn a_later_success_clears_the_problem_of_an_earlier_failure() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = a_png(dir.path(), "wall.png", 128, 128);
        let output = OutputName::from("HDMI-A-1");
        let mut snapshot = Snapshot::empty();

        assert!(
            !settle(
                &mut snapshot,
                Preparation::Degraded {
                    output: output.clone(),
                    path: PathBuf::from("/walls/gone.png"),
                    reason: Unusable::Missing,
                },
            )
            .is_applied()
        );
        assert!(snapshot.output(&output).expect("known").is_degraded());

        assert_eq!(
            settle(
                &mut snapshot,
                prepare(output.clone(), &selected(path), MODE, OUTPUT),
            ),
            CommandResult::Applied
        );

        let status = snapshot.output(&output).expect("known");
        assert!(
            !status.is_degraded(),
            "the report goes away with the problem"
        );
        assert_eq!(status.size(), Some(Size::new(128, 128)));
    }

    #[test]
    fn the_frame_that_is_replaced_stays_alive_for_whoever_is_drawing_it() {
        let mut snapshot = Snapshot::empty();
        let output = OutputName::from("HDMI-A-1");
        let (_, status) = an_output_with_image("HDMI-A-1", 64, 64);
        snapshot.set(output.clone(), status);

        let held = snapshot
            .output(&output)
            .and_then(OutputStatus::image)
            .cloned()
            .expect("an image");

        let (_, replacement) = an_output_with_image("HDMI-A-1", 128, 128);
        snapshot.set(output.clone(), replacement);

        // The frame that was replaced is still a usable image for anyone who
        // was drawing it, and it is no longer the one on the output.
        assert_eq!(
            snapshot.output(&output).and_then(OutputStatus::size),
            Some(Size::new(128, 128))
        );
        assert_ne!(
            held,
            *snapshot
                .output(&output)
                .expect("known")
                .image()
                .expect("an image")
        );
    }

    #[test]
    fn removing_an_output_reports_whether_there_was_one() {
        let mut snapshot = Snapshot::empty();
        let (output, status) = an_output_with_image("HDMI-A-1", 64, 64);
        snapshot.set(output.clone(), status);

        assert!(removed(&mut snapshot, &output));
        assert!(!removed(&mut snapshot, &output));
        assert!(snapshot.is_empty());
    }

    #[test]
    fn a_bundled_default_wallpaper_is_prepared_from_its_directory() {
        let wallpaper = inox_config::wallpapers::default_bundled().expect("shipped");
        let dir = tempfile::tempdir().expect("a temporary directory");
        let file = dir.path().join(wallpaper.file);
        image::RgbImage::from_pixel(320, 200, image::Rgb([3, 4, 5]))
            .save(&file)
            .expect("the fixture is written");

        let preparation = super::prepare(
            OutputName::from("HDMI-A-1"),
            &Selection::Bundled {
                wallpaper,
                explicit: false,
            },
            MODE,
            OUTPUT,
            &[dir.path().to_path_buf()],
        );

        let Preparation::Ready(prepared) = preparation else {
            panic!("expected the bundled wallpaper to be prepared, got {preparation:?}");
        };
        assert_eq!(prepared.path(), &file);
        assert_eq!(prepared.frame().size(), Size::new(320, 200));
    }

    #[test]
    fn the_default_wallpaper_absent_is_prepared_as_nothing_to_report() {
        let wallpaper = inox_config::wallpapers::default_bundled().expect("shipped");
        let dir = tempfile::tempdir().expect("a temporary directory");

        let preparation = super::prepare(
            OutputName::from("eDP-1"),
            &Selection::Bundled {
                wallpaper,
                explicit: false,
            },
            MODE,
            OUTPUT,
            &[dir.path().to_path_buf()],
        );

        assert_eq!(
            preparation,
            Preparation::Nothing {
                output: OutputName::from("eDP-1")
            }
        );
    }
}
