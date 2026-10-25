//! Liquid glass: the panel's client-side decisions, as pure data.
//!
//! The panel does not draw the effect itself. A client cannot sample the pixels
//! behind its own surface — those live in the compositor's framebuffer — so the
//! material the panel draws is a *translucent tint* with a hairline highlight and
//! a shade, and the backdrop behind it is asked of the compositor over
//! `inox-glass-v1` (ADR 0040).
//!
//! # What the panel decides, and what `inox-glass` decides
//!
//! The clamps, the version negotiation and the fallback *reasons* are one
//! implementation in [`inox_glass`] — every L3 component has to ask the same
//! questions, and a second copy of the arithmetic drifts. This module keeps only
//! what is the **panel's** policy:
//!
//! - **Which surfaces are panes.** The menu bar and the dock are glass: their
//!   root is the [`inox_config::Glass`] material and their backdrop should be
//!   softened and refracted. The taskbar is a strip of the `surface` colour
//!   token — an opaque strip has nothing behind it to show — so it is not a pane
//!   and never creates a glass object. That is why [`decide`] answers `None` for
//!   it instead of one of [`inox_glass::Plain`]'s reasons: the library's reasons
//!   are about a material that was asked for.
//! - **Whether a glass object is created.** The object is created when the
//!   material asks for *any* effect ([`Material::has_effect`]): a blur, a
//!   refraction band, or both. `glass.blur = 0` with `glass.refraction > 0` is
//!   therefore still an object — it is a lens without a blur, not plain
//!   translucency. Both being zero is the tokens saying "plain translucency":
//!   no object at all, which is a decision rather than an error.
//! - **The region is the pane's own area.** For these surfaces the view's root
//!   *is* the glass material, so the whole configured area is the pane. The
//!   region starts at `(0, 0)` and is re-sent whenever the configured size
//!   changes, because a region that stayed at the old size would frost the wrong
//!   part of the surface (or, for a smaller pane, outside it).
//!
//! Everything a compositor can answer — a maximum of zero, a v1 object with no
//! refraction vocabulary, an empty pane — comes back as
//! [`inox_glass::Outcome::Plain`], whose reason the caller turns into the one
//! `info!` line this crate logs for a real degradation. A pane that gets no
//! effect still draws its tint, highlight and shade; it never claims an effect it
//! did not get.
//!
//! A region with no positive area means "nothing to frost" in the protocol, so a
//! pane the compositor configured to nothing asks for nothing rather than for an
//! empty rectangle the compositor would have to interpret.

use inox_config::PanelSurface;
use inox_glass::{
    Capabilities, Corners, Material, Outcome, Region, Request, decide as material_decision,
};

/// Whether `surface` is a glass pane at all.
///
/// The menu bar and the dock float over the desktop with nothing of their own
/// behind them; the taskbar is an opaque strip, so a blur or a lens for it would
/// describe an effect nothing could see.
pub const fn is_pane(surface: PanelSurface) -> bool {
    matches!(surface, PanelSurface::Menubar | PanelSurface::Dock)
}

/// Whether a glass object is created for `surface`.
///
/// This is the question that has to be answered *before* the compositor is heard
/// from: a material that asks for neither a blur nor a refraction means no object
/// is created at all, which is a different thing from creating one and then
/// discovering the compositor will not draw on it.
pub fn asks_for_glass(surface: PanelSurface, material: Material) -> bool {
    is_pane(surface) && material.has_effect()
}

/// The material decision for one pane, or `None` when the surface is not a pane.
///
/// `material` is the tokens' material ([`crate::theme::glass_material`]),
/// `capabilities` what the compositor answered through `capabilities` /
/// `liquid_capabilities` (pass a large [`Capabilities::from_legacy`] maximum
/// before the answer arrives, when the caller only needs to know that *something*
/// will be asked for), and `size` the surface's configured size in logical
/// pixels.
pub fn decide(
    surface: PanelSurface,
    material: Material,
    capabilities: Capabilities,
    size: (u32, u32),
) -> Option<Outcome> {
    is_pane(surface)
        .then(|| material_decision(material, capabilities, Region::pane(size.0, size.1)))
}

