//! Frosted glass: the client-side decisions, as pure data.
//!
//! The panel does not blur anything itself. A client cannot sample the pixels
//! behind its own surface — those live in the compositor's framebuffer — so the
//! material the panel draws is a *translucent tint*, and the softening of
//! whatever is behind it is asked of the compositor over `inox-glass-v1`
//! (`docs/quality/ui-guidelines.md` §12, ADR 0031).
//!
//! This module holds the decisions that follow from that, with no Wayland type
//! in sight, so every one of them is unit-tested without a display:
//!
//! - **Which surfaces are panes.** The menu bar and the dock are glass: their
//!   root is the [`inox_config::Glass`] material and their backdrop should be
//!   softened. The taskbar is a strip of the `surface` colour token — an opaque
//!   strip has nothing to soften and nothing behind it to show, so it never
//!   asks for a glass object at all.
//! - **Whether a glass object is created.** `glass.blur == 0` is the tokens
//!   saying "plain translucency": no object is created, and none is needed. A
//!   blur of zero is a *decision*, not an error, which is why it is a `Plain`
//!   reason rather than a failure.
//! - **What to ask for once the compositor answers.** The protocol reports the
//!   largest radius the compositor will apply through `capabilities`
//!   (`max_radius`); the request is `min(glass.blur, max_radius)`, because
//!   `set_radius` is clamped rather than refused and a client that scales its
//!   pane keeps asking for "as much as the compositor can do". `max_radius == 0`
//!   means the compositor will not frost this surface's backdrop — no renderer
//!   support, a locked session, or a surface with no layer role. The panel then
//!   draws plain translucency and **says so once**; it never claims a blur it
//!   did not get, and it does not keep re-creating objects hoping for a
//!   different answer.
//! - **The region is the pane's own area.** For these surfaces the view's root
//!   *is* the glass material, so the whole configured area is the pane. The
//!   region is in the surface's own coordinates and starts at `(0, 0)`; it is
//!   re-sent whenever the configured size changes, because a region that stayed
//!   at the old size would frost the wrong part of the surface (or, for a
//!   smaller pane, outside it).
//!
//! A region with no positive area means "nothing to frost" in the protocol, so
//! a pane the compositor configured to nothing asks for nothing rather than for
//! an empty rectangle the compositor would have to interpret.

use inox_config::PanelSurface;

/// The area of a surface to frost, in the surface's own logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Dimensions larger than [`i32::MAX`] are clamped rather than wrapped: the
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

/// What the panel asks the compositor to frost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// Blur radius in logical pixels, already clamped to the compositor's
    /// maximum.
    pub radius: u32,
    /// The pane's area, surface-local.
    pub region: Region,
}

/// Why a pane is drawn as plain translucency rather than frosted glass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plain {
    /// `glass.blur == 0`: the tokens ask for no backdrop blur.
    BlurDisabled,
    /// The compositor answered `max_radius = 0`: it will not frost this
    /// surface's backdrop.
    CompositorCannotBlur,
    /// The surface has no area to frost.
    EmptyPane,
    /// The surface is not a pane: the taskbar draws an opaque strip.
    NotAPane,
}

/// The material one pane is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Material {
    /// No blur: the tint, the highlight and the shade are all that is drawn.
    Plain(Plain),
    /// Ask the compositor for this much blur over this region.
    Frosted(Request),
}

/// Whether `surface` is a glass pane at all.
///
/// The menu bar and the dock float over the desktop with nothing of their own
/// behind them; the taskbar is an opaque strip, so a blur request for it would
/// describe an effect nothing could see.
pub const fn is_pane(surface: PanelSurface) -> bool {
    matches!(surface, PanelSurface::Menubar | PanelSurface::Dock)
}

/// Whether a glass object is created for `surface`.
///
/// This is the question that has to be answered *before* the compositor is
/// heard from: `glass.blur == 0` means no object is created at all, which is a
/// different thing from creating one and then discovering the compositor will
/// not blur.
pub const fn asks_for_glass(surface: PanelSurface, blur: u16) -> bool {
    is_pane(surface) && blur > 0
}

