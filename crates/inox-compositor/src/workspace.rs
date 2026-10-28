//! Managed windows: their mode, workspace, output and the order they tile in.
//!
//! The module is pure state and pure geometry: it never touches Smithay's
//! compositor state or the Wayland display, so the window bookkeeping (add,
//! remove, focus, mode, workspace, output assignment, layout order) is unit
//! testable and property testable independently of the compositor integration.
//! The Smithay layer in [`crate::state`] owns one [`Workspace`] and maps its
//! [`WindowId`]s to real `desktop::Window`s.
//!
//! # The three rules the model is built on
//!
//! - **The mode is not a set of flags.** Every window is in exactly one of
//!   [`WindowMode::Tiled`], [`WindowMode::Floating`], [`WindowMode::Maximized`] and
//!   [`WindowMode::Fullscreen`]. Maximizing a tiled window replaces the tile it
//!   had; returning it to [`WindowMode::Tiled`] puts it back into the master/stack
//!   layout it would have had. Two independent flags ("maximized", "floating")
//!   could describe a maximized *and* floating window, which no compositor draws
//!   and no client can reason about; an enum cannot.
//! - **`minimized` is orthogonal to the mode.** Putting a window away is not a
//!   mode change: the window keeps its mode, its workspace and — for a floating
//!   window — its geometry, so activating it later restores exactly what was
//!   there. Were `Minimized` a fifth mode, that state would have to be stored
//!   somewhere else anyway, and every mode match would grow a `Minimized` arm
//!   that means nothing.
//! - **Only the non-minimized windows of the active workspace tile.** Every other
//!   window keeps its slot, mode and workspace but takes no tile, which makes a
//!   workspace switch a visibility change instead of a relayout of windows the
//!   user cannot see. [`Workspace::arrange`] therefore returns geometry only for
//!   the windows that are actually composited.
//!
//! A window belongs to exactly one output and exactly one workspace. The output
//! is what makes the multi-output migration rule expressible: when an output goes
//! away its windows are handed to a surviving one rather than left in a
//! coordinate space nothing covers. A window's output is `None` only while no
//! output is left at all; the window stays managed and is placed again as soon as
//! one appears.
//!
//! # Invariants
//!
//! The mutators maintain all of these, and the proptests pin them over arbitrary
//! operation sequences:
//!
//! - `focused` is a managed window or `None`;
//! - `focused`, when present, is visible: on the active workspace and not
//!   minimized — a window the user cannot see must never be the one that receives
//!   the keyboard, and the protocol's `focused` flag would be a lie otherwise;
//! - the active workspace and every window's workspace are below the workspace
//!   count, which is at least one;
//! - removing the last window clears the focus.

use smithay::utils::{Logical, Rectangle};

use crate::{
    layout::{LayoutConfig, default_floating_area},
    topology::OutputId,
};

/// Smallest number of workspaces a [`Workspace`] can have.
///
/// One is the floor because a desktop with no workspace has nowhere to put a
/// window: `focused`, `visible` and every workspace number would be meaningless.
pub const MIN_WORKSPACES: u32 = 1;

/// Largest number of workspaces a [`Workspace`] can have.
///
/// The upper bound exists because the compositor binds a workspace switch to
/// `Super+1..=9`; a workspace with no key to reach it would be unreachable state.
pub const MAX_WORKSPACES: u32 = 9;

/// Identifier of a window managed by the compositor.
///
/// Identifiers are handed out by [`Workspace`] and stay stable for the lifetime
/// of the window, which lets the Smithay layer correlate a protocol surface with
/// its slot in the layout order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    /// The raw identifier value.
    pub fn get(self) -> u64 {
        self.0
    }

    /// The identifier a raw value stands for.
    ///
    /// Used by the private window-management protocol: the `uint` handle it carries
    /// is the same number, and a value this run never issued simply names no
    /// window, which is what the protocol's `unknown_window` refuses. Making the
    /// constructor explicit keeps the conversion visible at the boundary instead of
    /// letting a bare `u64` travel as an identifier.
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// How the compositor is showing a managed window.
///
/// A window is in exactly one of these, never a combination: see the module
/// documentation for why this is an enum and not a set of flags. The value is the
/// compositor's statement about what it is doing, which is what the
/// `inox-window-manager-v1` `window_mode` enum reports and what a client learns
/// from the next configure rather than from the request it sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WindowMode {
    /// Placed by the master/stack tiling layout of its output.
    ///
    /// Tiled is the default because a window that has just drawn has not been
    /// given any other mode yet, and the tiling layout is what the compositor
    /// does with it.
    #[default]
    Tiled,
    /// Placed at its own geometry, outside the tiling layout.
    Floating,
    /// Filling its output's work area, so an exclusive-zone panel stays uncovered.
    Maximized,
    /// Filling its output's whole area, panel included.
    Fullscreen,
}

impl WindowMode {
    /// Every mode, in the order the protocol numbers them.
    ///
    /// Used by the tests and by the protocol mapping, so a fifth mode cannot be
    /// added without the mapping being looked at.
    pub const ALL: [Self; 4] = [
        Self::Tiled,
        Self::Floating,
        Self::Maximized,
        Self::Fullscreen,
    ];

    /// Whether the mode takes a tile from the master/stack layout.
    pub fn is_tiled(self) -> bool {
        matches!(self, Self::Tiled)
    }
}

/// One managed window: its identity and everything the compositor tracks about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    id: WindowId,
    output: Option<OutputId>,
    mode: WindowMode,
    minimized: bool,
    workspace: u32,
    /// Where a floating window was last placed, in global logical coordinates.
    ///
    /// `None` until the compositor gives the window a position of its own, which
    /// is what [`default_floating_area`] is the documented fallback for.
    floating_geometry: Option<Rectangle<i32, Logical>>,
}

/// One output as [`Workspace::arrange`] sees it.
///
/// The two rectangles are separate because the modes need both: a maximized window
/// fills the work area (the part an exclusive-zone panel leaves free), while a
/// fullscreen window fills the whole area (which is what the panel being covered
/// means).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputArrangement {
    /// Runtime identity of the output.
    pub id: OutputId,
    /// The output's full logical area.
    pub area: Rectangle<i32, Logical>,
    /// The part ordinary windows may use after exclusive-zone reservations.
    pub work_area: Rectangle<i32, Logical>,
}

impl OutputArrangement {
    /// Describe one output: its whole area and the work area left over.
    pub fn new(
        id: OutputId,
        area: Rectangle<i32, Logical>,
        work_area: Rectangle<i32, Logical>,
    ) -> Self {
        Self {
            id,
            area,
            work_area,
        }
    }
}

