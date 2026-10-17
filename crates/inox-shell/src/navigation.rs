//! Cross-entry navigation: the rules of ADR 0013 as a pure state machine.
//!
//! The shell coordinator draws nothing. What it owns is the rule that decides
//! where the keyboard goes when one of the session's entries opens or closes,
//! and ADR 0013 states it in one sentence: an entry saves a valid return target
//! when it opens, `Esc` closes exactly one layer at a time and returns, and a
//! window that no longer exists is never the target of a focus change — the
//! coordinator falls back to the compositor's focus policy instead of
//! dispatching to an invalid object.
//!
//! Nothing here performs I/O, touches an event loop or knows about Iced. The
//! component that owns the connection asks for the focus change; this module
//! only says *what* should happen, which is what makes the awkward cases
//! testable: a window dying while its menu is open, or a session restarting
//! underneath a launcher.
//!
//! Window identity is a compositor-assigned [`WindowId`] together with the
//! [`SessionGeneration`] it was assigned in — never a PID, a title or an index
//! in whatever list the caller happens to hold. All three of those can be reused
//! for a different window, and a return target that points at the wrong window
//! is worse than no return target at all (ADR 0013, decision 1).

use std::fmt;

use crate::diagnostic::Diagnostic;
use crate::i18n::id;

/// One of the entries the shell coordinates.
///
/// The four names are the entries a session knows about (ADR 0013): the panel,
/// the launcher, the notification daemon and the settings application. The
/// wallpaper is not an entry — nothing opens or closes it — and a general
/// third-party plugin platform is out of scope pending its own ADR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryId {
    /// The panel: task list, clock and tray.
    Panel,
    /// The application launcher.
    Launcher,
    /// The notification daemon's popups.
    Notifications,
    /// The settings application.
    Settings,
}

impl EntryId {
    /// Every entry, in the order the registry lists them.
    pub const ALL: [EntryId; 4] = [
        EntryId::Panel,
        EntryId::Launcher,
        EntryId::Notifications,
        EntryId::Settings,
    ];

    /// The name this entry is configured, logged and diagnosed under.
    pub const fn as_str(self) -> &'static str {
        match self {
            EntryId::Panel => "panel",
            EntryId::Launcher => "launcher",
            EntryId::Notifications => "notifications",
            EntryId::Settings => "settings",
        }
    }

    /// The entry called `name`, or `None` for a name no entry has.
    ///
    /// Names are what a registry entry, a configuration file or a diagnostic
    /// carries; nothing outside this crate may guess that one is valid.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|entry| entry.as_str() == name)
    }

    /// Every name, separated by `, `, for a diagnostic that has to list them.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|entry| entry.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A window, identified the way the compositor identifies it.
///
/// The value is a server-assigned id, not a handle to anything this process
/// owns: it is only ever compared and reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(u32);

impl WindowId {
    /// Wraps an id the compositor assigned.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The id itself.
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for WindowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The generation of the session a window belongs to.
///
/// A generation changes when the session is rebuilt (the compositor restarted,
/// or the session restarted its components). Window ids are only meaningful
/// within the generation that assigned them, so a return target recorded before
/// a change is stale even if the numeric id matches a live window afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionGeneration(u64);

impl SessionGeneration {
    /// Wraps a generation the session reported.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The generation itself.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SessionGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Where keyboard focus must go when a layer closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReturnTarget {
    /// The window that was focused when the entry opened.
    pub window: WindowId,
    /// The session generation `window` was assigned in.
    pub generation: SessionGeneration,
}

impl ReturnTarget {
    /// A return target for `window` in `generation`.
    pub const fn new(window: WindowId, generation: SessionGeneration) -> Self {
        Self { window, generation }
    }
}

impl fmt::Display for ReturnTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "window {} of {}", self.window, self.generation)
    }
}

/// Why focus could not return to the window a layer was opened from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FallbackReason {
    /// The window the layer was opened from has been destroyed.
    WindowDestroyed {
        /// The window that is gone.
        window: WindowId,
    },
    /// The layer was opened in a session generation that has since ended.
    ///
    /// The numeric window id is not reused across generations, so the saved
    /// target must not be dispatched to even if a window with that id exists.
    SessionChanged {
        /// The generation the layer recorded.
        saved: SessionGeneration,
        /// The generation the session is in now.
        current: SessionGeneration,
    },
}

