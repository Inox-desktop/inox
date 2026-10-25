//! The material contract, exercised from outside the crate.
//!
//! Everything here goes through the public API only, because that is what the
//! panel and every later L3 component consume: if a property of the clamps is
//! only true through a crate-private helper, it is not a property of the
//! contract.
//!
//! The proptests cover the four promises `decide` makes:
//!
//! - **idempotent**: feeding a negotiated request back in changes nothing;
//! - **monotonic**: a compositor that can do more never yields less;
//! - **honest**: a zero maximum never yields a request, and no request ever
//!   exceeds a maximum or claims a component the compositor did not advertise;
//! - **quiet**: an empty pane, a disabled material and a version 1 object all
//!   come out as "nothing asked for" rather than as an error.

use inox_glass::{
    Capabilities, Corners, GlassTarget, MAX_CORNER_RADIUS, Material, Outcome, Plain, Refraction,
    Region, Request, decide,
};
use proptest::prelude::*;

/// A request as a recording target saw it, after [`GlassTarget`] was called.
#[derive(Debug, Default, PartialEq, Eq)]
struct Recorder {
    version: u32,
    radius: Option<u32>,
    region: Option<Region>,
    corners: Option<Corners>,
    refraction: Option<(u32, u32, u32)>,
}

impl GlassTarget for Recorder {
    fn set_radius(&mut self, radius: u32) {
        self.radius = Some(radius);
    }

    fn set_region(&mut self, region: Region) {
        self.region = Some(region);
    }

    fn set_corner_radii(&mut self, corners: Corners) {
        self.corners = Some(corners);
    }

    fn set_refraction(&mut self, height: u32, amount: u32, flags: u32) {
        self.refraction = Some((height, amount, flags));
    }

    fn supports_refraction(&self) -> bool {
        self.version >= 2
    }
}