/// The material decision for one pane.
///
/// `blur` is `tokens.glass.blur`, `max_radius` what the compositor reported
/// through `capabilities` (pass [`u32::MAX`] before the answer arrives, when the
/// caller only needs to know that *something* will be asked for), and `size` the
/// surface's configured size in logical pixels.
pub fn decide(surface: PanelSurface, blur: u16, max_radius: u32, size: (u32, u32)) -> Material {
    if !is_pane(surface) {
        return Material::Plain(Plain::NotAPane);
    }
    if blur == 0 {
        return Material::Plain(Plain::BlurDisabled);
    }
    if max_radius == 0 {
        return Material::Plain(Plain::CompositorCannotBlur);
    }

    let region = Region::pane(size.0, size.1);

    if region.is_empty() {
        return Material::Plain(Plain::EmptyPane);
    }

    Material::Frosted(Request {
        radius: u32::from(blur).min(max_radius),
        region,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: (u32, u32) = (640, 68);

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
    fn a_blur_of_zero_creates_no_glass_object_at_all() {
        for surface in [PanelSurface::Menubar, PanelSurface::Dock] {
            assert!(
                !asks_for_glass(surface, 0),
                "glass.blur = 0 means plain translucency, not an object that asks for nothing"
            );
            assert_eq!(
                decide(surface, 0, 64, PANE),
                Material::Plain(Plain::BlurDisabled)
            );
        }

        assert!(asks_for_glass(PanelSurface::Menubar, 1));
        assert_eq!(
            decide(PanelSurface::Menubar, 1, 64, PANE),
            Material::Frosted(Request {
                radius: 1,
                region: Region::pane(PANE.0, PANE.1)
            })
        );
    }

    #[test]
    fn a_taskbar_never_asks_for_a_glass_object() {
        assert!(!asks_for_glass(PanelSurface::Taskbar, 128));
        assert_eq!(
            decide(PanelSurface::Taskbar, 128, 64, PANE),
            Material::Plain(Plain::NotAPane)
        );
    }

    #[test]
    fn a_compositor_that_cannot_blur_gets_no_radius_and_no_region() {
        assert_eq!(
            decide(PanelSurface::Menubar, 128, 0, PANE),
            Material::Plain(Plain::CompositorCannotBlur),
            "max_radius = 0 is the compositor saying it will not blur this surface"
        );
        assert!(
            asks_for_glass(PanelSurface::Dock, 128),
            "the object is still created: only the compositor's answer says it cannot blur"
        );
    }

    #[test]
    fn the_radius_is_clamped_to_what_the_compositor_reported() {
        assert_eq!(
            decide(PanelSurface::Dock, 200, 64, PANE),
            Material::Frosted(Request {
                radius: 64,
                region: Region::pane(PANE.0, PANE.1)
            }),
            "a request above the maximum is clamped, not refused"
        );
        assert_eq!(
            decide(PanelSurface::Dock, 40, 64, PANE),
            Material::Frosted(Request {
                radius: 40,
                region: Region::pane(PANE.0, PANE.1)
            }),
            "and a request below it is honoured as asked"
        );
        assert_eq!(
            decide(PanelSurface::Dock, 64, 64, PANE),
            Material::Frosted(Request {
                radius: 64,
                region: Region::pane(PANE.0, PANE.1)
            }),
            "the boundary belongs to the compositor's maximum"
        );
    }

    #[test]
    fn the_region_is_the_whole_pane_and_starts_at_its_origin() {
        let Material::Frosted(request) = decide(PanelSurface::Dock, 32, 64, (320, 68)) else {
            panic!("a pane with a blur and a maximum is frosted");
        };

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
        let narrow = decide(PanelSurface::Dock, 32, 64, (320, 68));
        let wide = decide(PanelSurface::Dock, 32, 64, (640, 68));
        let taller = decide(PanelSurface::Dock, 32, 64, (320, 96));

        assert_ne!(
            narrow, wide,
            "a deeper dock is a different region, so the request is re-sent"
        );
        assert_ne!(narrow, taller, "and so is a taller pane");
        assert_eq!(
            narrow,
            decide(PanelSurface::Dock, 32, 64, (320, 68)),
            "the same size asks for the same region: nothing is re-sent twice"
        );
    }

    #[test]
    fn a_pane_with_no_area_asks_for_no_region() {
        assert_eq!(
            decide(PanelSurface::Dock, 32, 64, (0, 68)),
            Material::Plain(Plain::EmptyPane)
        );
        assert_eq!(
            decide(PanelSurface::Dock, 32, 64, (320, 0)),
            Material::Plain(Plain::EmptyPane)
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
}
