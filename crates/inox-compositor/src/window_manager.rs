//! The server half of the private `inox-window-manager-v1` protocol.
//!
//! The protocol itself lives in `inox-protocols`; this module is the compositor's
//! implementation of it. It exists because the compositor owns the windows and
//! the panel draws them, and a Cargo dependency between the two layers is
//! forbidden, so the task bar's four facts and six verbs travel over Wayland
//! (ADR 0045).
//!
//! Three properties of the protocol shape everything here, and each is worth
//! understanding before changing this file:
//!
//! - **A request is confirmed by a state event, not by an acknowledgement.**
//!   There is no "done" event: the `window` event carrying the new state *is* the
//!   confirmation, and `request_failed` is the only negative outcome. A compositor
//!   that answered "done" before the change reached the screen would be lying, and
//!   a client would draw a state that is not the one it is in.
//! - **Events are complete state, not deltas.** [`WindowRecord`] carries every
//!   field of a window, and a client that missed an event converges on the next
//!   one. That is what makes [`diff`] safe to compute once for every observer
//!   instead of tracking what each one has seen.
//! - **There are exactly two places events are sent from**: the snapshot served by
//!   `get_window_list`, and [`WindowService::announce`], which the compositor calls
//!   after a window, mode, workspace or focus change. Sending a window event from
//!   anywhere else is how an observer loses track of the desktop.
//!
//! Handles are assigned by the compositor and are opaque to the client: a handle
//! is only meaningful together with the `session_generation` it was issued under,
//! which is why every list receives the generation before any handle.

use inox_protocols::window_manager::inox_window_manager::server::{
    inox_window_list_v1::{InoxWindowListV1, Request as ListRequest, WindowMode},
    inox_window_manager_v1::{InoxWindowManagerV1, Request as ManagerRequest},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, WEnum, backend::ClientId,
};
use tracing::debug;

use crate::{state::InoxState, workspace::WindowId};

pub use inox_protocols::window_manager::inox_window_manager::server::inox_window_list_v1::RequestError;

use crate::workspace::WindowMode as InoxWindowMode;

/// The complete state of one managed window, as the `window` event carries it.
///
/// This is the panel's whole window model: every field is something the task bar
/// draws or acts on, and nothing is derived here that the compositor could report
/// inconsistently later. An `output` of `""` means the window is managed but
/// unplaced — every display is gone — which is a state the compositor really can
/// be in, so it is a value rather than an omission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRecord {
    /// The compositor's opaque handle for the window.
    pub handle: u32,
    /// The application id the client set, possibly empty.
    pub app_id: String,
    /// The title the client set, possibly empty.
    pub title: String,
    /// The zero-based workspace the window is on.
    pub workspace: u32,
    /// The connector name the window is on, or empty while it is unplaced.
    pub output: String,
    /// How the compositor is showing the window.
    pub mode: InoxWindowMode,
    /// Whether the window is put away.
    pub minimized: bool,
    /// Whether the window is on the shown workspace and not minimized.
    pub visible: bool,
    /// Whether the window is the one with the keyboard.
    pub focused: bool,
}

/// One thing that changed between two record sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A window was added, or an already-known window changed a field.
    Window(WindowRecord),
    /// A window is gone; its handle is dead and is never re-issued.
    Removed(u32),
}

/// What an observer must be told to go from `previous` to `current`.
///
/// Pure, and unit tested, because it is the one piece of this module whose bugs
/// are invisible in a screenshot: a missed update leaves a task button showing a
/// state the window is not in. Removals come first and in handle order, then
/// additions and updates in `current` order, so two runs over the same inputs
/// produce the same wire traffic.
pub fn diff(previous: &[WindowRecord], current: &[WindowRecord]) -> Vec<Change> {
    let mut changes = Vec::new();

    // A handle in `previous` that is gone from `current` is a removal. Handles are
    // never re-used, so a handle present in both is the same window.
    for record in previous {
        if !current.iter().any(|next| next.handle == record.handle) {
            changes.push(Change::Removed(record.handle));
        }
    }

    for record in current {
        let unchanged = previous
            .iter()
            .any(|old| old.handle == record.handle && old == record);

        if !unchanged {
            changes.push(Change::Window(record.clone()));
        }
    }

    changes
}

