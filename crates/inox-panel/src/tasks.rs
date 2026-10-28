//! The task model: the windows the compositor reports, grouped for the panel.
//!
//! The interaction contract's rule 1 is the rule this module exists to keep: a window is
//! identified by the **server-assigned** handle the compositor gave it, together
//! with the **session generation** the handle belongs to — never by PID, by
//! title, or by its position in somebody's list. An X11 client via XWayland, a
//! client that restarts, and two windows of the same application are therefore
//! all distinguishable, and a window that disappears while a list index shifts
//! cannot be confused with its neighbour.
//!
//! `TaskList` is pure data. It applies the states a window service reports,
//! forgets a window when it goes away, and answers the two questions the panel
//! asks it: *which entry is active* (at most one, derived from which window the
//! compositor reports as focused) and *in what order are the entries drawn*
//! (insertion order, which does not reshuffle when an unrelated window updates —
//! a taskbar whose buttons swap places under the pointer is unusable).
//!
//! Applying a snapshot is explicit, because a generation change invalidates
//! everything the panel believed: [`TaskList::resync`] drops the old view and
//! [`TaskList::apply`] refuses any state that does not belong to the generation
//! the list is currently at, rather than silently mixing two sessions together.

use std::collections::BTreeMap;
use std::fmt;

/// The compositor's handle for one window.
/// Server-assigned: the panel never invents one. Opaque
/// Server-assigned: the panel never invents one. Opaque
/// outside the window service, which is why this is a newtype and not a `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(u64);

impl WindowId {
    /// Wraps a handle the window service assigned.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw handle, for the transport that carries it.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "window{}", self.0)
    }
}

/// The generation of a compositor session.
///
/// Every window handle is only meaningful within the generation that issued it:
/// a compositor restart re-uses handles for different windows, so a state that
/// arrives from another generation is a stale event, not an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SessionGeneration(u64);

impl SessionGeneration {
    /// The generation a session reports.
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw generation.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SessionGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "generation{}", self.0)
    }
}

/// A workspace the compositor keeps windows in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct WorkspaceId(u32);

// The window service reports workspaces through `workspace_state`, and the
// panel's indicator and `activate`/`move_to_workspace` requests carry them. The
// newtype keeps a window handle and a workspace number from being confused; a
// bare `u32` would let either be passed where the other is meant.
impl WorkspaceId {
    /// The workspace a compositor reports.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw workspace number.
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// How the compositor is showing a window.
///
/// The protocol's `window_mode`, one value per window and never a set of flags:
/// maximizing a tiled window replaces tiling, and returning to [`WindowMode::Tiled`]
/// puts the window back into the tiling layout (ADR 0045 §2). Whether the window
/// is *minimized* is a separate field on [`WindowState`], because minimizing
/// keeps the mode: a window that is put away and then restored comes back to the
/// mode it had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum WindowMode {
    /// Placed by the tiling layout.
    #[default]
    Tiled,
    /// Placed freely, outside the tiling layout.
    Floating,
    /// Filling the output's work area.
    Maximized,
    /// Filling the whole output, above the shell's surfaces.
    Fullscreen,
}

impl WindowMode {
    /// The mode in the protocol's numbering.
    ///
    /// The panel decodes the wire value with this table rather than a cast, so a
    /// compositor that grew a fifth mode is refused as unknown instead of drawn
    /// as a wrong one; the numbers themselves are frozen by
    /// `inox-protocols`'s metadata test.
    pub const fn from_protocol(raw: u32) -> Option<Self> {
        Some(match raw {
            0 => Self::Tiled,
            1 => Self::Floating,
            2 => Self::Maximized,
            3 => Self::Fullscreen,
            _ => return None,
        })
    }

    /// The mode the protocol numbers.
    pub const fn code(self) -> u32 {
        match self {
            Self::Tiled => 0,
            Self::Floating => 1,
            Self::Maximized => 2,
            Self::Fullscreen => 3,
        }
    }

    /// The next mode when the user cycles through them with the right button.
    ///
    /// The order is the protocol's own (`tiled → floating → maximized →
    /// fullscreen → tiled`), and it is a method here rather than arithmetic in
    /// the update loop so that the cycle is one testable definition.
    pub const fn cycled(self) -> Self {
        match self {
            Self::Tiled => Self::Floating,
            Self::Floating => Self::Maximized,
            Self::Maximized => Self::Fullscreen,
            Self::Fullscreen => Self::Tiled,
        }
    }

