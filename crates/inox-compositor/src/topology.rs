//! Output placement and pointer clamping for a multi-output topology.
//!
//! The module is pure geometry: it holds no Smithay state and performs no I/O,
//! so the placement rules are unit testable and property testable on their own —
//! the same shape `crate::layout` and `crate::layers` have. ADR 0019 fixes
//! the rules this module implements:
//!
//! - an output with an explicit position keeps it, including negative
//!   coordinates;
//! - an output without one is placed to the right of the bounding box of
//!   everything placed before it, aligned with that box's top edge;
//! - the pointer is clamped into the union of the output areas, not into a single
//!   output's rectangle, so a second display is reachable.
//!
//! The *sizes* here are logical sizes: physical mode size divided by the output's
//! scale, rounded up, which is the rounding Smithay's `Space::output_geometry`
//! applies. The compositor uses Smithay's geometry as the authority for work
//! areas and rendering; the size computed here only advances the automatic
//! placement cursor, so a disagreement of one pixel changes where a display
//! without a configured position lands, never how anything is drawn.

use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

/// A stable identity for one output during a compositor run.
///
/// Minted by the compositor and valid only for the current process: the identity
/// a display keeps across sessions is its connector name in the configuration,
/// not this value (ADR 0012 separates runtime identity from saved preference).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputId(u64);

impl OutputId {
    /// Mint an identifier from its raw value.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw identifier.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One output to place, as the placement rules see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// Logical size of the output's area.
    pub size: Size<i32, Logical>,
    /// Top-left corner the user asked for, or `None` to place automatically.
    pub position: Option<Point<i32, Logical>>,
}

/// Logical size of an output driven at `scale`, rounded up like Smithay's geometry.
///
/// A non-finite or non-positive scale is treated as `1.0`: the configuration
/// layer rejects such values, and this keeps a hand-built one from producing a
/// nonsensical area instead of an obviously wrong one.
pub fn logical_size(size: Size<i32, Physical>, scale: f64) -> Size<i32, Logical> {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let component = |value: i32| {
        let logical = (f64::from(value) / scale).ceil();
        logical.clamp(0.0, f64::from(i32::MAX)) as i32
    };

    Size::from((component(size.w), component(size.h)))
}

/// Place every output, honouring explicit positions and filling in the rest.
///
/// The result holds one area per request, in the order given, and is sized to the
/// request. Every automatically placed output is disjoint from every output
/// placed before it: it starts exactly at the right edge of their bounding box.
/// Negative sizes are clamped to zero rather than rejected — the caller validates
/// modes, and a zero-sized area is inert rather than harmful.
pub fn place(requests: &[Placement]) -> Vec<Rectangle<i32, Logical>> {
    let mut placed: Vec<Rectangle<i32, Logical>> = Vec::with_capacity(requests.len());
    let mut bounds: Option<Rectangle<i32, Logical>> = None;

    for request in requests {
        let size = Size::from((request.size.w.max(0), request.size.h.max(0)));
        let location = match request.position {
            Some(position) => position,
            None => match bounds {
                Some(bounds) => {
                    Point::from((bounds.loc.x.saturating_add(bounds.size.w), bounds.loc.y))
                }
                None => Point::from((0, 0)),
            },
        };
        let area = Rectangle::new(location, size);

        bounds = Some(match bounds {
            Some(bounds) => bounds.merge(area),
            None => area,
        });
        placed.push(area);
    }

    placed
}

/// The bounding box of `areas`, or a zero-sized area at the origin when empty.
///
/// Absolute input devices map their [0, 1] range onto this box, so it is also how
/// a tablet or touchscreen covers a multi-display arrangement.
pub fn bounds(areas: &[Rectangle<i32, Logical>]) -> Rectangle<i32, Logical> {
    areas
        .iter()
        .copied()
        .reduce(Rectangle::merge)
        .unwrap_or_default()
}