/// The protocol's mode for an Inox window mode.
///
/// One function rather than a `From` implementation because the mapping has to be
/// visible next to [`mode_from_protocol`]: the two halves are a single contract,
/// and a mode added to [`InoxWindowMode`] must break this match rather than fall
/// into a catch-all that silently reports the wrong mode.
pub fn protocol_mode(mode: InoxWindowMode) -> WindowMode {
    match mode {
        InoxWindowMode::Tiled => WindowMode::Tiled,
        InoxWindowMode::Floating => WindowMode::Floating,
        InoxWindowMode::Maximized => WindowMode::Maximized,
        InoxWindowMode::Fullscreen => WindowMode::Fullscreen,
    }
}

/// The Inox mode a protocol mode means, or `None` for a value this build does not
/// know.
///
/// An unknown mode is not an error: the protocol may grow one, and a client that
/// sends it is asking for something this compositor cannot do. `None` becomes
/// `request_failed(unsupported)` rather than a protocol error, so the task button
/// learns why nothing happened.
pub fn mode_from_protocol(mode: WEnum<WindowMode>) -> Option<InoxWindowMode> {
    let mode = match mode {
        WEnum::Value(mode) => mode,
        WEnum::Unknown(_) => return None,
    };

    Some(match mode {
        WindowMode::Tiled => InoxWindowMode::Tiled,
        WindowMode::Floating => InoxWindowMode::Floating,
        WindowMode::Maximized => InoxWindowMode::Maximized,
        WindowMode::Fullscreen => InoxWindowMode::Fullscreen,
        // The generated enum is `non_exhaustive`: a mode this build does not know
        // is one the compositor cannot apply, so it maps to `None` and becomes
        // `request_failed(unsupported)` rather than a wrong mode.
        _ => return None,
    })
}

/// The observers of one compositor run, and what they have been told.
///
/// Every `inox_window_list_v1` object is one observer. The service keeps one copy
/// of the last announced records rather than one per observer, because the events
/// carry complete state: a single [`diff`] is correct for every observer, and
/// per-observer bookkeeping would only invite them to drift.
#[derive(Debug, Default)]
pub struct WindowService {
    /// The generation this run's handles belong to.
    generation: u32,
    /// The live observers. A destroyed list removes itself.
    observers: Vec<InoxWindowListV1>,
    /// The records every observer has already been told about.
    last: Vec<WindowRecord>,
    /// The workspace state last announced, as `(active, count)`.
    last_workspace: Option<(u32, u32)>,
    /// The generation last announced, so a change sends the new one first.
    last_generation: Option<u32>,
}

impl WindowService {
    /// A service whose handles belong to `generation`, with no observers yet.
    pub fn new(generation: u32) -> Self {
        Self {
            generation,
            ..Self::default()
        }
    }

    /// The generation this run's handles belong to.
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// How many observers are live.
    pub fn observer_count(&self) -> usize {
        self.observers.len()
    }

    /// Add an observer.
    ///
    /// The caller serves it a snapshot immediately afterwards; an observer that is
    /// registered but not snapshotted would receive incremental events against a
    /// view it never built.
    pub fn register(&mut self, list: InoxWindowListV1) {
        self.observers.push(list);
    }

    /// Forget an observer that was destroyed.
    pub fn unregister(&mut self, list: &InoxWindowListV1) {
        self.observers.retain(|observer| observer != list);
    }

