//! Where an image's pixels land on an output.
//!
//! One function answers the whole question — [`place`] — for the five modes
//! `[wallpaper].mode` accepts. It is arithmetic over two sizes, which is why the
//! properties this crate is judged on can be stated as properties rather than as
//! screenshots: `fill` covers the output, `fit` stays inside it, both preserve
//! the aspect ratio to within a pixel, `stretch` fills exactly, `center` keeps
//! the native size, and `tile` repeats from the origin.
//!
//! Three decisions are worth reading before the code:
//!
//! - **Every rectangle is a destination for one whole copy of the image.** A
//!   rectangle may therefore start at a negative coordinate (`fill` centres an
//!   image that is wider than the output) or run past the output's edge (the
//!   last row and column of a tiling). The *surface* clips those; this module
//!   must not, because a rectangle narrower than the copy it describes would
//!   make the caller draw a distorted image rather than a cropped one.
//! - **Rounding happens once, at the end, to whole pixels.** Rounding the scale
//!   factor instead would let a 3840-wide image on a 1920-wide output land on
//!   1919 pixels and leave a hairline of background showing.
//! - **A degenerate input has no answer, it does not have an approximate one.**
//!   A zero-sized image or output returns `None` from [`place`] — never a panic,
//!   never `NaN`, never a zero-width rectangle — and [`plan`] turns the same
//!   case into a reason the interface can show.

use inox_config::WallpaperMode;

use crate::model::{Size, Unusable};

/// How many copies of an image one output may be tiled with.
///
/// A tile is drawn one copy at a time, so the count is the size of the frame:
/// 1024 copies is already a demanding one, and an unbounded count is a denial of
/// service a user can trigger with a tiny image and a large output — 1280×720
/// copies of a 1×1 pixel is 921 600 draws from a 70-byte file. Above the bound
/// the output degrades to the theme background and says why, like any other
/// unusable resource (acceptance D05). A larger tile image is the fix, and it is
/// the user's to choose.
pub const MAX_TILES: u32 = 1024;

/// A rectangle in output coordinates, in whole pixels.
///
/// `x` and `y` are signed because a copy that is larger than its output starts
/// outside it; a width or a height is never negative, because that would not be
/// a rectangle. The arithmetic that produces them is done in `i64` so that no
/// size a configuration can name overflows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge, in output coordinates.
    pub x: i64,
    /// Top edge, in output coordinates.
    pub y: i64,
    /// Width in pixels; never zero in a rectangle this crate returns.
    pub width: u32,
    /// Height in pixels; never zero in a rectangle this crate returns.
    pub height: u32,
}

impl Rect {
    /// A rectangle from its parts.
    #[must_use]
    pub const fn new(x: i64, y: i64, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The rectangle's size.
    #[must_use]
    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// The x coordinate just past the right edge.
    #[must_use]
    pub const fn right(&self) -> i64 {
        self.x + self.width as i64
    }

    /// The y coordinate just past the bottom edge.
    #[must_use]
    pub const fn bottom(&self) -> i64 {
        self.y + self.height as i64
    }

    /// Whether this rectangle contains all of `size`, measured from the origin.
    #[must_use]
    pub const fn covers(&self, size: Size) -> bool {
        self.x <= 0
            && self.y <= 0
            && self.right() >= size.width as i64
            && self.bottom() >= size.height as i64
    }

    /// Whether this rectangle lies entirely inside `size`, measured from the
    /// origin.
    #[must_use]
    pub const fn fits_in(&self, size: Size) -> bool {
        self.x >= 0
            && self.y >= 0
            && self.right() <= size.width as i64
            && self.bottom() <= size.height as i64
    }
}

/// Where each copy of the image goes.
///
/// Every mode but [`WallpaperMode::Tile`] produces exactly one rectangle;
/// tiling produces one per copy, in row-major order from the origin. The first
/// rectangle is a field rather than an entry in a vector, so that "a placement
/// has at least one rectangle" is a property of the type and not something a
/// caller has to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    first: Rect,
    rest: Vec<Rect>,
}

impl Placement {
    /// One copy, drawn at `rect`.
    fn single(rect: Rect) -> Self {
        Self {
            first: rect,
            rest: Vec::new(),
        }
    }

    /// The first copy to draw; the origin tile when the image is repeated.
    #[must_use]
    pub const fn first(&self) -> Rect {
        self.first
    }

    /// The copies after the first, in row-major order.
    #[must_use]
    pub fn rest(&self) -> &[Rect] {
        &self.rest
    }

