//! The server half of `inox-glass-v1`, Inox's frosted-glass (backdrop blur)
//! protocol: the parameters, the decisions and the pane list the renderer eats.
//!
//! The normative text is `crates/inox-protocols/protocol/inox-glass-v1.xml` and
//! the bindings are [`inox_protocols::glass`]. This module holds only the *pure*
//! part — no Wayland objects and no GL — so the semantics can be unit-tested
//! without a compositor: [`Frost`] is what a client asked for, [`normalise_region`]
//! is what its `set_region` integers mean, [`capabilities`] is the honest answer
//! to "will you blur this surface's backdrop", and [`collect_panes`] turns the
//! per-surface parameters into the physical-pixel panes
//! [`crate::render::HeadlessRenderer`] frosts.
//!
//! # Where the parameters live
//!
//! The compositor keys frosting by the `WlSurface` a glass object was created for
//! (see `InoxState::frost`), not by the glass object. The surface is the identity
//! that both the protocol and the render path are built around: the protocol
//! allows one glass object per surface, the object carries its surface as user
//! data, and the renderer already walks surfaces (`LayerEntry::surface`). Keying
//! on the surface also makes the lookup in `scene_elements` a single hash probe
//! per layer surface, and a destroyed surface can be dropped from the map in the
//! same place its layer entry is.
//!
//! # Why a maximum, and why one number
//!
//! [`MAX_BLUR_RADIUS`] is frozen and is exactly what the `capabilities` event
//! reports. The protocol forbids advertising more than the compositor will
//! honour, so [`capabilities`] reports it only when all three of these hold:
//!
//! - the surface has a **layer role** right now. This compositor frosts the
//!   backdrop the layer-shell path composites, and turns the request down for
//!   everything else — an ordinary `xdg_toplevel` is drawn by the tiling path,
//!   which does not implement frosting. The protocol asks a client to create the
//!   role first and the glass object second for exactly this reason.
//! - the session is **not locked**. While a lock is held no ordinary client is
//!   composited at all, so there is nothing whose backdrop could be frosted.
//!   `InoxState::scene_elements` returns the locked scene before
//!   any layer surface (and therefore any glass pane) is considered; the `locked`
//!   input of [`collect_panes`] and [`capabilities`] pins the same rule as a test.
//! - the compositor composites this surface through a path that **applies the
//!   effect**. That is the headless path today; the DRM path leaves the scene
//!   sharp and reports 0 rather than claiming a blur it does not draw (see
//!   `crate::backend::drm`).
//!
//! # What the integers mean
//!
//! `set_region` carries four `i32`s verbatim on the wire; this module is where
//! they get their meaning. A width or height that is not positive clears the
//! region ("nothing to frost"), and a region that is otherwise valid is stored
//! as-is: it is *not* clipped to the surface here, because the surface size is
//! only known when the surface commits, and the render path clips the stored
//! rectangle to the surface's own geometry and to the output anyway (see
//! [`collect_panes`]). The stored origin and extent are capped (see
//! [`MAX_REGION_ORIGIN`] / [`MAX_REGION_EXTENT`]) so an absurd request cannot
//! overflow the coordinate arithmetic that later maps the region into physical
//! pixels. The cap is far above any display this compositor can advertise, so no
//! honest client can observe it.

use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

/// The largest blur radius this compositor applies, in logical pixels.
///
/// Frozen at 64 and reported verbatim by the `capabilities` event. Larger
/// requests are clamped rather than refused, which is what lets a client that
/// scales its pane keep asking for "as much as you can do".
pub const MAX_BLUR_RADIUS: u32 = 64;

/// Largest absolute value stored for a region origin, in logical pixels.
///
/// 2^20 is far outside any coordinate a display can advertise, and the region
/// origin is clipped to the surface long before this matters, so the cap is not
/// observable — it exists so the mapping arithmetic is bounded by construction
/// rather than by trusting the client's `i32`.
pub const MAX_REGION_ORIGIN: i32 = 1 << 20;

/// Largest stored region extent, in logical pixels.
///
/// Kept above [`crate::config::MAX_DIMENSION`] (16384) with room to spare, so
/// clamping a region to this bound cannot clip a region that a real display
/// could have shown, while still making overflow impossible.
pub const MAX_REGION_EXTENT: i32 = 1 << 16;