impl FallbackReason {
    /// A user-readable explanation (ADR 0013: fall back to the compositor's
    /// focus policy rather than delivering to an invalid object).
    pub fn describe(&self) -> Diagnostic {
        match *self {
            FallbackReason::WindowDestroyed { window } => {
                Diagnostic::new(id::NAV_FALLBACK_WINDOW_DESTROYED)
                    .with("window", window.to_string())
            }
            FallbackReason::SessionChanged { saved, current } => {
                Diagnostic::new(id::NAV_FALLBACK_SESSION_CHANGED)
                    .with("saved", saved.to_string())
                    .with("current", current.to_string())
            }
        }
    }
}

/// Whether opening an entry takes the keyboard away from the focused window.
///
/// D01 separates ordinary display from explicit navigation: clicking something
/// on the panel must not steal focus, while entering an entry the keyboard is
/// meant for gives it exactly one focus. Which of the two an open is belongs to
/// the caller, because the panel itself is both — a click displays it, a
/// keyboard shortcut navigates into it.
///
/// Both are asked for through the same call, so the second one is not lost when
/// the entry is already on the stack: an [`EntryFocus::Keyboard`] request for an
/// entry that is only being displayed moves the keyboard into the layer that is
/// already there ([`NavigationStack::open`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryFocus {
    /// The entry is shown without taking keyboard focus.
    Display,
    /// The entry takes keyboard focus while it is open.
    Keyboard,
}

/// One entry, as it is being opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entry {
    /// Which entry this is.
    pub id: EntryId,
    /// Whether opening it takes the keyboard.
    pub focus: EntryFocus,
}

impl Entry {
    /// An entry that behaves as `focus` says when it opens.
    pub const fn new(id: EntryId, focus: EntryFocus) -> Self {
        Self { id, focus }
    }
}

/// What a navigation request did.
///
/// The three closing outcomes are deliberately distinct: ADR 0013 requires the
/// coordinator to fall back to the compositor's focus policy when the window it
/// saved is gone, and an outcome that cannot tell "nothing was open" from "the
/// target died" would let a stale window be focused again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavigationOutcome {
    /// The entry is open and the keyboard went where it says it went.
    ///
    /// Either the entry was pushed as the new innermost layer, or it was
    /// already on the stack as a display layer and this request navigated the
    /// keyboard into it; [`NavigationStack::open`] documents which.
    Opened {
        /// The entry the request named.
        entry: EntryId,
        /// The layer that holds the keyboard now, if any.
        ///
        /// `None` when the stack holds no keyboard-interactive layer: the
        /// focused window keeps the keyboard, which is what D01 asks of an
        /// ordinary display.
        focus: Option<EntryId>,
    },
    /// The entry was already open; the stack is unchanged.
    ///
    /// Opening the same entry twice never leaves two indistinguishable layers
    /// (ADR 0013: the coordinator keeps one layer per entry).
    AlreadyOpen {
        /// The entry that was already open.
        entry: EntryId,
    },
    /// One layer was closed and focus returns to the saved window.
    ///
    /// `returned_to` is `None` when the popped layer saved no target, or when
    /// another layer is still open and the shell keeps the keyboard; in both
    /// cases the focused window does not change.
    Closed {
        /// The window focus should return to, when there is one.
        returned_to: Option<ReturnTarget>,
    },
    /// One layer was closed, but its saved target can no longer be used.
    ReturnedToFallback {
        /// Why the compositor's own focus policy has to decide instead.
        reason: FallbackReason,
    },
    /// Nothing was open, so there was no layer to close.
    NothingToClose,
}

impl NavigationOutcome {
    /// A user-readable explanation, for the coordinator's log.
    pub fn describe(&self) -> Diagnostic {
        match self {
            NavigationOutcome::Opened { entry, focus } => match focus {
                Some(focus) => Diagnostic::new(id::NAV_OPENED)
                    .with("entry", entry.as_str())
                    .with("focus", focus.as_str()),
                None => Diagnostic::new(id::NAV_OPENED_DISPLAY).with("entry", entry.as_str()),
            },
            NavigationOutcome::AlreadyOpen { entry } => {
                Diagnostic::new(id::NAV_ALREADY_OPEN).with("entry", entry.as_str())
            }
            NavigationOutcome::Closed { returned_to } => match returned_to {
                Some(target) => {
                    Diagnostic::new(id::NAV_CLOSED).with("window", target.window.to_string())
                }
                None => Diagnostic::new(id::NAV_CLOSED_NO_WINDOW),
            },
            NavigationOutcome::ReturnedToFallback { reason } => {
                Diagnostic::new(id::NAV_RETURNED_TO_FALLBACK)
                    .with_message("reason", reason.describe())
            }
            NavigationOutcome::NothingToClose => Diagnostic::new(id::NAV_NOTHING_TO_CLOSE),
        }
    }
}