    /// The catalog id of this mode's name.
    pub const fn message_id(self) -> &'static str {
        match self {
            Self::Tiled => "panel-task-mode-tiled",
            Self::Floating => "panel-task-mode-floating",
            Self::Maximized => "panel-task-mode-maximized",
            Self::Fullscreen => "panel-task-mode-fullscreen",
        }
    }
}

/// The workspaces the service reports, and which one is shown.
///
/// `workspace_state` is the only event that produces one of these, and the
/// numbers are the service's: `active` is zero-based and always below `count`,
/// and `count` is at least one because a desktop with no workspace has nowhere
/// to put a window. Until the event arrives the panel knows of no workspaces at
/// all and draws no indicator ([`WorkspaceView::Unknown`]) rather than inventing
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkspaceView {
    /// The service has not said how many workspaces there are.
    #[default]
    Unknown,
    /// The service reports `count` workspaces with `active` shown.
    Known {
        /// The workspace currently shown.
        active: WorkspaceId,
        /// How many workspaces exist.
        count: u32,
    },
}

impl WorkspaceView {
    /// Records what the service reported.
    ///
    /// A count of zero is not a desktop the service can be in — the protocol
    /// makes one the floor — so a zero is treated as no report rather than as an
    /// empty workspace list the panel would have to draw.
    pub fn new(active: u32, count: u32) -> Self {
        if count == 0 {
            return Self::Unknown;
        }

        Self::Known {
            active: WorkspaceId::new(active.min(count - 1)),
            count,
        }
    }

    /// How many workspaces there are, or `None` before the service says.
    pub const fn count(self) -> Option<u32> {
        match self {
            Self::Unknown => None,
            Self::Known { count, .. } => Some(count),
        }
    }

    /// The workspace currently shown, or `None` before the service says.
    pub const fn active(self) -> Option<WorkspaceId> {
        match self {
            Self::Unknown => None,
            Self::Known { active, .. } => Some(active),
        }
    }

    /// Whether the indicator has anything to say.
    ///
    /// A single workspace is a real configuration and one where switching does
    /// nothing, so the panel draws no indicator for it: a control that cannot
    /// change anything is not a control.
    pub const fn is_shown(self) -> bool {
        matches!(self, Self::Known { count, .. } if count > 1)
    }

    /// The workspace after `active` when the user clicks the indicator.
    ///
    /// The cycle wraps, so the one click the indicator offers can reach every
    /// workspace the service reports.
    pub const fn next(self) -> Option<WorkspaceId> {
        match self {
            Self::Unknown => None,
            Self::Known { active, count } if count > 1 => {
                Some(WorkspaceId::new((active.get() + 1) % count))
            }
            Self::Known { .. } => None,
        }
    }
}

/// A named output, such as `HDMI-A-1` or `eDP-1`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct OutputId(String);

impl OutputId {
    /// The output's name, as the compositor spells it.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

impl fmt::Display for OutputId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for OutputId {
    fn from(name: &str) -> Self {
        Self(name.to_owned())
    }
}

/// The identity of one window: a server-assigned handle **and** its generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowKey {
    /// Handle the compositor assigned.
    pub window: WindowId,
    /// Session that assigned it.
    pub generation: SessionGeneration,
}

impl WindowKey {
    /// The key of `window` in `generation`.
    pub const fn new(window: WindowId, generation: SessionGeneration) -> Self {
        Self { window, generation }
    }
}

impl fmt::Display for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.window, self.generation)
    }
}

/// Everything the panel knows about one window.
///
/// These are the fields the window service reports; the panel adds nothing of
/// its own. [`WindowState::mode`] is the compositor's statement about how the
/// window is shown, and `maximized` / `fullscreen` are its two projections: they
/// are kept because the panel has always had them (ADR 0045 §4), and
/// [`WindowState::of_mode`] is the one place that sets all three so the three can
/// never disagree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowState {
    /// Application the window belongs to, as the server reports it.
    ///
    /// Grouping is by this string and by nothing else: an empty `app_id` is a
    /// distinct group the panel labels "unknown application", not a reason to
    /// fall back to PIDs.
    pub app_id: String,
    /// Window title.
    pub title: String,
    /// How the compositor is showing the window.
    pub mode: WindowMode,
    /// Whether the window is minimized.
    pub minimized: bool,
    /// Whether the window is maximized.
    ///
    /// Derived from [`WindowState::mode`]; see [`WindowState::of_mode`].
    pub maximized: bool,
    /// Whether the window is fullscreen.
    ///
    /// Derived from [`WindowState::mode`]; see [`WindowState::of_mode`].
    pub fullscreen: bool,
    /// Workspace the window lives on.
    pub workspace: WorkspaceId,
    /// Output the window is on.
    pub output: OutputId,
    /// Whether this is the window the compositor reports as focused.
    pub focused: bool,
}

