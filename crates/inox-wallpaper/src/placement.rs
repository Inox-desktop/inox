//! Where the wallpaper surface goes: the layer-shell policy, as pure data.
//!
//! `wlr-layer-shell-unstable-v1` is the protocol that lets a client put a
//! surface on the *background* layer, below the panel and below every window,
//! covering an output without being a window. The wallpaper is one of its
//! clients.
//!
//! **Nothing in this module is applied.** The compositor does not advertise the
//! protocol yet, so no layer surface can be created at all; the binary runs as an
//! ordinary window and the placement below is computed, tested and logged rather
//! than sent. It is written the way a future host will consume it unchanged — the
//! same shapes the protocol has, so that host is a translation and not a
//! redesign.
//!
//! The policy itself is short and unexciting, which is the point:
//!
//! - **All four anchors**, which is what makes the surface cover the output
//!   rather than stretch between two edges.
//! - **`exclusive_zone` 0**: a wallpaper reserves no space. A panel asks for the
//!   strip it occupies; a wallpaper that asked for the whole output would leave
//!   no room for anything else.
//! - **Keyboard interactivity `None`**: nothing the user types is meant for a
//!   wallpaper, and a background surface that took focus would swallow keys.
//! - **One surface per output**, each with its own image from
//!   [`crate::source`], so two screens can differ and one screen failing does not
//!   affect the other.

use std::collections::BTreeMap;

use crate::model::{OutputName, Size};

/// The layer-shell layer the wallpaper belongs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    /// Below everything, including other background surfaces.
    Background,
    /// Below ordinary windows but above the background.
    Bottom,
    /// Above ordinary windows.
    Top,
    /// Above everything, including lock screens and notifications.
    Overlay,
}

impl Layer {
    /// The layer a wallpaper uses.
    ///
    /// [`Layer::Background`] and not [`Layer::Bottom`]: on a layer-shell host,
    /// `bottom` is where a client that wants to sit under windows but above the
    /// wallpaper goes, and two components claiming the same layer have to be
    /// ordered by something else. PRD §2.3 names the background layer for the
    /// wallpaper explicitly.
    pub const WALLPAPER: Self = Self::Background;
}

/// Which edges a surface is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Anchors {
    /// Anchor to the top edge.
    pub top: bool,
    /// Anchor to the bottom edge.
    pub bottom: bool,
    /// Anchor to the left edge.
    pub left: bool,
    /// Anchor to the right edge.
    pub right: bool,
}

impl Anchors {
    /// Every edge: a surface that fills its output.
    pub const ALL: Self = Self {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };

    /// How many edges are anchored.
    #[must_use]
    pub const fn count(&self) -> u32 {
        self.top as u32 + self.bottom as u32 + self.left as u32 + self.right as u32
    }
}

/// What the surface may do with the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
// The wallpaper asks for no keyboard interaction; the other two values are the
// protocol's own vocabulary, kept so the model stays complete and a reader does
// not have to consult the protocol to see what was left out.
#[allow(dead_code)]
pub enum KeyboardInteractivity {
    /// The surface never receives keyboard input, and the compositor does not
    /// wait for it to be focused.
    None,
    /// The surface has exclusive keyboard access while it is on screen.
    Exclusive,
    /// The compositor focuses the surface when the user asks for it.
    OnDemand,
}

/// The distance between the surface and the edges it is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Margins {
    /// Distance from the top edge.
    pub top: i32,
    /// Distance from the right edge.
    pub right: i32,
    /// Distance from the bottom edge.
    pub bottom: i32,
    /// Distance from the left edge.
    pub left: i32,
}

impl Margins {
    /// No margin on any edge.
    pub const NONE: Self = Self {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
    };
}

/// The protocol-level description of one wallpaper surface.
///
/// Complete on purpose: a host that has this value needs nothing else to create
/// the surface, which is what makes the policy testable here instead of
/// observable only on a compositor with the protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfacePlacement {
    /// The output the surface belongs to.
    pub output: OutputName,
    /// The layer to create the surface on.
    pub layer: Layer,
    /// The edges the surface is anchored to.
    pub anchors: Anchors,
    /// How much of the output the surface reserves for itself: none.
    pub exclusive_zone: i32,
    /// What the surface may do with the keyboard: nothing.
    pub keyboard_interactivity: KeyboardInteractivity,
    /// The distance from the anchored edges: none.
    pub margin: Margins,
    /// The size of the surface, which is the output's, so that the compositor
    /// never rescales what the wallpaper drew.
    pub size: Size,
}

impl SurfacePlacement {
    /// The wallpaper placement for one output of `size`.
    ///
    /// Every output gets the same policy; only the name and the size differ,
    /// which is what makes "one surface per output" a list rather than a
    /// special case.
    #[must_use]
    pub fn for_output(output: OutputName, size: Size) -> Self {
        Self {
            output,
            layer: Layer::WALLPAPER,
            anchors: Anchors::ALL,
            exclusive_zone: 0,
            keyboard_interactivity: KeyboardInteractivity::None,
            margin: Margins::NONE,
            size,
        }
    }