/// What a layer remembers about the window it was opened from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SavedTarget {
    /// The entry opened with no focused window to return to.
    Absent,
    /// The saved window, believed usable.
    Live(ReturnTarget),
    /// The saved window, known not to be usable any more.
    Invalid {
        target: ReturnTarget,
        reason: FallbackReason,
    },
}

/// One open entry and its return target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layer {
    entry: Entry,
    target: SavedTarget,
}

/// The stack of entries a session has open, outermost first.
///
/// The stack belongs to the coordinator, not to the entries: the entries are
/// separate processes that draw themselves, and this is the rule they are all
/// held to. `Esc` pops one layer, no more, which is why every closing operation
/// here pops a single layer and reports exactly what the coordinator should do
/// about the keyboard next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationStack {
    layers: Vec<Layer>,
    generation: SessionGeneration,
}

impl NavigationStack {
    /// An empty stack in the session generation `generation`.
    pub fn new(generation: SessionGeneration) -> Self {
        Self {
            layers: Vec::new(),
            generation,
        }
    }

    /// Opens `entry`, remembering `return_target` as where focus goes back to.
    ///
    /// `return_target` is the window the compositor had focused, or `None` when
    /// it had none. A target recorded in an earlier session generation is
    /// remembered as unusable rather than trusted, so closing the layer reports
    /// the fallback instead of dispatching to a window of the previous session.
    ///
    /// An entry that is already open is not pushed a second time: the stack
    /// keeps one layer per entry, and the target it saved when it was first
    /// opened stays in force, because this call does not reopen the entry.
    /// Whether the request is a display or a keyboard navigation still decides,
    /// though, because the panel is both: a [`EntryFocus::Display`] request for
    /// a displayed layer, and any repeat of the policy a layer already has,
    /// change nothing and report [`NavigationOutcome::AlreadyOpen`], while an
    /// [`EntryFocus::Keyboard`] request for a layer that is only being displayed
    /// moves the keyboard into that layer and reports it as
    /// [`NavigationOutcome::Opened`], which is what D01 asks of an explicit
    /// navigation: exactly one focus. The layer keeps the place it was opened
    /// in, so a keyboard layer above it still holds the keyboard until it
    /// closes.
    pub fn open(&mut self, entry: Entry, return_target: Option<ReturnTarget>) -> NavigationOutcome {
        if let Some(layer) = self
            .layers
            .iter_mut()
            .find(|layer| layer.entry.id == entry.id)
        {
            if entry.focus == EntryFocus::Keyboard && layer.entry.focus == EntryFocus::Display {
                layer.entry.focus = EntryFocus::Keyboard;

                return NavigationOutcome::Opened {
                    entry: entry.id,
                    focus: self.focus(),
                };
            }

            return NavigationOutcome::AlreadyOpen { entry: entry.id };
        }

        self.layers.push(Layer {
            entry,
            target: self.classify(return_target),
        });

        NavigationOutcome::Opened {
            entry: entry.id,
            focus: self.focus(),
        }
    }

    /// Closes the innermost layer: exactly one, which is what `Esc` does.
    ///
    /// Focus can only leave the shell when no layer is left, so `returned_to`
    /// is filled in when this close emptied the stack; while a layer remains,
    /// the stack still holds the keyboard and the outcome reports that no window
    /// focus changes. A layer whose saved window is gone reports
    /// [`NavigationOutcome::ReturnedToFallback`] instead: ADR 0013 falls back to
    /// the compositor's focus policy rather than delivering to an object that no
    /// longer exists.
    pub fn close_top(&mut self) -> NavigationOutcome {
        let Some(layer) = self.layers.pop() else {
            return NavigationOutcome::NothingToClose;
        };

        if self.layers.is_empty() {
            resolve(layer.target)
        } else {
            // Another layer is still open, so the shell keeps the keyboard and
            // nothing about the focused window changes.
            NavigationOutcome::Closed { returned_to: None }
        }
    }

    /// Closes every layer at once (a session teardown, or leaving the desktop).
    ///
    /// Focus returns to where the navigation started: the target of the
    /// outermost layer, which is the one that was opened first. An empty stack
    /// is [`NavigationOutcome::NothingToClose`], as for [`NavigationStack::close_top`].
    pub fn close_all(&mut self) -> NavigationOutcome {
        let Some(outermost) = self.layers.first().copied() else {
            return NavigationOutcome::NothingToClose;
        };
        self.layers.clear();

        resolve(outermost.target)
    }

