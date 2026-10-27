//! The one clamp-and-degrade decision of a glass pane, and how it reaches the
//! wire.
//!
//! [`decide`] is the only function that turns a [`Material`] and a
//! [`Capabilities`] answer into protocol parameters, so every consumer clamps
//! the same way and every fallback has a name. [`Request::apply`] then writes the
//! result to a [`GlassTarget`] — a test double in tests, the real
//! `inox_glass_v1` object through [`crate::protocol`] in a session — and skips
//! the `since=2` requests when that object is a version 1 one.

use crate::capabilities::Capabilities;
use crate::material::{Corners, Material, Refraction, usable_corner_radius};
use crate::region::Region;
use crate::{REFRACTION_FLAG_DEPTH, REFRACTION_FLAG_DISPERSION};

/// The refraction parameters of a pane, clamped to what the compositor will
/// draw and in the widths the protocol carries.
///
/// This is deliberately separate from [`crate::Refraction`]: that one is what the
/// tokens ask for, this one is what survives negotiation. Conflating them is how
/// a client ends up "remembering" a clamp and reporting a maximum it never got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefractionRequest {
    /// Band thickness in logical pixels, already clamped to the maximum.
    pub height: u32,
    /// Peak displacement in logical pixels, already clamped to the maximum.
    pub amount: u32,
    /// The `refraction_flag` bitfield: [`crate::REFRACTION_FLAG_DEPTH`] and
    /// [`crate::REFRACTION_FLAG_DISPERSION`], when asked for and supported.
    pub flags: u32,
}

impl RefractionRequest {
    /// Whether these parameters ask for a visible refraction.
    ///
    /// A band with no displacement (or the reverse) is not refraction; the
    /// protocol treats either zero as "disabled" too.
    pub const fn is_effective(&self) -> bool {
        self.height > 0 && self.amount > 0
    }

    /// Whether the chromatic component was asked for and granted.
    pub const fn disperses(&self) -> bool {
        self.flags & REFRACTION_FLAG_DISPERSION != 0
    }

    /// Whether the radial component was asked for.
    pub const fn has_depth(&self) -> bool {
        self.flags & REFRACTION_FLAG_DEPTH != 0
    }
}

/// Everything one pane asks the compositor for, in logical surface coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// Blur radius in logical pixels, already clamped to the maximum; 0 means
    /// "no blur" and is a legitimate value for a refraction-only pane.
    pub radius: u32,
    /// The pane's area, surface-local.
    pub region: Region,
    /// The four corner radii of the shape that refracts.
    pub corners: Corners,
    /// The refraction band, or `None` for a blur-only request.
    pub refraction: Option<RefractionRequest>,
}

/// Why a pane is drawn as plain translucency rather than with a compositor
/// effect.
///
/// Every variant is a *decision*, not an error: the component draws its tint,
/// highlight and shade either way, and the reason exists so a caller can log one
/// honest line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plain {
    /// The tokens ask for neither blur nor refraction (`blur = 0`,
    /// `refraction = 0`): plain translucency, and no glass object is needed.
    BlurDisabled,
    /// The tokens ask for refraction but the compositor cannot refract, and
    /// there is no blur to fall back to.
    RefractionDisabled,
    /// The compositor answered with no usable maximum: it will not draw an
    /// effect for this surface (no renderer support, a locked session, or no
    /// layer role).
    CompositorCannotBlur,
    /// The pane has no area to draw an effect over.
    EmptyPane,
}

/// What a pane should ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Ask the compositor for this, through [`Request::apply`].
    Effect(Request),
    /// Draw plain translucency; the reason is for the caller's log line.
    Plain(Plain),
}