/// The `refraction_flag` word for "the compositor applies dispersion", built the
/// way the wire builds it.
fn dispersion_flag(dispersion: bool) -> u32 {
    if dispersion { 2 } else { 0 }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Re-deciding with the negotiated values is a no-op: a client that stores
    /// the clamp cannot drift into a second, smaller one on the next cycle.
    #[test]
    fn deciding_on_a_negotiated_request_changes_nothing(
        blur in 0u16..=u16::MAX,
        top_left in any::<u16>(),
        top_right in any::<u16>(),
        bottom_right in any::<u16>(),
        bottom_left in any::<u16>(),
        height in 0u16..=u16::MAX,
        amount in 0u16..=u16::MAX,
        depth in any::<bool>(),
        dispersion in any::<bool>(),
        max_radius in 0u32..=4_096,
        max_height in 0u32..=4_096,
        max_amount in 0u32..=4_096,
        advertised_dispersion in any::<bool>(),
        width in 1u32..=4_096,
        pane_height in 1u32..=4_096,
    ) {
        let material = Material::new(
            blur,
            Corners { top_left, top_right, bottom_right, bottom_left },
            Refraction::new(height, amount, depth, dispersion),
        );
        let capabilities = Capabilities::from_liquid(
            max_radius,
            max_height,
            max_amount,
            dispersion_flag(advertised_dispersion),
        );
        let region = Region::pane(width, pane_height);

        let Outcome::Effect(first) = decide(material, capabilities, region) else {
            return Ok(());
        };

        let again = Material::new(
            u16::try_from(first.radius).expect("the clamp keeps a radius inside u16"),
            first.corners,
            match first.refraction {
                Some(refraction) => Refraction::new(
                    u16::try_from(refraction.height).expect("the clamp keeps a band inside u16"),
                    u16::try_from(refraction.amount).expect("the clamp keeps an offset inside u16"),
                    refraction.has_depth(),
                    refraction.disperses(),
                ),
                None => Refraction::NONE,
            },
        );

        prop_assert_eq!(decide(again, capabilities, region), Outcome::Effect(first));
    }

    /// More capability never means less effect: a compositor that can do at
    /// least as much as another gets at least as large a request.
    #[test]
    fn a_more_capable_compositor_never_gets_a_smaller_request(
        blur in 0u16..=u16::MAX,
        height in 0u16..=u16::MAX,
        amount in 0u16..=u16::MAX,
        base_radius in 0u32..=512,
        extra_radius in 0u32..=512,
        base_height in 0u32..=512,
        extra_height in 0u32..=512,
        base_amount in 0u32..=512,
        extra_amount in 0u32..=512,
        base_dispersion in any::<bool>(),
        extra_dispersion in any::<bool>(),
        width in 1u32..=4_096,
        pane_height in 1u32..=4_096,
    ) {
        // Built by construction so the ordering precondition always holds:
        // rejecting random pairs until they happen to be ordered would burn the
        // case budget on the generator instead of on the property.
        let (smaller_radius, larger_radius) = (base_radius, base_radius + extra_radius);
        let (smaller_height, larger_height) = (base_height, base_height + extra_height);
        let (smaller_amount, larger_amount) = (base_amount, base_amount + extra_amount);
        let smaller_dispersion = base_dispersion;
        let larger_dispersion = base_dispersion || extra_dispersion;

        let material = Material::new(
            blur,
            Corners::uniform(8),
            Refraction::new(height, amount, false, true),
        );
        let region = Region::pane(width, pane_height);

        let smaller = Capabilities::from_liquid(
            smaller_radius,
            smaller_height,
            smaller_amount,
            dispersion_flag(smaller_dispersion),
        );
        let larger = Capabilities::from_liquid(
            larger_radius,
            larger_height,
            larger_amount,
            dispersion_flag(larger_dispersion),
        );

        match (decide(material, smaller, region), decide(material, larger, region)) {
            (Outcome::Effect(less), Outcome::Effect(more)) => {
                prop_assert!(more.radius >= less.radius);
                prop_assert_eq!(more.region, less.region);
                prop_assert_eq!(more.corners, less.corners);

                match (less.refraction, more.refraction) {
                    (Some(less), Some(more)) => {
                        prop_assert!(more.height >= less.height);
                        prop_assert!(more.amount >= less.amount);
                        prop_assert_eq!(more.flags & less.flags, less.flags);
                    }
                    (Some(_), None) => {
                        prop_assert!(false, "more capability lost the refraction band");
                    }
                    (None, _) => {}
                }
            }
            (Outcome::Effect(_), Outcome::Plain(reason)) => {
                prop_assert!(false, "a more capable compositor fell back to plain: {reason:?}");
            }
            (Outcome::Plain(_), _) => {}
        }
    }

    /// No request ever exceeds a maximum, and no request claims the dispersion
    /// component the compositor did not advertise.
    #[test]
    fn every_request_stays_inside_the_advertised_maxima(
        blur in 0u16..=u16::MAX,
        height in 0u16..=u16::MAX,
        amount in 0u16..=u16::MAX,
        depth in any::<bool>(),
        dispersion in any::<bool>(),
        max_radius in 0u32..=4_096,
        max_height in 0u32..=4_096,
        max_amount in 0u32..=4_096,
        advertised_dispersion in any::<bool>(),
        width in 1u32..=4_096,
        pane_height in 1u32..=4_096,
    ) {
        let material = Material::new(
            blur,
            Corners::uniform(MAX_CORNER_RADIUS),
            Refraction::new(height, amount, depth, dispersion),
        );
        let capabilities = Capabilities::from_liquid(
            max_radius,
            max_height,
            max_amount,
            dispersion_flag(advertised_dispersion),
        );
        let region = Region::pane(width, pane_height);

        let Outcome::Effect(request) = decide(material, capabilities, region) else {
            return Ok(());
        };

        prop_assert!(request.radius <= capabilities.max_radius);
        prop_assert_eq!(request.region, region);

        let half_short_side = u16::try_from(width.min(pane_height) / 2).expect("clamped");
        prop_assert!(request.corners.max_radius() <= half_short_side.min(MAX_CORNER_RADIUS));

        match request.refraction {
            Some(refraction) => {
                prop_assert!(refraction.is_effective());
                prop_assert!(refraction.height <= capabilities.max_refraction_height);
                prop_assert!(refraction.amount <= capabilities.max_refraction_amount);
                prop_assert_eq!(
                    refraction.disperses(),
                    dispersion && advertised_dispersion,
                    "the fringe is claimed only when asked for and advertised"
                );
                prop_assert_eq!(refraction.has_depth(), depth);
            }
            None => {
                prop_assert!(
                    !material.refraction.is_effective() || !capabilities.can_refract(),
                    "a refraction band that could be drawn must be asked for"
                );
            }
        }
    }

    /// A compositor that advertises nothing gets no request at all — never a
    /// request with zeros in it, which would be a claim of an effect.
    #[test]
    fn a_zero_capability_compositor_never_gets_a_request(
        blur in 0u16..=u16::MAX,
        height in 0u16..=u16::MAX,
        amount in 0u16..=u16::MAX,
        width in any::<u32>(),
        pane_height in any::<u32>(),
    ) {
        let material = Material::new(
            blur,
            Corners::uniform(12),
            Refraction::new(height, amount, true, true),
        );

        let outcome = decide(material, Capabilities::NONE, Region::pane(width, pane_height));

        prop_assert!(matches!(outcome, Outcome::Plain(_)), "{outcome:?}");
    }

    /// A pane with no area asks for nothing, and a material that asks for nothing
    /// never becomes an effect.
    #[test]
    fn an_empty_pane_or_a_disabled_material_is_always_plain(
        blur in 0u16..=u16::MAX,
        height in 0u16..=u16::MAX,
        amount in 0u16..=u16::MAX,
        max_radius in 0u32..=1_024,
        max_height in 0u32..=1_024,
        max_amount in 0u32..=1_024,
        width in -8i32..=0,
        pane_height in -8i32..=0,
    ) {
        let capabilities = Capabilities::from_liquid(max_radius, max_height, max_amount, 0b11);
        let material = Material::new(
            blur,
            Corners::uniform(4),
            Refraction::new(height, amount, false, true),
        );

        let empty = Region { x: 0, y: 0, width, height: pane_height };
        prop_assert!(matches!(decide(material, capabilities, empty), Outcome::Plain(_)));

        if !material.has_effect() {
            prop_assert_eq!(
                decide(material, capabilities, Region::pane(640, 68)),
                Outcome::Plain(Plain::BlurDisabled)
            );
        }
    }

    /// What `apply` writes is exactly what `decide` decided, and a version 1
    /// object never receives a `since=2` request.
    #[test]
    fn applying_writes_exactly_what_was_decided(
        blur in 0u16..=u16::MAX,
        height in 0u16..=u16::MAX,
        amount in 0u16..=u16::MAX,
        version_two in any::<bool>(),
        width in 1u32..=4_096,
        pane_height in 1u32..=4_096,
    ) {
        let material = Material::new(
            blur,
            Corners::uniform(8),
            Refraction::new(height, amount, false, true),
        );
        let capabilities = Capabilities::from_liquid(64, 128, 64, 0b11);

        let Outcome::Effect(request) =
            decide(material, capabilities, Region::pane(width, pane_height))
        else {
            return Ok(());
        };

        let mut recorder = Recorder {
            version: if version_two { 2 } else { 1 },
            ..Recorder::default()
        };
        request.apply(&mut recorder);

        prop_assert_eq!(recorder.radius, Some(request.radius));
        prop_assert_eq!(recorder.region, Some(request.region));

        if version_two {
            prop_assert_eq!(recorder.corners, request.refraction.map(|_| request.corners));
            prop_assert_eq!(
                recorder.refraction,
                request.refraction.map(|refraction| {
                    (refraction.height, refraction.amount, refraction.flags)
                })
            );
        } else {
            prop_assert_eq!(recorder.corners, None);
            prop_assert_eq!(recorder.refraction, None);
        }
    }

    /// Clamping a corner set twice changes nothing, and a smaller limit never
    /// clamps less.
    #[test]
    fn corner_clamping_is_idempotent_and_order_preserving(
        top_left in any::<u16>(),
        top_right in any::<u16>(),
        bottom_right in any::<u16>(),
        bottom_left in any::<u16>(),
        smaller in any::<u16>(),
        larger in any::<u16>(),
    ) {
        prop_assume!(smaller <= larger);

        let corners = Corners { top_left, top_right, bottom_right, bottom_left };

        prop_assert_eq!(corners.clamped(larger), corners.clamped(larger).clamped(larger));

        let smaller_radius = corners.clamped(smaller).max_radius();
        let larger_radius = corners.clamped(larger).max_radius();
        prop_assert!(smaller_radius <= larger_radius);
        prop_assert!(smaller_radius <= smaller);
        prop_assert!(larger_radius <= larger);
    }
}

