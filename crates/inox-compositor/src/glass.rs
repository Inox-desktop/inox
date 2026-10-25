//! The server half of `inox-glass-v1`, Inox's glass protocol: the parameters, the
//! decisions and the pane list the renderer eats.
//!
//! The normative text is `crates/inox-protocols/protocol/inox-glass-v1.xml` and
//! the bindings are [`inox_protocols::glass`]. This module holds only the *pure*
//! part — no Wayland objects and no GL — so the semantics can be unit-tested
//! without a compositor: [`Frost`] is what a client asked for, [`normalise_region`]
//! is what its `set_region` integers mean, [`capabilities`] and
//! [`liquid_capabilities`] are the honest answers to "will you blur / refract
//! this surface's backdrop", and [`collect_panes`] turns the per-surface
//! parameters into the physical-pixel panes [`crate::render::HeadlessRenderer`]
//! draws.
//!
//! # Two halves of one material
//!
//! Version 1 of the protocol carried the *blur* half: a radius and a region, and
//! the compositor frosts the backdrop behind the region. Version 2 adds the
//! *refraction* half, which is what makes the result read as glass rather than as
//! a translucent sheet: the rounded outline of the pane bends the backdrop, most
//! strongly at the edge and not at all once the band has been crossed, with an
//! optional radial depth term and an optional chromatic split ([`Lens`]).
//!
//! Both halves describe the same pane, so they live in one [`Frost`] and produce
//! one [`GlassPane`]: a pane with a lens is drawn once by the refraction pass,
//! a pane without one keeps the version 1 blur path byte for byte. The two
//! requests are independent, which is what lets a client ask for refraction
//! without blur (`radius = 0`, the case the pixel tests pin) as well as for a
//! frosted pane with square corners.
//!
//! # Where the parameters live
//!
//! The compositor keys glass by the `WlSurface` a glass object was created for
//! (see `InoxState::frost`), not by the glass object. The surface is the identity
//! that both the protocol and the render path are built around: the protocol
//! allows one glass object per surface, the object carries its surface as user
//! data, and the renderer already walks surfaces (`LayerEntry::surface`). Keying
//! on the surface also makes the lookup in `scene_elements` a single hash probe
//! per layer surface, and a destroyed surface can be dropped from the map in the
//! same place its layer entry is.
//!
//! # Why maxima, and why they are frozen
//!
//! [`MAX_BLUR_RADIUS`], [`MAX_CORNER_RADIUS`], [`MAX_REFRACTION_HEIGHT`] and
//! [`MAX_REFRACTION_AMOUNT`] are the compositor's own honest limits, and they are
//! exactly what the two capability events report. The protocol forbids
//! advertising more than the compositor will honour, so both events report their
//! non-zero maxima only when all three of these hold:
//!
//! - the surface has a **layer role** right now. This compositor draws the
//!   backdrop the layer-shell path composites, and turns the request down for
//!   everything else — an ordinary `xdg_toplevel` is drawn by the tiling path,
//!   which does not implement glass. The protocol asks a client to create the
//!   role first and the glass object second for exactly this reason.
//! - the session is **not locked**. While a lock is held no ordinary client is
//!   composited at all, so there is nothing whose backdrop could be drawn.
//!   `InoxState::scene_elements` returns the locked scene before any layer
//!   surface (and therefore any glass pane) is considered; the `locked` input of
//!   [`collect_panes`], [`capabilities`] and [`liquid_capabilities`] pins the
//!   same rule as a test.
//! - the compositor composites this surface through a path that **applies the
//!   effect**. That is the headless path today; the DRM path leaves the scene
//!   sharp and reports 0 rather than claiming an effect it does not draw (see
//!   `crate::backend::drm`).
//!
//! # What the integers mean
//!
//! `set_region` carries four `i32`s verbatim on the wire; this module is where
//! they get their meaning. A width or height that is not positive clears the
//! region ("nothing to draw"), and a region that is otherwise valid is stored
//! as-is: it is *not* clipped to the surface here, because the surface size is
//! only known when the surface commits, and the render path clips the stored
//! rectangle to the surface's own geometry and to the output anyway (see
//! [`collect_panes`]). The stored origin and extent are capped (see
//! [`MAX_REGION_ORIGIN`] / [`MAX_REGION_EXTENT`]) so an absurd request cannot
//! overflow the coordinate arithmetic that later maps the region into physical
//! pixels. The cap is far above any display this compositor can advertise, so no
//! honest client can observe it.
//!
//! The refraction integers get the same treatment: each is clamped to its frozen
//! maximum as it arrives ([`Lens`]), so the request cannot make the later
//! mapping arithmetic unbounded, and the corners are additionally clamped to half
//! the pane's shorter side when the pane is realised ([`Lens::resolve`]) — a
//! "fully round" pane is a legitimate request, not an error.

