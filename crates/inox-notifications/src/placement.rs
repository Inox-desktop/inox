//! Where the popup stack asks to be placed, as pure data.
//!
//! On a real session the notification stack is a `wlr-layer-shell` surface on
//! the **overlay** layer, in one corner of an output, with a margin, and — the
//! part acceptance D06 is specific about — **no keyboard interactivity at all**:
//! a notification must never take the keyboard away from the application the
//! user is typing in, and that is true whatever the urgency is and whether do
//! not disturb is on.
//!
//! Two more properties are policy rather than preference:
//!
//! - the **exclusive zone is zero**. A notification is a transient overlay; it
//!   must not reserve a strip of the output and push windows around, which is
//!   what a panel does and what a notification must not.
//! - the surface asks for **its own height** (`height = 0`, which is how
//!   layer-shell spells "size yourself"): the stack grows downwards from a top
//!   corner and upwards from a bottom one, so an empty stack is a thin surface
//!   rather than a full-height one over the user's windows.
//!
//! **This module computes that placement; nothing applies it yet.** The Inox
//! compositor does not advertise `zwlr_layer_shell_v1`, and `iced_layershell`
//! does not compile against iced 0.14's winit generation (ADR 0018), so the
//! stack runs as an ordinary window. Keeping the policy here, as data with
//! tests, is what makes the move to a layer surface a change of host rather than
//! a change of behaviour; `main.rs` logs exactly one warning naming ADR 0018 and
//! the fact that none of this is applied.

use std::collections::BTreeSet;

use inox_config::{DisplayConfig, NotificationPosition};

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
/// A notification stack is anchored to one corner, which is two edges: anchoring
/// to a single edge would stretch the surface along it.
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
    /// The surface never receives keyboard input, which is what a notification
    /// asks for: showing one must not take the keyboard from the focused
    /// application (acceptance D06).
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
    Top,
    /// Above the panel layer, which is where a notification belongs.
    ///
    /// The lock screen is not a layer and is not bypassed by this: ADR 0015
    /// makes it a protocol of its own, with the compositor owning the lock
    /// state.
    #[default]
    Overlay,
}

/// The name the compositor gives an output.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputId(String);

impl OutputId {
    /// Names an output.
    pub fn new(name: &str) -> Self {
        Self(name.to_owned())
    }

    /// The name itself.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The placement one output's popup stack asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationPlacement {
    /// The corner the stack sits in.
    pub corner: NotificationPosition,
    /// The edges it is anchored to: the corner's two.
    pub anchors: Anchors,
    /// The layer it belongs to.
    pub layer: Layer,
    /// The strip the compositor keeps free of windows: always zero, because a
    /// notification reserves nothing.
    pub exclusive_zone: u32,
    /// Distance between the stack and the edges it is anchored to.
    pub margin: u16,
    /// The width of the stack.
    pub width: u16,
    /// The height the surface asks for; `0` means "as tall as the content is".
    pub height: u32,
    /// Keyboard interactivity: always [`KeyboardInteractivity::None`].
    pub keyboard_interactivity: KeyboardInteractivity,
}

impl NotificationPlacement {
    /// Derives the placement a display configuration asks for.
    pub fn derive(config: &DisplayConfig) -> Self {
        let mut anchors = Anchors::new();
        match config.position {
            NotificationPosition::TopRight => {
                anchors.insert(Anchor::Top);
                anchors.insert(Anchor::Right);
            }
            NotificationPosition::TopLeft => {
                anchors.insert(Anchor::Top);
                anchors.insert(Anchor::Left);
            }
            NotificationPosition::BottomRight => {
                anchors.insert(Anchor::Bottom);
                anchors.insert(Anchor::Right);
            }
            NotificationPosition::BottomLeft => {
                anchors.insert(Anchor::Bottom);
                anchors.insert(Anchor::Left);
            }
        }

        Self {
            corner: config.position,
            anchors,
            layer: Layer::Overlay,
            exclusive_zone: 0,
            margin: config.margin,
            width: config.width,
            height: 0,
            keyboard_interactivity: KeyboardInteractivity::None,
        }
    }

    /// Whether the stack sits in the upper half of the output.
    pub const fn is_on_top(&self) -> bool {
        matches!(
            self.corner,
            NotificationPosition::TopRight | NotificationPosition::TopLeft
        )
    }

    /// Whether the stack sits against the right edge.
    pub const fn is_on_right(&self) -> bool {
        matches!(
            self.corner,
            NotificationPosition::TopRight | NotificationPosition::BottomRight
        )
    }

    /// The strip of the output the stack keeps to itself, which is none.
    pub const fn reserved_strip(&self) -> u32 {
        self.exclusive_zone
    }

    /// Whether the stack takes keyboard input, which it never does.
    pub const fn takes_keyboard_focus(&self) -> bool {
        !matches!(self.keyboard_interactivity, KeyboardInteractivity::None)
    }
}

/// The stack to create for one output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputAssignment {
    /// The output the stack belongs to.
    pub output: OutputId,
    /// The placement it asks for there.
    pub placement: NotificationPlacement,
}