/// The ordered set of managed windows plus the focused window, the modes and the
/// workspaces they live on.
///
/// The order is the layout order: the newest window is the first slot and
/// therefore the layout master. The order is global across outputs and
/// workspaces; [`Workspace::arrange`] filters it, so the relative order of two
/// windows on the same output never depends on what else is managed.
#[derive(Debug, Clone)]
pub struct Workspace {
    windows: Vec<Slot>,
    focused: Option<WindowId>,
    next_id: u64,
    active_workspace: u32,
    workspace_count: u32,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            focused: None,
            next_id: 0,
            active_workspace: 0,
            workspace_count: MIN_WORKSPACES,
        }
    }
}

impl Workspace {
    /// Create an empty workspace with a single workspace and nothing on it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an empty workspace with `count` workspaces.
    ///
    /// The count is clamped into [`MIN_WORKSPACES`]..=[`MAX_WORKSPACES`] rather
    /// than rejected: a configuration mistake should degrade into a usable desktop
    /// instead of refusing to start, which is the same direction
    /// [`LayoutConfig::sanitised`] takes.
    pub fn with_workspaces(count: u32) -> Self {
        Self {
            workspace_count: count.clamp(MIN_WORKSPACES, MAX_WORKSPACES),
            ..Self::default()
        }
    }