    /// Records that `window` has been destroyed and drops every saved target
    /// that pointed at it.
    ///
    /// Returns how many targets were invalidated. This is the compositor's
    /// window-destroyed event: after it, no close may yield `window` as a return
    /// target, whether the layer is closed now or later.
    pub fn target_destroyed(&mut self, window: WindowId) -> usize {
        let mut invalidated = 0;
        for layer in &mut self.layers {
            if let SavedTarget::Live(target) = layer.target
                && target.window == window
            {
                layer.target = SavedTarget::Invalid {
                    target,
                    reason: FallbackReason::WindowDestroyed { window },
                };
                invalidated += 1;
            }
        }
        invalidated
    }

    /// Moves the stack to a new session generation, invalidating every target
    /// recorded in an older one.
    ///
    /// Returns how many targets were invalidated. Called when the session is
    /// rebuilt: the windows the entries were opened over belong to the previous
    /// session, and their numeric ids must not be reused (ADR 0013, decision 1).
    pub fn set_generation(&mut self, generation: SessionGeneration) -> usize {
        if generation == self.generation {
            return 0;
        }
        self.generation = generation;

        let mut invalidated = 0;
        for layer in &mut self.layers {
            if let SavedTarget::Live(target) = layer.target
                && target.generation != generation
            {
                layer.target = SavedTarget::Invalid {
                    target,
                    reason: FallbackReason::SessionChanged {
                        saved: target.generation,
                        current: generation,
                    },
                };
                invalidated += 1;
            }
        }
        invalidated
    }

    /// The innermost layer, which is the one `Esc` would close.
    pub fn current(&self) -> Option<&Entry> {
        self.layers.last().map(|layer| &layer.entry)
    }

    /// The innermost layer that takes keyboard focus, if any.
    ///
    /// A layer shown without taking focus leaves the keyboard with whatever was
    /// below it: the innermost keyboard-interactive layer, or the focused window
    /// when there is none.
    pub fn focus(&self) -> Option<EntryId> {
        self.layers
            .iter()
            .rev()
            .find(|layer| layer.entry.focus == EntryFocus::Keyboard)
            .map(|layer| layer.entry.id)
    }

    /// How many layers are open.
    pub fn depth(&self) -> usize {
        self.layers.len()
    }

    /// Whether `entry` has a layer open.
    pub fn is_open(&self, entry: EntryId) -> bool {
        self.layers.iter().any(|layer| layer.entry.id == entry)
    }

    /// Classifies a return target the caller recorded.
    fn classify(&self, return_target: Option<ReturnTarget>) -> SavedTarget {
        match return_target {
            None => SavedTarget::Absent,
            Some(target) if target.generation == self.generation => SavedTarget::Live(target),
            Some(target) => SavedTarget::Invalid {
                target,
                reason: FallbackReason::SessionChanged {
                    saved: target.generation,
                    current: self.generation,
                },
            },
        }
    }
}