use smithay::utils::{Logical, Physical, Point, Rectangle, Size};

/// The largest blur radius this compositor applies, in logical pixels.
///
/// Frozen at 64 and reported verbatim by the `capabilities` event. Larger
/// requests are clamped rather than refused, which is what lets a client that
/// scales its pane keep asking for "as much as you can do".
pub const MAX_BLUR_RADIUS: u32 = 64;

/// The largest corner radius of the refracted shape, in logical pixels.
///
/// Reported by the `liquid_capabilities` event and clamped on arrival. It is a
/// sanity bound, not a design decision: the corners are also clamped to half the
/// pane's shorter side, so 16384 is simply "more than any display this composer
/// can host".
pub const MAX_CORNER_RADIUS: u32 = 16384;

/// The largest refraction band thickness, in logical pixels.
///
/// Reported by the `liquid_capabilities` event. A band of 128 logical pixels is
/// already thicker than any reference pane; the cap exists so a client cannot
/// ask for a band whose work per frame grows without bound.
pub const MAX_REFRACTION_HEIGHT: u32 = 128;

/// The largest refraction displacement, in logical pixels.
///
/// Reported by the `liquid_capabilities` event. This is also the widest offset
/// the refraction samples at, and therefore the amount the source rectangle of
/// the refraction pass is expanded by.
pub const MAX_REFRACTION_AMOUNT: u32 = 64;

/// The `refraction_flag` bit for the optional radial depth component.
///
/// A wire value, and the reason this module does not import the generated
/// bitflags itself: the pure half only needs the bits, and
/// `the_refraction_flag_bits_match_the_protocol` below pins them against
/// `inox_protocols` so the two cannot drift.
pub const REFRACTION_FLAG_DEPTH: u32 = 1;

/// The `refraction_flag` bit for the optional chromatic dispersion.
pub const REFRACTION_FLAG_DISPERSION: u32 = 2;

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

/// The refraction half of a request: the shape and the band that bends it.
///
/// All four values are in the logical pixels of the surface the pane belongs to,
/// like everything else a glass object sends; [`Lens::resolve`] is where they
/// become physical pixels.
///
/// The default is "no refraction": no corners, no band. A client that only wants
/// a blur therefore keeps sending exactly what version 1 sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lens {
    /// Corner radii in logical pixels, ordered `[top_left, top_right,
    /// bottom_right, bottom_left]`, each already clamped to
    /// [`MAX_CORNER_RADIUS`].
    pub corners: [u32; 4],
    /// Band thickness in logical pixels, already clamped to
    /// [`MAX_REFRACTION_HEIGHT`]; 0 means "no refraction".
    pub height: u32,
    /// Peak displacement in logical pixels, already clamped to
    /// [`MAX_REFRACTION_AMOUNT`]; 0 means "no refraction".
    pub amount: u32,
    /// Apply the optional radial (towards the centre) depth component.
    pub depth: bool,
    /// Apply the optional chromatic dispersion.
    pub dispersion: bool,
}