    /// Add a window on `output` in the master slot, on the active workspace, and
    /// focus it.
    ///
    /// The new window is [`WindowMode::Tiled`] and not minimized: it has just been
    /// drawn, so the user is looking at it.
    pub fn add(&mut self, output: OutputId) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);

        self.windows.insert(
            0,
            Slot {
                id,
                output: Some(output),
                mode: WindowMode::Tiled,
                minimized: false,
                workspace: self.active_workspace,
                floating_geometry: None,
            },
        );
        self.focused = Some(id);

        id
    }

    /// Remove a window, returning whether it was present.
    ///
    /// When the focused window is removed, focus moves to the first visible window
    /// in layout order, or to `None` when nothing visible is left.
    pub fn remove(&mut self, id: WindowId) -> bool {
        let Some(index) = self.windows.iter().position(|slot| slot.id == id) else {
            return false;
        };

        self.windows.remove(index);
        if self.focused == Some(id) {
            self.focused = None;
        }
        self.normalize_focus();

        true
    }

    /// Whether the window is managed by this workspace.
    pub fn contains(&self, id: WindowId) -> bool {
        self.windows.iter().any(|slot| slot.id == id)
    }

    /// Focus a managed window, returning whether it took the focus.
    ///
    /// Only a visible window can take the focus: focusing one on another workspace
    /// or a minimized one would hand the keyboard to a surface that is not on
    /// screen. Activating such a window means switching to its workspace and
    /// un-minimizing it first, which is the caller's job.
    pub fn focus(&mut self, id: WindowId) -> bool {
        match self.slot(id) {
            Some(slot) if self.visible_slot(slot) => {
                self.focused = Some(id);
                true
            }
            _ => false,
        }
    }

    /// Focus the next visible window in layout order, wrapping around.
    pub fn focus_next(&mut self) -> Option<WindowId> {
        self.cycle(1)
    }

    /// Focus the previous visible window in layout order, wrapping around.
    pub fn focus_prev(&mut self) -> Option<WindowId> {
        self.cycle(-1)
    }

    /// The managed windows in layout order, master first.
    ///
    /// Includes windows on other workspaces and minimized ones: this is the
    /// bookkeeping order, not the set the user is looking at.
    /// [`Self::visible_windows`] is the set that is composited.
    pub fn windows(&self) -> Vec<WindowId> {
        self.windows.iter().map(|slot| slot.id).collect()
    }

    /// The composited windows in layout order: on the active workspace and not
    /// minimized.
    pub fn visible_windows(&self) -> Vec<WindowId> {
        self.windows
            .iter()
            .filter(|slot| self.visible_slot(slot))
            .map(|slot| slot.id)
            .collect()
    }

    /// The focused window, if any.
    pub fn focused(&self) -> Option<WindowId> {
        self.focused
    }

    /// Number of managed windows, on every workspace and in every mode.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether no window is managed.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// The output a managed window is placed on, if any.
    pub fn output_of(&self, id: WindowId) -> Option<OutputId> {
        self.slot(id).and_then(|slot| slot.output)
    }

    /// The windows on `output`, in layout order, whatever workspace or mode they
    /// are in.
    pub fn windows_on(&self, output: OutputId) -> Vec<WindowId> {
        self.windows
            .iter()
            .filter(|slot| slot.output == Some(output))
            .map(|slot| slot.id)
            .collect()
    }

    /// Move a window to `output`, returning whether it is tracked.
    ///
    /// The layout order, the mode, the workspace and the floating geometry are
    /// unchanged: a moved window keeps its place among the windows of its new
    /// output, which is what makes a migration after an unplug look like a
    /// rearrangement rather than a reshuffle.
    pub fn assign(&mut self, id: WindowId, output: Option<OutputId>) -> bool {
        match self.slot_mut(id) {
            Some(slot) => {
                slot.output = output;
                true
            }
            None => false,
        }
    }

    /// Hand every window of `from` to `to`, returning the windows that moved.
    ///
    /// `from` is the output being taken away, or `None` for the windows left
    /// without one while no display was available: those are what an output that
    /// has just appeared adopts.
    ///
    /// `to` is `None` when no output is left at all; the windows stay managed and
    /// come back when an output does.
    pub fn reassign(&mut self, from: Option<OutputId>, to: Option<OutputId>) -> Vec<WindowId> {
        let mut moved = Vec::new();

        for slot in &mut self.windows {
            if slot.output == from {
                slot.output = to;
                moved.push(slot.id);
            }
        }

        moved
    }

    /// Put a window into `mode`, returning whether it is tracked.
    ///
    /// The mode replaces the previous one rather than adding to it: a maximized
    /// window put back into [`WindowMode::Tiled`] re-enters the tiling layout with
    /// the order it always had. The mode is independent of `minimized`, of the
    /// workspace and of the geometry a floating window remembers.
    pub fn set_mode(&mut self, id: WindowId, mode: WindowMode) -> bool {
        match self.slot_mut(id) {
            Some(slot) => {
                slot.mode = mode;
                true
            }
            None => false,
        }
    }

    /// The mode a window is in, or `None` when it is not managed.
    pub fn mode_of(&self, id: WindowId) -> Option<WindowMode> {
        self.slot(id).map(|slot| slot.mode)
    }

    /// Put a window away or bring it back, returning whether it is tracked.
    ///
    /// Minimizing the focused window hands the focus to the first other visible
    /// window (or `None` when it was the last one): the keyboard cannot stay on a
    /// surface that is no longer composited. The mode and geometry are kept, so
    /// activating the window later restores exactly what was there.
    pub fn set_minimized(&mut self, id: WindowId, minimized: bool) -> bool {
        let Some(slot) = self.slot_mut(id) else {
            return false;
        };
        slot.minimized = minimized;
        self.normalize_focus();

        true
    }

    /// Whether a window is minimized; `false` for a window that is not managed.
    pub fn is_minimized(&self, id: WindowId) -> bool {
        self.slot(id).is_some_and(|slot| slot.minimized)
    }

    /// The workspace a window is on, or `None` when it is not managed.
    pub fn workspace_of(&self, id: WindowId) -> Option<u32> {
        self.slot(id).map(|slot| slot.workspace)
    }

    /// Whether a window is composited: on the active workspace and not minimized.
    pub fn is_visible(&self, id: WindowId) -> bool {
        self.slot(id).is_some_and(|slot| self.visible_slot(slot))
    }

    /// Move a window to `workspace`, returning whether it moved.
    ///
    /// An out-of-range workspace is refused rather than created: a workspace with
    /// no key binding and no entry on the task bar would be state nothing can
    /// reach. A window moved to the workspace it is already on keeps the focus;
    /// moving the focused window away hands the focus to the next visible window of
    /// the workspace it left.
    pub fn move_to_workspace(&mut self, id: WindowId, workspace: u32) -> bool {
        if workspace >= self.workspace_count {
            return false;
        }
        let Some(slot) = self.slot_mut(id) else {
            return false;
        };
        slot.workspace = workspace;
        self.normalize_focus();

        true
    }

    /// The workspace currently shown.
    pub fn active_workspace(&self) -> u32 {
        self.active_workspace
    }

    /// Show `workspace`, returning whether it exists.
    ///
    /// The focus is re-established on the newly shown workspace when the window
    /// that had it is no longer visible. Windows of the workspace being left stop
    /// being composited; their state is untouched.
    pub fn switch_workspace(&mut self, workspace: u32) -> bool {
        if workspace >= self.workspace_count {
            return false;
        }
        self.active_workspace = workspace;
        self.normalize_focus();

        true
    }

    /// How many workspaces exist; always at least [`MIN_WORKSPACES`].
    pub fn workspace_count(&self) -> u32 {
        self.workspace_count
    }

    /// Replace the workspace count, returning whether it changed.
    ///
    /// The count is clamped into [`MIN_WORKSPACES`]..=[`MAX_WORKSPACES`]. Windows
    /// on a workspace that disappears move to the last one that remains, and the
    /// active workspace follows the same rule, so no window is ever left on a
    /// workspace number that no longer exists.
    pub fn set_workspace_count(&mut self, count: u32) -> bool {
        let count = count.clamp(MIN_WORKSPACES, MAX_WORKSPACES);
        if count == self.workspace_count {
            return false;
        }

        self.workspace_count = count;
        let last = count - 1;
        if self.active_workspace > last {
            self.active_workspace = last;
        }
        for slot in &mut self.windows {
            if slot.workspace > last {
                slot.workspace = last;
            }
        }
        self.normalize_focus();

        true
    }

    /// Remember the geometry of a floating window, returning whether it is tracked.
    ///
    /// Only [`WindowMode::Floating`] reads this back; the geometry is kept across
    /// mode changes so a window that floats, maximizes and floats again returns to
    /// where the user left it. The compositor clamps the rectangle to the output
    /// when it produces one from a pointer drag.
    pub fn set_floating_geometry(
        &mut self,
        id: WindowId,
        geometry: Rectangle<i32, Logical>,
    ) -> bool {
        match self.slot_mut(id) {
            Some(slot) => {
                slot.floating_geometry = Some(geometry);
                true
            }
            None => false,
        }
    }

    /// The geometry a floating window was last given, if any.
    pub fn floating_geometry_of(&self, id: WindowId) -> Option<Rectangle<i32, Logical>> {
        self.slot(id).and_then(|slot| slot.floating_geometry)
    }

    /// Compute one block per composited window, in layout order within each output.
    ///
    /// `outputs` is the arrangement of every advertised output, in the order the
    /// outputs should be processed. Windows on an output that is not listed — the
    /// case where every display is gone — get no block and are left as they are.
    ///
    /// The blocks follow the window's mode:
    ///
    /// - [`WindowMode::Tiled`]: the master/stack layout of the output's work area,
    ///   computed over the tiled windows of the active workspace only;
    /// - [`WindowMode::Maximized`]: exactly the output's work area, so an
    ///   exclusive-zone panel stays uncovered;
    /// - [`WindowMode::Fullscreen`]: exactly the output's whole area, which is what
    ///   covering the panel means;
    /// - [`WindowMode::Floating`]: the geometry the window remembers, or
    ///   [`default_floating_area`] while it has none.
    ///
    /// Windows on another workspace and minimized windows get no block: they are
    /// not composited, and a block for them would put them back on screen (the
    /// compositor maps exactly the windows this returns).
    pub fn arrange(
        &self,
        outputs: &[OutputArrangement],
        config: LayoutConfig,
    ) -> Vec<(WindowId, Rectangle<i32, Logical>)> {
        let mut blocks = Vec::with_capacity(self.windows.len());

        for output in outputs {
            let tiled: Vec<WindowId> = self
                .windows
                .iter()
                .filter(|slot| {
                    slot.output == Some(output.id)
                        && slot.mode.is_tiled()
                        && self.visible_slot(slot)
                })
                .map(|slot| slot.id)
                .collect();

            blocks.extend(
                config
                    .arrange(output.work_area, tiled.len())
                    .into_iter()
                    .zip(tiled)
                    .map(|(block, id)| (id, block)),
            );

            for slot in self.windows.iter().filter(|slot| {
                slot.output == Some(output.id) && !slot.mode.is_tiled() && self.visible_slot(slot)
            }) {
                let block = match slot.mode {
                    WindowMode::Tiled => continue,
                    WindowMode::Floating => slot
                        .floating_geometry
                        .unwrap_or_else(|| default_floating_area(output.work_area)),
                    WindowMode::Maximized => output.work_area,
                    WindowMode::Fullscreen => output.area,
                };
                blocks.push((slot.id, block));
            }
        }

        blocks
    }

    /// The slot of a managed window.
    fn slot(&self, id: WindowId) -> Option<&Slot> {
        self.windows.iter().find(|slot| slot.id == id)
    }

    /// The slot of a managed window, mutably.
    fn slot_mut(&mut self, id: WindowId) -> Option<&mut Slot> {
        self.windows.iter_mut().find(|slot| slot.id == id)
    }

    /// Whether `slot` is composited: on the active workspace and not minimized.
    fn visible_slot(&self, slot: &Slot) -> bool {
        slot.workspace == self.active_workspace && !slot.minimized
    }

    /// Put the focus back on a visible window whenever it is not one.
    ///
    /// Called after every mutation that can hide the focused window (remove,
    /// minimize, workspace moves and switches, count changes), so the invariant
    /// "the focus is on a window the user can see" is restored in one place rather
    /// than in each mutator.
    fn normalize_focus(&mut self) {
        if self.focused.is_some_and(|id| self.is_visible(id)) {
            return;
        }
        self.focused = self.visible_windows().into_iter().next();
    }

    /// Move the focus by `delta` slots among the visible windows, wrapping around.
    fn cycle(&mut self, delta: isize) -> Option<WindowId> {
        let visible = self.visible_windows();
        if visible.is_empty() {
            self.focused = None;
            return None;
        }

        let current = self
            .focused
            .and_then(|focused| visible.iter().position(|id| *id == focused));

        let next = match current {
            Some(index) => (index as isize + delta).rem_euclid(visible.len() as isize) as usize,
            None => 0,
        };

        let id = visible[next];
        self.focused = Some(id);

        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(raw: u64) -> OutputId {
        OutputId::new(raw)
    }

    fn area(w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), (w, h).into())
    }

    fn placed(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// One output whose full area and work area are the same rectangle.
    fn full_output(id: OutputId, rectangle: Rectangle<i32, Logical>) -> OutputArrangement {
        OutputArrangement::new(id, rectangle, rectangle)
    }

    /// A layout config without gaps, so the geometry assertions are exact.
    fn exact_layout() -> LayoutConfig {
        LayoutConfig {
            gap: 0,
            ..LayoutConfig::default()
        }
    }

    #[test]
    fn adding_windows_assigns_unique_ids_and_focuses_the_newest() {
        let mut workspace = Workspace::new();

        let first = workspace.add(output(0));
        let second = workspace.add(output(0));

        assert_ne!(first, second);
        assert_eq!(workspace.len(), 2);
        assert_eq!(workspace.focused(), Some(second));
        assert_eq!(workspace.windows(), vec![second, first]);
    }

    #[test]
    fn a_new_window_belongs_to_the_output_it_was_added_on() {
        let mut workspace = Workspace::new();
        let first = workspace.add(output(1));
        let second = workspace.add(output(2));

        assert_eq!(workspace.output_of(first), Some(output(1)));
        assert_eq!(workspace.output_of(second), Some(output(2)));
        assert_eq!(workspace.windows_on(output(1)), vec![first]);
        assert_eq!(workspace.windows_on(output(2)), vec![second]);
    }

    #[test]
    fn removing_the_focused_window_moves_focus_to_the_next_slot() {
        let mut workspace = Workspace::new();
        let first = workspace.add(output(0));
        let second = workspace.add(output(0));
        let third = workspace.add(output(0));

        // The newest window is the master and holds the focus.
        assert_eq!(workspace.windows(), vec![third, second, first]);
        assert_eq!(workspace.focused(), Some(third));

        assert!(workspace.remove(third));
        assert_eq!(workspace.windows(), vec![second, first]);
        assert_eq!(workspace.focused(), Some(second));
    }

    #[test]
    fn removing_an_unfocused_window_keeps_the_focus() {
        let mut workspace = Workspace::new();
        let first = workspace.add(output(0));
        let second = workspace.add(output(0));
        let third = workspace.add(output(0));

        assert!(workspace.remove(first));

        assert_eq!(workspace.windows(), vec![third, second]);
        assert_eq!(workspace.focused(), Some(third));
    }

    #[test]
    fn removing_an_unknown_window_changes_nothing() {
        let mut workspace = Workspace::new();
        let id = workspace.add(output(0));
        let unknown = WindowId(9_999);

        assert!(!workspace.remove(unknown));
        assert_eq!(workspace.len(), 1);
        assert_eq!(workspace.focused(), Some(id));
    }

    #[test]
    fn removing_the_last_window_clears_the_focus() {
        let mut workspace = Workspace::new();
        let id = workspace.add(output(0));

        assert!(workspace.remove(id));
        assert!(workspace.is_empty());
        assert_eq!(workspace.focused(), None);
    }

    #[test]
    fn focus_cycles_through_every_window_and_wraps() {
        let mut workspace = Workspace::new();
        let first = workspace.add(output(0));
        let second = workspace.add(output(0));
        let third = workspace.add(output(0));

        // Layout order is [third, second, first]; focus starts on third.
        assert_eq!(workspace.focus_next(), Some(second));
        assert_eq!(workspace.focus_next(), Some(first));
        assert_eq!(workspace.focus_next(), Some(third));
        assert_eq!(workspace.focus_prev(), Some(first));
    }

    #[test]
    fn focus_ignores_untracked_windows() {
        let mut workspace = Workspace::new();
        workspace.add(output(0));

        assert!(!workspace.focus(WindowId(42)));
    }

    #[test]
    fn fresh_windows_are_arranged_in_layout_order() {
        let mut workspace = Workspace::new();
        let first = workspace.add(output(0));
        let second = workspace.add(output(0));

        let config = LayoutConfig {
            gap: 0,
            ..LayoutConfig::default()
        };
        let tiles = workspace.arrange(&[full_output(output(0), area(100, 100))], config);

        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].0, second);
        assert_eq!(tiles[1].0, first);
    }

    #[test]
    fn each_output_tiles_its_own_windows_inside_its_own_work_area() {
        let mut workspace = Workspace::new();
        let left = workspace.add(output(0));
        let right = workspace.add(output(1));

        let config = LayoutConfig {
            gap: 0,
            ..LayoutConfig::default()
        };
        let tiles = workspace.arrange(
            &[
                full_output(output(0), Rectangle::new((0, 0).into(), (800, 600).into())),
                full_output(
                    output(1),
                    Rectangle::new((800, 0).into(), (400, 300).into()),
                ),
            ],
            config,
        );

        assert_eq!(tiles.len(), 2);
        // Output order first, then layout order within each output.
        assert_eq!(
            tiles[0],
            (left, Rectangle::new((0, 0).into(), (800, 600).into()))
        );
        assert_eq!(
            tiles[1],
            (right, Rectangle::new((800, 0).into(), (400, 300).into()))
        );
    }

    #[test]
    fn a_window_on_an_output_that_is_not_tiled_yields_no_tile() {
        let mut workspace = Workspace::new();
        workspace.add(output(0));
        workspace.add(output(1));

        let tiles = workspace.arrange(
            &[full_output(output(0), area(800, 600))],
            LayoutConfig::default(),
        );

        assert_eq!(tiles.len(), 1);
        assert_eq!(workspace.windows_on(output(1)).len(), 1);
    }

    #[test]
    fn an_unplugged_output_hands_its_windows_to_a_survivor() {
        let mut workspace = Workspace::new();
        let left = workspace.add(output(0));
        let right = workspace.add(output(1));

        let moved = workspace.reassign(Some(output(1)), Some(output(0)));

        assert_eq!(moved, vec![right]);
        assert_eq!(workspace.output_of(right), Some(output(0)));
        // The layout order is untouched by the move.
        assert_eq!(workspace.windows(), vec![right, left]);
        assert_eq!(workspace.windows_on(output(0)), vec![right, left]);
        assert!(workspace.windows_on(output(1)).is_empty());
    }

    #[test]
    fn losing_every_output_leaves_the_windows_managed() {
        let mut workspace = Workspace::new();
        let id = workspace.add(output(0));

        assert_eq!(workspace.reassign(Some(output(0)), None), vec![id]);
        assert_eq!(workspace.output_of(id), None);
        assert!(workspace.contains(id));
        assert!(workspace.arrange(&[], LayoutConfig::default()).is_empty());
    }

    #[test]
    fn an_output_that_appears_adopts_the_windows_left_without_one() {
        let mut workspace = Workspace::new();
        let id = workspace.add(output(0));
        workspace.reassign(Some(output(0)), None);

        let adopted = workspace.reassign(None, Some(output(2)));

        assert_eq!(adopted, vec![id]);
        assert_eq!(workspace.output_of(id), Some(output(2)));
    }

    #[test]
    fn assigning_an_unknown_window_changes_nothing() {
        assert!(!Workspace::new().assign(WindowId(7), Some(output(0))));
    }

    #[test]
    fn a_window_starts_tiled_unminimized_on_the_active_workspace() {
        let mut workspace = Workspace::with_workspaces(3);
        let id = workspace.add(output(0));

        assert_eq!(workspace.mode_of(id), Some(WindowMode::Tiled));
        assert!(!workspace.is_minimized(id));
        assert_eq!(
            workspace.workspace_of(id),
            Some(workspace.active_workspace())
        );
        assert!(workspace.is_visible(id));
        assert_eq!(workspace.active_workspace(), 0);
        assert_eq!(workspace.workspace_count(), 3);
    }

    #[test]
    fn a_window_added_while_another_workspace_is_shown_lands_on_it() {
        let mut workspace = Workspace::with_workspaces(2);
        assert!(workspace.switch_workspace(1));

        let id = workspace.add(output(0));

        assert_eq!(workspace.workspace_of(id), Some(1));
        assert!(workspace.is_visible(id));
        assert_eq!(workspace.focused(), Some(id));
    }

    #[test]
    fn a_maximized_window_takes_the_work_area_and_leaves_the_tiling_alone() {
        let mut workspace = Workspace::new();
        let tiled = workspace.add(output(0));
        let maximized = workspace.add(output(0));
        let full = placed(0, 0, 1000, 600);
        let work = placed(0, 30, 1000, 570);

        assert!(workspace.set_mode(maximized, WindowMode::Maximized));
        let tiles = workspace.arrange(
            &[OutputArrangement::new(output(0), full, work)],
            exact_layout(),
        );

        assert_eq!(tiles.len(), 2);
        assert_eq!(
            tiles
                .iter()
                .find(|(id, _)| *id == maximized)
                .map(|(_, block)| *block),
            Some(work)
        );
        // The other window is still tiled: a mode change replaces one window's
        // placement, not the layout of the windows behind it.
        assert_eq!(
            tiles
                .iter()
                .find(|(id, _)| *id == tiled)
                .map(|(_, block)| *block),
            Some(work)
        );
        assert_eq!(workspace.mode_of(maximized), Some(WindowMode::Maximized));
    }

    #[test]
    fn a_fullscreen_window_takes_the_whole_output_area() {
        let mut workspace = Workspace::new();
        let id = workspace.add(output(0));
        let full = placed(0, 0, 1000, 600);
        let work = placed(0, 30, 1000, 570);

        assert!(workspace.set_mode(id, WindowMode::Fullscreen));
        let tiles = workspace.arrange(
            &[OutputArrangement::new(output(0), full, work)],
            exact_layout(),
        );

        // Fullscreen covers the exclusive zone: the composited pixels reach the
        // output edge, which is what covering the panel means.
        assert_eq!(tiles, vec![(id, full)]);
    }

    #[test]
    fn a_floating_window_uses_its_own_geometry() {
        let mut workspace = Workspace::new();
        let id = workspace.add(output(0));
        let geometry = placed(120, 90, 400, 300);

        assert!(workspace.set_mode(id, WindowMode::Floating));
        assert!(workspace.set_floating_geometry(id, geometry));
        let tiles = workspace.arrange(&[full_output(output(0), area(1000, 600))], exact_layout());

        assert_eq!(tiles, vec![(id, geometry)]);
        assert_eq!(workspace.floating_geometry_of(id), Some(geometry));
    }

    #[test]
    fn a_floating_window_without_geometry_falls_back_to_a_centred_default() {
        let mut workspace = Workspace::new();
        let id = workspace.add(output(0));

        assert!(workspace.set_mode(id, WindowMode::Floating));
        let tiles = workspace.arrange(&[full_output(output(0), area(900, 600))], exact_layout());

        // Two thirds of the work area, centred: 594x396 at (153, 102).
        assert_eq!(tiles, vec![(id, placed(153, 102, 594, 396))]);
    }

    #[test]
    fn minimizing_keeps_the_mode_and_the_floating_geometry() {
        let mut workspace = Workspace::new();
        let first = workspace.add(output(0));
        let id = workspace.add(output(0));
        let geometry = placed(10, 20, 300, 200);
        assert!(workspace.set_mode(id, WindowMode::Floating));
        assert!(workspace.set_floating_geometry(id, geometry));

        assert!(workspace.set_minimized(id, true));

        assert!(workspace.is_minimized(id));
        assert!(!workspace.is_visible(id));
        assert_eq!(workspace.mode_of(id), Some(WindowMode::Floating));
        assert_eq!(workspace.floating_geometry_of(id), Some(geometry));
        assert_eq!(workspace.workspace_of(id), Some(0));
        let tiles = workspace.arrange(&[full_output(output(0), area(1000, 600))], exact_layout());
        assert_eq!(
            tiles.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![first]
        );
        // The window that had the focus is away, so the focus moved on.
        assert_eq!(workspace.focused(), Some(first));

        assert!(workspace.set_minimized(id, false));
        assert!(workspace.is_visible(id));
        assert_eq!(workspace.mode_of(id), Some(WindowMode::Floating));
    }

    #[test]
    fn minimizing_an_unfocused_window_keeps_the_focus() {
        let mut workspace = Workspace::new();
        let first = workspace.add(output(0));
        let second = workspace.add(output(0));

        assert!(workspace.set_minimized(first, true));

        assert_eq!(workspace.focused(), Some(second));
        assert!(!workspace.is_visible(first));
    }

    #[test]
    fn focus_cycling_skips_minimized_windows() {
        let mut workspace = Workspace::new();
        let first = workspace.add(output(0));
        let second = workspace.add(output(0));
        let third = workspace.add(output(0));

        assert!(workspace.set_minimized(second, true));

        // Visible order is [third, first].
        assert_eq!(workspace.focus_next(), Some(first));
        assert_eq!(workspace.focus_next(), Some(third));
        assert_eq!(workspace.focus_prev(), Some(first));
    }

    #[test]
    fn focus_refuses_a_window_that_is_not_composited() {
        let mut workspace = Workspace::with_workspaces(2);
        let on_first = workspace.add(output(0));
        assert!(workspace.switch_workspace(1));
        let on_second = workspace.add(output(0));

        assert!(!workspace.focus(on_first));
        assert_eq!(workspace.focused(), Some(on_second));

        assert!(workspace.set_minimized(on_second, true));
        assert!(!workspace.focus(on_second));
        assert_eq!(workspace.focused(), None);
    }

    #[test]
    fn switching_workspace_hides_the_old_windows_and_focuses_the_new_one() {
        let mut workspace = Workspace::with_workspaces(2);
        let first = workspace.add(output(0));
        assert!(workspace.switch_workspace(1));
        let second = workspace.add(output(0));

        assert!(workspace.switch_workspace(0));

        assert_eq!(workspace.focused(), Some(first));
        assert!(workspace.is_visible(first));
        assert!(!workspace.is_visible(second));
        assert_eq!(workspace.visible_windows(), vec![first]);
        // The window on the other workspace takes no tile.
        let tiles = workspace.arrange(&[full_output(output(0), area(800, 600))], exact_layout());
        assert_eq!(
            tiles.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![first]
        );
        // Its state is untouched: it comes back on the workspace switch.
        assert_eq!(workspace.workspace_of(second), Some(1));
        assert!(workspace.switch_workspace(1));
        assert!(workspace.is_visible(second));
    }

    #[test]
    fn switching_to_an_out_of_range_workspace_is_refused() {
        let mut workspace = Workspace::with_workspaces(2);
        let id = workspace.add(output(0));

        assert!(!workspace.switch_workspace(2));
        assert_eq!(workspace.active_workspace(), 0);
        assert!(workspace.is_visible(id));
        assert!(workspace.switch_workspace(0));
    }

    #[test]
    fn moving_the_focused_window_away_hands_the_focus_on() {
        let mut workspace = Workspace::with_workspaces(3);
        let first = workspace.add(output(0));
        let second = workspace.add(output(0));
        assert_eq!(workspace.focused(), Some(second));

        assert!(workspace.move_to_workspace(second, 1));

        assert_eq!(workspace.workspace_of(second), Some(1));
        assert!(!workspace.is_visible(second));
        assert_eq!(workspace.focused(), Some(first));
    }

    #[test]
    fn moving_a_window_to_the_workspace_it_is_already_on_keeps_the_focus() {
        let mut workspace = Workspace::with_workspaces(2);
        let id = workspace.add(output(0));

        assert!(workspace.move_to_workspace(id, 0));

        assert_eq!(workspace.focused(), Some(id));
        assert!(workspace.is_visible(id));
    }

    #[test]
    fn moving_to_an_out_of_range_workspace_is_refused() {
        let mut workspace = Workspace::with_workspaces(2);
        let id = workspace.add(output(0));

        assert!(!workspace.move_to_workspace(id, 2));
        assert!(!workspace.move_to_workspace(id, u32::MAX));
        assert_eq!(workspace.workspace_of(id), Some(0));
        assert!(!workspace.move_to_workspace(WindowId(999), 1));
    }

    #[test]
    fn a_window_moved_onto_the_shown_workspace_becomes_visible() {
        let mut workspace = Workspace::with_workspaces(2);
        let shown = workspace.add(output(0));
        assert!(workspace.switch_workspace(1));
        let hidden = workspace.add(output(0));
        assert!(!workspace.is_visible(shown));

        assert!(workspace.move_to_workspace(shown, 1));

        assert!(workspace.is_visible(shown));
        // Moving a window in does not steal the focus from the one already there.
        assert_eq!(workspace.focused(), Some(hidden));
    }

    #[test]
    fn the_workspace_count_stays_inside_its_range() {
        assert_eq!(Workspace::new().workspace_count(), MIN_WORKSPACES);
        assert_eq!(
            Workspace::with_workspaces(0).workspace_count(),
            MIN_WORKSPACES
        );
        assert_eq!(
            Workspace::with_workspaces(u32::MAX).workspace_count(),
            MAX_WORKSPACES
        );
    }

    #[test]
    fn shrinking_the_workspace_count_keeps_every_window_on_a_valid_workspace() {
        let mut workspace = Workspace::with_workspaces(4);
        let first = workspace.add(output(0));
        assert!(workspace.switch_workspace(3));
        let last = workspace.add(output(0));

        assert!(workspace.set_workspace_count(2));

        assert_eq!(workspace.workspace_count(), 2);
        assert_eq!(workspace.active_workspace(), 1);
        assert_eq!(workspace.workspace_of(last), Some(1));
        assert_eq!(workspace.workspace_of(first), Some(0));
        for id in workspace.windows() {
            assert!(workspace.workspace_of(id).is_some_and(|ws| ws < 2));
        }
        // `last` ends up on the workspace that is now shown, so the focus could
        // stay where it was; `first` never moved.
        assert!(workspace.is_visible(last));
        assert!(!workspace.set_workspace_count(2));
    }

    #[test]
    fn growing_the_workspace_count_does_not_move_anything() {
        let mut workspace = Workspace::new();
        let id = workspace.add(output(0));

        assert!(workspace.set_workspace_count(4));

        assert_eq!(workspace.workspace_of(id), Some(0));
        assert_eq!(workspace.active_workspace(), 0);
        assert_eq!(workspace.focused(), Some(id));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// One mutation a user or a compositor can perform, with indices instead of
    /// window identifiers so the strategy does not have to know them.
    #[derive(Debug, Clone)]
    enum Op {
        Add,
        Remove(usize),
        Focus(usize),
        SetMode(usize, usize),
        Minimize(usize, bool),
        SwitchWorkspace(usize),
        MoveToWorkspace(usize, usize),
        SetWorkspaceCount(usize),
    }

    fn operation() -> impl Strategy<Value = Op> {
        prop_oneof![
            Just(Op::Add),
            (0usize..12).prop_map(Op::Remove),
            (0usize..12).prop_map(Op::Focus),
            ((0usize..12), (0usize..4)).prop_map(|(window, mode)| Op::SetMode(window, mode)),
            ((0usize..12), any::<bool>()).prop_map(|(window, flag)| Op::Minimize(window, flag)),
            (0usize..12).prop_map(Op::SwitchWorkspace),
            ((0usize..12), (0usize..12))
                .prop_map(|(window, workspace)| Op::MoveToWorkspace(window, workspace)),
            (0usize..12).prop_map(Op::SetWorkspaceCount),
        ]
    }

    fn mode(index: usize) -> WindowMode {
        WindowMode::ALL[index % WindowMode::ALL.len()]
    }

    /// Apply one operation, keeping `ids` exactly the set of managed windows.
    fn apply(workspace: &mut Workspace, ids: &mut Vec<WindowId>, op: &Op) {
        let len = ids.len();

        match *op {
            Op::Add => {
                let id = workspace.add(OutputId::new(ids.len() as u64 % 4));
                ids.push(id);
            }
            Op::Remove(index) if len > 0 => {
                let id = ids.remove(index % len);
                assert!(workspace.remove(id));
            }
            Op::Focus(index) if len > 0 => {
                workspace.focus(ids[index % len]);
            }
            Op::SetMode(index, choice) if len > 0 => {
                assert!(workspace.set_mode(ids[index % len], mode(choice)));
            }
            Op::Minimize(index, flag) if len > 0 => {
                assert!(workspace.set_minimized(ids[index % len], flag));
            }
            Op::SwitchWorkspace(index) => {
                workspace.switch_workspace(index as u32 % 4);
            }
            Op::MoveToWorkspace(index, workspace_index) if len > 0 => {
                workspace.move_to_workspace(ids[index % len], workspace_index as u32 % 5);
            }
            Op::SetWorkspaceCount(count) => {
                workspace.set_workspace_count(1 + count as u32 % 5);
            }
            _ => {}
        }
    }

    fn placed(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Four outputs side by side, each with an exclusive-zone-sized work area.
    fn outputs() -> Vec<OutputArrangement> {
        (0..4u64)
            .map(|index| {
                let x = index as i32 * 1280;
                OutputArrangement::new(
                    OutputId::new(index),
                    placed(x, 0, 1280, 720),
                    placed(x, 30, 1280, 690),
                )
            })
            .collect()
    }

    fn run_operations(operations: &[Op]) -> (Workspace, Vec<WindowId>) {
        let mut workspace = Workspace::with_workspaces(3);
        let mut ids = Vec::new();
        for op in operations {
            apply(&mut workspace, &mut ids, op);
        }
        (workspace, ids)
    }

    /// The invariant block every arbitrary operation sequence must preserve.
    fn assert_invariants(workspace: &Workspace, ids: &[WindowId]) {
        assert_eq!(workspace.len(), ids.len());
        for id in ids {
            assert!(workspace.contains(*id));
        }

        assert!(workspace.active_workspace() < workspace.workspace_count());
        for id in ids {
            let workspace_number = workspace
                .workspace_of(*id)
                .expect("a managed window is on a workspace");
            assert!(workspace_number < workspace.workspace_count());
        }

        match workspace.focused() {
            Some(focused) => {
                assert!(workspace.contains(focused));
                assert!(workspace.is_visible(focused));
            }
            None => assert!(workspace.visible_windows().is_empty()),
        }

        // `is_visible` is exactly "on the shown workspace and not minimized".
        for id in ids {
            let expected = workspace.workspace_of(*id) == Some(workspace.active_workspace())
                && !workspace.is_minimized(*id);
            assert_eq!(workspace.is_visible(*id), expected);
        }

        let expected: Vec<WindowId> = workspace
            .windows()
            .into_iter()
            .filter(|id| workspace.is_visible(*id))
            .collect();
        assert_eq!(workspace.visible_windows(), expected);
    }

    /// Whether `inner` lies inside `outer`, edges included.
    fn inside(inner: Rectangle<i32, Logical>, outer: Rectangle<i32, Logical>) -> bool {
        inner.loc.x >= outer.loc.x
            && inner.loc.y >= outer.loc.y
            && inner.loc.x + inner.size.w <= outer.loc.x + outer.size.w
            && inner.loc.y + inner.size.h <= outer.loc.y + outer.size.h
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn added_ids_are_unique_and_tracked(operations in 1usize..24) {
            let mut workspace = Workspace::new();
            let mut ids = Vec::new();

            for index in 0..operations {
                let id = workspace.add(OutputId::new(index as u64 % 4));
                prop_assert!(!ids.contains(&id));
                ids.push(id);
            }

            prop_assert_eq!(workspace.len(), operations);
            for id in &ids {
                prop_assert!(workspace.contains(*id));
            }
            prop_assert!(workspace.windows().iter().all(|id| ids.contains(id)));
        }

        #[test]
        fn removed_windows_leave_the_focus_valid(operations in 1usize..24) {
            let mut workspace = Workspace::new();
            let mut ids = Vec::new();

            for index in 0..operations {
                ids.push(workspace.add(OutputId::new(index as u64 % 4)));
            }

            while let Some(id) = ids.pop() {
                prop_assert!(workspace.remove(id));
                prop_assert_eq!(workspace.len(), ids.len());

                match workspace.focused() {
                    Some(focused) => prop_assert!(workspace.contains(focused)),
                    None => prop_assert!(workspace.is_empty()),
                }
            }
        }

        #[test]
        fn focus_cycling_stays_within_the_window_set(steps in 0usize..40) {
            let mut workspace = Workspace::new();
            for index in 0..3 {
                workspace.add(OutputId::new(index));
            }

            for step in 0..steps {
                let focused = if step % 2 == 0 {
                    workspace.focus_next()
                } else {
                    workspace.focus_prev()
                };

                let focused = focused.expect("a non-empty workspace always has a focused window");
                prop_assert!(workspace.contains(focused));
                prop_assert_eq!(workspace.focused(), Some(focused));
            }
        }

        #[test]
        fn arranging_covers_every_window_on_a_tiled_output(
            per_output in prop::collection::vec(0usize..4, 4),
            gap in 0i32..32,
        ) {
            let mut workspace = Workspace::new();
            let mut expected: Vec<WindowId> = Vec::new();
            for (output, count) in per_output.iter().enumerate() {
                for _ in 0..*count {
                    expected.push(workspace.add(OutputId::new(output as u64)));
                }
            }

            let config = LayoutConfig { gap, ..LayoutConfig::default() };
            let areas: Vec<_> = per_output
                .iter()
                .enumerate()
                .map(|(output, _)| {
                    let area = Rectangle::<i32, Logical>::new((0, 0).into(), (1280, 720).into());
                    OutputArrangement::new(OutputId::new(output as u64), area, area)
                })
                .collect();
            let tiles = workspace.arrange(&areas, config);

            prop_assert_eq!(tiles.len(), expected.len());
            for (id, _) in &tiles {
                prop_assert!(expected.contains(id));
            }
        }

        #[test]
        fn a_tile_never_escapes_its_work_area(
            per_output in prop::collection::vec(0usize..4, 4),
            gap in 0i32..32,
        ) {
            let mut workspace = Workspace::new();
            for (output, count) in per_output.iter().enumerate() {
                for _ in 0..*count {
                    workspace.add(OutputId::new(output as u64));
                }
            }

            let config = LayoutConfig { gap, ..LayoutConfig::default() };
            let areas: Vec<_> = per_output
                .iter()
                .enumerate()
                .map(|(output, _)| {
                    let area = Rectangle::<i32, Logical>::new(
                        (output as i32 * 1280, 0).into(),
                        (1280, 720).into(),
                    );
                    OutputArrangement::new(OutputId::new(output as u64), area, area)
                })
                .collect();

            for (id, tile) in workspace.arrange(&areas, config) {
                let output = workspace.output_of(id).expect("a tiled window has an output");
                let area = areas
                    .iter()
                    .find(|candidate| candidate.id == output)
                    .map(|candidate| candidate.work_area)
                    .expect("the tiled output has a work area");
                prop_assert!(tile.loc.x >= area.loc.x);
                prop_assert!(tile.loc.y >= area.loc.y);
                prop_assert!(tile.loc.x + tile.size.w <= area.loc.x + area.size.w);
                prop_assert!(tile.loc.y + tile.size.h <= area.loc.y + area.size.h);
            }
        }

        #[test]
        fn reassigning_an_output_leaves_no_window_stranded(
            outputs in prop::collection::vec(0u64..4, 0..8),
            survivor_is_live in any::<bool>(),
        ) {
            let mut workspace = Workspace::new();
            for output in &outputs {
                workspace.add(OutputId::new(*output));
            }
            let count = outputs.len();
            let doomed = OutputId::new(0);
            let survivor = survivor_is_live.then(|| OutputId::new(1));

            let moved = workspace.reassign(Some(doomed), survivor);

            prop_assert!(workspace.windows_on(doomed).is_empty());
            for id in moved {
                prop_assert_eq!(workspace.output_of(id), survivor);
                prop_assert!(workspace.contains(id));
            }
            prop_assert_eq!(workspace.len(), count);
        }

        #[test]
        fn arbitrary_operations_keep_the_invariants(
            operations in prop::collection::vec(operation(), 0..40),
        ) {
            let (workspace, ids) = run_operations(&operations);
            assert_invariants(&workspace, &ids);
        }

        #[test]
        fn removing_every_window_clears_the_focus(
            operations in prop::collection::vec(operation(), 0..40),
        ) {
            let (mut workspace, mut ids) = run_operations(&operations);

            for id in ids.drain(..) {
                prop_assert!(workspace.remove(id));
            }

            assert_invariants(&workspace, &ids);
            prop_assert!(workspace.is_empty());
            prop_assert_eq!(workspace.focused(), None);
        }

        #[test]
        fn every_composited_window_gets_exactly_one_block(
            operations in prop::collection::vec(operation(), 0..40),
            gap in 0i32..32,
        ) {
            let (workspace, _) = run_operations(&operations);
            let areas = outputs();
            let config = LayoutConfig { gap, ..LayoutConfig::default() };
            let blocks = workspace.arrange(&areas, config);

            let mut seen = Vec::new();
            for (id, _) in &blocks {
                prop_assert!(!seen.contains(id), "{id:?} got more than one block");
                seen.push(*id);
            }

            let expected: Vec<WindowId> = workspace
                .visible_windows()
                .into_iter()
                .filter(|id| workspace.output_of(*id).is_some())
                .collect();
            prop_assert_eq!(seen.len(), expected.len());
            for id in &expected {
                prop_assert!(seen.contains(id));
            }

            // A window that is not composited takes no block at all: no tile on
            // another workspace, none while minimized.
            for id in workspace.windows() {
                if !workspace.is_visible(id) {
                    prop_assert!(!seen.contains(&id));
                }
            }
        }

        #[test]
        fn blocks_follow_the_mode_of_the_window(
            operations in prop::collection::vec(operation(), 0..40),
            gap in 0i32..32,
        ) {
            let (workspace, _) = run_operations(&operations);
            let areas = outputs();
            let config = LayoutConfig { gap, ..LayoutConfig::default() };

            for (id, block) in workspace.arrange(&areas, config) {
                let output = workspace
                    .output_of(id)
                    .and_then(|output| areas.iter().find(|candidate| candidate.id == output))
                    .expect("a composited window has an arranged output");
                match workspace.mode_of(id).expect("a composited window has a mode") {
                    WindowMode::Tiled => prop_assert!(inside(block, output.work_area)),
                    WindowMode::Floating => {
                        let expected = workspace
                            .floating_geometry_of(id)
                            .unwrap_or_else(|| default_floating_area(output.work_area));
                        prop_assert_eq!(block, expected);
                    }
                    WindowMode::Maximized => prop_assert_eq!(block, output.work_area),
                    WindowMode::Fullscreen => prop_assert_eq!(block, output.area),
                }
            }
        }

        #[test]
        fn tiled_blocks_of_one_output_do_not_overlap(
            operations in prop::collection::vec(operation(), 0..40),
            gap in 0i32..32,
        ) {
            let (workspace, _) = run_operations(&operations);
            let areas = outputs();
            let config = LayoutConfig { gap, ..LayoutConfig::default() };

            for output in &areas {
                let blocks: Vec<Rectangle<i32, Logical>> = workspace
                    .arrange(std::slice::from_ref(output), config)
                    .into_iter()
                    .filter(|(id, _)| workspace.mode_of(*id) == Some(WindowMode::Tiled))
                    .map(|(_, block)| block)
                    .collect();

                for (index, block) in blocks.iter().enumerate() {
                    for other in blocks.iter().skip(index + 1) {
                        prop_assert!(!block.overlaps(*other), "{block:?} overlaps {other:?}");
                    }
                }
            }
        }

        #[test]
        fn the_master_column_stays_left_of_the_stack(
            per_output in prop::collection::vec(0usize..5, 4),
            gap in 0i32..32,
            master_count in 0usize..4,
            master_ratio in 0u8..=120,
        ) {
            let mut workspace = Workspace::new();
            for (output, count) in per_output.iter().enumerate() {
                for _ in 0..*count {
                    workspace.add(OutputId::new(output as u64));
                }
            }

            let config = LayoutConfig {
                master_ratio,
                master_count,
                gap,
                ..LayoutConfig::default()
            };
            for output in outputs() {
                let tiles = workspace.arrange(std::slice::from_ref(&output), config);
                let count = tiles.len();
                if count < 2 {
                    continue;
                }
                let master_count = config.sanitised().master_count.min(count);
                if master_count >= count {
                    continue;
                }
                let master_right = tiles[..master_count]
                    .iter()
                    .map(|(_, block)| block.loc.x + block.size.w)
                    .max()
                    .unwrap_or(0);
                for (_, stack) in &tiles[master_count..] {
                    prop_assert!(stack.loc.x >= master_right);
                }
            }
        }
    }
}
