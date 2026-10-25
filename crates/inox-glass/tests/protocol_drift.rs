//! Drift guard: the material API and the generated `inox-glass-v1` bindings must
//! not disagree.
//!
//! The material API speaks numbers the wire also speaks — the two refraction flag
//! bits, the versions at which the refraction vocabulary exists, and the maxima
//! ADR 0040 froze. A copy of any of them that drifts is a silent protocol bug (a
//! client asking for a different effect than the one it logs), so each one is
//! asserted against the generated bindings here rather than only being described
//! in a comment.
//!
//! One caveat, deliberately stated instead of hidden: the protocol XML does not
//! carry the numeric maxima. They are the compositor's honest limits, advertised
//! at runtime through `capabilities` / `liquid_capabilities`, and frozen in ADR
//! 0040 §1; this file pins the ADR numbers and their widths. The compositor crate
//! pins its own copies in its tests, and a request above either crate's maximum is
//! clamped by the compositor anyway.

use inox_glass::{
    Capabilities, MAX_CORNER_RADIUS, MAX_REFRACTION_AMOUNT, MAX_REFRACTION_HEIGHT,
    REFRACTION_FLAG_DEPTH, REFRACTION_FLAG_DISPERSION, REFRACTION_SINCE_VERSION,
};
use inox_protocols::glass::inox_glass::client::inox_glass_v1::RefractionFlag;

/// Every request of `inox_glass_v1` as `(name, since)`.
fn requests() -> Vec<(&'static str, u32)> {
    inox_protocols::glass::inox_glass::client::__interfaces::INOX_GLASS_V1_INTERFACE
        .requests
        .iter()
        .map(|request| (request.name, request.since))
        .collect()
}

/// Every event of `inox_glass_v1` as `(name, since)`.
fn events() -> Vec<(&'static str, u32)> {
    inox_protocols::glass::inox_glass::client::__interfaces::INOX_GLASS_V1_INTERFACE
        .events
        .iter()
        .map(|event| (event.name, event.since))
        .collect()
}

#[test]
fn the_frozen_maxima_are_the_adr_numbers() {
    // ADR 0040 §1, "冻结上限".
    assert_eq!(MAX_CORNER_RADIUS, 16_384);
    assert_eq!(MAX_REFRACTION_HEIGHT, 128);
    assert_eq!(MAX_REFRACTION_AMOUNT, 64);

    // The protocol carries every one of them as a `uint`, and the material API
    // narrows them to `u16`: no value may be lost in that narrowing.
    assert!(u32::from(MAX_CORNER_RADIUS) < u32::MAX);
    assert!(u32::from(MAX_REFRACTION_HEIGHT) <= u32::from(MAX_CORNER_RADIUS));
    assert!(u32::from(MAX_REFRACTION_AMOUNT) <= u32::from(MAX_REFRACTION_HEIGHT));
}

#[test]
fn the_refraction_flag_bits_are_the_generated_ones() {
    assert_eq!(REFRACTION_FLAG_DEPTH, RefractionFlag::Depth.bits());
    assert_eq!(
        REFRACTION_FLAG_DISPERSION,
        RefractionFlag::Dispersion.bits()
    );

    assert_eq!(REFRACTION_FLAG_DEPTH, 1, "the wire value of `depth`");
    assert_eq!(
        REFRACTION_FLAG_DISPERSION, 2,
        "the wire value of `dispersion`"
    );
    assert_eq!(
        REFRACTION_FLAG_DEPTH & REFRACTION_FLAG_DISPERSION,
        0,
        "the two flags are separate bits"
    );
}

#[test]
fn a_liquid_capability_reads_the_generated_dispersion_bit() {
    assert!(
        Capabilities::from_liquid(64, 128, 64, RefractionFlag::Dispersion.bits()).dispersion,
        "the generated dispersion bit is what turns the chromatic component on"
    );
    assert!(
        !Capabilities::from_liquid(64, 128, 64, RefractionFlag::Depth.bits()).dispersion,
        "depth alone does not ask for a fringe"
    );
    assert!(!Capabilities::from_legacy(64).dispersion);
}

#[test]
fn the_refraction_requests_and_the_event_share_one_since_version() {
    let refraction_since = REFRACTION_SINCE_VERSION;

    assert_eq!(refraction_since, 2, "ADR 0040 §1 bumped the protocol to v2");
    assert_eq!(
        requests(),
        [
            ("set_radius", 1),
            ("set_region", 1),
            ("set_corner_radii", refraction_since),
            ("set_refraction", refraction_since),
            ("destroy", 1),
        ],
        "the adapter gates exactly these two requests on the object version"
    );
    assert_eq!(
        events(),
        [
            ("capabilities", 1),
            ("liquid_capabilities", refraction_since)
        ],
        "a version 1 object never receives the refraction maximums"
    );
}

#[test]
fn both_interfaces_are_advertised_at_the_version_the_adapter_gates_on() {
    use inox_protocols::glass::inox_glass::client::__interfaces::{
        INOX_GLASS_MANAGER_V1_INTERFACE, INOX_GLASS_V1_INTERFACE,
    };

    assert_eq!(
        INOX_GLASS_MANAGER_V1_INTERFACE.version,
        REFRACTION_SINCE_VERSION
    );
    assert_eq!(INOX_GLASS_V1_INTERFACE.version, REFRACTION_SINCE_VERSION);
    assert_eq!(
        INOX_GLASS_MANAGER_V1_INTERFACE.version, 2,
        "a manager bound at v1 creates v1 objects, which is the fallback path"
    );
}

#[test]
fn the_fallback_path_is_a_real_version_one_object() {
    // The whole point of the version gate: a v1 object must still be able to do
    // the v1 half, so the crate never refuses to draw a blur on an old
    // compositor.
    let v1 = Capabilities::from_legacy(64);

    assert!(v1.can_blur(), "v1 blurs");
    assert!(!v1.can_refract(), "v1 does not refract");
    assert!(!v1.is_none());
}

#[test]
fn the_capability_maxima_fit_the_wire_integers() {
    // `from_liquid` takes the four `uint` arguments verbatim; nothing narrows
    // them before the decision does, which is what keeps `decide` honest about
    // very large maxima.
    let wide = Capabilities::from_liquid(u32::MAX, u32::MAX, u32::MAX, 0b11);

    assert!(wide.can_blur());
    assert!(wide.can_refract());
    assert!(wide.dispersion);
}