    /// Every copy, first one included, in draw order.
    pub fn rects(&self) -> impl Iterator<Item = Rect> + '_ {
        std::iter::once(self.first).chain(self.rest().iter().copied())
    }

    /// How many copies of the image there are.
    #[must_use]
    pub fn copies(&self) -> usize {
        self.rest.len() + 1
    }

    /// Whether the image is repeated.
    #[must_use]
    pub fn is_tiled(&self) -> bool {
        !self.rest.is_empty()
    }

    /// The smallest rectangle containing every copy.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        let mut left = self.first.x;
        let mut top = self.first.y;
        let mut right = self.first.right();
        let mut bottom = self.first.bottom();

        for rect in &self.rest {
            left = left.min(rect.x);
            top = top.min(rect.y);
            right = right.max(rect.right());
            bottom = bottom.max(rect.bottom());
        }

        Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
    }
}

/// The destination rectangle(s) for `image` drawn on `output`.
///
/// Returns `None` when the question has no answer: a zero-sized image or output
/// (nothing can be drawn on nothing), or a tiling that would need more than
/// [`MAX_TILES`] copies. [`plan`] is the same function with the reason attached.
#[must_use]
pub fn place(mode: WallpaperMode, image: Size, output: Size) -> Option<Placement> {
    plan(mode, image, output).ok()
}

/// [`place`] with the reason a placement is impossible.
///
/// # Errors
///
/// [`Unusable::Degenerate`] when either size has a zero side, and
/// [`Unusable::TooManyTiles`] when tiling exceeds [`MAX_TILES`]. Both are states
/// an output survives: the caller falls back to the theme background and shows
/// the reason.
pub fn plan(mode: WallpaperMode, image: Size, output: Size) -> Result<Placement, Unusable> {
    if image.is_empty() || output.is_empty() {
        return Err(Unusable::Degenerate { image, output });
    }

    Ok(match mode {
        WallpaperMode::Fill => Placement::single(centred(scaled_to_cover(image, output), output)),
        WallpaperMode::Fit => Placement::single(centred(scaled_to_contain(image, output), output)),
        WallpaperMode::Stretch => Placement::single(Rect::new(0, 0, output.width, output.height)),
        WallpaperMode::Center => Placement::single(centred(image, output)),
        WallpaperMode::Tile => tiles(image, output)?,
    })
}

/// The size an image takes when scaled to cover `output` completely.
#[must_use]
pub fn scaled_to_cover(image: Size, output: Size) -> Size {
    scaled(
        image,
        f64::max(
            f64::from(output.width) / f64::from(image.width),
            f64::from(output.height) / f64::from(image.height),
        ),
    )
}

/// The size an image takes when scaled to fit inside `output`.
#[must_use]
pub fn scaled_to_contain(image: Size, output: Size) -> Size {
    scaled(
        image,
        f64::min(
            f64::from(output.width) / f64::from(image.width),
            f64::from(output.height) / f64::from(image.height),
        ),
    )
}

/// `image` scaled by `factor`, rounded to whole pixels and never to zero.
///
/// The `max(1.0)` is what keeps a very wide, very short image from rounding to a
/// zero-height copy, which would be an invisible one.
fn scaled(image: Size, factor: f64) -> Size {
    let width = (f64::from(image.width) * factor)
        .round()
        .clamp(1.0, f64::from(u32::MAX));
    let height = (f64::from(image.height) * factor)
        .round()
        .clamp(1.0, f64::from(u32::MAX));

    Size::new(width as u32, height as u32)
}

/// `size` centred on `output`; the offsets are negative when `size` is larger.
///
/// An odd remainder is floored in both directions, so the extra pixel always
/// lands on the right or bottom edge and the result never alternates with the
/// sign of the difference.
fn centred(size: Size, output: Size) -> Rect {
    Rect::new(
        (i64::from(output.width) - i64::from(size.width)).div_euclid(2),
        (i64::from(output.height) - i64::from(size.height)).div_euclid(2),
        size.width,
        size.height,
    )
}