    /// Serve one observer the complete snapshot, in the order the protocol fixes.
    ///
    /// `session_generation` first, because no handle that follows is meaningful
    /// without it; then `workspace_state`, because a window's workspace number is
    /// relative to the count; then one `window` per record in layout order; then
    /// `snapshot_done`, so an empty desktop is distinguishable from a snapshot
    /// still in flight.
    pub fn snapshot(
        &mut self,
        list: &InoxWindowListV1,
        active_workspace: u32,
        count: u32,
        records: &[WindowRecord],
    ) {
        list.session_generation(self.generation);
        list.workspace_state(active_workspace, count);
        for record in records {
            send_window(list, record);
        }
        list.snapshot_done();

        // Every observer must end up with the same view. A list that snapshot
        // *later* than the others sees the newest state, so before that becomes the
        // shared baseline the observers already registered are brought up to it;
        // otherwise a change that happened between two snapshots would be lost for
        // the first observer for good, because the next diff would be against a
        // state it never received. The list being snapshotted is deliberately not
        // registered yet (see `serve_snapshot`), so it does not hear its own state
        // twice.
        self.converge(active_workspace, count, records, self.generation);

        self.last = records.to_vec();
        self.last_workspace = Some((active_workspace, count));
        self.last_generation = Some(self.generation);
    }

    /// Tell every observer everything that changed since the last announcement.
    ///
    /// This is one of the two places window events leave the compositor. The
    /// generation and workspace facts go out first when they changed, because a
    /// client cannot interpret a window's workspace number or a handle before it
    /// has them.
    ///
    /// Nothing is sent when nothing changed: `announce` is called from every
    /// arrange and every focus change, and a task bar that re-received its whole
    /// window list on each of those would flicker and waste a wake-up.
    pub fn announce(
        &mut self,
        generation: u32,
        active_workspace: u32,
        count: u32,
        records: &[WindowRecord],
    ) {
        if self.observers.is_empty() {
            // Nobody is watching: remember the state anyway, so the first
            // observer's snapshot does not have to reason about a stale "last".
            self.last = records.to_vec();
            self.last_workspace = Some((active_workspace, count));
            self.last_generation = Some(generation);
            return;
        }

        self.converge(active_workspace, count, records, generation);

        self.last = records.to_vec();
        self.last_workspace = Some((active_workspace, count));
        self.last_generation = Some(generation);
    }

    /// Bring every registered observer from what it has been told to `records`.
    ///
    /// Shared by `announce` and `snapshot` so there is one implementation of "what
    /// an observer still has to hear". `snapshot_done` is **not** sent here: it
    /// answers a snapshot, and sending it with an incremental update would tell a
    /// client a snapshot had ended that never started.
    fn converge(
        &mut self,
        active_workspace: u32,
        count: u32,
        records: &[WindowRecord],
        generation: u32,
    ) {
        let pending = self.pending(active_workspace, count, records, generation);

        for observer in &self.observers {
            if pending.generation {
                observer.session_generation(generation);
            }
            if pending.workspace {
                observer.workspace_state(active_workspace, count);
            }
            for change in &pending.changes {
                match change {
                    Change::Window(record) => send_window(observer, record),
                    Change::Removed(handle) => observer.window_removed(*handle),
                }
            }
        }

        if !pending.changes.is_empty() {
            debug!(
                changes = pending.changes.len(),
                observers = self.observers.len(),
                "announced a window-management change"
            );
        }

        if pending.generation {
            // A new run's handles are meaningless to a client that still holds the
            // old ones, so the generation goes first and the receiver drops the
            // views it had: every window it knew about is gone as far as it knows.
            self.last = Vec::new();
            self.last_workspace = None;
        }
        self.last = records.to_vec();
        self.last_workspace = Some((active_workspace, count));
        self.last_generation = Some(generation);
    }

    /// What an observer that has been told `self.last` still has to hear.
    ///
    /// Pure and separate from the send loop so the decision — which is what a
    /// missed event would break — is unit testable without a Wayland object. The
    /// order is the protocol's: a new generation first, then the workspace facts,
    /// then the windows.
    fn pending(
        &self,
        active_workspace: u32,
        count: u32,
        records: &[WindowRecord],
        generation: u32,
    ) -> Pending {
        let generation_changed = self.last_generation != Some(generation);
        // A new generation invalidates every handle the client holds, so the window
        // changes are computed from nothing rather than from the old records.
        let baseline: &[WindowRecord] = if generation_changed { &[] } else { &self.last };

        Pending {
            generation: generation_changed,
            workspace: generation_changed || self.last_workspace != Some((active_workspace, count)),
            changes: diff(baseline, records),
        }
    }