impl WindowState {
    /// The state of a window shown in `mode`.
    ///
    /// The one constructor the protocol leg uses, because it is the one place
    /// that keeps `mode`, `maximized` and `fullscreen` in agreement: a state
    /// built field by field could say `mode: Maximized` and `maximized: false`,
    /// and two representations of the same fact would drift.
    pub fn of_mode(mode: WindowMode) -> Self {
        Self {
            mode,
            maximized: mode == WindowMode::Maximized,
            fullscreen: mode == WindowMode::Fullscreen,
            ..Self::default()
        }
    }
}

/// What [`TaskList::apply`] did with a state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// The window was new to the list and took the last position.
    Added,
    /// The window was already known; its state was replaced in place.
    Updated,
    /// The state belongs to another session generation and was dropped.
    Stale,
}

/// One application's windows, in the order the panel draws them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGroup {
    /// The `app_id` every window in this group reports.
    pub app_id: String,
    /// The group's windows, in stable order.
    pub windows: Vec<WindowKey>,
}

impl TaskGroup {
    /// How many windows the group holds.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether the group holds no window.
    ///
    /// A group with no window is never stored — the last window leaving removes
    /// the group — so this is only ever true for a value built by hand.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// The window of this group the compositor reports as focused, if any.
    pub fn active_window(&self, list: &TaskList) -> Option<WindowKey> {
        self.windows
            .iter()
            .copied()
            .find(|key| list.is_focused(*key))
    }

    /// Whether one of the group's windows is focused.
    pub fn is_active(&self, list: &TaskList) -> bool {
        self.active_window(list).is_some()
    }

    /// Whether every window of the group is minimized.
    ///
    /// A group counts as minimized only when nothing of it is on screen, which
    /// is what makes a minimized group's button safe to style as inactive.
    pub fn is_minimized(&self, list: &TaskList) -> bool {
        !self.windows.is_empty()
            && self.windows.iter().all(|key| {
                list.window(*key)
                    .map(|state| state.minimized)
                    .unwrap_or(false)
            })
    }

    /// The title the group's button shows: the focused window's, else the first.
    pub fn title<'a>(&self, list: &'a TaskList) -> &'a str {
        let key = self
            .active_window(list)
            .or_else(|| self.windows.first().copied());

        key.and_then(|key| list.window(key))
            .map(|state| state.title.as_str())
            .unwrap_or_default()
    }
}

/// The windows of one session, in the order the panel draws them.
#[derive(Debug, Clone)]
pub struct TaskList {
    generation: SessionGeneration,
    windows: BTreeMap<WindowKey, WindowState>,
    /// Every key of `windows`, in insertion order. The map is only ever reached
    /// through this vector, which is what keeps the ordering stable.
    order: Vec<WindowKey>,
}

impl Default for TaskList {
    fn default() -> Self {
        Self::new(SessionGeneration::default())
    }
}

impl TaskList {
    /// An empty list for `generation`.
    pub fn new(generation: SessionGeneration) -> Self {
        Self {
            generation,
            windows: BTreeMap::new(),
            order: Vec::new(),
        }
    }

    /// The session generation the list describes.
    pub const fn generation(&self) -> SessionGeneration {
        self.generation
    }

    /// How many windows the list holds.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether the list holds no window.
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// The state of `key`, if the list holds it.
    pub fn window(&self, key: WindowKey) -> Option<&WindowState> {
        self.windows.get(&key)
    }

    /// Whether the list holds `key`.
    pub fn contains(&self, key: WindowKey) -> bool {
        self.windows.contains_key(&key)
    }