/// The grid of copies that covers `output`, from the origin.
///
/// The grid is what the *surface* clips, so the last row and the last column are
/// whole copies: cropping them here would scale them down instead.
fn tiles(image: Size, output: Size) -> Result<Placement, Unusable> {
    let columns = u64::from(output.width.div_ceil(image.width));
    let rows = u64::from(output.height.div_ceil(image.height));
    let count = columns * rows;

    if count > u64::from(MAX_TILES) {
        return Err(Unusable::TooManyTiles {
            tile: image,
            tiles: count,
            limit: MAX_TILES,
        });
    }

    let first = Rect::new(0, 0, image.width, image.height);
    let mut rest = Vec::with_capacity((count - 1) as usize);

    for index in 1..count {
        let column = index % columns;
        let row = index / columns;

        rest.push(Rect::new(
            (column * u64::from(image.width)) as i64,
            (row * u64::from(image.height)) as i64,
            image.width,
            image.height,
        ));
    }

    Ok(Placement { first, rest })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;

    const OUTPUT: Size = Size::new(1920, 1080);
    const MODES: [WallpaperMode; 5] = [
        WallpaperMode::Fill,
        WallpaperMode::Fit,
        WallpaperMode::Stretch,
        WallpaperMode::Center,
        WallpaperMode::Tile,
    ];

    fn an_image() -> impl Strategy<Value = Size> {
        (1u32..10_000, 1u32..10_000).prop_map(|(width, height)| Size::new(width, height))
    }

    fn an_output() -> impl Strategy<Value = Size> {
        (1u32..8_192, 1u32..4_320).prop_map(|(width, height)| Size::new(width, height))
    }

    fn every_mode() -> impl Strategy<Value = WallpaperMode> {
        prop_oneof![
            Just(WallpaperMode::Fill),
            Just(WallpaperMode::Fit),
            Just(WallpaperMode::Stretch),
            Just(WallpaperMode::Center),
            Just(WallpaperMode::Tile),
        ]
    }

    fn rects(placement: &Placement) -> Vec<Rect> {
        placement.rects().collect()
    }

    /// How far apart the two scale factors a rectangle implies are.
    ///
    /// Each is read off one dimension, and each dimension is scaled by one
    /// factor and then rounded to a whole pixel.
    fn scale_gap(image: Size, rect: Rect) -> f64 {
        let horizontal = f64::from(rect.width) / f64::from(image.width);
        let vertical = f64::from(rect.height) / f64::from(image.height);

        (horizontal - vertical).abs()
    }

    /// The largest gap rounding both dimensions to a whole pixel can produce:
    /// half a pixel is `0.5 / dimension` in scale units, on each axis.
    fn rounding_budget(image: Size) -> f64 {
        0.5 / f64::from(image.width) + 0.5 / f64::from(image.height)
    }

    /// Whether both dimensions of `rect` are explained by one scale factor.
    ///
    /// A copy whose smaller side rounds away to nothing is clamped up to one
    /// pixel instead, so that it stays visible; that case is deliberately not a
    /// distortion, and this is where the tests say so once.
    fn is_undistorted(image: Size, rect: Rect) -> bool {
        scale_gap(image, rect) <= rounding_budget(image) || rect.width == 1 || rect.height == 1
    }

    #[test]
    fn fill_covers_the_output_and_keeps_the_aspect_ratio() {
        let placement = place(WallpaperMode::Fill, Size::new(4000, 3000), OUTPUT);
        let rect = placement.expect("a placement").first();

        assert!(rect.covers(OUTPUT), "{rect:?} should cover {OUTPUT}");
        assert!(is_undistorted(Size::new(4000, 3000), rect));
    }

    #[test]
    fn fill_scales_a_small_image_up_and_centres_the_overflow() {
        let rect = place(WallpaperMode::Fill, Size::new(100, 50), OUTPUT)
            .expect("a placement")
            .first();

        // 100×50 scaled by 21.6 — the factor that covers 1920×1080 — is
        // 2160×1080, so the overflow is horizontal only.
        assert_eq!(rect.size(), Size::new(2160, 1080));
        assert_eq!(rect.x, (1920i64 - 2160).div_euclid(2));
        assert_eq!(rect.y, 0);
    }

    #[test]
    fn fit_stays_inside_the_output_and_keeps_the_aspect_ratio() {
        let rect = place(WallpaperMode::Fit, Size::new(4000, 3000), OUTPUT)
            .expect("a placement")
            .first();

        assert!(rect.fits_in(OUTPUT), "{rect:?} should fit in {OUTPUT}");
        assert_eq!(rect.size(), Size::new(1440, 1080));
        assert!(is_undistorted(Size::new(4000, 3000), rect));
    }

    #[test]
    fn fit_of_a_wider_image_leaves_equal_margins_above_and_below() {
        let rect = place(WallpaperMode::Fit, Size::new(3840, 1080), OUTPUT)
            .expect("a placement")
            .first();

        assert_eq!(rect.size(), Size::new(1920, 540));
        assert!(rect.fits_in(OUTPUT));
        assert_eq!(rect.y, 270, "the two margins are equal");
    }

    #[test]
    fn stretch_fills_the_output_exactly_and_does_not_keep_the_aspect_ratio() {
        let rect = place(WallpaperMode::Stretch, Size::new(100, 50), OUTPUT)
            .expect("a placement")
            .first();

        assert_eq!(rect, Rect::new(0, 0, 1920, 1080));
    }

    #[test]
    fn center_keeps_the_native_size_and_centres_it() {
        let rect = place(WallpaperMode::Center, Size::new(800, 600), OUTPUT)
            .expect("a placement")
            .first();

        assert_eq!(rect.size(), Size::new(800, 600));
        assert_eq!(rect.x, 560);
        assert_eq!(rect.y, 240);
        assert!(rect.fits_in(OUTPUT));
    }

    #[test]
    fn center_of_an_oversized_image_starts_before_the_origin() {
        // The surface clips; the placement must not shrink the copy, because a
        // narrower rectangle would squash the image rather than crop it.
        let rect = place(WallpaperMode::Center, Size::new(3000, 2000), OUTPUT)
            .expect("a placement")
            .first();

        assert_eq!(rect.size(), Size::new(3000, 2000));
        assert_eq!(rect.x, -540);
        assert_eq!(rect.y, -460);
        assert!(!rect.fits_in(OUTPUT));
    }

    #[test]
    fn center_of_an_odd_margin_keeps_the_extra_pixel_on_the_right() {
        let rect = place(WallpaperMode::Center, Size::new(801, 601), OUTPUT)
            .expect("a placement")
            .first();

        assert_eq!(rect.x, 559);
        assert_eq!(rect.y, 239);
        assert_eq!(rect.x + 801 + 560, 1920);
    }

    #[test]
    fn tile_repeats_from_the_origin() {
        let placement =
            place(WallpaperMode::Tile, Size::new(700, 400), OUTPUT).expect("a placement");

        assert!(placement.is_tiled());
        assert_eq!(placement.copies(), 3 * 3);
        assert_eq!(placement.first(), Rect::new(0, 0, 700, 400));
        assert_eq!(
            rects(&placement)[1],
            Rect::new(700, 0, 700, 400),
            "row-major order"
        );
        assert_eq!(rects(&placement)[3], Rect::new(0, 400, 700, 400));
        assert_eq!(
            placement.bounds(),
            Rect::new(0, 0, 2100, 1200),
            "the grid runs past the output and is clipped by the surface"
        );
    }

    #[test]
    fn tile_of_an_exact_division_has_no_partial_copy() {
        let placement =
            place(WallpaperMode::Tile, Size::new(640, 360), OUTPUT).expect("a placement");

        assert_eq!(placement.copies(), 3 * 3);
        assert_eq!(placement.bounds(), Rect::new(0, 0, 1920, 1080));
    }

    #[test]
    fn tile_refuses_a_grid_larger_than_the_bound_and_says_how_large_it_would_be() {
        let error =
            plan(WallpaperMode::Tile, Size::new(1, 1), OUTPUT).expect_err("1×1 cannot tile");

        match error {
            Unusable::TooManyTiles { tile, tiles, limit } => {
                assert_eq!(tile, Size::new(1, 1));
                assert_eq!(tiles, 1920 * 1080);
                assert_eq!(limit, MAX_TILES);
            }
            other => panic!("expected the tiling bound, got {other:?}"),
        }
    }

    #[test]
    fn tile_accepts_a_grid_exactly_at_the_bound_and_refuses_the_next_one() {
        // 32 columns of 60 pixels and 32 rows of 34: exactly MAX_TILES copies.
        let placement = place(
            WallpaperMode::Tile,
            Size::new(60, 34),
            Size::new(1920, 1088),
        )
        .expect("a placement");
        assert_eq!(placement.copies(), MAX_TILES as usize);

        // One more column is one copy per row too many.
        assert!(
            place(
                WallpaperMode::Tile,
                Size::new(60, 34),
                Size::new(1921, 1088)
            )
            .is_none()
        );
    }

    #[test]
    fn a_zero_sized_image_has_no_placement_in_any_mode_and_a_reason() {
        for mode in MODES {
            assert_eq!(place(mode, Size::new(0, 100), OUTPUT), None);
            assert_eq!(place(mode, Size::new(100, 0), OUTPUT), None);
            assert_eq!(
                plan(mode, Size::new(0, 100), OUTPUT),
                Err(Unusable::Degenerate {
                    image: Size::new(0, 100),
                    output: OUTPUT,
                })
            );
        }
    }

    #[test]
    fn a_zero_sized_output_has_no_placement_in_any_mode_and_a_reason() {
        for mode in MODES {
            assert_eq!(place(mode, Size::new(100, 100), Size::new(1920, 0)), None);
            assert_eq!(
                plan(mode, Size::new(100, 100), Size::new(0, 1080)),
                Err(Unusable::Degenerate {
                    image: Size::new(100, 100),
                    output: Size::new(0, 1080),
                })
            );
        }
    }

    #[test]
    fn a_wide_short_image_never_rounds_away_to_nothing() {
        let rect = place(WallpaperMode::Fit, Size::new(10_000, 1), OUTPUT)
            .expect("a placement")
            .first();

        assert_eq!(rect.width, 1920);
        assert_eq!(rect.height, 1);
    }

    #[test]
    fn every_rectangle_a_placement_returns_has_a_positive_size() {
        for mode in MODES {
            let placement = place(mode, Size::new(37, 91), OUTPUT).expect("a placement");

            for rect in placement.rects() {
                assert!(rect.width > 0 && rect.height > 0, "{rect:?} in {mode:?}");
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// `fill` covers the whole output and preserves the aspect ratio.
        #[test]
        fn fill_covers_and_keeps_the_ratio(image in an_image(), output in an_output()) {
            let rect = place(WallpaperMode::Fill, image, output)
                .expect("a placement")
                .first();

            prop_assert!(
                rect.covers(output),
                "{}×{} does not cover {}",
                image.width,
                image.height,
                output
            );
            prop_assert!(is_undistorted(image, rect), "{rect:?} is distorted");
        }

        /// `fit` fits inside the output and preserves the aspect ratio.
        #[test]
        fn fit_fits_and_keeps_the_ratio(image in an_image(), output in an_output()) {
            let rect = place(WallpaperMode::Fit, image, output)
                .expect("a placement")
                .first();

            prop_assert!(rect.fits_in(output), "{rect:?} escapes {output}");
            prop_assert!(is_undistorted(image, rect), "{rect:?} is distorted");
        }

        /// A scaled dimension is never negative or zero.
        #[test]
        fn no_scaled_dimension_is_negative_or_zero(
            mode in every_mode(),
            image in an_image(),
            output in an_output(),
        ) {
            let Ok(placement) = plan(mode, image, output) else {
                // Above the tiling bound there is no placement at all, which is
                // the documented degradation rather than a bad rectangle.
                return Ok(());
            };

            for rect in placement.rects() {
                prop_assert!(rect.width > 0, "{rect:?}");
                prop_assert!(rect.height > 0, "{rect:?}");
                prop_assert!(rect.x >= -(i64::from(u32::MAX)), "{rect:?}");
                prop_assert!(rect.y >= -(i64::from(u32::MAX)), "{rect:?}");
            }
        }

        /// The same input always yields the same rectangle.
        #[test]
        fn the_placement_is_deterministic(
            mode in every_mode(),
            image in an_image(),
            output in an_output(),
        ) {
            prop_assert_eq!(place(mode, image, output), place(mode, image, output));
        }

        /// `stretch` fills the output exactly, whatever the shapes are.
        #[test]
        fn stretch_always_fills_exactly(image in an_image(), output in an_output()) {
            let rect = place(WallpaperMode::Stretch, image, output)
                .expect("a placement")
                .first();

            prop_assert_eq!(rect, Rect::new(0, 0, output.width, output.height));
        }

        /// Tiling covers the output once, from the origin, and never overlaps
        /// itself.
        #[test]
        fn tile_covers_the_output_once(image in an_image(), output in an_output()) {
            let Ok(placement) = plan(WallpaperMode::Tile, image, output) else {
                // Above the bound the output degrades, which has its own test.
                return Ok(());
            };

            let drawn: HashSet<(i64, i64, u32, u32)> =
                placement.rects().map(|r| (r.x, r.y, r.width, r.height)).collect();
            prop_assert_eq!(drawn.len(), placement.copies(), "copies must not overlap");

            let bounds = placement.bounds();
            prop_assert!(bounds.covers(output), "{:?} does not cover {}", bounds, output);
            prop_assert_eq!((bounds.x, bounds.y), (0, 0), "tiling starts at the origin");
            prop_assert!(
                bounds.right() - i64::from(output.width) < i64::from(image.width),
                "the grid should not run past one extra column"
            );
            prop_assert!(
                bounds.bottom() - i64::from(output.height) < i64::from(image.height),
                "the grid should not run past one extra row"
            );
        }
    }
}