    /// Report that a request named by `list` was refused.
    ///
    /// The failure goes only to the list that asked: the capture is that one
    /// object, and a different observer's view is unaffected by another client's
    /// refused request.
    pub fn fail(&mut self, list: &InoxWindowListV1, window: u32, reason: RequestError) {
        list.request_failed(window, reason);
    }

    /// Drop every observer, for teardown.
    pub fn clear(&mut self) {
        self.observers.clear();
    }
}

/// The events one observer still has to receive to be current.
#[derive(Debug, Default, PartialEq, Eq)]
struct Pending {
    /// The observer must be told a new `session_generation` before any handle.
    generation: bool,
    /// The observer must be told the shown workspace and the count.
    workspace: bool,
    /// The window additions, updates and removals, in send order.
    changes: Vec<Change>,
}

/// Send one `window` event.
///
/// A single function because the argument order is the protocol's and is easy to
/// transpose by hand: handle, app id, title, workspace, output, mode, minimized,
/// visible, focused. The client decodes them in that order.
fn send_window(list: &InoxWindowListV1, record: &WindowRecord) {
    list.window(
        record.handle,
        record.app_id.clone(),
        record.title.clone(),
        record.workspace,
        record.output.clone(),
        protocol_mode(record.mode),
        u32::from(record.minimized),
        u32::from(record.visible),
        u32::from(record.focused),
    );
}

/// One observer's snapshot, served from the compositor's current state.
///
/// This is one of the two places window-management events leave the compositor.
/// The snapshot is sent synchronously from `get_window_list`, because a client
/// that creates a list and then hears nothing else has no view to update; the
/// other entry point is [`InoxState::announce_windows`].
pub(crate) fn serve_snapshot(state: &mut InoxState, list: &InoxWindowListV1) {
    let records = state.window_records();
    let (active, count) = state.workspace_state();
    let service = state.window_service();

    // The snapshot is served *before* the list is registered: `snapshot` brings the
    // observers that were already there up to this state, and the new list must not
    // receive that catch-up as a second copy of the state it just read.
    service.snapshot(list, active, count, &records);
    service.register(list.clone());
}

impl Dispatch<InoxWindowManagerV1, ()> for InoxState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &InoxWindowManagerV1,
        request: ManagerRequest,
        _data: &(),
        _display_handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            ManagerRequest::GetWindowList { id } => {
                // The object exists before the snapshot is built, because the
                // snapshot is the first thing the client can receive and it has
                // to arrive in the order the protocol fixes.
                let list = data_init.init(id, ());
                serve_snapshot(state, &list);
                debug!("served a window-list snapshot");
            }
            // A destructor: wayland-server removes the object after dispatch, and
            // there is nothing else to clean up for the manager.
            ManagerRequest::Destroy => {}
            // `Request` is `#[non_exhaustive]`: a later revision may add a
            // request, which this build ignores rather than second-guessing.
            _ => {}
        }
    }
}

impl GlobalDispatch<InoxWindowManagerV1, ()> for InoxState {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<InoxWindowManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        // Binding grants nothing beyond becoming an observer: the list objects a
        // client creates from here are what carry the events and verbs.
        data_init.init(resource, ());
    }
}

