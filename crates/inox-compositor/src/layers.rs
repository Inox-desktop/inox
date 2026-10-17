//! Layer-shell placement and exclusive work-area calculation.

use smithay::{
    utils::{Logical, Rectangle},
    wayland::shell::wlr_layer::{Anchor, ExclusiveZone, LayerSurfaceCachedState},
};

/// Place a surface in its available rectangle without overflowing on client input.
pub(crate) fn geometry(
    area: Rectangle<i32, Logical>,
    state: LayerSurfaceCachedState,
) -> Rectangle<i32, Logical> {
    let (x, width) = axis(
        area.loc.x,
        area.size.w,
        state.size.w,
        state.anchor.contains(Anchor::LEFT),
        state.anchor.contains(Anchor::RIGHT),
        state.margin.left,
        state.margin.right,
    );
    let (y, height) = axis(
        area.loc.y,
        area.size.h,
        state.size.h,
        state.anchor.contains(Anchor::TOP),
        state.anchor.contains(Anchor::BOTTOM),
        state.margin.top,
        state.margin.bottom,
    );
    Rectangle::new((x, y).into(), (width, height).into())
}

fn axis(
    start: i32,
    extent: i32,
    requested: i32,
    near: bool,
    far: bool,
    near_margin: i32,
    far_margin: i32,
) -> (i32, i32) {
    let start = i64::from(start);
    let extent = i64::from(extent.max(0));
    let near_margin = if near { i64::from(near_margin) } else { 0 };
    let far_margin = if far { i64::from(far_margin) } else { 0 };
    let available = (extent - near_margin - far_margin).clamp(1, i64::from(i32::MAX));
    let size = if requested == 0 {
        available
    } else {
        i64::from(requested).clamp(1, available)
    };
    let position = match (near, far) {
        (true, false) => start + near_margin,
        (false, true) => start + extent - far_margin - size,
        _ => start + near_margin + (available - size) / 2,
    };
    // Leave enough integer range for Smithay's rectangle endpoint arithmetic.
    let position = position.clamp(i64::from(i32::MIN), i64::from(i32::MAX) - size);
    (position as i32, size as i32)
}

/// Reserve only a single edge or an edge with its two perpendicular anchors.
pub(crate) fn reserve(area: &mut Rectangle<i32, Logical>, state: LayerSurfaceCachedState) {
    let ExclusiveZone::Exclusive(zone) = state.exclusive_zone else {
        return;
    };
    let anchor = state.anchor;
    let horizontal = Anchor::LEFT | Anchor::RIGHT;
    let vertical = Anchor::TOP | Anchor::BOTTOM;
    let (extent, location, margin, near) =
        if anchor == Anchor::TOP || anchor == Anchor::TOP | horizontal {
            (&mut area.size.h, &mut area.loc.y, state.margin.top, true)
        } else if anchor == Anchor::BOTTOM || anchor == Anchor::BOTTOM | horizontal {
            (
                &mut area.size.h,
                &mut area.loc.y,
                state.margin.bottom,
                false,
            )
        } else if anchor == Anchor::LEFT || anchor == Anchor::LEFT | vertical {
            (&mut area.size.w, &mut area.loc.x, state.margin.left, true)
        } else if anchor == Anchor::RIGHT || anchor == Anchor::RIGHT | vertical {
            (&mut area.size.w, &mut area.loc.x, state.margin.right, false)
        } else {
            return;
        };
    let amount = (i64::from(zone) + i64::from(margin)).clamp(0, i64::from((*extent).max(0))) as i32;
    *extent -= amount;
    if near {
        *location = location.saturating_add(amount);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use smithay::wayland::shell::wlr_layer::Margins;

    #[test]
    fn fixed_size_centres_between_opposite_anchors() {
        let state = LayerSurfaceCachedState {
            size: (200, 30).into(),
            anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            margin: Margins {
                left: 10,
                right: 30,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            geometry(Rectangle::from_size((800, 600).into()), state),
            Rectangle::new((290, 0).into(), (200, 30).into())
        );
    }

    #[test]
    fn invalid_exclusive_anchors_do_not_reserve_space() {
        for anchor in [
            Anchor::empty(),
            Anchor::TOP | Anchor::LEFT,
            Anchor::LEFT | Anchor::RIGHT,
            Anchor::all(),
        ] {
            let mut area = Rectangle::from_size((800, 600).into());
            reserve(
                &mut area,
                LayerSurfaceCachedState {
                    anchor,
                    exclusive_zone: ExclusiveZone::Exclusive(40),
                    ..Default::default()
                },
            );
            assert_eq!(area, Rectangle::from_size((800, 600).into()));
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn arbitrary_reservations_remain_inside_output(
            width in 1i32..16384, height in 1i32..16384,
            anchors in prop::collection::vec(0u32..16, 0..32),
            zone in any::<u32>(), margin in any::<i32>(),
        ) {
            let mut area = Rectangle::from_size((width, height).into());
            for bits in anchors {
                reserve(&mut area, LayerSurfaceCachedState {
                    anchor: Anchor::from_bits_retain(bits),
                    exclusive_zone: ExclusiveZone::Exclusive(zone),
                    margin: Margins { top: margin, right: margin, bottom: margin, left: margin },
                    ..Default::default()
                });
                prop_assert!(area.loc.x >= 0 && area.loc.y >= 0);
                prop_assert!(area.size.w >= 0 && area.size.h >= 0);
                prop_assert!(area.loc.x + area.size.w <= width);
                prop_assert!(area.loc.y + area.size.h <= height);
            }
        }

        #[test]
        fn arbitrary_geometry_has_positive_representable_endpoints(
            width in 1i32..16384, height in 1i32..16384,
            requested_width in 0i32..=i32::MAX, requested_height in 0i32..=i32::MAX,
            bits in 0u32..16, top in any::<i32>(), right in any::<i32>(),
            bottom in any::<i32>(), left in any::<i32>(),
        ) {
            let rect = geometry(Rectangle::from_size((width, height).into()), LayerSurfaceCachedState {
                size: (requested_width, requested_height).into(),
                anchor: Anchor::from_bits_retain(bits),
                margin: Margins { top, right, bottom, left },
                ..Default::default()
            });
            prop_assert!(rect.size.w > 0 && rect.size.h > 0);
            prop_assert!(rect.loc.x.checked_add(rect.size.w).is_some());
            prop_assert!(rect.loc.y.checked_add(rect.size.h).is_some());
        }
    }
}