/// Turns a popped layer's saved target into the outcome for its close.
fn resolve(target: SavedTarget) -> NavigationOutcome {
    match target {
        SavedTarget::Absent => NavigationOutcome::Closed { returned_to: None },
        SavedTarget::Live(target) => NavigationOutcome::Closed {
            returned_to: Some(target),
        },
        SavedTarget::Invalid { reason, .. } => NavigationOutcome::ReturnedToFallback { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERATION: SessionGeneration = SessionGeneration::new(1);

    fn window(value: u32) -> WindowId {
        WindowId::new(value)
    }

    fn target(value: u32) -> ReturnTarget {
        ReturnTarget::new(window(value), GENERATION)
    }

    fn keyboard(entry: EntryId) -> Entry {
        Entry::new(entry, EntryFocus::Keyboard)
    }

    fn display(entry: EntryId) -> Entry {
        Entry::new(entry, EntryFocus::Display)
    }

    fn stack() -> NavigationStack {
        NavigationStack::new(GENERATION)
    }

    #[test]
    fn entry_names_round_trip_and_unknown_names_are_none() {
        for entry in EntryId::ALL {
            assert_eq!(EntryId::parse(entry.as_str()), Some(entry));
        }

        assert_eq!(EntryId::parse("tray"), None);
        assert_eq!(EntryId::parse(""), None);
        assert_eq!(EntryId::parse("Panel"), None);
    }

    #[test]
    fn entry_names_lists_every_entry() {
        let names = EntryId::names();

        for entry in EntryId::ALL {
            assert!(names.contains(entry.as_str()), "{names}");
        }
    }

    #[test]
    fn opening_a_keyboard_entry_takes_focus() {
        let mut stack = stack();

        let outcome = stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        assert_eq!(
            outcome,
            NavigationOutcome::Opened {
                entry: EntryId::Launcher,
                focus: Some(EntryId::Launcher),
            }
        );
        assert_eq!(stack.depth(), 1);
        assert_eq!(
            stack.current().map(|entry| entry.id),
            Some(EntryId::Launcher)
        );
    }

    #[test]
    fn a_display_entry_does_not_take_focus_from_a_layer_below_it() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        let outcome = stack.open(display(EntryId::Notifications), None);

        assert_eq!(
            outcome,
            NavigationOutcome::Opened {
                entry: EntryId::Notifications,
                focus: Some(EntryId::Launcher),
            }
        );
        assert_eq!(stack.focus(), Some(EntryId::Launcher));
    }

    #[test]
    fn a_display_entry_alone_leaves_the_keyboard_with_the_window() {
        let mut stack = stack();

        let outcome = stack.open(display(EntryId::Panel), Some(target(7)));

        assert_eq!(
            outcome,
            NavigationOutcome::Opened {
                entry: EntryId::Panel,
                focus: None,
            }
        );
        assert_eq!(stack.focus(), None);
    }

    #[test]
    fn opening_the_same_entry_twice_keeps_one_layer() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        let outcome = stack.open(keyboard(EntryId::Launcher), Some(target(9)));

        assert_eq!(
            outcome,
            NavigationOutcome::AlreadyOpen {
                entry: EntryId::Launcher
            }
        );
        assert_eq!(stack.depth(), 1);
        // The first return target is the one that stays in force: the entry was
        // not reopened, so it did not save a new one.
        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed {
                returned_to: Some(target(7))
            }
        );
    }

    #[test]
    fn a_keyboard_navigation_moves_the_keyboard_into_a_displayed_entry() {
        let mut stack = stack();
        stack.open(display(EntryId::Panel), Some(target(7)));

        let outcome = stack.open(keyboard(EntryId::Panel), Some(target(9)));

        assert_eq!(
            outcome,
            NavigationOutcome::Opened {
                entry: EntryId::Panel,
                focus: Some(EntryId::Panel),
            }
        );
        assert_eq!(stack.depth(), 1, "the entry keeps its single layer");
        assert_eq!(stack.focus(), Some(EntryId::Panel));
        // The layer was not reopened, so the target it saved when the click
        // first displayed it is the one `Esc` returns to.
        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed {
                returned_to: Some(target(7))
            }
        );
    }

    #[test]
    fn a_display_open_never_takes_the_keyboard_from_an_open_layer() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Panel), Some(target(7)));

        assert_eq!(
            stack.open(display(EntryId::Panel), Some(target(9))),
            NavigationOutcome::AlreadyOpen {
                entry: EntryId::Panel
            }
        );
        assert_eq!(stack.focus(), Some(EntryId::Panel));
        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed {
                returned_to: Some(target(7))
            }
        );
    }

    #[test]
    fn a_keyboard_navigation_does_not_reach_past_a_layer_above_it() {
        let mut stack = stack();
        stack.open(display(EntryId::Panel), Some(target(7)));
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        let outcome = stack.open(keyboard(EntryId::Panel), Some(target(7)));

        // The panel is a keyboard layer now, but the launcher above it is the
        // innermost one and keeps the keyboard: one focus, not two.
        assert_eq!(
            outcome,
            NavigationOutcome::Opened {
                entry: EntryId::Panel,
                focus: Some(EntryId::Launcher),
            }
        );
        assert_eq!(stack.depth(), 2);
        assert_eq!(
            stack.current().map(|entry| entry.id),
            Some(EntryId::Launcher)
        );

        // Closing the launcher leaves the keyboard with the panel.
        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed { returned_to: None }
        );
        assert_eq!(stack.focus(), Some(EntryId::Panel));
    }

    #[test]
    fn closing_an_empty_stack_changes_nothing() {
        let mut stack = stack();

        assert_eq!(stack.close_top(), NavigationOutcome::NothingToClose);
        assert_eq!(stack.close_all(), NavigationOutcome::NothingToClose);
        assert_eq!(stack.depth(), 0);
        assert_eq!(stack.current(), None);
    }

    #[test]
    fn esc_closes_one_layer_returns_focus_and_stops() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed {
                returned_to: Some(target(7))
            }
        );
        assert_eq!(stack.close_top(), NavigationOutcome::NothingToClose);
    }

    #[test]
    fn closing_with_a_layer_left_does_not_move_window_focus() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Panel), Some(target(7)));
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed { returned_to: None }
        );
        assert_eq!(stack.depth(), 1);
        assert_eq!(stack.current().map(|entry| entry.id), Some(EntryId::Panel));

        // The last layer is the one that returns focus to the window.
        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed {
                returned_to: Some(target(7))
            }
        );
    }

    #[test]
    fn closing_a_layer_opened_without_a_target_focuses_no_window() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Launcher), None);

        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed { returned_to: None }
        );
    }

    #[test]
    fn close_all_returns_to_where_the_navigation_started() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Panel), Some(target(7)));
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        assert_eq!(
            stack.close_all(),
            NavigationOutcome::Closed {
                returned_to: Some(target(7))
            }
        );
        assert_eq!(stack.depth(), 0);
        assert_eq!(stack.current(), None);
    }

    #[test]
    fn a_destroyed_window_is_never_yielded_as_a_return_target() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        assert_eq!(stack.target_destroyed(window(7)), 1);

        assert_eq!(
            stack.close_top(),
            NavigationOutcome::ReturnedToFallback {
                reason: FallbackReason::WindowDestroyed { window: window(7) }
            }
        );
    }

    #[test]
    fn destroying_an_unrelated_window_invalidates_nothing() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        assert_eq!(stack.target_destroyed(window(8)), 0);
        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed {
                returned_to: Some(target(7))
            }
        );
    }

    #[test]
    fn a_new_session_generation_invalidates_saved_targets() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        let next = SessionGeneration::new(2);
        assert_eq!(stack.set_generation(next), 1);

        assert_eq!(
            stack.close_top(),
            NavigationOutcome::ReturnedToFallback {
                reason: FallbackReason::SessionChanged {
                    saved: GENERATION,
                    current: next,
                }
            }
        );
    }

    #[test]
    fn re_setting_the_same_generation_invalidates_nothing() {
        let mut stack = stack();
        stack.open(keyboard(EntryId::Launcher), Some(target(7)));

        assert_eq!(stack.set_generation(GENERATION), 0);
        assert_eq!(
            stack.close_top(),
            NavigationOutcome::Closed {
                returned_to: Some(target(7))
            }
        );
    }

    #[test]
    fn a_target_from_a_previous_session_is_never_believed() {
        let mut stack = NavigationStack::new(SessionGeneration::new(2));

        let stale = ReturnTarget::new(window(7), SessionGeneration::new(1));
        stack.open(keyboard(EntryId::Launcher), Some(stale));

        assert_eq!(
            stack.close_top(),
            NavigationOutcome::ReturnedToFallback {
                reason: FallbackReason::SessionChanged {
                    saved: SessionGeneration::new(1),
                    current: SessionGeneration::new(2),
                }
            }
        );
    }

    #[test]
    fn every_outcome_has_a_readable_explanation() {
        for outcome in [
            NavigationOutcome::Opened {
                entry: EntryId::Launcher,
                focus: Some(EntryId::Launcher),
            },
            NavigationOutcome::Opened {
                entry: EntryId::Panel,
                focus: None,
            },
            NavigationOutcome::AlreadyOpen {
                entry: EntryId::Launcher,
            },
            NavigationOutcome::Closed {
                returned_to: Some(target(7)),
            },
            NavigationOutcome::Closed { returned_to: None },
            NavigationOutcome::ReturnedToFallback {
                reason: FallbackReason::WindowDestroyed { window: window(7) },
            },
            NavigationOutcome::NothingToClose,
        ] {
            let diagnostic = outcome.describe();
            assert!(id::ALL.contains(&diagnostic.id()), "{}", diagnostic.id());
        }
    }
}

