//! Where the panel asks to be placed, as pure data.
//!
//! On a real session the panel is a `wlr-layer-shell` surface, and the placement
//! it asks for — the anchored edges, the exclusive zone the compositor keeps free
//! of windows, the margin, the height and the keyboard interactivity — is what
//! acceptance D01 is about: *the strip must not cover things, and ordinary
//! display must not take focus*.
//!
//! **This module computes that placement; nothing applies it yet.** The Inox
//! compositor does not advertise `zwlr_layer_shell_v1`, and `iced_layershell`
//! 0.19 does not compile against iced 0.14's winit generation (ADR 0018), so the
//! panel currently runs as an ordinary window. Keeping the policy here, as data
//! with tests, is what makes the switch to a layer surface a change of host
//! rather than a change of behaviour: `main.rs` logs exactly one warning naming
//! ADR 0018 and the fact that the placement below is not applied.

use std::collections::BTreeSet;

use inox_config::{PanelConfig, PanelPosition};

use crate::tasks::OutputId;

/// An edge a layer surface can be anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Anchor {
    /// The top edge of the output.
    Top,
    /// The bottom edge of the output.
    Bottom,
    /// The left edge of the output.
    Left,
    /// The right edge of the output.
    Right,
}

/// The edges a surface is anchored to.
///
/// Anchoring to two opposite edges stretches the surface between them, which is
/// how a panel spans an output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Anchors(BTreeSet<Anchor>);

impl Anchors {
    /// No anchor at all.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the surface is anchored to `anchor`.
    pub fn contains(&self, anchor: Anchor) -> bool {
        self.0.contains(&anchor)
    }

    /// How many edges the surface is anchored to.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the surface is anchored to nothing.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The anchored edges, in a stable order.
    pub fn iter(&self) -> impl Iterator<Item = Anchor> + '_ {
        self.0.iter().copied()
    }

    fn insert(&mut self, anchor: Anchor) {
        self.0.insert(anchor);
    }
}

/// How much keyboard input the surface receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyboardInteractivity {
    /// The surface never receives keyboard input.
    ///
    /// What an ordinary panel asks for (D01): showing a strip must not take the
    /// keyboard away from the window the user is typing in. Explicit navigation
    /// and a tray menu are separate interactions with their own focus handling
    /// (ADR 0013), not a permanently focused panel.
    #[default]
    None,
    /// The surface receives input while the pointer is over it.
    OnDemand,
    /// The surface receives all keyboard input.
    Exclusive,
}

/// The layer the surface belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    /// Below everything, above the wallpaper.
    Background,
    /// Above ordinary windows.
    Bottom,
    /// The panel layer.
    #[default]
    Top,
    /// Above the panel layer, for popups and menus.
    Overlay,
}

/// How the panel hides itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Autohide {
    /// The panel is always visible.
    #[default]
    Never,
    /// The panel hides until the pointer reaches its edge.
    EdgeHover,
}

/// The placement one output's panel asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelPlacement {
    /// Edges the panel is anchored to.
    pub anchors: Anchors,
    /// Strip the compositor keeps free of windows, measured from the anchored
    /// edge; `0` when the panel does not reserve anything.
    pub exclusive_zone: u32,
    /// Distance between the panel and the anchored edge.
    pub margin: u16,
    /// Thickness of the panel along the anchored edge.
    pub height: u16,
    /// Keyboard interactivity, `None` for an ordinary panel.
    pub keyboard_interactivity: KeyboardInteractivity,
    /// Layer the panel belongs to.
    pub layer: Layer,
    /// Whether the panel hides itself.
    pub autohide: Autohide,
}

impl PanelPlacement {
    /// Derives the placement `config` asks for.
    ///
    /// A top or bottom panel is anchored to that edge and to both left and right,
    /// so it spans the output regardless of its width. The exclusive zone is the
    /// panel's thickness plus its margin, which is what keeps a maximized window
    /// from slipping underneath it (D01); a panel that does not reserve its strip
    /// asks for no zone at all.
    pub fn derive(config: &PanelConfig) -> Self {
        let mut anchors = Anchors::new();
        match config.position {
            PanelPosition::Top => anchors.insert(Anchor::Top),
            PanelPosition::Bottom => anchors.insert(Anchor::Bottom),
        }
        anchors.insert(Anchor::Left);
        anchors.insert(Anchor::Right);

        Self {
            anchors,
            exclusive_zone: if config.exclusive {
                u32::from(config.height) + u32::from(config.margin)
            } else {
                0
            },
            margin: config.margin,
            height: config.height,
            keyboard_interactivity: KeyboardInteractivity::None,
            layer: Layer::Top,
            autohide: if config.autohide {
                Autohide::EdgeHover
            } else {
                Autohide::Never
            },
        }
    }

    /// The strip of the output the panel keeps to itself, in logical pixels.
    pub const fn reserved_strip(&self) -> u32 {
        self.exclusive_zone
    }

    /// Whether the panel takes keyboard input.
    pub const fn takes_keyboard_focus(&self) -> bool {
        !matches!(self.keyboard_interactivity, KeyboardInteractivity::None)
    }
}

/// The panel to create for one output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputAssignment {
    /// The output the panel belongs to.
    pub output: OutputId,
    /// The placement it asks for there.
    pub placement: PanelPlacement,
}

