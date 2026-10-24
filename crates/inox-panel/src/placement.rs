//! Where the panel asks to be placed, as pure data.
//!
//! On a real session the panel is a `wlr-layer-shell` surface, and the placement
//! it asks for — the anchored edges, the exclusive zone the compositor keeps free
//! of windows, the margin, the height and the keyboard interactivity — is what
//! acceptance D01 is about: *the strip must not cover things, and ordinary
//! display must not take focus*.
//!
//! Each surface asks for its own placement: [`PanelPlacement::derive`] is the
//! taskbar's strip, [`PanelPlacement::menubar`] is the full-width bar along the
//! top edge, and [`PanelPlacement::dock`] is the floating pane above the bottom
//! edge — anchored to the bottom only, so the compositor centres it, and sized
//! from its own content.
//!
//! **[`crate::layer`] applies these placements.** The policy lives here, as data
//! with tests, so that the protocol leg only has to turn it into requests: which
//! anchors, which layer, how much exclusive zone and which size are decided here
//! and nowhere else. The leg commits a placement, and asks for a new one when a
//! reload or a growing task list changes it — the dock's width follows its tiles.
//!
//! Three of the values are what make the panel a panel rather than a window:
//! the exclusive zone keeps maximized windows from slipping under the strip
//! (D01), the keyboard interactivity is `None` so ordinary display never takes
//! the keyboard away from a person, and the layer puts the strip above ordinary
//! windows. A surface anchored to both horizontal edges is given the output's
//! width by the compositor; a dock, anchored to the bottom only, asks for
//! [`PanelPlacement::width`] of its own — the width of everything it will draw,
//! which is what [`DockContent`] describes.

use std::collections::BTreeSet;

use inox_config::{DockConfig, MenubarConfig, PanelConfig, PanelPosition, Theme as Tokens};

use crate::dock::{DockContent, DockGeometry};
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
    /// Width the surface asks for when it is not stretched between two opposite
    /// anchors. `None` is a bar, which spans the output; `Some` is the floating
    /// dock, which is as wide as the tiles it holds.
    ///
    /// A layer surface anchored to only one horizontal edge is centred on that
    /// axis, which is exactly what a dock asks for, so no separate "centred"
    /// flag is needed.
    pub width: Option<u16>,
    /// Keyboard interactivity, `None` for an ordinary panel.
    pub keyboard_interactivity: KeyboardInteractivity,
    /// Layer the panel belongs to.
    pub layer: Layer,
    /// Whether the panel hides itself.
    pub autohide: Autohide,
}