/// Clamp a pointer position into the union of the output areas.
///
/// A position already inside an area is returned unchanged, so ordinary motion is
/// never disturbed. A position outside every area moves to the closest point of
/// the closest area, which is what keeps the pointer on a display after an output
/// is unplugged from under it. With no areas at all — every display gone — the
/// position collapses to the origin, where nothing can be hit.
///
/// Non-finite coordinates are treated as the origin: they can only come from
/// injected input, and they must not propagate into protocol state.
pub fn clamp(point: Point<f64, Logical>, areas: &[Rectangle<i32, Logical>]) -> Point<f64, Logical> {
    if !point.x.is_finite() || !point.y.is_finite() {
        return Point::from((0.0, 0.0));
    }
    if areas.is_empty() {
        return Point::from((0.0, 0.0));
    }
    if areas.iter().any(|area| inside(*area, point)) {
        return point;
    }

    areas
        .iter()
        .map(|area| project(*area, point))
        .min_by(|left, right| {
            distance_squared(*left, point)
                .partial_cmp(&distance_squared(*right, point))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(point)
}

/// Whether `point` lies inside `area`, which must have a positive extent.
fn inside(area: Rectangle<i32, Logical>, point: Point<f64, Logical>) -> bool {
    area.size.w > 0
        && area.size.h > 0
        && point.x >= f64::from(area.loc.x)
        && point.x < f64::from(area.loc.x.saturating_add(area.size.w))
        && point.y >= f64::from(area.loc.y)
        && point.y < f64::from(area.loc.y.saturating_add(area.size.h))
}

/// The closest point of `area` to `point`.
fn project(area: Rectangle<i32, Logical>, point: Point<f64, Logical>) -> Point<f64, Logical> {
    let axis = |value: f64, start: i32, extent: i32| {
        let end = f64::from(start.saturating_add(extent.max(0)));
        value.clamp(f64::from(start), end)
    };

    Point::from((
        axis(point.x, area.loc.x, area.size.w),
        axis(point.y, area.loc.y, area.size.h),
    ))
}

/// Squared distance between two positions, used to pick the closest area.
fn distance_squared(left: Point<f64, Logical>, right: Point<f64, Logical>) -> f64 {
    let dx = left.x - right.x;
    let dy = left.y - right.y;

    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(width: i32, height: i32, position: Option<(i32, i32)>) -> Placement {
        Placement {
            size: Size::from((width, height)),
            position: position.map(Point::from),
        }
    }

    fn area(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn an_explicit_position_is_kept_including_negative_coordinates() {
        let placed = place(&[
            placement(1920, 1080, Some((1920, 0))),
            placement(2560, 1440, Some((-2560, -1440))),
        ]);

        assert_eq!(placed[0], area(1920, 0, 1920, 1080));
        assert_eq!(placed[1], area(-2560, -1440, 2560, 1440));
    }

    #[test]
    fn an_output_without_a_position_lands_to_the_right_of_what_came_before() {
        let placed = place(&[
            placement(1920, 1080, None),
            placement(1280, 720, None),
            placement(800, 600, None),
        ]);

        assert_eq!(placed[0], area(0, 0, 1920, 1080));
        assert_eq!(placed[1], area(1920, 0, 1280, 720));
        assert_eq!(placed[2], area(3200, 0, 800, 600));
    }

    #[test]
    fn automatic_placement_follows_the_bounding_box_of_explicit_ones() {
        let placed = place(&[
            placement(1920, 1080, Some((100, -200))),
            placement(1280, 720, None),
        ]);

        assert_eq!(placed[1], area(2020, -200, 1280, 720));
    }

    #[test]
    fn a_single_output_without_a_position_starts_at_the_origin() {
        assert_eq!(
            place(&[placement(1280, 720, None)]),
            vec![area(0, 0, 1280, 720)]
        );
    }

    #[test]
    fn no_outputs_place_nothing() {
        assert!(place(&[]).is_empty());
    }

    #[test]
    fn a_pointer_inside_an_output_is_left_alone() {
        let areas = [area(0, 0, 1920, 1080), area(1920, 0, 1280, 720)];

        assert_eq!(
            clamp(Point::from((3000.5, 719.5)), &areas),
            Point::from((3000.5, 719.5))
        );
    }

    #[test]
    fn a_pointer_outside_every_output_moves_to_the_closest_one() {
        let areas = [area(0, 0, 1920, 1080), area(1920, 0, 1280, 720)];

        // Far below the taller output: its bottom-right corner is closer than
        // anything on the shorter one, so that is where the pointer lands.
        assert_eq!(
            clamp(Point::from((2500.0, 5000.0)), &areas),
            Point::from((1920.0, 1080.0))
        );
        // Below the shorter output only, it is that output's bottom edge.
        assert_eq!(
            clamp(Point::from((2500.0, 800.0)), &areas),
            Point::from((2500.0, 720.0))
        );
        // Far left: the first output's left edge, at its own vertical range.
        assert_eq!(
            clamp(Point::from((-400.0, 100.0)), &areas),
            Point::from((0.0, 100.0))
        );
    }

    #[test]
    fn without_any_output_the_pointer_collapses_to_the_origin() {
        assert_eq!(
            clamp(Point::from((500.0, 500.0)), &[]),
            Point::from((0.0, 0.0))
        );
    }

    #[test]
    fn a_non_finite_position_becomes_the_origin() {
        let areas = [area(0, 0, 1920, 1080)];

        assert_eq!(
            clamp(Point::from((f64::NAN, 10.0)), &areas),
            Point::from((0.0, 0.0))
        );
        assert_eq!(
            clamp(Point::from((10.0, f64::INFINITY)), &areas),
            Point::from((0.0, 0.0))
        );
    }

    #[test]
    fn logical_sizes_divide_by_the_scale_and_round_up() {
        assert_eq!(
            logical_size(Size::from((1920, 1080)), 1.0),
            Size::from((1920, 1080))
        );
        assert_eq!(
            logical_size(Size::from((1920, 1080)), 2.0),
            Size::from((960, 540))
        );
        assert_eq!(
            logical_size(Size::from((2560, 1440)), 1.25),
            Size::from((2048, 1152))
        );
        // 1366 / 1.25 = 1092.8, which rounds up rather than truncating.
        assert_eq!(
            logical_size(Size::from((1366, 768)), 1.25),
            Size::from((1093, 615))
        );
    }

    #[test]
    fn a_nonsensical_scale_falls_back_to_one() {
        for scale in [0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                logical_size(Size::from((800, 600)), scale),
                Size::from((800, 600)),
                "scale {scale}"
            );
        }
    }

    #[test]
    fn the_bounds_of_no_area_is_an_empty_rectangle() {
        assert_eq!(bounds(&[]), Rectangle::default());
    }

    #[test]
    fn the_bounds_cover_every_area() {
        let areas = [area(0, 0, 1920, 1080), area(1920, -200, 1280, 720)];

        assert_eq!(bounds(&areas), area(0, -200, 3200, 1280));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn placement() -> impl Strategy<Value = Placement> {
        (
            0i32..4_000,
            0i32..4_000,
            prop::option::of((-4_000i32..4_000, -4_000i32..4_000)),
        )
            .prop_map(|(width, height, position)| Placement {
                size: Size::from((width, height)),
                position: position.map(Point::from),
            })
    }

    fn area() -> impl Strategy<Value = Rectangle<i32, Logical>> {
        (-2_000i32..2_000, -2_000i32..2_000, 0i32..3_000, 0i32..3_000)
            .prop_map(|(x, y, w, h)| Rectangle::new((x, y).into(), (w, h).into()))
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn one_area_per_request_in_order(requests in prop::collection::vec(placement(), 0..8)) {
            let placed = place(&requests);

            prop_assert_eq!(placed.len(), requests.len());
            for (request, area) in requests.iter().zip(placed.iter()) {
                prop_assert_eq!(area.size.w, request.size.w.max(0));
                prop_assert_eq!(area.size.h, request.size.h.max(0));
            }
        }

        #[test]
        fn explicit_positions_are_never_moved(requests in prop::collection::vec(placement(), 0..8)) {
            for (request, area) in requests.iter().zip(place(&requests)) {
                if let Some(position) = request.position {
                    prop_assert_eq!(area.loc, position);
                }
            }
        }

        #[test]
        fn automatic_placement_never_overlaps_what_came_before(
            requests in prop::collection::vec(placement(), 0..8)
        ) {
            let placed = place(&requests);

            for (index, area) in placed.iter().enumerate() {
                if requests[index].position.is_some() {
                    continue;
                }
                for earlier in &placed[..index] {
                    prop_assert!(!area.overlaps(*earlier), "{area:?} overlaps {earlier:?}");
                }
            }
        }

        #[test]
        fn clamping_lands_inside_the_bounding_box(
            areas in prop::collection::vec(area(), 1..6),
            x in -8_000.0f64..8_000.0, y in -8_000.0f64..8_000.0,
        ) {
            let point = clamp(Point::from((x, y)), &areas);
            let bounds = bounds(&areas);

            prop_assert!(point.x >= f64::from(bounds.loc.x));
            prop_assert!(point.y >= f64::from(bounds.loc.y));
            prop_assert!(point.x <= f64::from(bounds.loc.x.saturating_add(bounds.size.w)));
            prop_assert!(point.y <= f64::from(bounds.loc.y.saturating_add(bounds.size.h)));
        }

        #[test]
        fn clamping_leaves_a_contained_position_alone(
            areas in prop::collection::vec(area(), 1..6),
        ) {
            for area in &areas {
                if area.size.w <= 0 || area.size.h <= 0 {
                    continue;
                }
                let point = Point::from((
                    area.loc.x as f64 + f64::from(area.size.w) / 2.0,
                    area.loc.y as f64 + f64::from(area.size.h) / 2.0,
                ));
                prop_assert_eq!(clamp(point, &areas), point);
            }
        }

        #[test]
        fn logical_sizes_divide_by_the_scale_and_round_up(
            width in 1i32..16_384, height in 1i32..16_384,
            scale in prop::sample::select(vec![0.25f64, 0.5, 1.0, 1.25, 1.5, 2.0, 3.0, 8.0]),
        ) {
            let logical = logical_size(Size::from((width, height)), scale);

            // Never zero: a zero-sized area would be invisible to hit testing.
            prop_assert!(logical.w >= 1 && logical.h >= 1);
            prop_assert_eq!(logical.w, (f64::from(width) / scale).ceil() as i32);
            prop_assert_eq!(logical.h, (f64::from(height) / scale).ceil() as i32);
            // Scaling up shrinks the logical area, scaling down enlarges it.
            if scale >= 1.0 {
                prop_assert!(logical.w <= width && logical.h <= height);
            } else {
                prop_assert!(logical.w >= width && logical.h >= height);
            }
        }
    }
}