impl Lens {
    /// Clamp a requested corner radius the way `set_corner_radii` does.
    #[must_use]
    pub fn clamp_corner_radius(requested: u32) -> u32 {
        requested.min(MAX_CORNER_RADIUS)
    }

    /// Clamp a requested band thickness the way `set_refraction` does.
    #[must_use]
    pub fn clamp_height(requested: u32) -> u32 {
        requested.min(MAX_REFRACTION_HEIGHT)
    }

    /// Clamp a requested displacement the way `set_refraction` does.
    #[must_use]
    pub fn clamp_amount(requested: u32) -> u32 {
        requested.min(MAX_REFRACTION_AMOUNT)
    }

    /// Apply a `set_corner_radii` request, in wire order.
    pub fn set_corner_radii(&mut self, corners: [u32; 4]) {
        self.corners = corners.map(Self::clamp_corner_radius);
    }

    /// Apply a `set_refraction` request.
    ///
    /// A zero `height` or a zero `amount` disables refraction, which is how a
    /// client asks for plain backdrop blur on the same object; the flags are
    /// stored regardless, so a client may set them once and toggle the band.
    pub fn set_refraction(&mut self, height: u32, amount: u32, depth: bool, dispersion: bool) {
        self.height = Self::clamp_height(height);
        self.amount = Self::clamp_amount(amount);
        self.depth = depth;
        self.dispersion = dispersion;
    }

    /// Whether this lens would move any pixel: both a band and a displacement.
    #[must_use]
    pub fn is_effective(&self) -> bool {
        self.height > 0 && self.amount > 0
    }

    /// The `refraction_flag` bits this lens asked for.
    #[must_use]
    pub fn flags(&self) -> u32 {
        let mut flags = 0;
        if self.depth {
            flags |= REFRACTION_FLAG_DEPTH;
        }
        if self.dispersion {
            flags |= REFRACTION_FLAG_DISPERSION;
        }
        flags
    }

    /// Resolve this logical lens into the physical pixels the renderer draws.
    ///
    /// `shape` is the pane's own rectangle in the target's physical pixels,
    /// before clipping to the output. The scale factor maps logical to physical
    /// exactly like the pane's region, and every value is then bounded by the
    /// shape: a corner may be at most half of the pane's shorter side (a
    /// "fully round" pane), and neither the band nor the displacement may exceed
    /// the pane itself. Returns `None` when there is nothing to draw, so a caller
    /// cannot accidentally emit a zero-radius, zero-band lens pass.
    #[must_use]
    pub fn resolve(&self, shape: Rectangle<i32, Physical>, scale: f64) -> Option<LensPass> {
        if !self.is_effective() {
            return None;
        }
        let short = shape.size.w.min(shape.size.h);
        if short <= 0 {
            return None;
        }
        let short = short as f32;
        let scale = scale as f32;
        let corner_limit = short / 2.0;
        let mut corners = [0.0f32; 4];
        for (corner, requested) in corners.iter_mut().zip(self.corners) {
            *corner = (requested as f32 * scale).min(corner_limit);
        }
        Some(LensPass {
            corners,
            height: (self.height as f32 * scale).min(short),
            amount: (self.amount as f32 * scale).min(short),
            depth: self.depth,
            dispersion: self.dispersion,
            shape,
        })
    }
}

/// One pane's refraction, resolved into the target's physical pixels.
///
/// Produced by [`Lens::resolve`]; this is what the refraction pass in
/// `crate::render` consumes. `shape` is the pane's full rectangle, so the pass
/// can keep the rounded outline in the right place even where the drawable
/// region was clipped by the output edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LensPass {
    /// Corner radii in physical pixels, in wire order (top-left first).
    pub corners: [f32; 4],
    /// Band thickness in physical pixels.
    pub height: f32,
    /// Peak displacement in physical pixels.
    pub amount: f32,
    /// Whether the optional radial depth component is applied.
    pub depth: bool,
    /// Whether the optional chromatic dispersion is applied.
    pub dispersion: bool,
    /// The pane's full rectangle in physical pixels, before clipping to the output.
    pub shape: Rectangle<i32, Physical>,
}