#[cfg(test)]
mod properties {
    use super::*;
    use proptest::prelude::*;

    /// One thing the coordinator does to the stack.
    #[derive(Debug, Clone, Copy)]
    enum Op {
        /// Open the entry `entry` over the window the live pool's `slot` names.
        Open { entry: u8, slot: u8 },
        /// `Esc`.
        Close,
        /// A session teardown.
        CloseAll,
        /// The compositor reports a window destroyed.
        Destroy { slot: u8 },
        /// The session was rebuilt.
        NewSession,
    }

    /// The entries the coordinator can open, as `u8`, so that a generated
    /// opening can be split into its two independent choices.
    const ENTRIES: u8 = EntryId::ALL.len() as u8;

    /// The entries the coordinator opens, and how they behave.
    ///
    /// The index carries both choices and keeps them independent: its low half
    /// names the entry, its high half says whether this is an ordinary display
    /// or a keyboard navigation. A generator that tied the two together could
    /// never open one entry both ways, and the rule for a second open of an
    /// entry would go unexercised.
    fn entry_of(index: u8) -> Entry {
        let entry = EntryId::ALL[usize::from(index) % EntryId::ALL.len()];
        let focus = if index < ENTRIES {
            EntryFocus::Display
        } else {
            EntryFocus::Keyboard
        };
        Entry::new(entry, focus)
    }