/// What one surface asked for: a blur radius and an optional region.
///
/// The default is what a freshly created glass object means — "frosting
/// disabled", because a client that creates an object and sets nothing has asked
/// for nothing (and the protocol's initial region is empty).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Frost {
    /// Blur radius in logical pixels, already clamped to [`MAX_BLUR_RADIUS`].
    pub radius: u32,
    /// Region to frost, in surface-local logical coordinates.
    pub region: Option<Rectangle<i32, Logical>>,
}

impl Frost {
    /// Clamp `requested` the way `set_radius` does, without storing it.
    #[must_use]
    pub fn clamp_radius(requested: u32) -> u32 {
        requested.min(MAX_BLUR_RADIUS)
    }

    /// Apply a `set_radius` request.
    pub fn set_radius(&mut self, requested: u32) {
        self.radius = Self::clamp_radius(requested);
    }

    /// Apply a `set_region` request; a non-positive extent clears the region.
    pub fn set_region(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.region = normalise_region(x, y, width, height);
    }

    /// Whether this can frost anything at all: a radius and a region are both needed.
    #[must_use]
    pub fn is_effective(&self) -> bool {
        self.radius > 0 && self.region.is_some()
    }
}

/// The meaning of a `set_region` request.
///
/// `None` means "nothing to frost": the protocol says a width or height that is
/// not positive clears the region, so a client can stop frosting a pane without
/// destroying its object. Otherwise the rectangle is stored as requested (with
/// the documented cap), surface-local and unclipped.
#[must_use]
pub fn normalise_region(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Option<Rectangle<i32, Logical>> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let origin = Point::from((
        x.clamp(-MAX_REGION_ORIGIN, MAX_REGION_ORIGIN),
        y.clamp(-MAX_REGION_ORIGIN, MAX_REGION_ORIGIN),
    ));
    let size = Size::from((width.min(MAX_REGION_EXTENT), height.min(MAX_REGION_EXTENT)));
    Some(Rectangle::new(origin, size))
}

/// The `capabilities` event's `max_radius` for a surface.
///
/// `layer_role` is whether the surface is one of this compositor's layer
/// surfaces right now, `locked` whether a session lock is held, and
/// `frosting_supported` whether the compositor composites this surface through a
/// path that applies the effect. All three must hold, or the honest answer is 0:
/// the client decides its fallback (plain translucency) from this number, so
/// advertising a blur the compositor would not draw is the one thing the event
/// must never do.
#[must_use]
pub fn capabilities(layer_role: bool, locked: bool, frosting_supported: bool) -> u32 {
    if frosting_supported && layer_role && !locked {
        MAX_BLUR_RADIUS
    } else {
        0
    }
}

/// One layer surface's contribution to the pane list, resolved by the compositor.
///
/// The compositor fills this in while it pushes the surface's render elements: it
/// knows the surface's placement and the index its elements landed at, and the
/// pure part below turns that into physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneInput {
    /// Index of this surface's **back-most** render element in the front-to-back
    /// element list: the pane frosts what is drawn behind that element.
    pub at: usize,
    /// The surface's own box in the output's global logical coordinates.
    pub geometry: Rectangle<i32, Logical>,
    /// What the client asked for.
    pub frost: Frost,
}

/// One frosted backdrop to composite, in one output's physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassPane {
    /// Index of the pane's own back-most render element (see [`PaneInput::at`]).
    pub at: usize,
    /// The part of the target to replace with the frosted backdrop.
    pub region: Rectangle<i32, Physical>,
    /// Blur radius in physical pixels.
    pub radius: f32,
}