    /// Every window, in the order the panel draws them.
    pub fn windows(&self) -> impl Iterator<Item = (WindowKey, &WindowState)> + '_ {
        self.order
            .iter()
            .filter_map(|key| self.windows.get(key).map(|state| (*key, state)))
    }

    /// The window the compositor reports as focused.
    ///
    /// At most one, and `None` when focus is nowhere — a destroyed focused
    /// window leaves the panel with no active entry rather than a stale one.
    pub fn active(&self) -> Option<WindowKey> {
        self.windows()
            .find(|(_, state)| state.focused)
            .map(|(key, _)| key)
    }

    /// Whether `key` is the focused window.
    pub fn is_focused(&self, key: WindowKey) -> bool {
        self.window(key).map(|state| state.focused).unwrap_or(false)
    }

    /// Every application's windows, grouped by the server-provided `app_id`.
    ///
    /// Groups appear in the order their first window was added, so a group does
    /// not jump when another window of it updates or closes.
    pub fn groups(&self) -> Vec<TaskGroup> {
        let mut groups: Vec<TaskGroup> = Vec::new();
        let mut index: BTreeMap<&str, usize> = BTreeMap::new();

        for (key, state) in self.windows() {
            match index.get(state.app_id.as_str()) {
                Some(position) => groups[*position].windows.push(key),
                None => {
                    index.insert(state.app_id.as_str(), groups.len());
                    groups.push(TaskGroup {
                        app_id: state.app_id.clone(),
                        windows: vec![key],
                    });
                }
            }
        }

        groups
    }

    /// Applies a window state the service reported.
    ///
    /// A new window takes the last position; a known one is replaced in place,
    /// so an unrelated window updating never reorders the panel. A state whose
    /// generation is not the list's is dropped as stale (see [`TaskList::resync`]
    /// for the generation change itself).
    pub fn apply(&mut self, key: WindowKey, state: WindowState) -> Applied {
        if key.generation != self.generation {
            return Applied::Stale;
        }

        // the interaction contract, rule 1: focus has exactly one owner. The service says
        // which window is focused, and the panel keeps that true even if a
        // service sends two focused windows in a row without an unfocus.
        if state.focused {
            for (other_key, other) in &mut self.windows {
                if *other_key != key && other.focused {
                    other.focused = false;
                }
            }
        }

        match self.windows.insert(key, state) {
            None => {
                self.order.push(key);
                Applied::Added
            }
            Some(_) => Applied::Updated,
        }
    }

    /// Forgets a window that went away, returning what was known about it.
    ///
    /// Exiting an application removes its windows and, with the last of them,
    /// the group: there is no empty group to draw and no entry to activate.
    pub fn remove(&mut self, key: WindowKey) -> Option<WindowState> {
        let removed = self.windows.remove(&key)?;
        self.order.retain(|candidate| *candidate != key);
        Some(removed)
    }

    /// Forgets every window and moves the list to `generation`.
    ///
    /// Returns how many windows were discarded, which is what a caller logs when
    /// a compositor restart invalidated its view.
    pub fn resync(&mut self, generation: SessionGeneration) -> usize {
        let discarded = self.windows.len();
        self.windows.clear();
        self.order.clear();
        self.generation = generation;
        discarded
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn window(key: u64, app: &str, title: &str) -> (WindowKey, WindowState) {
        let key = WindowKey::new(WindowId::new(key), SessionGeneration::new(1));
        let state = WindowState {
            app_id: app.to_owned(),
            title: title.to_owned(),
            ..WindowState::default()
        };

        (key, state)
    }

    #[test]
    fn a_new_window_takes_the_last_position() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (first, first_state) = window(1, "a", "one");
        let (second, second_state) = window(2, "b", "two");

        assert_eq!(list.apply(first, first_state), Applied::Added);
        assert_eq!(list.apply(second, second_state), Applied::Added);

        let order: Vec<_> = list.windows().map(|(key, _)| key.window.get()).collect();
        assert_eq!(order, vec![1, 2]);
    }

    #[test]
    fn updating_a_window_keeps_its_position() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (first, first_state) = window(1, "a", "one");
        let (second, second_state) = window(2, "b", "two");
        let (third, third_state) = window(3, "c", "three");
        list.apply(first, first_state);
        list.apply(second, second_state);
        list.apply(third, third_state);

        let (_, mut retitled) = window(2, "b", "two, renamed");
        retitled.focused = true;

        assert_eq!(list.apply(second, retitled), Applied::Updated);
        assert_eq!(list.len(), 3);

        let order: Vec<_> = list.windows().map(|(key, _)| key.window.get()).collect();
        assert_eq!(order, vec![1, 2, 3]);
        assert_eq!(
            list.window(second).map(|s| s.title.as_str()),
            Some("two, renamed")
        );
        assert_eq!(list.active(), Some(second));
    }

    #[test]
    fn an_update_from_another_generation_is_stale() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let stale = WindowKey::new(WindowId::new(1), SessionGeneration::new(2));
        let state = WindowState::default();

        assert_eq!(list.apply(stale, state), Applied::Stale);
        assert!(list.is_empty());
    }

    #[test]
    fn a_second_focus_unfocuses_the_first() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (first, mut first_state) = window(1, "a", "one");
        first_state.focused = true;
        let (second, mut second_state) = window(2, "b", "two");
        second_state.focused = true;

        list.apply(first, first_state);
        list.apply(second, second_state);

        assert_eq!(list.active(), Some(second));
        assert!(!list.is_focused(first));
        assert_eq!(
            list.windows().filter(|(_, s)| s.focused).count(),
            1,
            "at most one window is focused"
        );
    }

    #[test]
    fn removing_the_focused_window_leaves_no_active_entry() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (key, mut state) = window(1, "a", "one");
        state.focused = true;
        list.apply(key, state);

        assert_eq!(
            list.remove(key).map(|state| state.title),
            Some("one".to_owned())
        );
        assert_eq!(list.active(), None);
        assert!(list.is_empty());
        assert_eq!(list.remove(key), None, "removing twice is not an error");
    }

    #[test]
    fn a_group_disappears_with_its_last_window() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (first, first_state) = window(1, "terminal", "term");
        let (second, second_state) = window(2, "terminal", "term 2");
        list.apply(first, first_state);
        list.apply(second, second_state);

        assert_eq!(list.groups().len(), 1);
        assert_eq!(list.groups()[0].len(), 2);

        list.remove(first);
        assert_eq!(list.groups().len(), 1, "one window left keeps the group");

        list.remove(second);
        assert!(list.groups().is_empty(), "the last window takes the group");
    }

    #[test]
    fn groups_keep_the_order_of_their_first_window() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        for (id, app, title) in [(1, "editor", "a"), (2, "terminal", "b"), (3, "editor", "c")] {
            let (key, state) = window(id, app, title);
            list.apply(key, state);
        }

        let apps: Vec<_> = list.groups().into_iter().map(|g| g.app_id).collect();
        assert_eq!(apps, vec!["editor", "terminal"]);
        assert_eq!(list.groups()[0].len(), 2);
    }

    #[test]
    fn an_empty_app_id_is_its_own_group() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (first, first_state) = window(1, "", "one");
        let (second, second_state) = window(2, "", "two");
        list.apply(first, first_state);
        list.apply(second, second_state);

        let groups = list.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].app_id, "");
    }

    #[test]
    fn a_group_reports_its_active_and_minimized_windows() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (first, mut first_state) = window(1, "app", "one");
        first_state.minimized = true;
        let (second, mut second_state) = window(2, "app", "two");
        second_state.minimized = true;
        second_state.focused = true;
        list.apply(first, first_state);
        list.apply(second, second_state);

        let group = &list.groups()[0];
        assert!(group.is_active(&list));
        assert!(group.is_minimized(&list));
        assert_eq!(group.active_window(&list), Some(second));
        assert_eq!(group.title(&list), "two");
    }

    #[test]
    fn a_group_with_one_window_on_screen_is_not_minimized() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (first, mut first_state) = window(1, "app", "one");
        first_state.minimized = true;
        let (second, second_state) = window(2, "app", "two");
        list.apply(first, first_state);
        list.apply(second, second_state);

        assert!(!list.groups()[0].is_minimized(&list));
    }

    #[test]
    fn resync_discards_the_old_view() {
        let mut list = TaskList::new(SessionGeneration::new(1));
        let (key, state) = window(1, "a", "one");
        list.apply(key, state);

        assert_eq!(list.resync(SessionGeneration::new(2)), 1);
        assert!(list.is_empty());
        assert_eq!(list.generation(), SessionGeneration::new(2));

        // The handle from the old generation cannot come back into the new one.
        assert_eq!(list.apply(key, WindowState::default()), Applied::Stale);
    }

    #[test]
    fn an_empty_list_has_no_active_window() {
        let list = TaskList::new(SessionGeneration::new(1));
        assert_eq!(list.active(), None);
        assert!(list.groups().is_empty());
        assert!(list.windows().next().is_none());
    }

    #[test]
    fn the_mode_cycle_is_the_protocol_order_and_wraps() {
        // The right button cycles through the four modes the protocol defines.
        // The order is the compositor's, not the panel's: tiled is where a
        // window starts and where it returns.
        assert_eq!(WindowMode::Tiled.cycled(), WindowMode::Floating);
        assert_eq!(WindowMode::Floating.cycled(), WindowMode::Maximized);
        assert_eq!(WindowMode::Maximized.cycled(), WindowMode::Fullscreen);
        assert_eq!(WindowMode::Fullscreen.cycled(), WindowMode::Tiled);

        let mut mode = WindowMode::Tiled;
        for _ in 0..4 {
            mode = mode.cycled();
        }
        assert_eq!(mode, WindowMode::Tiled, "four steps return to the start");
    }

    #[test]
    fn the_mode_numbers_are_the_protocols() {
        // The numbers are frozen by `inox-protocols`'s metadata test; the panel
        // decodes them with the same table, so a silent renumbering is caught on
        // either side.
        for (mode, code) in [
            (WindowMode::Tiled, 0),
            (WindowMode::Floating, 1),
            (WindowMode::Maximized, 2),
            (WindowMode::Fullscreen, 3),
        ] {
            assert_eq!(mode.code(), code);
            assert_eq!(WindowMode::from_protocol(code), Some(mode));
        }

        assert_eq!(
            WindowMode::from_protocol(4),
            None,
            "a mode this build does not know is unknown, not a wrong one"
        );
        assert_eq!(WindowState::default().mode, WindowMode::Tiled);
    }

    #[test]
    fn a_state_built_from_a_mode_keeps_its_two_projections_in_agreement() {
        for mode in [
            WindowMode::Tiled,
            WindowMode::Floating,
            WindowMode::Maximized,
            WindowMode::Fullscreen,
        ] {
            let state = WindowState::of_mode(mode);

            assert_eq!(state.mode, mode);
            assert_eq!(state.maximized, mode == WindowMode::Maximized);
            assert_eq!(state.fullscreen, mode == WindowMode::Fullscreen);
        }

        let mut floating = WindowState::of_mode(WindowMode::Floating);
        floating.minimized = true;
        assert_eq!(
            floating.mode,
            WindowMode::Floating,
            "minimizing is orthogonal to the mode: a put-away window keeps it"
        );
    }

    #[test]
    fn a_workspace_report_before_any_report_is_unknown() {
        let view = WorkspaceView::default();

        assert_eq!(view, WorkspaceView::Unknown);
        assert_eq!(view.count(), None);
        assert_eq!(view.active(), None);
        assert!(!view.is_shown(), "nothing is drawn before the service says");
        assert_eq!(view.next(), None);
    }

    #[test]
    fn one_workspace_is_a_report_but_has_no_indicator() {
        let view = WorkspaceView::new(0, 1);

        assert_eq!(view.active(), Some(WorkspaceId::new(0)));
        assert_eq!(view.count(), Some(1));
        assert!(
            !view.is_shown(),
            "a control that cannot change anything is not a control"
        );
        assert_eq!(view.next(), None, "and so nothing to switch to");
    }

    #[test]
    fn the_indicator_cycles_through_every_workspace() {
        let view = WorkspaceView::new(0, 3);
        assert!(view.is_shown());
        assert_eq!(view.next(), Some(WorkspaceId::new(1)));

        let view = WorkspaceView::new(2, 3);
        assert_eq!(
            view.next(),
            Some(WorkspaceId::new(0)),
            "the last workspace wraps to the first"
        );
    }

    #[test]
    fn a_report_outside_its_own_bounds_is_clamped_rather_than_drawn() {
        let view = WorkspaceView::new(7, 3);
        assert_eq!(
            view.active(),
            Some(WorkspaceId::new(2)),
            "a service cannot show a workspace it does not have"
        );

        assert_eq!(
            WorkspaceView::new(0, 0),
            WorkspaceView::Unknown,
            "a desktop with no workspace has nowhere to put a window, so a zero \
             count is no report rather than an empty list"
        );
    }

    /// Every invariant the panel relies on, checked after every step of a
    /// randomly generated sequence of service events.
    #[derive(Debug, Clone)]
    enum Op {
        Apply {
            window: u8,
            app: u8,
            focused: bool,
            minimized: bool,
        },
        Remove {
            window: u8,
        },
        Resync {
            generation: u8,
        },
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            6 => (0u8..6, 0u8..3, any::<bool>(), any::<bool>()).prop_map(
                |(window, app, focused, minimized)| Op::Apply { window, app, focused, minimized }
            ),
            2 => (0u8..6).prop_map(|window| Op::Remove { window }),
            1 => (0u8..3).prop_map(|generation| Op::Resync { generation }),
        ]
    }

    fn check(list: &TaskList) {
        // The order vector and the map describe the same set, without duplicates.
        assert_eq!(list.order.len(), list.windows.len());
        let mut sorted = list.order.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), list.order.len(), "no window appears twice");
        for key in &list.order {
            assert!(list.windows.contains_key(key), "{key} is in the order only");
        }

        // Focus has one owner at most.
        assert!(
            list.windows().filter(|(_, state)| state.focused).count() <= 1,
            "at most one window is focused"
        );
        assert_eq!(
            list.active(),
            list.windows()
                .find(|(_, state)| state.focused)
                .map(|(key, _)| key)
        );

        // The groups partition the windows.
        let groups = list.groups();
        let mut seen: Vec<WindowKey> = groups.iter().flat_map(|g| g.windows.clone()).collect();
        assert_eq!(
            seen.len(),
            list.len(),
            "every window is in exactly one group"
        );
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), list.len(), "no window is in two groups");
        for group in &groups {
            assert!(!group.is_empty(), "a stored group always has a window");
            for key in &group.windows {
                assert_eq!(
                    list.window(*key).map(|state| state.app_id.as_str()),
                    Some(group.app_id.as_str()),
                    "a group only holds windows of its own application"
                );
            }
        }

        // The group order follows the first window of each group.
        let mut first_seen: Vec<String> = Vec::new();
        for (_, state) in list.windows() {
            if !first_seen.iter().any(|app| app == &state.app_id) {
                first_seen.push(state.app_id.clone());
            }
        }
        assert_eq!(
            groups.iter().map(|g| g.app_id.clone()).collect::<Vec<_>>(),
            first_seen,
            "groups appear in the order of their first window"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn the_model_keeps_its_invariants(ops in prop::collection::vec(op(), 0..40)) {
            let mut list = TaskList::new(SessionGeneration::new(1));
            let mut order: Vec<WindowKey> = Vec::new();

            for op in ops {
                match op {
                    Op::Apply { window, app, focused, minimized } => {
                        let key = WindowKey::new(
                            WindowId::new(u64::from(window)),
                            list.generation(),
                        );
                        let state = WindowState {
                            app_id: format!("app{app}"),
                            title: format!("window {window}"),
                            minimized,
                            focused,
                            ..WindowState::default()
                        };
                        list.apply(key, state);
                        if !order.contains(&key) {
                            order.push(key);
                        }
                    }
                    Op::Remove { window } => {
                        // Remove from either generation, so stale removals are
                        // exercised too.
                        let key = WindowKey::new(
                            WindowId::new(u64::from(window)),
                            list.generation(),
                        );
                        list.remove(key);
                        order.retain(|candidate| *candidate != key);
                    }
                    Op::Resync { generation } => {
                        list.resync(SessionGeneration::new(u64::from(generation)));
                        order.clear();
                    }
                }

                prop_assert_eq!(list.order.clone(), order.clone());
                check(&list);
            }
        }

        /// Applying a state to a known window never moves any other window.
        #[test]
        fn unrelated_updates_do_not_reorder(
            windows in prop::collection::vec((0u64..8, 0u8..3), 0..12),
            target in 0usize..12,
        ) {
            let generation = SessionGeneration::new(1);
            let mut list = TaskList::new(generation);
            for (id, app) in &windows {
                let key = WindowKey::new(WindowId::new(*id), generation);
                let state = WindowState {
                    app_id: format!("app{app}"),
                    title: format!("window {id}"),
                    ..WindowState::default()
                };
                list.apply(key, state);
            }

            let before: Vec<WindowKey> = list.windows().map(|(key, _)| key).collect();
            if let Some((id, _)) = windows.get(target % windows.len().max(1)) {
                let key = WindowKey::new(WindowId::new(*id), generation);
                if list.contains(key) {
                    let mut state = list.window(key).cloned().unwrap_or_default();
                    state.title.push_str(" (renamed)");
                    prop_assert_eq!(list.apply(key, state), Applied::Updated);
                }
            }

            let after: Vec<WindowKey> = list.windows().map(|(key, _)| key).collect();
            prop_assert_eq!(before, after);
        }
    }
}