impl PanelPlacement {
    /// Derives the placement the taskbar `config` asks for.
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
            width: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            layer: Layer::Top,
            autohide: if config.autohide {
                Autohide::EdgeHover
            } else {
                Autohide::Never
            },
        }
    }

    /// Derives the placement the menu bar `config` asks for.
    ///
    /// The bar is anchored to the top, left and right edges, so it spans the
    /// output and only its configured height is its own; when it reserves its
    /// strip the exclusive zone is that height plus the margin. It never takes
    /// keyboard focus, and it does not autohide: the schema has no such field
    /// for a bar that lives on the screen edge.
    pub fn menubar(config: &MenubarConfig) -> Self {
        let mut anchors = Anchors::new();
        anchors.insert(Anchor::Top);
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
            width: None,
            keyboard_interactivity: KeyboardInteractivity::None,
            layer: Layer::Top,
            autohide: Autohide::Never,
        }
    }

    /// Derives the placement the dock `config` asks for, sized for `content`.
    ///
    /// The dock is anchored to the bottom edge only. Anchoring to no horizontal
    /// edge is what centres a layer surface on that axis, which is the floating
    /// placement a dock asks for; its width is its content's, derived from
    /// `dock.icon_size`, its tiles, its tray group and its own sentence
    /// ([`DockContent`]) — never below [`DockGeometry::minimum_width`], so a dock
    /// that has nothing to show but its state still has room to say it. The tray
    /// group is text and how wide text is depends on the font, so its width is
    /// the estimate [`DockGeometry::tray_width`] makes and the layout keeps the
    /// last word.
    ///
    /// A dock floats *over* windows, so it asks for no exclusive zone unless
    /// `dock.exclusive` says otherwise.
    pub fn dock(config: &DockConfig, tokens: &Tokens, content: DockContent) -> Self {
        let geometry = DockGeometry::new(tokens, config);
        let height = saturating(geometry.pane_height());
        let width = saturating(geometry.surface_width(content));

        let mut anchors = Anchors::new();
        anchors.insert(Anchor::Bottom);

        Self {
            anchors,
            exclusive_zone: if config.exclusive {
                u32::from(height) + u32::from(config.margin)
            } else {
                0
            },
            margin: config.margin,
            height,
            width: Some(width),
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

    /// Whether the surface is sized from its content rather than stretched
    /// between two opposite anchors.
    pub const fn is_content_sized(&self) -> bool {
        self.width.is_some()
    }

    /// The width the pane is drawn at on an output `output_width` logical pixels
    /// wide.
    ///
    /// A layer surface's size is a *request*: the compositor clamps it to the
    /// room the output has before it configures the surface, so what is drawn is
    /// `min(requested, output)`. An Inox session does exactly that — see
    /// `inox-compositor`'s `layers::axis`, `requested.clamp(1, available)` — and
    /// so does every other `wlr-layer-shell` host. This is that arithmetic as
    /// data, so that a test can ask the question the property is about:
    /// *whatever the dock holds, the pane that is drawn fits the output.* A bar
    /// asks for no width of its own and is given the whole output.
    pub fn drawn_width(&self, output_width: u16) -> u16 {
        match self.width {
            Some(requested) => requested.min(output_width),
            None => output_width,
        }
    }
}

/// A logical size, rounded and bounded so it can be a layer-surface dimension.
fn saturating(size: f32) -> u16 {
    size.round().clamp(0.0, f32::from(u16::MAX)) as u16
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

    /// A dock's content with `tiles` tiles, a tray group `tray_width` wide and
    /// no sentence of its own.
    fn content(tiles: usize, tray_width: f32) -> DockContent {
        DockContent {
            tiles,
            tray_width,
            notice_width: 0.0,
        }
    }

    /// The same, with a sentence (and its action) to show.
    fn notice_content(tiles: usize, tray_width: f32, notice_width: f32) -> DockContent {
        DockContent {
            tiles,
            tray_width,
            notice_width,
        }
    }

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
    fn a_taskbar_spans_the_output_and_is_not_content_sized() {
        let placement = PanelPlacement::derive(&PanelConfig::default());

        assert_eq!(placement.width, None);
        assert!(!placement.is_content_sized());
    }

    #[test]
    fn the_menu_bar_spans_the_top_of_the_output() {
        let placement = PanelPlacement::menubar(&MenubarConfig::default());

        assert!(placement.anchors.contains(Anchor::Top));
        assert!(placement.anchors.contains(Anchor::Left));
        assert!(placement.anchors.contains(Anchor::Right));
        assert!(!placement.anchors.contains(Anchor::Bottom));
        assert_eq!(placement.anchors.len(), 3);
        assert_eq!(placement.height, 28);
        assert_eq!(placement.margin, 0);
        assert_eq!(
            placement.exclusive_zone, 28,
            "an exclusive bar reserves its own height"
        );
        assert_eq!(
            placement.reserved_strip(),
            placement.exclusive_zone,
            "what is reserved is what the compositor keeps free"
        );
        assert_eq!(placement.width, None, "the bar spans the output");
        assert_eq!(placement.layer, Layer::Top);
        assert_eq!(placement.autohide, Autohide::Never);
        assert!(!placement.takes_keyboard_focus());
    }

    #[test]
    fn a_menu_bar_that_does_not_reserve_its_strip_keeps_no_zone() {
        let placement = PanelPlacement::menubar(&MenubarConfig {
            height: 40,
            margin: 6,
            exclusive: false,
            ..MenubarConfig::default()
        });

        assert_eq!(placement.height, 40);
        assert_eq!(placement.margin, 6);
        assert_eq!(placement.exclusive_zone, 0);
        assert_eq!(placement.reserved_strip(), 0);
    }

    #[test]
    fn the_dock_is_anchored_to_the_bottom_edge_only_so_it_is_centred() {
        let tokens = Tokens::builtin_dark();
        let placement = PanelPlacement::dock(&DockConfig::default(), &tokens, content(0, 0.0));

        assert!(placement.anchors.contains(Anchor::Bottom));
        assert_eq!(
            placement.anchors.len(),
            1,
            "with no left or right anchor the compositor centres the pane"
        );
        assert!(!placement.anchors.contains(Anchor::Top));
        assert!(!placement.anchors.contains(Anchor::Left));
        assert!(!placement.anchors.contains(Anchor::Right));
        assert!(placement.is_content_sized());
    }

    #[test]
    fn the_dock_is_sized_from_its_content_and_its_icon_size() {
        let tokens = Tokens::builtin_dark();
        let geometry = DockGeometry::new(&tokens, &DockConfig::default());
        let empty = PanelPlacement::dock(&DockConfig::default(), &tokens, content(0, 0.0));
        let two = PanelPlacement::dock(&DockConfig::default(), &tokens, content(2, 0.0));
        let twelve = PanelPlacement::dock(&DockConfig::default(), &tokens, content(12, 0.0));

        assert_eq!(empty.height, saturating(geometry.pane_height()));
        assert_eq!(
            empty.width,
            Some(saturating(geometry.surface_width(content(0, 0.0))))
        );
        assert_eq!(
            twelve.width,
            Some(saturating(geometry.surface_width(content(12, 0.0))))
        );
        assert_eq!(
            two.width,
            Some(saturating(geometry.surface_width(content(2, 0.0)))),
            "every request is exactly the row the dock draws"
        );
        assert!(
            two.width > empty.width && twelve.width > two.width,
            "a dock is as wide as the tiles it holds: {:?}, {:?}, {:?}",
            empty.width,
            two.width,
            twelve.width
        );

        let bigger = PanelPlacement::dock(
            &DockConfig {
                icon_size: 96,
                ..DockConfig::default()
            },
            &tokens,
            content(12, 0.0),
        );
        assert!(
            bigger.height > twelve.height && bigger.width > twelve.width,
            "a bigger tile is a bigger pane"
        );
    }

    #[test]
    fn an_empty_dock_still_asks_for_room_to_show_its_own_sentence() {
        let tokens = Tokens::builtin_dark();
        let geometry = DockGeometry::new(&tokens, &DockConfig::default());
        let notice = "The window service is not available";
        let empty = PanelPlacement::dock(
            &DockConfig::default(),
            &tokens,
            notice_content(0, 0.0, geometry.notice_width(notice, Some("Retry"))),
        );
        let width = u32::from(empty.width.expect("a dock is content-sized"));

        assert_eq!(
            empty.width,
            Some(saturating(geometry.minimum_width)),
            "a dock with nothing but its own sentence asks for exactly the documented minimum"
        );
        assert!(
            u32::from(empty.width.unwrap_or(0))
                >= geometry.notice_width(notice, Some("Retry")).ceil() as u32,
            "the requested width holds the whole sentence and its retry, not a fragment"
        );
        assert!(
            width > 16,
            "the old empty dock was 16 logical pixels wide, which is not a pane"
        );
    }

    #[test]
    fn a_tray_group_makes_the_dock_wider_by_its_own_labels() {
        let tokens = Tokens::builtin_dark();
        let geometry = DockGeometry::new(&tokens, &DockConfig::default());
        let config = DockConfig::default();
        let plate = PanelPlacement::dock(&config, &tokens, content(2, 0.0));
        let one = PanelPlacement::dock(&config, &tokens, content(2, geometry.tray_width(["Mail"])));
        let two = PanelPlacement::dock(
            &config,
            &tokens,
            content(2, geometry.tray_width(["Mail", "Music"])),
        );
        let longer = PanelPlacement::dock(
            &config,
            &tokens,
            content(2, geometry.tray_width(["Mail", "Music and podcasts"])),
        );

        assert!(
            one.width > plate.width,
            "a tray group widens the pane instead of being clipped by it"
        );
        assert!(two.width > one.width, "more items are more width");
        assert!(longer.width > two.width, "longer labels are more width");
    }

    #[test]
    fn the_dock_grows_with_its_tiles_and_never_shrinks() {
        let tokens = Tokens::builtin_dark();
        let config = DockConfig::default();
        let widths: Vec<u16> = (0..12)
            .map(|tiles| {
                PanelPlacement::dock(&config, &tokens, content(tiles, 0.0))
                    .width
                    .expect("a dock is content-sized")
            })
            .collect();

        for pair in widths.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "one more tile never makes the pane narrower: {widths:?}"
            );
        }
        assert!(
            widths[11] > widths[0],
            "and enough tiles make it wider: {widths:?}"
        );
    }

    #[test]
    fn the_drawn_pane_never_exceeds_the_output_it_is_placed_on() {
        // The size the panel asks for is a *request*: the compositor clamps it to
        // the room the output has before it configures the surface, so what is
        // drawn is `min(requested, output)` (inox-compositor's `layers::axis`).
        // Whatever the dock holds, the pane that reaches the screen fits.
        let tokens = Tokens::builtin_dark();
        let geometry = DockGeometry::new(&tokens, &DockConfig::default());
        let config = DockConfig::default();
        let contents = [
            content(0, 0.0),
            content(0, geometry.tray_width(["Mail", "Music and podcasts"])),
            content(12, geometry.tray_width(["Mail", "Music"])),
            content(12, 0.0),
            notice_content(
                12,
                0.0,
                geometry.notice_width("The window service is not available", Some("Retry")),
            ),
        ];

        for output in [320u16, 640, 1280, 3840] {
            for content in contents {
                let placement = PanelPlacement::dock(&config, &tokens, content);
                let drawn = placement.drawn_width(output);

                assert!(
                    drawn <= output,
                    "a pane wider than the screen cannot be centred on it: {drawn} > {output}"
                );
                assert!(drawn > 0, "and a pane with no width draws nothing at all");
            }
        }

        // A request that fits is not cut down, and a bar takes the whole output.
        let fits = PanelPlacement::dock(&config, &tokens, content(0, 0.0));
        let requested = fits.width.expect("a dock is content-sized");
        assert_eq!(fits.drawn_width(3840), requested);
        assert_eq!(
            PanelPlacement::derive(&PanelConfig::default()).drawn_width(1920),
            1920,
            "a surface anchored to both horizontal edges is given the output's width"
        );
    }

    #[test]
    fn a_dock_floats_over_windows_unless_it_is_asked_to_reserve_its_strip() {
        let tokens = Tokens::builtin_dark();
        let floating = PanelPlacement::dock(&DockConfig::default(), &tokens, content(3, 0.0));
        let reserved = PanelPlacement::dock(
            &DockConfig {
                exclusive: true,
                ..DockConfig::default()
            },
            &tokens,
            content(3, 0.0),
        );

        assert_eq!(
            floating.exclusive_zone, 0,
            "a dock floats over windows by default"
        );
        assert_eq!(floating.margin, 8, "the margin still applies to the pane");
        assert_eq!(
            reserved.exclusive_zone,
            u32::from(reserved.height) + u32::from(reserved.margin)
        );
    }

    #[test]
    fn the_dock_carries_its_margin_and_autohide_as_data() {
        let tokens = Tokens::builtin_dark();
        let placement = PanelPlacement::dock(
            &DockConfig {
                margin: 24,
                autohide: true,
                ..DockConfig::default()
            },
            &tokens,
            content(1, 0.0),
        );

        assert_eq!(placement.margin, 24);
        assert_eq!(placement.autohide, Autohide::EdgeHover);
        assert!(!placement.takes_keyboard_focus());
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