    /// One open layer, tracked the way the rule says it should behave.
    #[derive(Debug, Clone, Copy)]
    struct OpenLayer {
        entry: Entry,
        target: Option<ReturnTarget>,
        invalid: Option<FallbackReason>,
    }

    /// A deliberately plain implementation of the navigation rules.
    ///
    /// It is written from the documented behaviour rather than from the stack's
    /// code, so that the two agreeing is evidence and not a tautology.
    struct Model {
        layers: Vec<OpenLayer>,
        live: Vec<WindowId>,
        destroyed: Vec<(WindowId, SessionGeneration)>,
        generation: SessionGeneration,
    }

    impl Model {
        fn new() -> Self {
            Self {
                layers: Vec::new(),
                live: (1..=3).map(WindowId::new).collect(),
                destroyed: Vec::new(),
                generation: SessionGeneration::new(1),
            }
        }

        fn window_for(&self, slot: u8) -> Option<WindowId> {
            if self.live.is_empty() {
                return None;
            }
            Some(self.live[usize::from(slot) % self.live.len()])
        }

        fn focus(&self) -> Option<EntryId> {
            self.layers
                .iter()
                .rev()
                .find(|layer| layer.entry.focus == EntryFocus::Keyboard)
                .map(|layer| layer.entry.id)
        }

        fn open(&mut self, entry: Entry, target: Option<ReturnTarget>) -> NavigationOutcome {
            if let Some(open) = self
                .layers
                .iter_mut()
                .find(|layer| layer.entry.id == entry.id)
            {
                if entry.focus != EntryFocus::Keyboard || open.entry.focus != EntryFocus::Display {
                    return NavigationOutcome::AlreadyOpen { entry: entry.id };
                }

                // A keyboard navigation into a layer that is only displayed
                // moves the keyboard into that layer, in place.
                open.entry.focus = EntryFocus::Keyboard;

                return NavigationOutcome::Opened {
                    entry: entry.id,
                    focus: self.focus(),
                };
            }

            let invalid = match target {
                Some(target) if target.generation != self.generation => {
                    Some(FallbackReason::SessionChanged {
                        saved: target.generation,
                        current: self.generation,
                    })
                }
                _ => None,
            };
            self.layers.push(OpenLayer {
                entry,
                target,
                invalid,
            });

            NavigationOutcome::Opened {
                entry: entry.id,
                focus: self.focus(),
            }
        }

        fn resolve(&self, layer: OpenLayer) -> NavigationOutcome {
            match (layer.target, layer.invalid) {
                (None, _) => NavigationOutcome::Closed { returned_to: None },
                (Some(target), None) => NavigationOutcome::Closed {
                    returned_to: Some(target),
                },
                (Some(_), Some(reason)) => NavigationOutcome::ReturnedToFallback { reason },
            }
        }

        fn close_top(&mut self) -> NavigationOutcome {
            let Some(layer) = self.layers.pop() else {
                return NavigationOutcome::NothingToClose;
            };
            if self.layers.is_empty() {
                self.resolve(layer)
            } else {
                NavigationOutcome::Closed { returned_to: None }
            }
        }

        fn close_all(&mut self) -> NavigationOutcome {
            let Some(outermost) = self.layers.first().copied() else {
                return NavigationOutcome::NothingToClose;
            };
            self.layers.clear();
            self.resolve(outermost)
        }

        fn destroy(&mut self, slot: u8) -> usize {
            let Some(window) = self.window_for(slot) else {
                return 0;
            };
            self.live.retain(|live| *live != window);
            self.destroyed.push((window, self.generation));

            let mut invalidated = 0;
            for layer in &mut self.layers {
                if layer.invalid.is_some() {
                    continue;
                }
                let Some(target) = layer.target else { continue };
                if target.window == window && target.generation == self.generation {
                    layer.invalid = Some(FallbackReason::WindowDestroyed { window });
                    invalidated += 1;
                }
            }
            invalidated
        }

        fn new_session(&mut self) -> usize {
            self.generation = SessionGeneration::new(self.generation.value() + 1);
            self.live = (1..=3).map(WindowId::new).collect();

            let mut invalidated = 0;
            for layer in &mut self.layers {
                if layer.invalid.is_some() {
                    continue;
                }
                let Some(target) = layer.target else { continue };
                if target.generation != self.generation {
                    layer.invalid = Some(FallbackReason::SessionChanged {
                        saved: target.generation,
                        current: self.generation,
                    });
                    invalidated += 1;
                }
            }
            invalidated
        }
    }