/// The single clamp-and-fallback entry point of the material.
///
/// The precedence is deliberate and is what the tests pin:
///
/// 1. **nothing asked for** — `blur = 0` and no refraction band means the tokens
///    chose plain translucency, whatever the compositor can do
///    ([`Plain::BlurDisabled`]);
/// 2. **nothing to cover** — an empty pane asks for nothing
///    ([`Plain::EmptyPane`]);
/// 3. **nothing available** — neither half of the effect is drawable. If the
///    tokens asked for a blur, the reason is the compositor
///    ([`Plain::CompositorCannotBlur`]); if they asked only for refraction, it is
///    [`Plain::RefractionDisabled`];
/// 4. otherwise a [`Request`] carrying whatever survived: a blur radius clamped
///    to `max_radius`, corner radii clamped to the protocol maximum and to half
///    the region's shorter side, and a refraction band clamped to both refraction
///    maximums with its dispersion bit dropped when the compositor does not
///    advertise it.
///
/// Note the asymmetry in step 4: a pane may end up with `radius = 0` and a
/// refraction band, or the other way round. That is not a compromise — it is the
/// compositor's honest answer, and the client draws its tint over whatever it
/// got.
pub fn decide(material: Material, capabilities: Capabilities, region: Region) -> Outcome {
    if !material.has_effect() {
        return Outcome::Plain(Plain::BlurDisabled);
    }

    if region.is_empty() {
        return Outcome::Plain(Plain::EmptyPane);
    }

    let blur_available = material.blur > 0 && capabilities.can_blur();
    let refraction_available = material.refraction.is_effective() && capabilities.can_refract();

    if !blur_available && !refraction_available {
        return Outcome::Plain(if material.blur == 0 {
            Plain::RefractionDisabled
        } else {
            Plain::CompositorCannotBlur
        });
    }

    let limit = usable_corner_radius(region);

    Outcome::Effect(Request {
        radius: u32::from(material.blur).min(capabilities.max_radius),
        region,
        corners: material.corners.clamped(limit),
        refraction: refraction_available.then(|| RefractionRequest {
            height: u32::from(material.refraction.height).min(capabilities.max_refraction_height),
            amount: u32::from(material.refraction.amount).min(capabilities.max_refraction_amount),
            flags: flags_of(&material.refraction, capabilities.dispersion),
        }),
    })
}

/// The `refraction_flag` bitfield for a band the compositor will draw.
///
/// Depth is passed through if asked for: the protocol's contract is that a client
/// sets the bits it understands and the compositor ignores the ones it does not
/// apply, and [`Capabilities`] does not track depth. Dispersion is only asked for
/// when the compositor advertised it, because a missing fringing is something a
/// user can see and a client must not claim it.
const fn flags_of(refraction: &Refraction, dispersion_supported: bool) -> u32 {
    let mut flags = 0;

    if refraction.depth {
        flags |= REFRACTION_FLAG_DEPTH;
    }
    if refraction.dispersion && dispersion_supported {
        flags |= REFRACTION_FLAG_DISPERSION;
    }

    flags
}

/// Where a [`Request`] can be written: the real `inox_glass_v1` object behind
/// [`crate::protocol`], or a recording test double.
///
/// `supports_refraction` is asked before any `since=2` request is sent; a
/// version 1 object that is asked for refraction silently gets the blur only,
/// which is the protocol's own rule and not a client-side invention.
pub trait GlassTarget {
    /// `set_radius`: the blur radius in logical pixels.
    fn set_radius(&mut self, radius: u32);
    /// `set_region`: the area to frost, surface-local.
    fn set_region(&mut self, region: Region);
    /// `set_corner_radii` (`since=2`): the shape that refracts.
    fn set_corner_radii(&mut self, corners: Corners);
    /// `set_refraction` (`since=2`): the band and its flags.
    fn set_refraction(&mut self, height: u32, amount: u32, flags: u32);
    /// Whether this target is a version 2 object.
    fn supports_refraction(&self) -> bool;
}