/// What one surface asked for: a blur radius, an optional region and a lens.
///
/// The default is what a freshly created glass object means — "nothing", because
/// a client that creates an object and sets nothing has asked for nothing (and
/// the protocol's initial region is empty).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Frost {
    /// Blur radius in logical pixels, already clamped to [`MAX_BLUR_RADIUS`].
    pub radius: u32,
    /// Region to draw, in surface-local logical coordinates.
    pub region: Option<Rectangle<i32, Logical>>,
    /// Refraction parameters, in surface-local logical coordinates.
    pub lens: Lens,
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

    /// Apply a `set_corner_radii` request.
    pub fn set_corner_radii(&mut self, corners: [u32; 4]) {
        self.lens.set_corner_radii(corners);
    }

    /// Apply a `set_refraction` request.
    pub fn set_refraction(&mut self, height: u32, amount: u32, depth: bool, dispersion: bool) {
        self.lens.set_refraction(height, amount, depth, dispersion);
    }

    /// Whether this frost draws anything at all: a region, and either a blur or
    /// an effective lens. A radius alone, or a lens alone, draws nothing.
    #[must_use]
    pub fn is_effective(&self) -> bool {
        self.region.is_some() && (self.radius > 0 || self.lens.is_effective())
    }
}

/// The meaning of a `set_region` request.
///
/// `None` means "nothing to draw": the protocol says a width or height that is
/// not positive clears the region, so a client can stop drawing a pane without
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

/// What this compositor draws for a surface: the two capability events' contents.
///
/// `max_radius` is what the version 1 `capabilities` event reports; the three
/// refraction fields are what version 2's `liquid_capabilities` adds. Both
/// events carry the same honesty rule (see the module documentation): all zero
/// unless the surface has a layer role, the session is unlocked, and the
/// compositing path really applies the effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LiquidCapabilities {
    /// Largest accepted blur radius in logical pixels, 0 if none.
    pub max_radius: u32,
    /// Largest accepted refraction band thickness, 0 if none.
    pub max_refraction_height: u32,
    /// Largest accepted refraction displacement, 0 if none.
    pub max_refraction_amount: u32,
    /// The [`REFRACTION_FLAG_DEPTH`] / [`REFRACTION_FLAG_DISPERSION`] bits this
    /// compositor applies, 0 when it refracts nothing.
    pub flags: u32,
}

impl LiquidCapabilities {
    /// The honest answer for a surface this compositor will not draw for.
    pub const NONE: Self = Self {
        max_radius: 0,
        max_refraction_height: 0,
        max_refraction_amount: 0,
        flags: 0,
    };

    /// Whether this compositor blurs this surface's backdrop.
    #[must_use]
    pub fn can_blur(&self) -> bool {
        self.max_radius > 0
    }

    /// Whether this compositor refracts this surface's backdrop.
    ///
    /// Both maxima have to be non-zero: a band without a displacement (or the
    /// other way round) moves nothing, so it would be a promise with no effect.
    #[must_use]
    pub fn can_refract(&self) -> bool {
        self.max_refraction_height > 0 && self.max_refraction_amount > 0
    }
}

/// The `capabilities` event's `max_radius` for a surface.
///
/// `layer_role` is whether the surface is one of this compositor's layer
/// surfaces right now, `locked` whether a session lock is held, and
/// `frosting_supported` whether the compositor composites this surface through a
/// path that applies the effect. All three must hold, or the honest answer is 0:
/// the client decides its fallback (plain translucency) from this number, so
/// advertising an effect the compositor would not draw is the one thing the
/// event must never do.
#[must_use]
pub fn capabilities(layer_role: bool, locked: bool, frosting_supported: bool) -> u32 {
    liquid_capabilities(layer_role, locked, frosting_supported).max_radius
}