impl Dispatch<InoxWindowListV1, ()> for InoxState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &InoxWindowListV1,
        request: ListRequest,
        _data: &(),
        _display_handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Every verb is a request to change window management, and its
        // confirmation is the state event that follows: see `announce_windows`.
        // A verb that cannot be carried out reports why, because a silent no-op
        // would leave a task button disabled for ever with nothing to log.
        let result = match request {
            ListRequest::Activate { window } => activate(state, window),
            ListRequest::Minimize { window } => minimize(state, window),
            ListRequest::Close { window } => close(state, window),
            ListRequest::SetMode { window, mode } => {
                match mode_from_protocol(mode) {
                    Some(mode) => state
                        .set_window_mode(handle_to_window(window), mode)
                        .map_err(|error| (window, error)),
                    // An unknown mode is the protocol growing a value this build
                    // does not implement, which is exactly `unsupported`.
                    None => Err((window, RequestError::Unsupported)),
                }
            }
            ListRequest::MoveToWorkspace { window, workspace } => state
                .move_window_to_workspace(handle_to_window(window), workspace)
                .map_err(|error| (window, error)),
            ListRequest::SwitchWorkspace { workspace } => {
                state.switch_workspace(workspace).map_err(|error| {
                    // `request_failed` names a window; a workspace switch has none,
                    // so it names the handle the protocol reserves for "no window".
                    (NO_WINDOW_HANDLE, error)
                })
            }
            // The destructor: `destroyed` below drops the observer.
            ListRequest::Destroy => return,
            // `Request` is `#[non_exhaustive]`.
            _ => return,
        };

        if let Err((window, reason)) = result {
            debug!(
                window,
                reason = ?reason,
                "refused a window-management request"
            );
            state.window_service().fail(resource, window, reason);
        }
    }

    /// Called for every way a list can end, `destroy` and a client disconnect
    /// included.
    fn destroyed(state: &mut Self, _client: ClientId, resource: &InoxWindowListV1, _data: &()) {
        state.window_service().unregister(resource);
    }
}

/// The handle `request_failed` carries for a request that names no window.
///
/// The protocol's error event always has a `window` argument because a task button
/// matches a failure to the request it has in flight. A workspace switch has no
/// window, so it reports this reserved handle; a client that sent a workspace
/// switch knows which of its requests this answers because it has at most one
/// outstanding workspace request at a time.
///
/// `u32::MAX` cannot collide with a real window: a run would have to mint more
/// than four billion handles to reach it, and [`handle_of`] clamps at the same
/// value, so the two spellings of "no window" are one number.
pub const NO_WINDOW_HANDLE: u32 = u32::MAX;

/// The protocol handle of a managed window.
///
/// Handles are the workspace's identifiers and never repeat within a run, so the
/// protocol's `uint` is the same number the compositor uses internally. A
/// `try_from` rather than an `as`: truncating a handle silently would turn two
/// different windows into one task button, whereas clamping to the reserved
/// sentinel is at least a value no real window can have.
pub fn handle_of(id: WindowId) -> u32 {
    u32::try_from(id.get()).unwrap_or(NO_WINDOW_HANDLE)
}

/// The [`WindowId`] a protocol handle names.
///
/// A handle this build never issued becomes a `WindowId` that no window has, which
/// is exactly what `unknown_window` means. It does not have to be rejected here:
/// the state methods answer for the ids they actually know.
fn handle_to_window(handle: u32) -> WindowId {
    WindowId::from_raw(u64::from(handle))
}

/// `activate`, with the protocol's error type.
fn activate(state: &mut InoxState, window: u32) -> Result<(), (u32, RequestError)> {
    state
        .activate_window(handle_to_window(window))
        .map_err(|error| (window, error))
}

/// `minimize`, with the protocol's error type.
fn minimize(state: &mut InoxState, window: u32) -> Result<(), (u32, RequestError)> {
    state
        .minimize_window(handle_to_window(window), true)
        .map_err(|error| (window, error))
}