/// One stack per output, in a stable order.
///
/// `outputs` is the set the compositor currently reports, in any order and with
/// any repetition: an output that is removed and reconnected, or reported twice
/// while a topology change settles, must not produce a second stack. The result
/// is therefore sorted by output name and deduplicated, and every output gets
/// the same placement.
pub fn assign(config: &DisplayConfig, outputs: &[OutputId]) -> Vec<OutputAssignment> {
    let placement = NotificationPlacement::derive(config);
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

    fn display(position: NotificationPosition) -> DisplayConfig {
        DisplayConfig {
            position,
            ..DisplayConfig::default()
        }
    }

    #[test]
    fn every_corner_anchors_to_its_two_edges() {
        let cases = [
            (NotificationPosition::TopRight, [Anchor::Top, Anchor::Right]),
            (NotificationPosition::TopLeft, [Anchor::Top, Anchor::Left]),
            (
                NotificationPosition::BottomRight,
                [Anchor::Bottom, Anchor::Right],
            ),
            (
                NotificationPosition::BottomLeft,
                [Anchor::Bottom, Anchor::Left],
            ),
        ];

        for (position, expected) in cases {
            let placement = NotificationPlacement::derive(&display(position));

            assert_eq!(placement.corner, position);
            assert_eq!(placement.anchors.len(), 2, "a corner is two edges");
            for anchor in expected {
                assert!(
                    placement.anchors.contains(anchor),
                    "{position:?} must anchor to {anchor:?}"
                );
            }
        }
    }

    #[test]
    fn the_stack_is_on_the_overlay_layer() {
        for position in [
            NotificationPosition::TopRight,
            NotificationPosition::TopLeft,
            NotificationPosition::BottomRight,
            NotificationPosition::BottomLeft,
        ] {
            assert_eq!(
                NotificationPlacement::derive(&display(position)).layer,
                Layer::Overlay
            );
        }
    }

    #[test]
    fn a_notification_reserves_no_strip_of_the_output() {
        for position in [
            NotificationPosition::TopRight,
            NotificationPosition::TopLeft,
            NotificationPosition::BottomRight,
            NotificationPosition::BottomLeft,
        ] {
            let placement = NotificationPlacement::derive(&display(position));

            assert_eq!(placement.exclusive_zone, 0);
            assert_eq!(
                placement.reserved_strip(),
                0,
                "a notification must not push windows around"
            );
        }
    }

    #[test]
    fn a_notification_never_takes_the_keyboard() {
        for position in [
            NotificationPosition::TopRight,
            NotificationPosition::TopLeft,
            NotificationPosition::BottomRight,
            NotificationPosition::BottomLeft,
        ] {
            let placement = NotificationPlacement::derive(&display(position));

            assert_eq!(
                placement.keyboard_interactivity,
                KeyboardInteractivity::None
            );
            assert!(
                !placement.takes_keyboard_focus(),
                "a notification must never take the keyboard from the focused \
                 application (D06)"
            );
        }

        assert_eq!(
            KeyboardInteractivity::default(),
            KeyboardInteractivity::None
        );
    }

    #[test]
    fn the_surface_sizes_itself_so_an_empty_stack_is_thin() {
        for position in [
            NotificationPosition::TopRight,
            NotificationPosition::BottomLeft,
        ] {
            let placement = NotificationPlacement::derive(&display(position));

            assert_eq!(
                placement.height, 0,
                "layer-shell reads a height of zero as `as tall as I am`"
            );
        }
    }

    #[test]
    fn the_margin_and_the_width_come_from_the_configuration() {
        let placement = NotificationPlacement::derive(&DisplayConfig {
            position: NotificationPosition::BottomLeft,
            margin: 24,
            width: 420,
            ..DisplayConfig::default()
        });

        assert_eq!(placement.margin, 24);
        assert_eq!(placement.width, 420);
    }

    #[test]
    fn which_half_and_which_side_a_corner_is_in() {
        let top_right = NotificationPlacement::derive(&display(NotificationPosition::TopRight));
        let bottom_left = NotificationPlacement::derive(&display(NotificationPosition::BottomLeft));

        assert!(top_right.is_on_top());
        assert!(top_right.is_on_right());
        assert!(!bottom_left.is_on_top());
        assert!(!bottom_left.is_on_right());
    }

    #[test]
    fn every_output_gets_the_same_placement() {
        let assignments = assign(
            &DisplayConfig::default(),
            &[OutputId::new("HDMI-A-1"), OutputId::new("eDP-1")],
        );

        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].output, OutputId::new("HDMI-A-1"));
        assert_eq!(assignments[1].output, OutputId::new("eDP-1"));
        assert_eq!(assignments[0].placement, assignments[1].placement);
        assert_eq!(assignments[0].output.as_str(), "HDMI-A-1");
    }

    #[test]
    fn an_output_reported_twice_gets_one_stack() {
        let config = DisplayConfig::default();
        let outputs = [
            OutputId::new("eDP-1"),
            OutputId::new("HDMI-A-1"),
            OutputId::new("eDP-1"),
        ];

        assert_eq!(assign(&config, &outputs).len(), 2);
    }

    #[test]
    fn no_output_means_no_stack() {
        assert!(assign(&DisplayConfig::default(), &[]).is_empty());
        assert!(Anchors::new().is_empty());
        assert_eq!(Anchors::new().len(), 0);
        assert_eq!(Anchors::new().iter().count(), 0);
    }
}