/// The request that turns the effect off without destroying the glass object.
///
/// A radius of 0 and a region with no area both mean "nothing to frost", and
/// neither is a protocol error, so a token change does not have to destroy and
/// recreate the object. The corners and the refraction band are dropped with it:
/// the shape only describes a lens, and there is no lens left to shape.
pub fn off() -> Request {
    Request {
        radius: 0,
        region: Region::pane(0, 0),
        corners: Corners::ZERO,
        refraction: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_glass::{Plain, REFRACTION_FLAG_DEPTH, REFRACTION_FLAG_DISPERSION, Refraction};

    const PANE: (u32, u32) = (640, 68);

    /// A compositor that can draw everything ADR 0040 allows.
    fn liquid() -> Capabilities {
        Capabilities::from_liquid(
            64,
            128,
            64,
            REFRACTION_FLAG_DEPTH | REFRACTION_FLAG_DISPERSION,
        )
    }

    /// A version 1 compositor: it can blur, and has no refraction vocabulary.
    fn legacy() -> Capabilities {
        Capabilities::from_legacy(64)
    }

    /// A material with the reference glass's refraction band and an 8 px corner.
    fn glass(blur: u16, band: u16, amount: u16) -> Material {
        Material::new(
            blur,
            Corners::uniform(8),
            Refraction::new(band, amount, false, true),
        )
    }

    /// The request of an `Effect` outcome.
    fn effect(outcome: Option<Outcome>) -> Request {
        match outcome {
            Some(Outcome::Effect(request)) => request,
            other => panic!("expected an effect, got {other:?}"),
        }
    }

    #[test]
    fn the_menu_bar_and_the_dock_are_panes_and_the_taskbar_is_not() {
        assert!(is_pane(PanelSurface::Menubar));
        assert!(is_pane(PanelSurface::Dock));
        assert!(
            !is_pane(PanelSurface::Taskbar),
            "the taskbar draws the opaque `surface` token: there is nothing to frost"
        );
    }

    #[test]
    fn a_taskbar_never_asks_for_a_glass_object() {
        let material = glass(128, 24, 12);

        assert!(!asks_for_glass(PanelSurface::Taskbar, material));
        assert_eq!(
            decide(PanelSurface::Taskbar, material, liquid(), PANE),
            None,
            "not being a pane is the panel's answer, not a material fallback"
        );
    }

    #[test]
    fn a_material_with_neither_a_blur_nor_a_lens_creates_no_object_at_all() {
        for surface in [PanelSurface::Menubar, PanelSurface::Dock] {
            assert!(
                !asks_for_glass(surface, glass(0, 0, 0)),
                "both halves off is plain translucency, not an object that asks for nothing"
            );
            assert_eq!(
                decide(surface, glass(0, 0, 0), liquid(), PANE),
                Some(Outcome::Plain(Plain::BlurDisabled))
            );
        }

        assert!(asks_for_glass(PanelSurface::Menubar, glass(1, 0, 0)));
        assert_eq!(
            effect(decide(
                PanelSurface::Menubar,
                glass(1, 0, 0),
                liquid(),
                PANE
            )),
            Request {
                radius: 1,
                region: Region::pane(PANE.0, PANE.1),
                corners: Corners::uniform(8),
                refraction: None,
            }
        );
    }

    #[test]
    fn a_refraction_band_alone_still_creates_a_glass_object() {
        // ADR 0040 §5: the object is created when *either* half is asked for.
        // A lens without a blur is a legitimate material, not plain translucency.
        let lens = glass(0, 24, 12);

        assert!(asks_for_glass(PanelSurface::Menubar, lens));

        let request = effect(decide(PanelSurface::Menubar, lens, liquid(), PANE));
        assert_eq!(request.radius, 0, "the tokens asked for no blur");
        assert_eq!(
            request.refraction,
            Some(inox_glass::RefractionRequest {
                height: 24,
                amount: 12,
                flags: REFRACTION_FLAG_DISPERSION,
            }),
            "and the band is what the pane's edge is made of"
        );
        assert_eq!(request.corners, Corners::uniform(8));
    }

    #[test]
    fn the_refraction_band_is_clamped_to_what_the_compositor_reported() {
        let narrow = Capabilities::from_liquid(64, 10, 4, REFRACTION_FLAG_DISPERSION);
        let request = effect(decide(PanelSurface::Dock, glass(40, 24, 12), narrow, PANE));

        assert_eq!(
            request.refraction,
            Some(inox_glass::RefractionRequest {
                height: 10,
                amount: 4,
                flags: REFRACTION_FLAG_DISPERSION,
            }),
            "a request above the maximum is clamped, not refused"
        );

        // Depth is passed through whenever the tokens ask for it (the protocol
        // says a compositor ignores the bits it does not apply); dispersion is
        // only asked for when the compositor advertised it, because a missing
        // fringe is something a user can see.
        let with_depth =
            Material::new(40, Corners::uniform(8), Refraction::new(24, 12, true, true));
        let no_dispersion = Capabilities::from_liquid(64, 128, 64, REFRACTION_FLAG_DEPTH);
        let request = effect(decide(PanelSurface::Dock, with_depth, no_dispersion, PANE));

        assert_eq!(
            request.refraction,
            Some(inox_glass::RefractionRequest {
                height: 24,
                amount: 12,
                flags: REFRACTION_FLAG_DEPTH,
            }),
            "the depth bit the tokens ask for is sent; the dispersion they also ask for \
             is not, because this compositor does not advertise it"
        );

        // The reference tokens have depth off, so a compositor that advertises
        // neither component gets a band with no flags at all: the panel must not
        // claim a component nothing asked for and nothing offers.
        let bare = Capabilities::from_liquid(64, 128, 64, 0);
        let request = effect(decide(PanelSurface::Dock, glass(40, 24, 12), bare, PANE));

        assert_eq!(
            request.refraction.map(|band| band.flags),
            Some(0),
            "no optional component is claimed"
        );
    }

    #[test]
    fn a_version_one_compositor_falls_back_to_the_blur_only() {
        // A v1 object has no `set_refraction` and no `liquid_capabilities`; the
        // blur is all there is, and the pane is still frosted.
        let request = effect(decide(
            PanelSurface::Menubar,
            glass(40, 24, 12),
            legacy(),
            PANE,
        ));

        assert_eq!(request.radius, 40);
        assert_eq!(
            request.refraction, None,
            "a v1 compositor cannot refract: the request must not describe one"
        );
    }

    #[test]
    fn a_compositor_that_can_do_nothing_gets_a_plain_pane_and_the_reason() {
        assert_eq!(
            decide(
                PanelSurface::Menubar,
                glass(128, 24, 12),
                Capabilities::NONE,
                PANE
            ),
            Some(Outcome::Plain(Plain::CompositorCannotBlur)),
            "max_radius = 0 is the compositor saying it will not frost this surface"
        );

        assert_eq!(
            decide(
                PanelSurface::Dock,
                glass(0, 24, 12),
                Capabilities::from_legacy(64),
                PANE
            ),
            Some(Outcome::Plain(Plain::RefractionDisabled)),
            "a lens without a blur has nothing left when the compositor cannot refract"
        );

        assert!(
            asks_for_glass(PanelSurface::Dock, glass(128, 24, 12)),
            "the object is still created: only the compositor's answer says it cannot blur"
        );
    }

    #[test]
    fn the_radius_is_clamped_to_what_the_compositor_reported() {
        assert_eq!(
            effect(decide(PanelSurface::Dock, glass(200, 0, 0), legacy(), PANE)).radius,
            64,
            "a request above the maximum is clamped, not refused"
        );
        assert_eq!(
            effect(decide(PanelSurface::Dock, glass(40, 0, 0), legacy(), PANE)).radius,
            40,
            "and a request below it is honoured as asked"
        );
        assert_eq!(
            effect(decide(PanelSurface::Dock, glass(64, 0, 0), legacy(), PANE)).radius,
            64,
            "the boundary belongs to the compositor's maximum"
        );
    }

    #[test]
    fn the_region_is_the_whole_pane_and_starts_at_its_origin() {
        let outcome = decide(PanelSurface::Dock, glass(32, 0, 0), legacy(), (320, 68));
        let request = effect(outcome);

        assert_eq!(
            request.region,
            Region {
                x: 0,
                y: 0,
                width: 320,
                height: 68
            },
            "the view's root is the glass material, so the pane is the whole surface"
        );
    }

    #[test]
    fn a_configure_of_a_different_size_asks_for_a_different_region() {
        let narrow = decide(PanelSurface::Dock, glass(32, 0, 0), legacy(), (320, 68));
        let wide = decide(PanelSurface::Dock, glass(32, 0, 0), legacy(), (640, 68));
        let taller = decide(PanelSurface::Dock, glass(32, 0, 0), legacy(), (320, 96));

        assert_ne!(
            narrow, wide,
            "a deeper dock is a different region, so the request is re-sent"
        );
        assert_ne!(narrow, taller, "and so is a taller pane");
        assert_eq!(
            narrow,
            decide(PanelSurface::Dock, glass(32, 0, 0), legacy(), (320, 68)),
            "the same size asks for the same region: nothing is re-sent twice"
        );
    }

    #[test]
    fn a_pane_with_no_area_asks_for_no_region() {
        assert_eq!(
            decide(PanelSurface::Dock, glass(32, 24, 12), liquid(), (0, 68)),
            Some(Outcome::Plain(Plain::EmptyPane))
        );
        assert_eq!(
            decide(PanelSurface::Dock, glass(32, 24, 12), liquid(), (320, 0)),
            Some(Outcome::Plain(Plain::EmptyPane))
        );
        assert!(Region::pane(0, 68).is_empty());
        assert!(Region::pane(320, 0).is_empty());
        assert!(!Region::pane(1, 1).is_empty());
        assert_eq!(
            Region::pane(u32::MAX, u32::MAX),
            Region {
                x: 0,
                y: 0,
                width: i32::MAX,
                height: i32::MAX
            },
            "a protocol extent that cannot be represented is clamped, not wrapped"
        );
    }

    #[test]
    fn turning_the_effect_off_clears_the_region_radius_and_lens() {
        let off = off();

        assert_eq!(off.radius, 0);
        assert!(off.region.is_empty());
        assert_eq!(off.refraction, None);
        assert_eq!(
            off.corners,
            Corners::ZERO,
            "with no lens there is no shape to describe"
        );
        assert_ne!(
            off,
            effect(decide(
                PanelSurface::Dock,
                glass(32, 24, 12),
                liquid(),
                PANE
            )),
            "the off request is not the material's request"
        );
    }
}