/// `close`, with the protocol's error type.
fn close(state: &mut InoxState, window: u32) -> Result<(), (u32, RequestError)> {
    state
        .close_window(handle_to_window(window))
        .map_err(|error| (window, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(handle: u32) -> WindowRecord {
        WindowRecord {
            handle,
            app_id: "org.example.Editor".to_owned(),
            title: "notes.txt".to_owned(),
            workspace: 0,
            output: OUTPUT.to_owned(),
            mode: InoxWindowMode::Tiled,
            minimized: false,
            visible: true,
            focused: false,
        }
    }

    const OUTPUT: &str = "eDP-1";

    #[test]
    fn an_unchanged_set_produces_no_changes() {
        let records = vec![record(1), record(2)];

        assert_eq!(diff(&records, &records), Vec::new());
    }

    #[test]
    fn a_new_window_is_announced_once() {
        let changes = diff(&[record(1)], &[record(1), record(2)]);

        assert_eq!(changes, vec![Change::Window(record(2))]);
    }

    #[test]
    fn a_vanished_window_is_a_removal() {
        let changes = diff(&[record(1), record(2)], &[record(1)]);

        assert_eq!(changes, vec![Change::Removed(2)]);
    }

    /// Every field is part of the record, so every field changing has to produce
    /// an event: a task button that ignores one of them shows a stale fact.
    #[test]
    fn every_field_change_is_announced() {
        let original = record(1);
        let mutations: Vec<(&str, WindowRecord)> = vec![
            ("app id", {
                let mut next = original.clone();
                next.app_id = "org.example.Terminal".to_owned();
                next
            }),
            ("title", {
                let mut next = original.clone();
                next.title = "other.txt".to_owned();
                next
            }),
            ("workspace", {
                let mut next = original.clone();
                next.workspace = 1;
                next
            }),
            ("output", {
                let mut next = original.clone();
                next.output = "HDMI-A-1".to_owned();
                next
            }),
            ("mode", {
                let mut next = original.clone();
                next.mode = InoxWindowMode::Fullscreen;
                next
            }),
            ("minimized", {
                let mut next = original.clone();
                next.minimized = true;
                next
            }),
            ("visible", {
                let mut next = original.clone();
                next.visible = false;
                next
            }),
            ("focused", {
                let mut next = original.clone();
                next.focused = true;
                next
            }),
        ];

        for (field, changed) in mutations {
            assert_eq!(
                diff(
                    std::slice::from_ref(&original),
                    std::slice::from_ref(&changed)
                ),
                vec![Change::Window(changed)],
                "a {field} change must be announced"
            );
        }
    }

    #[test]
    fn removals_come_before_the_current_set() {
        let changes = diff(&[record(1), record(2)], &[record(3)]);

        assert_eq!(
            changes,
            vec![
                Change::Removed(1),
                Change::Removed(2),
                Change::Window(record(3))
            ]
        );
    }

    #[test]
    fn a_snapshot_from_nothing_is_every_window() {
        let records = vec![record(4), record(5)];
        let changes = diff(&[], &records);

        assert_eq!(
            changes,
            vec![Change::Window(record(4)), Change::Window(record(5))]
        );
    }

    #[test]
    fn losing_every_window_removes_every_handle() {
        let changes = diff(&[record(1), record(2)], &[]);

        assert_eq!(changes, vec![Change::Removed(1), Change::Removed(2)]);
    }

    /// A record set that arrived after an event was missed still converges: the
    /// announcement carries the complete state of anything that differs, so the
    /// client never has to ask for a snapshot.
    #[test]
    fn a_missed_event_converges_on_the_next_announcement() {
        let stale = vec![record(1), record(2)];
        let current = {
            let mut current = vec![record(1), record(2)];
            current[0].title = "changed while nobody looked".to_owned();
            current
        };

        assert_eq!(
            diff(&stale, &current),
            vec![Change::Window(current[0].clone())]
        );
    }

    #[test]
    fn the_mode_mapping_round_trips() {
        for mode in InoxWindowMode::ALL {
            let protocol = protocol_mode(mode);
            let back = mode_from_protocol(WEnum::Value(protocol));

            assert_eq!(back, Some(mode), "the mode {mode:?} must round trip");
        }

        assert_eq!(mode_from_protocol(WEnum::Unknown(99)), None);
    }

    /// The very first announcement tells a client the generation and the workspace
    /// facts before any handle, because a handle is meaningless without them.
    #[test]
    fn the_first_announcement_establishes_the_generation_and_the_workspaces() {
        let service = WindowService::new(7);

        let pending = service.pending(0, 3, &[record(1)], 7);

        assert!(pending.generation, "the generation precedes the handles");
        assert!(
            pending.workspace,
            "the workspace count has to arrive before a window's number"
        );
        assert_eq!(pending.changes, vec![Change::Window(record(1))]);
    }

    /// A change that happens between two snapshots must reach the observer that
    /// snapshotted first. This is the shared-baseline property: the service tracks
    /// one record set for every observer, so a second list arriving *after* a window
    /// appeared must bring the first one up to it rather than moving the baseline
    /// past a state it never received.
    #[test]
    fn an_observer_is_brought_up_to_the_newest_state_not_past_it() {
        let mut service = WindowService::new(1);

        // Establish the baseline the way a first observer's snapshot would.
        service.last = vec![record(1)];
        service.last_workspace = Some((0, 1));
        service.last_generation = Some(1);

        // A window appeared since: the observer has to hear about it, not skip it.
        let newer = vec![record(1), record(2)];
        let pending = service.pending(0, 1, &newer, 1);

        assert!(!pending.generation, "the generation did not change");
        assert!(!pending.workspace, "the workspace facts did not change");
        assert_eq!(
            pending.changes,
            vec![Change::Window(record(2))],
            "each observer is sent exactly what it has not been told"
        );
    }

    /// A new generation invalidates every handle a client holds, so the whole window
    /// set is re-sent rather than diffed against handles that now mean different
    /// windows.
    #[test]
    fn a_new_generation_re_announces_every_window() {
        let mut service = WindowService::new(1);
        service.last = vec![record(1), record(2)];
        service.last_workspace = Some((0, 1));
        service.last_generation = Some(1);

        // The same records, but under a new generation: the client must not match
        // its old handle 1 against whatever window has handle 1 now.
        let pending = service.pending(0, 1, &[record(1), record(2)], 2);

        assert!(pending.generation, "the new generation goes first");
        assert!(
            pending.workspace,
            "the workspace facts follow a new generation"
        );
        assert_eq!(
            pending.changes,
            vec![Change::Window(record(1)), Change::Window(record(2))],
            "every handle is re-issued, so every window is re-announced"
        );
    }

    /// Nothing is sent when nothing changed: `announce` runs after every arrange and
    /// every focus change, and an observer that was re-sent its whole list each time
    /// would flicker and wake up for nothing.
    #[test]
    fn an_announcement_with_no_changes_is_empty() {
        let mut service = WindowService::new(1);
        service.last = vec![record(1)];
        service.last_workspace = Some((0, 1));
        service.last_generation = Some(1);

        let pending = service.pending(0, 1, &[record(1)], 1);

        assert!(!pending.generation);
        assert!(!pending.workspace);
        assert!(
            pending.changes.is_empty(),
            "an unchanged desktop sends nothing"
        );
    }

    /// A workspace change is announced even when the window set is identical,
    /// because it is what makes a window visible or invisible.
    #[test]
    fn a_workspace_change_is_announced_without_a_window_change() {
        let mut service = WindowService::new(1);
        service.last = vec![record(1)];
        service.last_workspace = Some((0, 3));
        service.last_generation = Some(1);

        let pending = service.pending(2, 3, &[record(1)], 1);

        assert!(!pending.generation);
        assert!(pending.workspace, "the shown workspace changed");
    }

    #[test]
    fn the_sentinel_handle_cannot_collide_with_a_real_window() {
        // A run would need more than four billion windows to reach it, and
        // `handle_of` clamps there rather than wrapping, so the sentinel is never
        // a handle that names a window.
        assert_eq!(NO_WINDOW_HANDLE, u32::MAX);
        assert_eq!(handle_of(WindowId::from_raw(u64::MAX)), NO_WINDOW_HANDLE);
        assert_eq!(handle_of(WindowId::from_raw(7)), 7);
    }

    #[test]
    fn a_handle_round_trips_through_its_protocol_number() {
        for raw in [0, 1, 42, u64::from(u32::MAX)] {
            let id = WindowId::from_raw(raw);
            assert_eq!(handle_to_window(handle_of(id)), id);
        }
    }

    #[test]
    fn the_advertised_mode_values_are_the_protocols() {
        // The panel branches on these across a process boundary, so a silent
        // renumbering would draw a fullscreen window as tiled.
        assert_eq!(u32::from(protocol_mode(InoxWindowMode::Tiled)), 0);
        assert_eq!(u32::from(protocol_mode(InoxWindowMode::Floating)), 1);
        assert_eq!(u32::from(protocol_mode(InoxWindowMode::Maximized)), 2);
        assert_eq!(u32::from(protocol_mode(InoxWindowMode::Fullscreen)), 3);
    }
}
