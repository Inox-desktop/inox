//! Managed windows, the output each one belongs to, and the order they tile in.
//!
//! The module is pure state: it never touches Smithay or the Wayland display, so
//! the window bookkeeping (add, remove, focus, output assignment, layout order)
//! is unit testable and property testable independently of the compositor
//! integration. The Smithay layer in [`crate::state`] owns one [`Workspace`] and
//! maps its [`WindowId`]s to real `desktop::Window`s.
//!
//! Every window belongs to exactly one output, which is what makes the multi-output topology's
//! migration rule expressible: when an output goes away its windows are handed to
//! a surviving one rather than left in a coordinate space nothing covers. A
//! window's output is `None` only while no output is left at all; the window stays
//! managed and is tiled again as soon as one appears.

use smithay::utils::{Logical, Rectangle};

use crate::{layout::LayoutConfig, topology::OutputId};

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
}

/// One managed window: its identity and the output it is tiled on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Slot {
    id: WindowId,
    output: Option<OutputId>,
}

/// The ordered set of managed windows plus the focused window.
///
/// The order is the layout order: the window in the first slot is the layout
/// master and every window added later is placed before it, so the most recently
/// added window becomes the master. The order is global; [`Workspace::arrange`]
/// filters it per output, so the relative order of two windows on the same output
/// never depends on what else is mapped.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    windows: Vec<Slot>,
    focused: Option<WindowId>,
    next_id: u64,
}

impl Workspace {
    /// Create an empty workspace.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a window on `output` in the master slot and focus it.
    pub fn add(&mut self, output: OutputId) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);

        self.windows.insert(
            0,
            Slot {
                id,
                output: Some(output),
            },
        );
        self.focused = Some(id);

        id
    }

    /// Remove a window, returning whether it was present.
    ///
    /// When the focused window is removed, focus moves to the window that took
    /// its slot, or to the last window if it was the final one.
    pub fn remove(&mut self, id: WindowId) -> bool {
        let Some(index) = self.windows.iter().position(|slot| slot.id == id) else {
            return false;
        };

        self.windows.remove(index);

        if self.focused == Some(id) {
            self.focused = self
                .windows
                .get(index)
                .or_else(|| self.windows.last())
                .map(|slot| slot.id);
        }

        true
    }

    /// Whether the window is managed by this workspace.
    pub fn contains(&self, id: WindowId) -> bool {
        self.windows.iter().any(|slot| slot.id == id)
    }

    /// Focus a managed window, returning whether it is tracked.
    pub fn focus(&mut self, id: WindowId) -> bool {
        if self.contains(id) {
            self.focused = Some(id);
            true
        } else {
            false
        }
    }

    /// Focus the next window in layout order, wrapping around.
    pub fn focus_next(&mut self) -> Option<WindowId> {
        self.cycle(1)
    }

    /// Focus the previous window in layout order, wrapping around.
    pub fn focus_prev(&mut self) -> Option<WindowId> {
        self.cycle(-1)
    }

    /// The managed windows in layout order, master first.
    ///
    /// Returns identifiers rather than a borrowed slice because the slots now
    /// carry an output as well; callers ask for a window's output separately with
    /// [`Workspace::output_of`].
    pub fn windows(&self) -> Vec<WindowId> {
        self.windows.iter().map(|slot| slot.id).collect()
    }

    /// The focused window, if any.
    pub fn focused(&self) -> Option<WindowId> {
        self.focused
    }

    /// Number of managed windows.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether no window is managed.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// The output a managed window is tiled on, if any.
    pub fn output_of(&self, id: WindowId) -> Option<OutputId> {
        self.windows
            .iter()
            .find(|slot| slot.id == id)
            .and_then(|slot| slot.output)
    }

    /// The windows on `output`, in layout order.
    pub fn windows_on(&self, output: OutputId) -> Vec<WindowId> {
        self.windows
            .iter()
            .filter(|slot| slot.output == Some(output))
            .map(|slot| slot.id)
            .collect()
    }

    /// Move a window to `output`, returning whether it is tracked.
    ///
    /// The layout order is unchanged: a moved window keeps its place among the
    /// windows of its new output, which is what makes a migration after an
    /// unplug look like a rearrangement rather than a reshuffle.
    pub fn assign(&mut self, id: WindowId, output: Option<OutputId>) -> bool {
        match self.windows.iter_mut().find(|slot| slot.id == id) {
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

    /// Compute one tile per managed window that has an output, in layout order.
    ///
    /// `work_areas` is the work area of each output, in the order the outputs
    /// should be tiled. Windows on an output that is not listed — the case where
    /// every display is gone — are skipped and left untouched.
    pub fn arrange(
        &self,
        work_areas: &[(OutputId, Rectangle<i32, Logical>)],
        config: LayoutConfig,
    ) -> Vec<(WindowId, Rectangle<i32, Logical>)> {
        let mut tiles = Vec::with_capacity(self.windows.len());

        for (output, area) in work_areas {
            let windows = self.windows_on(*output);
            tiles.extend(
                config
                    .arrange(*area, windows.len())
                    .into_iter()
                    .zip(windows)
                    .map(|(tile, id)| (id, tile)),
            );
        }

        tiles
    }

    /// Move the focus by `delta` slots in layout order, wrapping around.
    fn cycle(&mut self, delta: isize) -> Option<WindowId> {
        if self.windows.is_empty() {
            self.focused = None;
            return None;
        }

        let current = self
            .focused
            .and_then(|focused| self.windows.iter().position(|slot| slot.id == focused));

        let next = match current {
            Some(index) => {
                (index as isize + delta).rem_euclid(self.windows.len() as isize) as usize
            }
            None => 0,
        };

        let id = self.windows[next].id;
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
        let tiles = workspace.arrange(&[(output(0), area(100, 100))], config);

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
                (output(0), Rectangle::new((0, 0).into(), (800, 600).into())),
                (
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

        let tiles = workspace.arrange(&[(output(0), area(800, 600))], LayoutConfig::default());

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
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
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
                    (
                        OutputId::new(output as u64),
                        Rectangle::<i32, Logical>::new((0, 0).into(), (1280, 720).into()),
                    )
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
                    (
                        OutputId::new(output as u64),
                        Rectangle::<i32, Logical>::new(
                            (output as i32 * 1280, 0).into(),
                            (1280, 720).into(),
                        ),
                    )
                })
                .collect();

            for (id, tile) in workspace.arrange(&areas, config) {
                let output = workspace.output_of(id).expect("a tiled window has an output");
                let area = areas
                    .iter()
                    .find(|(candidate, _)| *candidate == output)
                    .map(|(_, area)| *area)
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
    }
}