    /// Whether this surface covers its whole output.
    #[must_use]
    pub const fn covers_its_output(&self) -> bool {
        self.anchors.count() == 4
            && self.margin.top == 0
            && self.margin.right == 0
            && self.margin.bottom == 0
            && self.margin.left == 0
    }
}

/// The placement of every output's surface, in output-name order.
///
/// One entry per output and no entries for outputs that are not there: an
/// unplugged output takes its surface with it (acceptance O02).
#[must_use]
pub fn for_outputs(outputs: &BTreeMap<OutputName, Size>) -> Vec<SurfacePlacement> {
    outputs
        .iter()
        .map(|(output, size)| SurfacePlacement::for_output(output.clone(), *size))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs() -> BTreeMap<OutputName, Size> {
        let mut outputs = BTreeMap::new();
        outputs.insert(OutputName::from("HDMI-A-1"), Size::new(2560, 1440));
        outputs.insert(OutputName::from("eDP-1"), Size::new(1920, 1080));

        outputs
    }

    #[test]
    fn a_wallpaper_surface_covers_its_output_and_reserves_nothing() {
        let placement =
            SurfacePlacement::for_output(OutputName::from("HDMI-A-1"), Size::new(1920, 1080));

        assert_eq!(placement.anchors, Anchors::ALL);
        assert_eq!(placement.exclusive_zone, 0);
        assert_eq!(placement.margin, Margins::NONE);
        assert_eq!(placement.size, Size::new(1920, 1080));
        assert!(placement.covers_its_output());
    }

    #[test]
    fn a_wallpaper_never_takes_the_keyboard() {
        let placement =
            SurfacePlacement::for_output(OutputName::from("eDP-1"), Size::new(1920, 1080));

        assert_eq!(
            placement.keyboard_interactivity,
            KeyboardInteractivity::None,
            "a background surface that could take focus would swallow keys"
        );
    }

    #[test]
    fn a_wallpaper_is_on_the_background_layer_below_bottom() {
        let placement =
            SurfacePlacement::for_output(OutputName::from("eDP-1"), Size::new(1920, 1080));

        assert_eq!(placement.layer, Layer::Background);
        assert!(Layer::WALLPAPER < Layer::Bottom);
        assert!(Layer::Bottom < Layer::Top);
        assert!(Layer::Top < Layer::Overlay);
    }

    #[test]
    fn every_output_gets_exactly_one_surface_of_its_own_size() {
        let placements = for_outputs(&outputs());

        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].output.as_str(), "HDMI-A-1");
        assert_eq!(placements[0].size, Size::new(2560, 1440));
        assert_eq!(placements[1].output.as_str(), "eDP-1");
        assert_eq!(placements[1].size, Size::new(1920, 1080));
    }

    #[test]
    fn the_placement_order_is_the_output_order_and_never_varies() {
        let first = for_outputs(&outputs());
        let second = for_outputs(&outputs());

        assert_eq!(first, second);
        let names: Vec<&str> = first.iter().map(|p| p.output.as_str()).collect();
        assert_eq!(names, ["HDMI-A-1", "eDP-1"], "sorted, deterministic order");
    }

    #[test]
    fn no_outputs_means_no_surfaces() {
        assert!(for_outputs(&BTreeMap::new()).is_empty());
    }

    #[test]
    fn an_output_that_disappears_takes_its_surface_with_it() {
        let mut outputs = outputs();
        outputs.remove(&OutputName::from("HDMI-A-1"));

        let placements = for_outputs(&outputs);

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].output.as_str(), "eDP-1");
    }

    #[test]
    fn the_anchor_count_is_a_fill_and_nothing_less() {
        assert_eq!(Anchors::ALL.count(), 4);
        assert_eq!(Anchors::default().count(), 0);
        assert_eq!(
            Anchors {
                top: true,
                bottom: true,
                ..Anchors::default()
            }
            .count(),
            2
        );
    }

    #[test]
    fn a_margin_or_a_missing_anchor_stops_calling_it_a_cover() {
        let mut placement =
            SurfacePlacement::for_output(OutputName::from("eDP-1"), Size::new(1920, 1080));

        placement.anchors.left = false;
        assert!(!placement.covers_its_output());

        placement.anchors = Anchors::ALL;
        placement.margin.top = 1;
        assert!(!placement.covers_its_output());
    }

    #[test]
    fn the_policy_in_one_surface_is_the_policy_in_all_of_them() {
        let placements = for_outputs(&outputs());

        for placement in &placements {
            assert_eq!(placement.layer, Layer::WALLPAPER);
            assert_eq!(placement.exclusive_zone, 0);
            assert_eq!(
                placement.keyboard_interactivity,
                KeyboardInteractivity::None
            );
            assert_eq!(placement.margin, Margins::NONE);
            assert!(placement.covers_its_output());
        }
    }
}