/// The `liquid_capabilities` event's contents for a surface.
///
/// The same three-way rule as [`capabilities`], and deliberately computed from
/// one place: the version 1 event reports this struct's `max_radius`, so the two
/// events can never disagree about the blur. The flags are the full set this
/// compositor implements — depth and dispersion — because the headless pass
/// draws both; they are 0 exactly when the maxima are.
#[must_use]
pub fn liquid_capabilities(
    layer_role: bool,
    locked: bool,
    frosting_supported: bool,
) -> LiquidCapabilities {
    if frosting_supported && layer_role && !locked {
        LiquidCapabilities {
            max_radius: MAX_BLUR_RADIUS,
            max_refraction_height: MAX_REFRACTION_HEIGHT,
            max_refraction_amount: MAX_REFRACTION_AMOUNT,
            flags: REFRACTION_FLAG_DEPTH | REFRACTION_FLAG_DISPERSION,
        }
    } else {
        LiquidCapabilities::NONE
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
    /// element list: the pane draws what is behind that element.
    pub at: usize,
    /// The surface's own box in the output's global logical coordinates.
    pub geometry: Rectangle<i32, Logical>,
    /// What the client asked for.
    pub frost: Frost,
}

/// One glass backdrop to composite, in one output's physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlassPane {
    /// Index of the pane's own back-most render element (see [`PaneInput::at`]).
    pub at: usize,
    /// The part of the target to replace with the glass backdrop.
    pub region: Rectangle<i32, Physical>,
    /// Blur radius in physical pixels.
    pub radius: f32,
    /// Refraction parameters in physical pixels, or `None` for the blur-only path.
    pub lens: Option<LensPass>,
}