/// One placement per output, in a stable order.
///
/// `outputs` is the set the compositor currently reports, in any order and with
/// any repetition: an output that is removed and reconnected, or reported twice
/// while a topology change settles, must not produce a second panel (O02:
/// "重接不重复创建面板"). The result is therefore sorted by output name and
/// deduplicated, and every output gets the same placement — the panel shows on
/// each of them.
pub fn assign(config: &PanelConfig, outputs: &[OutputId]) -> Vec<OutputAssignment> {
    let placement = PanelPlacement::derive(config);
    let mut outputs: Vec<&OutputId> = outputs.iter().collect();
    outputs.sort();
    outputs.dedup();

    outputs
        .into_iter()
        .map(|output| OutputAssignment {
            output: output.clone(),
            placement: placement.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bottom_panel_spans_the_output_from_the_bottom() {
        let placement = PanelPlacement::derive(&PanelConfig {
            position: PanelPosition::Bottom,
            ..PanelConfig::default()
        });

        assert!(placement.anchors.contains(Anchor::Bottom));
        assert!(placement.anchors.contains(Anchor::Left));
        assert!(placement.anchors.contains(Anchor::Right));
        assert!(!placement.anchors.contains(Anchor::Top));
        assert_eq!(placement.anchors.len(), 3);
    }

    #[test]
    fn a_top_panel_spans_the_output_from_the_top() {
        let placement = PanelPlacement::derive(&PanelConfig {
            position: PanelPosition::Top,
            ..PanelConfig::default()
        });

        assert!(placement.anchors.contains(Anchor::Top));
        assert!(!placement.anchors.contains(Anchor::Bottom));
        assert_eq!(placement.anchors.len(), 3);
    }

    #[test]
    fn an_exclusive_panel_reserves_its_height_and_margin() {
        let placement = PanelPlacement::derive(&PanelConfig {
            height: 40,
            margin: 6,
            exclusive: true,
            ..PanelConfig::default()
        });

        assert_eq!(placement.height, 40);
        assert_eq!(placement.margin, 6);
        assert_eq!(placement.exclusive_zone, 46);
        assert_eq!(placement.reserved_strip(), 46);
    }

    #[test]
    fn a_panel_that_does_not_reserve_anything_keeps_no_zone() {
        let placement = PanelPlacement::derive(&PanelConfig {
            height: 40,
            margin: 6,
            exclusive: false,
            ..PanelConfig::default()
        });

        assert_eq!(placement.exclusive_zone, 0);
        assert_eq!(placement.reserved_strip(), 0);
        assert_eq!(placement.margin, 6, "the margin still applies to the panel");
    }

    #[test]
    fn ordinary_display_never_takes_keyboard_focus() {
        for exclusive in [true, false] {
            let placement = PanelPlacement::derive(&PanelConfig {
                exclusive,
                ..PanelConfig::default()
            });

            assert_eq!(
                placement.keyboard_interactivity,
                KeyboardInteractivity::None
            );
            assert!(
                !placement.takes_keyboard_focus(),
                "the panel must not steal focus for ordinary display (D01)"
            );
        }
    }

    #[test]
    fn autohide_is_carried_as_data() {
        let visible = PanelPlacement::derive(&PanelConfig {
            autohide: false,
            ..PanelConfig::default()
        });
        let hidden = PanelPlacement::derive(&PanelConfig {
            autohide: true,
            ..PanelConfig::default()
        });

        assert_eq!(visible.autohide, Autohide::Never);
        assert_eq!(hidden.autohide, Autohide::EdgeHover);
    }

    #[test]
    fn the_panel_lands_on_the_panel_layer() {
        let placement = PanelPlacement::derive(&PanelConfig::default());

        assert_eq!(placement.layer, Layer::Top);
    }

    #[test]
    fn every_output_gets_the_same_placement() {
        let assignments = assign(
            &PanelConfig::default(),
            &[OutputId::new("HDMI-A-1"), OutputId::new("eDP-1")],
        );

        assert_eq!(assignments.len(), 2);
        // Sorted by output name, which is byte order: a stable order that does
        // not depend on the order the compositor happened to report them in.
        assert_eq!(assignments[0].output, OutputId::new("HDMI-A-1"));
        assert_eq!(assignments[1].output, OutputId::new("eDP-1"));
        assert_eq!(assignments[0].placement, assignments[1].placement);
    }

    #[test]
    fn an_output_reported_twice_gets_one_panel() {
        let config = PanelConfig::default();
        let outputs = [
            OutputId::new("eDP-1"),
            OutputId::new("HDMI-A-1"),
            OutputId::new("eDP-1"),
        ];

        let assignments = assign(&config, &outputs);

        assert_eq!(assignments.len(), 2);
        assert_eq!(
            assign(&config, &[OutputId::new("eDP-1")])[0].placement,
            assign(&config, &outputs)[0].placement,
            "reconnecting an output keeps the placement it had"
        );
    }

    #[test]
    fn no_output_means_no_panel() {
        assert!(assign(&PanelConfig::default(), &[]).is_empty());
        assert!(Anchors::new().is_empty());
        assert_eq!(Anchors::new().len(), 0);
        assert_eq!(Anchors::new().iter().count(), 0);
    }
}