#[test]
fn a_version_one_compositor_falls_back_to_blur_only() {
    let Outcome::Effect(request) = decide(
        Material::from_tokens(&inox_config::Theme::builtin_dark().glass, 8),
        Capabilities::from_legacy(64),
        Region::pane(640, 68),
    ) else {
        panic!("the dark theme asks for a blur");
    };

    assert!(request.radius > 0);
    assert_eq!(
        request.radius, 64,
        "the theme's 128 px radius is clamped to what the compositor reported"
    );
    assert_eq!(request.refraction, None, "a v1 compositor cannot refract");
}

#[test]
fn a_compositor_that_can_do_nothing_is_reported_with_its_reason() {
    let material = Material::from_tokens(&inox_config::Theme::builtin_dark().glass, 8);

    assert_eq!(
        decide(material, Capabilities::NONE, Region::pane(640, 68)),
        Outcome::Plain(Plain::CompositorCannotBlur)
    );
}

#[test]
fn a_refraction_only_pane_is_an_effect_on_its_own() {
    // blur = 0 with a refraction band is a liquid pane, not a plain one: the
    // panel creates the glass object for it, and this is the request it sends.
    let material = Material::new(0, Corners::uniform(8), Refraction::new(24, 12, false, true));

    let Outcome::Effect(request) = decide(
        material,
        Capabilities::from_liquid(64, 128, 64, 0b11),
        Region::pane(320, 68),
    ) else {
        panic!("a refraction band is an effect without a blur");
    };

    assert_eq!(request.radius, 0);
    let refraction = request.refraction.expect("the band survives");
    assert_eq!(refraction.height, 24);
    assert_eq!(refraction.amount, 12);
    assert!(refraction.disperses());
}

#[test]
fn the_panel_contract_regions_and_requests_are_the_same_as_before() {
    // The panel used to decide this itself (`crates/inox-panel/src/frost.rs`
    // before ADR 0040). These are the values its tests pinned, so migrating the
    // panel must not change the wire for a v1 compositor.
    let Outcome::Effect(request) = decide(
        Material::plain(32),
        Capabilities::from_legacy(64),
        Region::pane(320, 68),
    ) else {
        panic!("a 32 px blur is frosted");
    };

    assert_eq!(
        request,
        Request {
            radius: 32,
            region: Region {
                x: 0,
                y: 0,
                width: 320,
                height: 68
            },
            corners: Corners::ZERO,
            refraction: None,
        }
    );
}