impl Request {
    /// Writes the request to `target`, in the protocol's order: blur, region,
    /// then — only on a version 2 object and only when a shape is needed — the
    /// corner radii and the refraction band.
    ///
    /// The corner radii are only sent with a refraction band: the protocol says
    /// `set_corner_radii` "only changes the shape of the refraction", so sending
    /// them for a blur-only pane would describe a shape nothing uses.
    pub fn apply<G: GlassTarget + ?Sized>(&self, target: &mut G) {
        target.set_radius(self.radius);
        target.set_region(self.region);

        if !target.supports_refraction() {
            return;
        }

        if let Some(refraction) = self.refraction {
            target.set_corner_radii(self.corners);
            target.set_refraction(refraction.height, refraction.amount, refraction.flags);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::Refraction;

    const PANE: (u32, u32) = (640, 68);

    /// A compositor that can do everything ADR 0040's maxima allow.
    fn liquid() -> Capabilities {
        Capabilities::from_liquid(
            64,
            128,
            64,
            REFRACTION_FLAG_DEPTH | REFRACTION_FLAG_DISPERSION,
        )
    }

    fn glass(blur: u16, height: u16, amount: u16) -> Material {
        Material::new(
            blur,
            Corners::uniform(8),
            Refraction::new(height, amount, false, true),
        )
    }

    #[test]
    fn a_material_that_asks_for_nothing_is_plain_translucency() {
        assert_eq!(
            decide(Material::plain(0), liquid(), Region::pane(PANE.0, PANE.1)),
            Outcome::Plain(Plain::BlurDisabled)
        );
        assert_eq!(
            decide(
                Material::new(0, Corners::ZERO, Refraction::new(0, 12, true, true)),
                liquid(),
                Region::pane(PANE.0, PANE.1)
            ),
            Outcome::Plain(Plain::BlurDisabled),
            "refraction with no band is no refraction"
        );
    }

    #[test]
    fn an_empty_pane_asks_for_nothing() {
        assert_eq!(
            decide(Material::plain(32), liquid(), Region::pane(0, 68)),
            Outcome::Plain(Plain::EmptyPane)
        );
        assert_eq!(
            decide(Material::plain(32), Capabilities::NONE, Region::pane(0, 68)),
            Outcome::Plain(Plain::EmptyPane),
            "an empty pane is reported as empty before the compositor is blamed"
        );
    }

    #[test]
    fn a_compositor_that_can_do_nothing_falls_back_to_plain_translucency() {
        assert_eq!(
            decide(
                Material::plain(32),
                Capabilities::NONE,
                Region::pane(PANE.0, PANE.1)
            ),
            Outcome::Plain(Plain::CompositorCannotBlur)
        );
        assert_eq!(
            decide(
                glass(0, 24, 12),
                Capabilities::NONE,
                Region::pane(PANE.0, PANE.1)
            ),
            Outcome::Plain(Plain::RefractionDisabled),
            "a refraction-only pane on a compositor with no refraction has nothing left"
        );
    }

    #[test]
    fn a_v1_compositor_gets_the_blur_and_no_refraction() {
        let Outcome::Effect(request) = decide(
            glass(32, 24, 12),
            Capabilities::from_legacy(64),
            Region::pane(PANE.0, PANE.1),
        ) else {
            panic!("a blur is available on a v1 compositor");
        };

        assert_eq!(request.radius, 32);
        assert_eq!(
            request.refraction, None,
            "a v1 compositor cannot refract, so no lens is claimed"
        );
    }

    #[test]
    fn a_refraction_only_pane_has_no_blur_and_still_has_an_effect() {
        let Outcome::Effect(request) =
            decide(glass(0, 24, 12), liquid(), Region::pane(PANE.0, PANE.1))
        else {
            panic!("refraction alone is an effect");
        };

        assert_eq!(request.radius, 0);
        let refraction = request.refraction.expect("the band survives");
        assert_eq!(refraction.height, 24);
        assert_eq!(refraction.amount, 12);
        assert!(refraction.disperses());
    }

    #[test]
    fn every_parameter_is_clamped_to_the_compositor_maximum() {
        let Outcome::Effect(request) = decide(
            glass(200, 500, 500),
            Capabilities::from_liquid(64, 128, 64, REFRACTION_FLAG_DISPERSION),
            Region::pane(PANE.0, PANE.1),
        ) else {
            panic!("an over-large request is clamped, not refused");
        };

        assert_eq!(request.radius, 64, "the blur is clamped to max_radius");
        let refraction = request.refraction.expect("the band survives");
        assert_eq!(refraction.height, 128, "the band is clamped to its maximum");
        assert_eq!(refraction.amount, 64, "the displacement is too");
    }

    #[test]
    fn a_request_below_the_maximum_is_honoured_as_asked() {
        let Outcome::Effect(request) =
            decide(glass(8, 16, 4), liquid(), Region::pane(PANE.0, PANE.1))
        else {
            panic!("a small pane is still a pane");
        };

        assert_eq!(request.radius, 8);
        assert_eq!(
            request.refraction,
            Some(RefractionRequest {
                height: 16,
                amount: 4,
                flags: REFRACTION_FLAG_DISPERSION,
            })
        );
    }

    #[test]
    fn refraction_is_only_asked_for_when_the_pixels_actually_refract() {
        let capped = Capabilities::from_liquid(64, 0, 64, REFRACTION_FLAG_DISPERSION);
        assert!(
            !capped.can_refract(),
            "a band height of zero is no refraction at all"
        );

        let Outcome::Effect(request) = decide(glass(32, 24, 12), capped, Region::pane(PANE.0, 68))
        else {
            panic!("the blur still reaches the compositor");
        };
        assert_eq!(
            request.refraction, None,
            "max_refraction_height = 0 means this compositor does not refract"
        );
    }

    #[test]
    fn dispersion_is_dropped_when_the_compositor_does_not_advertise_it() {
        let no_dispersion = Capabilities::from_liquid(64, 128, 64, REFRACTION_FLAG_DEPTH);
        let Outcome::Effect(request) = decide(
            glass(32, 24, 12),
            no_dispersion,
            Region::pane(PANE.0, PANE.1),
        ) else {
            panic!("refraction without dispersion is still refraction");
        };

        let refraction = request.refraction.expect("the band survives");
        assert!(
            !refraction.disperses(),
            "a client must not claim a fringe the compositor will not draw"
        );
    }

    #[test]
    fn the_depth_bit_is_passed_through_even_though_capabilities_do_not_track_it() {
        let material = Material::new(
            32,
            Corners::uniform(8),
            Refraction::new(24, 12, true, false),
        );

        let Outcome::Effect(request) = decide(material, liquid(), Region::pane(PANE.0, PANE.1))
        else {
            panic!("a pane with a band is an effect");
        };

        let refraction = request.refraction.expect("the band survives");
        assert!(refraction.has_depth());
        assert!(!refraction.disperses());
    }

    #[test]
    fn the_corner_radii_are_clamped_to_half_the_shorter_side() {
        let material = Material::new(
            32,
            Corners {
                top_left: 16_384,
                top_right: 16_384,
                bottom_right: 16_384,
                bottom_left: 16_384,
            },
            Refraction::new(24, 12, false, false),
        );

        let Outcome::Effect(request) = decide(material, liquid(), Region::pane(PANE.0, PANE.1))
        else {
            panic!("a pane with a blur is an effect");
        };

        assert_eq!(
            request.corners,
            Corners::uniform(34),
            "half of the 68 px short side is as round as this pane can be"
        );
    }

    /// The recording target: every call, in the order it was made.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct Recorder {
        version: u32,
        calls: Vec<Call>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        Radius(u32),
        Region(Region),
        Corners(Corners),
        Refraction {
            height: u32,
            amount: u32,
            flags: u32,
        },
    }

    impl GlassTarget for Recorder {
        fn set_radius(&mut self, radius: u32) {
            self.calls.push(Call::Radius(radius));
        }

        fn set_region(&mut self, region: Region) {
            self.calls.push(Call::Region(region));
        }

        fn set_corner_radii(&mut self, corners: Corners) {
            self.calls.push(Call::Corners(corners));
        }

        fn set_refraction(&mut self, height: u32, amount: u32, flags: u32) {
            self.calls.push(Call::Refraction {
                height,
                amount,
                flags,
            });
        }

        fn supports_refraction(&self) -> bool {
            self.version >= 2
        }
    }

    fn request_for(material: Material) -> Request {
        let Outcome::Effect(request) = decide(material, liquid(), Region::pane(PANE.0, PANE.1))
        else {
            panic!("the test material has an effect");
        };

        request
    }

    #[test]
    fn applying_a_request_writes_every_parameter_in_protocol_order() {
        let request = request_for(glass(32, 24, 12));
        let mut recorder = Recorder {
            version: 2,
            ..Recorder::default()
        };

        request.apply(&mut recorder);

        assert_eq!(
            recorder.calls,
            [
                Call::Radius(32),
                Call::Region(Region::pane(PANE.0, PANE.1)),
                Call::Corners(Corners::uniform(8)),
                Call::Refraction {
                    height: 24,
                    amount: 12,
                    flags: REFRACTION_FLAG_DISPERSION,
                },
            ]
        );
    }

    #[test]
    fn a_version_one_object_gets_the_blur_and_the_region_only() {
        let request = request_for(glass(32, 24, 12));
        let mut recorder = Recorder {
            version: 1,
            ..Recorder::default()
        };

        request.apply(&mut recorder);

        assert_eq!(
            recorder.calls,
            [Call::Radius(32), Call::Region(Region::pane(PANE.0, PANE.1)),],
            "the since=2 requests must never reach a version 1 object"
        );
        assert!(
            !request.refraction.is_none(),
            "the request still carries the lens"
        );
    }

    #[test]
    fn a_blur_only_pane_sends_no_corner_radii_on_a_version_two_object() {
        let request = request_for(Material::plain(32));
        let mut recorder = Recorder {
            version: 2,
            ..Recorder::default()
        };

        request.apply(&mut recorder);

        assert_eq!(
            recorder.calls,
            [Call::Radius(32), Call::Region(Region::pane(PANE.0, PANE.1)),],
            "corner radii only describe the refraction shape"
        );
    }
}