    fn operation() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0u8..2 * ENTRIES, 0u8..4).prop_map(|(entry, slot)| Op::Open { entry, slot }),
            Just(Op::Close),
            Just(Op::CloseAll),
            (0u8..4).prop_map(|slot| Op::Destroy { slot }),
            Just(Op::NewSession),
        ]
    }

    /// Checks everything that must hold after any single operation.
    fn check(
        stack: &NavigationStack,
        model: &Model,
        outcome: &NavigationOutcome,
        destroyed: &[(WindowId, SessionGeneration)],
    ) {
        assert_eq!(stack.depth(), model.layers.len(), "depth after {outcome:?}");
        assert!(
            stack.depth() <= EntryId::ALL.len(),
            "one layer per entry, so the depth is bounded: {outcome:?}"
        );
        for entry in EntryId::ALL {
            assert_eq!(
                stack.is_open(entry),
                model.layers.iter().any(|layer| layer.entry.id == entry),
                "membership of {entry} after {outcome:?}"
            );
        }
        assert_eq!(
            stack.current().map(|entry| entry.id),
            model.layers.last().map(|layer| layer.entry.id),
            "current layer after {outcome:?}"
        );
        assert_eq!(stack.focus(), model.focus(), "focus after {outcome:?}");

        if let NavigationOutcome::Closed {
            returned_to: Some(target),
        } = outcome
        {
            assert!(
                model.live.contains(&target.window),
                "a live window is returned to: {outcome:?}"
            );
            assert_eq!(target.generation, model.generation, "{outcome:?}");
            assert!(
                !destroyed.contains(&(target.window, target.generation)),
                "a destroyed window is never yielded: {outcome:?}"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        /// Drives the stack and the model with the same operations and checks
        /// that they agree on every outcome and on every invariant.
        #[test]
        fn the_stack_follows_the_navigation_rules(ops in prop::collection::vec(operation(), 0..24)) {
            let mut stack = NavigationStack::new(SessionGeneration::new(1));
            let mut model = Model::new();

            for op in ops {
                let outcome = match op {
                    Op::Open { entry, slot } => {
                        let entry = entry_of(entry);
                        let target = model.window_for(slot).map(|window| {
                            ReturnTarget::new(window, model.generation)
                        });
                        let expected = model.open(entry, target);
                        let actual = stack.open(entry, target);
                        prop_assert_eq!(&actual, &expected, "open of {}", entry.id);
                        actual
                    }
                    Op::Close => {
                        let expected = model.close_top();
                        let actual = stack.close_top();
                        prop_assert_eq!(&actual, &expected, "close_top");
                        actual
                    }
                    Op::CloseAll => {
                        let expected = model.close_all();
                        let actual = stack.close_all();
                        prop_assert_eq!(&actual, &expected, "close_all");
                        actual
                    }
                    Op::Destroy { slot } => {
                        let Some(window) = model.window_for(slot) else { continue };
                        let expected = model.destroy(slot);
                        let actual = stack.target_destroyed(window);
                        prop_assert_eq!(actual, expected, "target_destroyed({})", window);
                        continue;
                    }
                    Op::NewSession => {
                        let expected = model.new_session();
                        let actual = stack.set_generation(model.generation);
                        prop_assert_eq!(actual, expected, "set_generation({})", model.generation);
                        continue;
                    }
                };

                check(&stack, &model, &outcome, &model.destroyed);
            }
        }

        /// `Esc` on an empty stack is exactly `NothingToClose` and changes nothing.
        #[test]
        fn closing_an_empty_stack_is_a_no_op(ops in prop::collection::vec(operation(), 0..12)) {
            let mut stack = NavigationStack::new(SessionGeneration::new(1));
            for op in ops {
                match op {
                    Op::Open { entry, slot } => {
                        let _ = stack.open(entry_of(entry), Some(ReturnTarget::new(WindowId::new(u32::from(slot) + 1), SessionGeneration::new(1))));
                    }
                    Op::Close => { let _ = stack.close_top(); }
                    Op::CloseAll => { let _ = stack.close_all(); }
                    Op::Destroy { slot } => { let _ = stack.target_destroyed(WindowId::new(u32::from(slot) + 1)); }
                    Op::NewSession => { let _ = stack.set_generation(SessionGeneration::new(1)); }
                }
            }

            // Whatever is left, emptying the stack ends in `NothingToClose` and
            // leaves an empty stack that stays empty.
            let _ = stack.close_all();
            prop_assert_eq!(stack.depth(), 0);
            prop_assert_eq!(stack.close_top(), NavigationOutcome::NothingToClose);
            prop_assert_eq!(stack.close_all(), NavigationOutcome::NothingToClose);
            prop_assert_eq!(stack.depth(), 0);
            prop_assert_eq!(stack.current(), None);
        }
    }
}
