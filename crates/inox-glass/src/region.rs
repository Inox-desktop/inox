//! The part of a surface a pane covers.
//!
//! The region is in the surface's own **logical** coordinates, exactly as
//! `set_region` carries it. The compositor clips it to the surface, so a region
//! that reaches outside is not an error; a region with no positive area is the
//! protocol's "nothing to frost", which is why a pane configured to zero size
//! asks for nothing rather than for an empty rectangle.

/// The area of a surface to soften and refract, in surface-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Region {
    /// Region origin on the x axis, surface-local.
    pub x: i32,
    /// Region origin on the y axis, surface-local.
    pub y: i32,
    /// Region width; not positive means "nothing to frost".
    pub width: i32,
    /// Region height; not positive means "nothing to frost".
    pub height: i32,
}

impl Region {
    /// The pane's own area: the whole configured surface.
    ///
    /// Extents larger than [`i32::MAX`] are clamped rather than wrapped: the
    /// protocol carries `i32`, and a wrapped negative extent would mean the
    /// opposite of what was asked for.
    pub fn pane(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width: i32::try_from(width).unwrap_or(i32::MAX),
            height: i32::try_from(height).unwrap_or(i32::MAX),
        }
    }

    /// Whether this region has an area the compositor could frost.
    pub const fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_starts_at_the_surface_origin() {
        assert_eq!(
            Region::pane(640, 68),
            Region {
                x: 0,
                y: 0,
                width: 640,
                height: 68,
            }
        );
    }

    #[test]
    fn a_region_with_no_area_is_empty() {
        assert!(Region::pane(0, 68).is_empty());
        assert!(Region::pane(640, 0).is_empty());
        assert!(Region::default().is_empty());
        assert!(
            Region {
                x: 0,
                y: 0,
                width: -4,
                height: 8,
            }
            .is_empty(),
            "a negative extent is the protocol's `clear the region` value"
        );
        assert!(!Region::pane(1, 1).is_empty());
    }

    #[test]
    fn an_extent_that_does_not_fit_is_clamped_not_wrapped() {
        assert_eq!(
            Region::pane(u32::MAX, u32::MAX),
            Region {
                x: 0,
                y: 0,
                width: i32::MAX,
                height: i32::MAX,
            }
        );
    }
}