/// Turn the per-surface parameters into this output's panes, back-most first.
///
/// A pane is produced for every input that has an effective [`Frost`] and whose
/// region — clipped to the surface's own geometry and to `bounds` — still has
/// area. `output_loc` is the output's top-left corner in the global logical
/// space, `bounds` its physical area (so a pane can never name a region outside
/// the target it will be drawn into), and `scale` the output's scale factor:
/// fractional scaling maps the region from surface-local logical coordinates
/// into physical pixels with the same factor the scene itself is drawn at, and
/// the lens's corners, band and displacement are scaled by it too.
///
/// The lens is resolved against the pane's **unclipped** physical rectangle and
/// then attached to the clipped region, so a pane hanging off the edge of the
/// output keeps its rounded outline where it belongs instead of growing a new
/// one at the clip.
///
/// `locked` short-circuits to an empty list: while a session lock is held, no
/// ordinary client is composited, so no pane may be drawn. This is the same rule
/// the locked branch of `InoxState::scene_elements` enforces by returning early,
/// and the unit test below pins it as a behaviour rather than a comment.
///
/// The returned list is sorted by `at` **descending**, which is back-most first:
/// the renderer draws from the back of the scene towards the front so that a pane
/// never samples a pane that was supposed to be behind it.
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
            let lens = input.frost.lens.resolve(physical, scale);
            if input.frost.radius == 0 && lens.is_none() {
                return None;
            }
            Some(GlassPane {
                at: input.at,
                region,
                radius: input.frost.radius as f32 * scale as f32,
                lens,
            })
        })
        .collect::<Vec<_>>();

    // Back-most first: the renderer draws from the back of the scene towards the
    // front, so a pane never samples a pane that was supposed to be behind it.
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

    fn physical(x: i32, y: i32, width: i32, height: i32) -> Rectangle<i32, Physical> {
        Rectangle::new(Point::from((x, y)), Size::from((width, height)))
    }

    #[test]
    fn a_fresh_frost_object_asks_for_nothing() {
        let frost = Frost::default();
        assert_eq!(frost.radius, 0);
        assert_eq!(frost.region, None);
        assert_eq!(frost.lens, Lens::default());
        assert!(!frost.lens.is_effective());
        assert!(!frost.is_effective());
    }

    #[test]
    fn a_zero_or_negative_extent_clears_the_region() {
        let mut frost = Frost::default();
        frost.set_region(4, 4, 10, 10);
        assert!(frost.region.is_some());
        // A client that wants to stop drawing a pane without destroying the
        // object: any non-positive extent means "nothing to draw".
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
            "a radius without a region draws nothing"
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
    fn liquid_capabilities_report_the_lens_and_follow_the_same_three_way_rule() {
        let liquid = liquid_capabilities(true, false, true);
        assert_eq!(liquid.max_radius, MAX_BLUR_RADIUS);
        assert_eq!(liquid.max_refraction_height, MAX_REFRACTION_HEIGHT);
        assert_eq!(liquid.max_refraction_amount, MAX_REFRACTION_AMOUNT);
        assert_eq!(
            liquid.flags,
            REFRACTION_FLAG_DEPTH | REFRACTION_FLAG_DISPERSION
        );
        assert!(liquid.can_blur() && liquid.can_refract());

        for (layer_role, locked, supported) in [
            (false, false, true),
            (true, true, true),
            (true, false, false),
        ] {
            let liquid = liquid_capabilities(layer_role, locked, supported);
            assert_eq!(liquid, LiquidCapabilities::NONE);
            assert!(!liquid.can_blur());
            assert!(!liquid.can_refract());
        }

        // The version 1 event reports the same blur maximum as the version 2 one:
        // a client that only listens for the old event is not told something
        // different from a client that listens for both.
        for (layer_role, locked, supported) in [(true, false, true), (false, true, true)] {
            assert_eq!(
                capabilities(layer_role, locked, supported),
                liquid_capabilities(layer_role, locked, supported).max_radius
            );
        }
    }

    #[test]
    fn the_refraction_flag_bits_match_the_protocol() {
        use inox_protocols::glass::inox_glass::client::inox_glass_v1::RefractionFlag;

        assert_eq!(
            REFRACTION_FLAG_DEPTH,
            u32::from(RefractionFlag::Depth),
            "the depth bit is a wire value"
        );
        assert_eq!(
            REFRACTION_FLAG_DISPERSION,
            u32::from(RefractionFlag::Dispersion),
            "the dispersion bit is a wire value"
        );
        let mut lens = Lens::default();
        lens.set_refraction(1, 1, true, true);
        assert_eq!(
            lens.flags(),
            (RefractionFlag::Depth | RefractionFlag::Dispersion).bits()
        );
    }

    #[test]
    fn the_lens_is_clamped_and_a_zero_dimension_disables_it() {
        let mut lens = Lens::default();
        lens.set_refraction(u32::MAX, u32::MAX, false, false);
        assert_eq!(lens.height, MAX_REFRACTION_HEIGHT);
        assert_eq!(lens.amount, MAX_REFRACTION_AMOUNT);
        assert!(lens.is_effective());

        // Either dimension being zero is the protocol's "no refraction" value.
        for (height, amount) in [(0, 10), (10, 0), (0, 0)] {
            let mut lens = Lens::default();
            lens.set_refraction(height, amount, true, true);
            assert!(!lens.is_effective(), "({height}, {amount}) must be no lens");
            assert_eq!(
                lens.resolve(physical(0, 0, 100, 100), 1.0),
                None,
                "an ineffective lens must not resolve into a pass"
            );
        }

        lens.set_corner_radii([u32::MAX, 0, 4, 5]);
        assert_eq!(
            lens.corners,
            [MAX_CORNER_RADIUS, 0, 4, 5],
            "each corner is clamped on arrival, in wire order"
        );
    }

    #[test]
    fn a_lens_resolves_to_physical_pixels_bounded_by_the_pane() {
        let mut lens = Lens::default();
        lens.set_refraction(32, 16, true, false);
        lens.set_corner_radii([8, 8, 8, 8]);

        let resolved = lens
            .resolve(physical(10, 20, 120, 80), 1.0)
            .expect("an effective lens resolves");
        assert_eq!(resolved.corners, [8.0; 4]);
        assert_eq!(resolved.height, 32.0);
        assert_eq!(resolved.amount, 16.0);
        assert!(resolved.depth);
        assert!(!resolved.dispersion);
        assert_eq!(resolved.shape, physical(10, 20, 120, 80));

        // A "fully round" corner and an oversized band are clamped to the pane
        // rather than refused: the shorter side is 80, so half of it is 40, the
        // band may not be thicker than the pane itself, and the displacement keeps
        // its own frozen maximum (64) because that bound is smaller here.
        lens.set_corner_radii([u32::MAX; 4]);
        lens.set_refraction(u32::MAX, u32::MAX, false, false);
        let resolved = lens
            .resolve(physical(0, 0, 200, 80), 1.0)
            .expect("a clamped lens still resolves");
        assert_eq!(resolved.corners, [40.0; 4]);
        assert_eq!(resolved.height, 80.0);
        assert_eq!(resolved.amount, MAX_REFRACTION_AMOUNT as f32);

        // Scale maps logical to physical, corners included.
        let mut lens = Lens::default();
        lens.set_refraction(24, 12, false, true);
        lens.set_corner_radii([6, 6, 6, 6]);
        let resolved = lens
            .resolve(physical(0, 0, 300, 300), 1.5)
            .expect("an effective lens resolves");
        assert_eq!(resolved.corners, [9.0; 4]);
        assert_eq!(resolved.height, 36.0);
        assert_eq!(resolved.amount, 18.0);
        assert!(resolved.dispersion);
    }

    #[test]
    fn a_locked_session_contributes_no_panes() {
        let inputs = [PaneInput {
            at: 3,
            geometry: logical(0, 0, 200, 100),
            frost: Frost {
                radius: 24,
                region: normalise_region(0, 0, 200, 100),
                lens: Lens::default(),
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
            "a locked session composites no ordinary client, so it draws nothing"
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
                lens: Lens::default(),
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
        assert_eq!(panes[0].lens, None, "a v1 request carries no lens");
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
                lens: Lens::default(),
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
                lens: Lens::default(),
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
                lens: Lens::default(),
            },
        };
        let no_radius = PaneInput {
            at: 0,
            geometry,
            frost: Frost {
                radius: 0,
                region: normalise_region(0, 0, 100, 100),
                lens: Lens::default(),
            },
        };
        let empty_region = PaneInput {
            at: 0,
            geometry,
            frost: Frost {
                radius: 16,
                region: normalise_region(0, 0, 0, 0),
                lens: Lens::default(),
            },
        };
        assert!(collect_panes(false, Point::from((0, 0)), bounds, 1.0, &[no_region]).is_empty());
        assert!(collect_panes(false, Point::from((0, 0)), bounds, 1.0, &[no_radius]).is_empty());
        assert!(collect_panes(false, Point::from((0, 0)), bounds, 1.0, &[empty_region]).is_empty());
    }

    #[test]
    fn refraction_alone_is_a_pane_and_carries_its_lens_through_clipping() {
        let bounds = Rectangle::from_size(Size::from((320, 240)));
        let mut lens = Lens::default();
        lens.set_refraction(32, 16, true, true);
        lens.set_corner_radii([12, 12, 12, 12]);
        // No blur at all: the lens is the whole effect.
        let inputs = [PaneInput {
            at: 2,
            geometry: logical(0, 0, 120, 80),
            frost: Frost {
                radius: 0,
                region: normalise_region(0, 0, 120, 80),
                lens,
            },
        }];
        let panes = collect_panes(false, Point::from((0, 0)), bounds, 1.0, &inputs);
        assert_eq!(panes.len(), 1, "a lens without a blur still draws a pane");
        assert_eq!(panes[0].radius, 0.0);
        let pass = panes[0].lens.expect("the lens resolved");
        assert_eq!(pass.corners, [12.0; 4]);
        assert_eq!(pass.height, 32.0);
        assert_eq!(pass.amount, 16.0);

        // A pane half off the left edge: the drawable region is clipped, but the
        // shape — and therefore the rounded outline the pass computes — stays the
        // pane's own rectangle.
        let inputs = [PaneInput {
            at: 0,
            geometry: logical(-60, 0, 120, 80),
            frost: Frost {
                radius: 0,
                region: normalise_region(0, 0, 120, 80),
                lens,
            },
        }];
        let panes = collect_panes(false, Point::from((0, 0)), bounds, 1.0, &inputs);
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].region.loc, Point::from((0, 0)));
        assert_eq!(panes[0].region.size, Size::from((60, 80)));
        assert_eq!(
            panes[0].lens.expect("the lens resolved").shape,
            Rectangle::new(Point::from((-60, 0)), Size::from((120, 80))),
            "the shape is the pane's own rectangle, not the clip"
        );
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
                lens: Lens::default(),
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
            "the renderer draws from the back of the scene towards the front"
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
        fn lens_clamping_is_idempotent_and_monotone(first in any::<u32>(), second in any::<u32>()) {
            for (clamp, maximum) in [
                (Lens::clamp_corner_radius as fn(u32) -> u32, MAX_CORNER_RADIUS),
                (Lens::clamp_height as fn(u32) -> u32, MAX_REFRACTION_HEIGHT),
                (Lens::clamp_amount as fn(u32) -> u32, MAX_REFRACTION_AMOUNT),
            ] {
                let once = clamp(first);
                prop_assert_eq!(clamp(once), once);
                prop_assert!(once <= maximum);
                if first <= second {
                    prop_assert!(clamp(first) <= clamp(second));
                }
            }
        }

        #[test]
        fn a_resolved_lens_never_exceeds_the_pane(
            width in 1i32..400,
            height in 1i32..400,
            corner in any::<u32>(),
            band in any::<u32>(),
            amount in any::<u32>(),
            scale in prop::sample::select(vec![0.25f64, 1.0, 1.5, 2.0, 8.0]),
        ) {
            let mut lens = Lens::default();
            lens.set_corner_radii([corner; 4]);
            lens.set_refraction(band, amount, true, false);
            let shape = physical(0, 0, width, height);
            match lens.resolve(shape, scale) {
                None => prop_assert!(!lens.is_effective() || width <= 0 || height <= 0),
                Some(pass) => {
                    let short = width.min(height) as f32;
                    for radius in pass.corners {
                        prop_assert!(radius >= 0.0 && radius <= short / 2.0);
                    }
                    prop_assert!(pass.height > 0.0 && pass.height <= short);
                    prop_assert!(pass.amount > 0.0 && pass.amount <= short);
                    prop_assert_eq!(pass.shape, shape);
                }
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
            band in any::<u32>(),
            amount in any::<u32>(),
        ) {
            let (gx, gy, gw, gh) = geometry;
            let (rx, ry, rw, rh) = region;
            let bounds = Rectangle::from_size(Size::from((320, 240)));
            let mut lens = Lens::default();
            lens.set_refraction(band, amount, false, true);
            let inputs = [PaneInput {
                at: 4,
                geometry: logical(gx, gy, gw, gh),
                frost: Frost { radius: 32, region: normalise_region(rx, ry, rw, rh), lens },
            }];
            let panes = collect_panes(locked, Point::from((0, 0)), bounds, 1.0, &inputs);
            if locked {
                prop_assert!(panes.is_empty());
            }
            for pane in &panes {
                prop_assert!(!pane.region.is_empty());
                prop_assert_eq!(pane.region.intersection(bounds), Some(pane.region));
                if let Some(pass) = pane.lens {
                    // The lens covers the whole pane it was resolved for.
                    prop_assert_eq!(pane.region.intersection(pass.shape), Some(pane.region));
                }
            }
            let ats = panes.iter().map(|pane| pane.at).collect::<Vec<_>>();
            prop_assert!(ats.windows(2).all(|pair| pair[0] >= pair[1]));
        }
    }
}