/// Turn the per-surface parameters into this output's panes, back-most first.
///
/// A pane is produced for every input that has an effective [`Frost`] and whose
/// region — clipped to the surface's own geometry and to `bounds` — still has
/// area. `output_loc` is the output's top-left corner in the global logical
/// space, `bounds` its physical area (so a pane can never name a region outside
/// the target it will be drawn into), and `scale` the output's scale factor
/// (fractional scaling: the region is mapped from surface-local logical coordinates into
/// physical pixels with the same factor the scene itself is drawn at).
///
/// `locked` short-circuits to an empty list: while a session lock is held, no
/// ordinary client is composited, so no pane may be frosted. This is the same
/// rule the locked branch of
/// `InoxState::scene_elements` enforces by returning early, and
/// the unit test below pins it as a behaviour rather than a comment.
///
/// The returned list is sorted by `at` **descending**, which is back-most first:
/// the renderer frosts from the back of the scene towards the front so that a
/// frost never blurs a pane that was supposed to be behind it.
#[must_use]
pub fn collect_panes(
    locked: bool,
    output_loc: Point<i32, Logical>,
    bounds: Rectangle<i32, Physical>,
    scale: f64,
    inputs: &[PaneInput],
) -> Vec<GlassPane> {
    if locked {
        return Vec::new();
    }

    let mut panes = inputs
        .iter()
        .filter_map(|input| {
            let requested = input.frost.region?;
            if input.frost.radius == 0 {
                return None;
            }
            // Clip to the surface the request was made for: the protocol promises
            // that a region reaching outside the surface is not an error and only
            // the part inside is used.
            let own = Rectangle::from_size(input.geometry.size);
            let clipped = requested.intersection(own)?;
            let global = Rectangle::new(input.geometry.loc + clipped.loc, clipped.size);
            let physical = Rectangle::new(global.loc - output_loc, global.size)
                .to_physical_precise_round(scale);
            let region = physical.intersection(bounds)?;
            if region.is_empty() {
                return None;
            }
            Some(GlassPane {
                at: input.at,
                region,
                radius: input.frost.radius as f32 * scale as f32,
            })
        })
        .collect::<Vec<_>>();

    // Back-most first: the renderer frosts from the back of the scene towards the
    // front, so a pane never blurs a pane that was supposed to be behind it.
    panes.sort_by_key(|pane| std::cmp::Reverse(pane.at));
    panes
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn logical(x: i32, y: i32, width: i32, height: i32) -> Rectangle<i32, Logical> {
        Rectangle::new(Point::from((x, y)), Size::from((width, height)))
    }

    #[test]
    fn a_fresh_frost_object_asks_for_nothing() {
        let frost = Frost::default();
        assert_eq!(frost.radius, 0);
        assert_eq!(frost.region, None);
        assert!(!frost.is_effective());
    }

    #[test]
    fn a_zero_or_negative_extent_clears_the_region() {
        let mut frost = Frost::default();
        frost.set_region(4, 4, 10, 10);
        assert!(frost.region.is_some());
        // A client that wants to stop frosting a pane without destroying the
        // object: any non-positive extent means "nothing to frost".
        for (width, height) in [
            (0, 10),
            (10, 0),
            (-1, 10),
            (10, -1),
            (0, 0),
            (i32::MIN, i32::MIN),
        ] {
            frost.set_region(0, 0, width, height);
            assert_eq!(
                frost.region, None,
                "({width}, {height}) must clear the region"
            );
        }
    }

    #[test]
    fn all_four_quadrants_survive_normalisation() {
        for (x, y) in [(3, 7), (-3, 7), (3, -7), (-3, -7)] {
            let region = normalise_region(x, y, 12, 9).expect("a positive extent is a region");
            assert_eq!(region, logical(x, y, 12, 9));
            assert_eq!(region.size, Size::from((12, 9)));
        }
    }

    #[test]
    fn an_enormous_rectangle_is_capped_and_cannot_overflow() {
        let region = normalise_region(i32::MIN, i32::MAX, i32::MAX, i32::MAX)
            .expect("a positive extent is a region");
        assert_eq!(region.loc.x, -MAX_REGION_ORIGIN);
        assert_eq!(region.loc.y, MAX_REGION_ORIGIN);
        assert_eq!(region.size.w, MAX_REGION_EXTENT);
        assert_eq!(region.size.h, MAX_REGION_EXTENT);
        // The point of the cap: the far edge is representable without wrapping.
        assert_eq!(
            region.loc.x + region.size.w,
            -MAX_REGION_ORIGIN + MAX_REGION_EXTENT
        );
        assert!(region.loc.y.checked_add(region.size.h).is_some());
    }

    #[test]
    fn the_radius_is_clamped_to_the_frozen_maximum() {
        assert_eq!(Frost::clamp_radius(0), 0);
        assert_eq!(Frost::clamp_radius(MAX_BLUR_RADIUS), MAX_BLUR_RADIUS);
        assert_eq!(Frost::clamp_radius(MAX_BLUR_RADIUS + 1), MAX_BLUR_RADIUS);
        assert_eq!(Frost::clamp_radius(u32::MAX), MAX_BLUR_RADIUS);

        let mut frost = Frost::default();
        frost.set_radius(u32::MAX);
        assert_eq!(frost.radius, MAX_BLUR_RADIUS);
        assert!(
            !frost.is_effective(),
            "a radius without a region frosts nothing"
        );
        frost.set_region(0, 0, 8, 8);
        assert!(frost.is_effective());
    }

    #[test]
    fn capabilities_need_a_layer_role_a_renderer_and_an_unlocked_session() {
        assert_eq!(capabilities(true, false, true), MAX_BLUR_RADIUS);
        assert_eq!(capabilities(false, false, true), 0, "an ordinary toplevel");
        assert_eq!(capabilities(true, true, true), 0, "a locked session");
        assert_eq!(
            capabilities(true, false, false),
            0,
            "a path without frosting"
        );
        assert_eq!(capabilities(false, true, false), 0);
    }

    #[test]
    fn a_locked_session_contributes_no_panes() {
        let inputs = [PaneInput {
            at: 3,
            geometry: logical(0, 0, 200, 100),
            frost: Frost {
                radius: 24,
                region: normalise_region(0, 0, 200, 100),
            },
        }];
        let bounds = Rectangle::from_size(Size::from((320, 240)));

        // The same input that does produce a pane when the session is not locked.
        assert_eq!(
            collect_panes(false, Point::from((0, 0)), bounds, 1.0, &inputs).len(),
            1
        );
        assert_eq!(
            collect_panes(true, Point::from((0, 0)), bounds, 1.0, &inputs),
            Vec::new(),
            "a locked session composites no ordinary client, so it frosts nothing"
        );
    }

    #[test]
    fn a_pane_is_clipped_to_the_surface_and_the_output() {
        // Surface at (100, 50), 80x40; the client asks for a region reaching far
        // outside it and partly outside the output.
        let inputs = [PaneInput {
            at: 0,
            geometry: logical(100, 50, 80, 40),
            frost: Frost {
                radius: 16,
                region: normalise_region(-1000, -1000, 4000, 4000),
            },
        }];
        let bounds = Rectangle::from_size(Size::from((320, 240)));
        let panes = collect_panes(false, Point::from((0, 0)), bounds, 1.0, &inputs);
        assert_eq!(panes.len(), 1);
        assert_eq!(
            panes[0].region,
            Rectangle::new(Point::from((100, 50)), Size::from((80, 40))),
            "the region is the surface's own box, inside the output"
        );
        assert_eq!(panes[0].radius, 16.0);
    }

    #[test]
    fn a_pane_outside_the_output_is_dropped_and_scale_is_applied() {
        let bounds = Rectangle::from_size(Size::from((200, 200)));
        let off_screen = [PaneInput {
            at: 0,
            geometry: logical(400, 400, 50, 50),
            frost: Frost {
                radius: 8,
                region: normalise_region(0, 0, 50, 50),
            },
        }];
        assert!(collect_panes(false, Point::from((0, 0)), bounds, 1.0, &off_screen).is_empty());

        // A 1.5x output maps surface-local logical coordinates to 1.5x the pixels
        // and 1.5x the radius. The bounds are roomy here so the mapping is what is
        // being asserted, not the clipping (which the first half covers).
        let inputs = [PaneInput {
            at: 0,
            geometry: logical(100, 100, 60, 40),
            frost: Frost {
                radius: 16,
                region: normalise_region(0, 0, 60, 40),
            },
        }];
        let roomy = Rectangle::from_size(Size::from((400, 400)));
        let panes = collect_panes(false, Point::from((0, 0)), roomy, 1.5, &inputs);
        assert_eq!(panes.len(), 1);
        assert_eq!(
            panes[0].region,
            Rectangle::new(Point::from((150, 150)), Size::from((90, 60)))
        );
        assert_eq!(panes[0].radius, 24.0);
    }

    #[test]
    fn an_ineffective_frost_produces_no_pane() {
        let bounds = Rectangle::from_size(Size::from((320, 240)));
        let geometry = logical(0, 0, 100, 100);
        let no_region = PaneInput {
            at: 0,
            geometry,
            frost: Frost {
                radius: 16,
                region: None,
            },
        };
        let no_radius = PaneInput {
            at: 0,
            geometry,
            frost: Frost {
                radius: 0,
                region: normalise_region(0, 0, 100, 100),
            },
        };
        let empty_region = PaneInput {
            at: 0,
            geometry,
            frost: Frost {
                radius: 16,
                region: normalise_region(0, 0, 0, 0),
            },
        };
        assert!(collect_panes(false, Point::from((0, 0)), bounds, 1.0, &[no_region]).is_empty());
        assert!(collect_panes(false, Point::from((0, 0)), bounds, 1.0, &[no_radius]).is_empty());
        assert!(collect_panes(false, Point::from((0, 0)), bounds, 1.0, &[empty_region]).is_empty());
    }

    #[test]
    fn panes_are_returned_back_most_first() {
        let bounds = Rectangle::from_size(Size::from((320, 240)));
        let input = |at: usize| PaneInput {
            at,
            geometry: logical(0, 0, 320, 240),
            frost: Frost {
                radius: 8,
                region: normalise_region(0, 0, 320, 240),
            },
        };
        let panes = collect_panes(
            false,
            Point::from((0, 0)),
            bounds,
            1.0,
            &[input(2), input(9), input(5)],
        );
        assert_eq!(
            panes.iter().map(|pane| pane.at).collect::<Vec<_>>(),
            vec![9, 5, 2],
            "the renderer frosts from the back of the scene towards the front"
        );
    }

    proptest! {
        #[test]
        fn clamping_is_idempotent_and_monotone(first in any::<u32>(), second in any::<u32>()) {
            let once = Frost::clamp_radius(first);
            prop_assert_eq!(Frost::clamp_radius(once), once);
            prop_assert!(once <= MAX_BLUR_RADIUS);
            if first <= second {
                prop_assert!(Frost::clamp_radius(first) <= Frost::clamp_radius(second));
            }
        }

        #[test]
        fn normalisation_is_total_bounded_and_idempotent(
            x in any::<i32>(),
            y in any::<i32>(),
            width in any::<i32>(),
            height in any::<i32>(),
        ) {
            let region = normalise_region(x, y, width, height);
            if width <= 0 || height <= 0 {
                prop_assert_eq!(region, None);
            } else {
                let region = region.expect("a positive extent yields a region");
                prop_assert!(region.size.w > 0 && region.size.w <= MAX_REGION_EXTENT);
                prop_assert!(region.size.h > 0 && region.size.h <= MAX_REGION_EXTENT);
                prop_assert!(region.loc.x.abs() <= MAX_REGION_ORIGIN);
                prop_assert!(region.loc.y.abs() <= MAX_REGION_ORIGIN);
                // Re-normalising the stored value is a no-op: the cap is a fixed
                // point, so a request cannot drift when it is applied again.
                prop_assert_eq!(
                    normalise_region(region.loc.x, region.loc.y, region.size.w, region.size.h),
                    Some(region)
                );
                prop_assert!(region.loc.x.checked_add(region.size.w).is_some());
                prop_assert!(region.loc.y.checked_add(region.size.h).is_some());
            }
        }

        #[test]
        fn collected_panes_stay_inside_the_output_and_are_sorted(
            locked in any::<bool>(),
            geometry in (-500i32..500, -500i32..500, 1i32..400, 1i32..400),
            region in (-2000i32..2000, -2000i32..2000, 1i32..2000, 1i32..2000),
        ) {
            let (gx, gy, gw, gh) = geometry;
            let (rx, ry, rw, rh) = region;
            let bounds = Rectangle::from_size(Size::from((320, 240)));
            let inputs = [PaneInput {
                at: 4,
                geometry: logical(gx, gy, gw, gh),
                frost: Frost { radius: 32, region: normalise_region(rx, ry, rw, rh) },
            }];
            let panes = collect_panes(locked, Point::from((0, 0)), bounds, 1.0, &inputs);
            if locked {
                prop_assert!(panes.is_empty());
            }
            for pane in &panes {
                prop_assert!(!pane.region.is_empty());
                prop_assert_eq!(pane.region.intersection(bounds), Some(pane.region));
            }
            let ats = panes.iter().map(|pane| pane.at).collect::<Vec<_>>();
            prop_assert!(ats.windows(2).all(|pair| pair[0] >= pair[1]));
        }
    }
}
