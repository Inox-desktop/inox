//! The panel's window-service transport: `inox-window-manager-v1` on the wire.
//!
//! The panel has owned a window model ([`crate::tasks`]), a service contract
//! ([`crate::contract`]) and an Elm state machine ([`crate::app`]) since long
//! before anything could reach a compositor: [`crate::app::State`] queued
//! [`crate::app::OutgoingCommand`]s and [`crate::layer`] had nowhere to carry
//! them. This module is that missing line — the protocol leg ADR 0045 §4 asks
//! for — and it adds **no window model of its own**: every fact it reads becomes
//! one of the [`Message`]s the panel already had, and every command it sends is
//! one of the [`OutgoingCommand`]s the panel already queued.
//!
//! # What it translates
//!
//! | `inox_window_list_v1` event | [`Message`] |
//! |---|---|
//! | `session_generation` | [`Message::ServiceReady`] |
//! | `workspace_state` | [`Message::WorkspacesChanged`] |
//! | `window` | [`Message::WindowUpdated`] (the snapshot's records too) |
//! | `window_removed` | [`Message::WindowRemoved`] |
//! | `snapshot_done` | [`Message::Snapshot`] |
//! | `request_failed` (a window) | [`Message::CommandSettled`] with a rejection |
//! | `request_failed` (the no-window handle) | [`Message::WorkspaceSwitchFailed`] |
//!
//! and, in the other direction, every [`OutgoingCommand`] becomes the protocol
//! request of the same name: `Snapshot` creates a fresh `inox_window_list_v1` (a
//! list *is* a snapshot, so reconnecting means a new observer), `Activate` →
//! `activate`, `Toggle` → `minimize`, `Close` → `close`, `SetMode` → `set_mode`,
//! `MoveToWorkspace` → `move_to_workspace`, `SwitchWorkspace` →
//! `switch_workspace`.
//!
//! # Three rules from the protocol, not from this module
//!
//! - **A request is confirmed by a state event, never by an acknowledgement.**
//!   The leg therefore never produces [`crate::contract::CommandState::Applied`]:
//!   it records the request as sent, and the `window` / `window_removed` /
//!   `workspace_state` event that follows settles it through
//!   [`crate::app::update`]. [`crate::contract::CommandState::Rejected`] comes
//!   only from `request_failed`.
//! - **Handles are only meaningful within their `session_generation`.** Every
//!   `WindowKey` built here pairs the wire handle with the generation the list
//!   announced, and a new list starts a new collection.
//! - **`request_failed` answers a workspace switch against the reserved
//!   no-window handle** (`u32::MAX`, the compositor's `NO_WINDOW_HANDLE`),
//!   because the protocol's error event always names a window and a workspace
//!   switch names none. Such a failure is **not** turned into a per-window
//!   [`Message::CommandSettled`]: there is no window to settle. It becomes
//!   [`Message::WorkspaceSwitchFailed`], which settles the single workspace
//!   switch the panel has in flight and nothing else.
//!
//! # A service that is not there is a state, not a crash
//!
//! A compositor that does not advertise `inox_window_manager_v1` leaves the
//! panel with a readable [`crate::contract::ServiceState::Unavailable`] and its
//! retry action (D05). The leg reports that once with a single `info!`; it never
//! returns an error, never exits non-zero and never panics — the clock and the
//! tray are still a usable panel (ADR 0045 §4: 「服务不可用是设计状态，不是崩溃」).
//!
//! # Testability
//!
//! [`ListState`] and [`rejection`] are pure: they take the wire values and give
//! back the messages, so the whole event→message table is unit tested without a
//! compositor. The proxy-owning [`WindowService`] on top of them is the small
//! part that needs a `wayland-client` queue, and `headless_tests::protocol`
//! drives it through a real `wayland-server` peer.

use std::collections::BTreeMap;

use inox_protocols::window_manager::inox_window_manager::client::{
    inox_window_list_v1::{self, InoxWindowListV1},
    inox_window_manager_v1::InoxWindowManagerV1,
};
use tracing::{debug, info};
use wayland_client::{Dispatch, QueueHandle};

use crate::app::{Message, OutgoingCommand};
use crate::contract::{CommandState, RejectionReason, ServiceFailure};
use crate::tasks::{SessionGeneration, WindowId, WindowKey, WindowMode, WindowState, WorkspaceId};

/// The handle `request_failed` carries when the refused request named no window.
///
/// The protocol's error event always has a `window` argument, so a
/// `switch_workspace` failure is reported against this reserved value
/// (`inox-compositor`'s `NO_WINDOW_HANDLE`). It cannot collide with a real
/// window: a run would have to mint more than four billion handles.
pub const NO_WINDOW_HANDLE: u32 = u32::MAX;

/// The rejection a wire `request_error` value means.
///
/// The three protocol values map one-to-one onto the contract's
/// [`RejectionReason`] — the contract is not a second error vocabulary, it is the
/// panel's name for the protocol's answers:
///
/// | wire | reason |
/// |---|---|
/// | 0 `unknown_window` | [`RejectionReason::UnknownWindow`] |
/// | 1 `not_permitted` | [`RejectionReason::NotPermitted`] |
/// | 2 `unsupported` | [`RejectionReason::Protocol`] naming the word |
///
/// A value this build does not know is [`RejectionReason::Protocol`] too: it is
/// the service speaking a revision the panel predates, which is exactly what that
/// variant is for.
pub fn rejection(raw: u32) -> RejectionReason {
    match raw {
        0 => RejectionReason::UnknownWindow,
        1 => RejectionReason::NotPermitted,
        2 => RejectionReason::Protocol {
            detail: "unsupported".to_owned(),
        },
        other => RejectionReason::Protocol {
            detail: format!("unknown request error {other}"),
        },
    }
}

/// The state one `inox_window_list_v1` object accumulates while it serves its
/// snapshot.
///
/// Pure, and unit tested: it is where the wire's flat `window` arguments become
/// the panel's [`WindowKey`] + [`WindowState`], which is the one translation in
/// this leg that can silently get a field's order wrong.
#[derive(Debug, Default)]
pub struct ListState {
    /// The generation the list announced, once it has.
    generation: Option<SessionGeneration>,
    /// The windows collected since the list was created, in the order they
    /// arrived — the compositor's layout order, which is the order the panel
    /// draws them in.
    windows: Vec<(WindowKey, WindowState)>,
    /// Whether the list's snapshot is still being served.
    collecting: bool,
    /// The workspaces the list reported.
    workspace: Option<(u32, u32)>,
}

impl ListState {
    /// A list that has just been created and has heard nothing yet.
    pub fn new() -> Self {
        Self {
            collecting: true,
            ..Self::default()
        }
    }

    /// Forgets everything, for a list that was replaced.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// The generation the list announced.
    pub const fn generation(&self) -> Option<SessionGeneration> {
        self.generation
    }

    /// The windows collected so far, in the order they arrived.
    pub fn windows(&self) -> &[(WindowKey, WindowState)] {
        &self.windows
    }

    /// Takes the windows collected so far, leaving the list collecting again.
    ///
    /// A snapshot is the point at which the panel's view is rebuilt, so the
    /// collection is drained: the next snapshot is built from scratch.
    pub fn take_windows(&mut self) -> Vec<(WindowKey, WindowState)> {
        std::mem::take(&mut self.windows)
    }

    /// `session_generation`: the session the handles below belong to.
    ///
    /// The message is returned only when the generation is news. A list that
    /// announces the generation the leg already knows is the answer to the
    /// panel's *own* snapshot request, and re-announcing it would make the panel
    /// drop the view it is about to receive (and ask for the snapshot it is
    /// already getting).
    pub fn session_generation(&mut self, raw: u32) -> Option<Message> {
        let generation = SessionGeneration::new(u64::from(raw));

        if self.generation == Some(generation) {
            return None;
        }

        self.generation = Some(generation);
        self.windows.clear();
        self.collecting = true;

        Some(Message::ServiceReady { generation })
    }

    /// `workspace_state`: how many workspaces there are and which is shown.
    pub fn workspace_state(&mut self, active: u32, count: u32) -> Message {
        self.workspace = Some((active, count));

        Message::WorkspacesChanged {
            active: WorkspaceId::new(active),
            count,
        }
    }

    /// `window`: one window's complete state.
    ///
    /// During the snapshot and after it this is the same message: the event
    /// carries the complete state both times, so a panel that missed an update
    /// converges on the next one.
    ///
    /// The event's `visible` flag is deliberately not stored: the panel's model
    /// derives it from what it already has — the window is minimized, or it is on
    /// a workspace that is not the active one — and a stored copy that disagreed
    /// with those two would be a third fact to keep in step. The flag is read here
    /// only to be checked against that derivation in the protocol tests.
    #[allow(clippy::too_many_arguments)]
    pub fn window(
        &mut self,
        window: u32,
        app_id: String,
        title: String,
        workspace: u32,
        output: String,
        mode: u32,
        minimized: bool,
        _visible: bool,
    ) -> Option<Message> {
        let generation = self.generation?;
        let key = WindowKey::new(WindowId::new(u64::from(window)), generation);

        // The mode is decoded rather than cast: a value this build does not know
        // would draw a wrong one, and `tiled` is the safe reading of "the
        // compositor is showing it the ordinary way".
        let mode = match WindowMode::from_protocol(mode) {
            Some(mode) => mode,
            None => {
                debug!(
                    window,
                    mode, "the compositor reported a window mode this build does not know"
                );
                WindowMode::Tiled
            }
        };

        let state = WindowState {
            app_id,
            title,
            minimized,
            workspace: WorkspaceId::new(workspace),
            output: crate::tasks::OutputId::new(output),
            ..WindowState::of_mode(mode)
        };

        if self.collecting {
            match self
                .windows
                .iter_mut()
                .find(|(candidate, _)| *candidate == key)
            {
                Some((_, slot)) => *slot = state.clone(),
                None => self.windows.push((key, state.clone())),
            }
        }

        if self.generation.is_some() {
            Some(Message::WindowUpdated { key, window: state })
        } else {
            None
        }
    }

    /// `snapshot_done`: the initial snapshot is complete.
    ///
    /// The panel applies the collection as one snapshot — that is what puts it in
    /// [`crate::contract::ServiceState::Ready`], and nothing else does.
    pub fn snapshot_done(&mut self) -> Option<Message> {
        let generation = self.generation?;
        let windows = self.take_windows();

        self.collecting = false;

        Some(Message::Snapshot {
            generation,
            windows,
        })
    }

    /// `window_removed`: a window's handle is dead.
    pub fn window_removed(&mut self, window: u32) -> Option<Message> {
        let generation = self.generation?;
        let key = WindowKey::new(WindowId::new(u64::from(window)), generation);

        self.windows.retain(|(candidate, _)| *candidate != key);

        Some(Message::WindowRemoved { key })
    }

    /// `request_failed`: the only negative outcome the protocol has.
    ///
    /// Returns the message that settles the request, and the identifier the
    /// caller recorded for it. A failure against the reserved no-window handle
    /// settles the workspace switch rather than a window (see the module docs).
    pub fn request_failed(
        &self,
        window: u32,
        reason: u32,
        in_flight: &BTreeMap<WindowKey, crate::contract::RequestId>,
        workspace_request: Option<crate::contract::RequestId>,
    ) -> Option<Failure> {
        let reason = rejection(reason);

        if window == NO_WINDOW_HANDLE {
            let request = workspace_request?;

            return Some(Failure::Workspace { request, reason });
        }

        let generation = self.generation?;
        let key = WindowKey::new(WindowId::new(u64::from(window)), generation);
        let request = in_flight.get(&key).copied()?;

        Some(Failure::Window {
            request,
            key,
            reason,
        })
    }
}

/// What a `request_failed` event settles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// A request that named a window was refused.
    Window {
        /// The request the leg sent for that window.
        request: crate::contract::RequestId,
        /// The window it named.
        key: WindowKey,
        /// Why it was refused.
        reason: RejectionReason,
    },
    /// The workspace switch in flight was refused.
    Workspace {
        /// The request the leg sent for the switch.
        request: crate::contract::RequestId,
        /// Why it was refused.
        reason: RejectionReason,
    },
}

impl Failure {
    /// The message the panel applies for this failure.
    pub fn message(&self) -> Message {
        match self {
            Self::Window {
                request,
                key,
                reason,
            } => {
                debug!(
                    request = request.get(),
                    %key,
                    ?reason,
                    "the window service refused a request"
                );

                Message::CommandSettled {
                    request: *request,
                    outcome: CommandState::Rejected {
                        reason: reason.clone(),
                    },
                }
            }
            Self::Workspace { request, reason } => {
                debug!(
                    request = request.get(),
                    ?reason,
                    "the window service refused a workspace switch"
                );

                Message::WorkspaceSwitchFailed {
                    reason: reason.clone(),
                }
            }
        }
    }
}

/// The leg: the manager and list objects, and what the panel asked that has not
/// been confirmed yet.
///
/// It owns no window state — [`ListState`] holds only what a snapshot collection
/// needs, and the panel's [`crate::tasks::TaskList`] is the model. The two maps
/// here are transport bookkeeping, not a second window table: they are the
/// identifiers the panel minted for requests the service has not answered.
pub struct WindowService {
    /// The manager global, once bound. `None` means this compositor does not
    /// offer the protocol, which is a state the panel draws (D05).
    manager: Option<InoxWindowManagerV1>,
    /// The list object serving this panel, once a snapshot was asked for.
    list: Option<InoxWindowListV1>,
    /// What the current list has collected.
    state: ListState,
    /// The request identifiers in flight, by window.
    in_flight: BTreeMap<WindowKey, crate::contract::RequestId>,
    /// The identifier of the workspace switch in flight, if there is one.
    workspace_request: Option<crate::contract::RequestId>,
    /// Whether the "this compositor does not offer the window service" line has
    /// been said. Once, not once per frame.
    unavailable_reported: bool,
}

impl Default for WindowService {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowService {
    /// A leg with nothing bound.
    pub fn new() -> Self {
        Self {
            manager: None,
            list: None,
            state: ListState::new(),
            in_flight: BTreeMap::new(),
            workspace_request: None,
            unavailable_reported: false,
        }
    }

    /// Whether the compositor offers the window service at all.
    pub const fn is_available(&self) -> bool {
        self.manager.is_some()
    }

    /// Takes the manager the registry bound.
    pub fn bind_manager(&mut self, manager: InoxWindowManagerV1) {
        debug!("bound inox_window_manager_v1");

        self.manager = Some(manager);
    }

    /// Records that this compositor does not offer the window service, and reports
    /// whether this is the first time it was said.
    ///
    /// The panel turns the answer into [`crate::contract::ServiceState::Unavailable`] with its
    /// retry action; this only owns the log line, so the "one `info!`" rule is a
    /// field rather than a guess about how often the caller asks.
    pub fn report_unavailable(&mut self) -> bool {
        if self.unavailable_reported {
            return false;
        }

        self.unavailable_reported = true;

        info!(
            "the compositor does not offer inox-window-manager-v1; the panel keeps its clock and \
             tray and shows the window service as unavailable"
        );

        true
    }

    /// The message the panel applies when the global is missing.
    pub fn unavailable_message() -> Message {
        Message::ServiceUnavailable {
            reason: ServiceFailure::Disconnected,
        }
    }

    /// Creates a fresh list — and with it a fresh snapshot — for the panel's
    /// [`OutgoingCommand::Snapshot`].
    ///
    /// A list object *is* an observer with a snapshot: the protocol has no
    /// "resend" request, so the way to rebuild a view is to drop the observer and
    /// create another (ADR 0045 §1). Returns whether the request reached the wire;
    /// a compositor that never offered the manager cannot be asked.
    pub fn request_snapshot<D>(&mut self, qh: &QueueHandle<D>) -> bool
    where
        D: Dispatch<InoxWindowListV1, ()> + 'static,
    {
        let Some(manager) = self.manager.clone() else {
            return false;
        };

        if let Some(list) = self.list.take() {
            list.destroy();
        }

        self.state.reset();
        self.forget_requests();

        let list = manager.get_window_list(qh, ());

        debug!("created a window-list observer for a fresh snapshot");

        self.list = Some(list);

        true
    }

    /// Carries one command to the wire.
    ///
    /// A command the service cannot be asked to carry — because there is no
    /// global, or because the list is gone — is left alone: the panel's own
    /// [`crate::contract::PendingRequests`] keeps the window in flight, so the
    /// second click is already suppressed, and the leg logs rather than inventing
    /// an outcome.
    pub fn dispatch(&mut self, command: &OutgoingCommand) -> bool {
        let Some(list) = self.list.clone() else {
            debug!("no window-list observer; the panel's request has nowhere to go");

            return false;
        };

        match command {
            // A snapshot is a *list*, so it needs the queue handle and is taken
            // by [`WindowService::request_snapshot`] from the loop that has one.
            OutgoingCommand::Snapshot => false,
            OutgoingCommand::Activate { request, window } => {
                list.activate(handle_of(*window));
                self.in_flight.insert(*window, *request);

                true
            }
            OutgoingCommand::Toggle { request, window } => {
                list.minimize(handle_of(*window));
                self.in_flight.insert(*window, *request);

                true
            }
            OutgoingCommand::Close { request, window } => {
                list.close(handle_of(*window));
                self.in_flight.insert(*window, *request);

                true
            }
            OutgoingCommand::SetMode {
                request,
                window,
                mode,
            } => {
                list.set_mode(handle_of(*window), protocol_mode(*mode));
                self.in_flight.insert(*window, *request);

                true
            }
            OutgoingCommand::MoveToWorkspace {
                request,
                window,
                workspace,
            } => {
                list.move_to_workspace(handle_of(*window), workspace.get());
                self.in_flight.insert(*window, *request);

                true
            }
            OutgoingCommand::SwitchWorkspace { request, workspace } => {
                list.switch_workspace(workspace.get());
                self.workspace_request = Some(*request);

                true
            }
        }
    }

    /// Turns one list event into the message the panel applies, if it is news.
    pub fn event(&mut self, event: inox_window_list_v1::Event) -> Option<Message> {
        use inox_window_list_v1::Event;

        match event {
            Event::SessionGeneration { generation } => self.state.session_generation(generation),
            Event::WorkspaceState { active, count } => {
                Some(self.state.workspace_state(active, count))
            }
            Event::Window {
                window,
                app_id,
                title,
                workspace,
                output,
                mode,
                minimized,
                visible,
                ..
            } => {
                // The confirmation of any request this window had in flight: the
                // protocol has no acknowledgement, so the state it reports *is*
                // the outcome (ADR 0045 §1). The `focused` argument is not needed
                // here because the panel's own settling is done by
                // `Message::WindowUpdated` in `app::update`; this forgets the
                // identifier so a later `request_failed` cannot resurrect it.
                self.forget_window(handle_to_key(&self.state, window));

                self.state.window(
                    window,
                    app_id,
                    title,
                    workspace,
                    output,
                    mode_code(mode),
                    minimized != 0,
                    visible != 0,
                )
            }
            Event::WindowRemoved { window } => {
                self.forget_window(handle_to_key(&self.state, window));

                self.state.window_removed(window)
            }
            Event::SnapshotDone => {
                let message = self.state.snapshot_done();

                self.forget_requests();

                message
            }
            Event::RequestFailed { window, reason } => {
                let failure = self.state.request_failed(
                    window,
                    reason_code(reason),
                    &self.in_flight,
                    self.workspace_request,
                )?;

                match &failure {
                    Failure::Window { key, .. } => {
                        self.in_flight.remove(key);
                    }
                    Failure::Workspace { .. } => {
                        self.workspace_request = None;
                    }
                }

                Some(failure.message())
            }
            // `Event` is `#[non_exhaustive]`: a later revision may add an event,
            // which this build ignores rather than guessing at.
            _ => None,
        }
    }

    /// Forgets the request in flight for `key`, if the list announces it.
    fn forget_window(&mut self, key: Option<WindowKey>) {
        if let Some(key) = key {
            self.in_flight.remove(&key);
        }
    }

    /// Forgets every request in flight.
    fn forget_requests(&mut self) {
        self.in_flight.clear();
        self.workspace_request = None;
    }

    /// Destroys the objects this leg holds, in the order the protocol wants.
    pub fn teardown(&mut self) {
        if let Some(list) = self.list.take() {
            list.destroy();
        }

        if let Some(manager) = self.manager.take() {
            manager.destroy();
        }
    }
}

/// The raw mode number of a wire `window_mode`, known or not.
///
/// A `WEnum` keeps both halves: the value when this build knows it and the raw
/// number when a later revision added one. The panel decides what to draw from
/// the number ([`WindowMode::from_protocol`]), which is where "unknown" is turned
/// into the safe reading.
fn mode_code(mode: wayland_client::WEnum<inox_window_list_v1::WindowMode>) -> u32 {
    match mode {
        wayland_client::WEnum::Value(mode) => mode.into(),
        wayland_client::WEnum::Unknown(raw) => raw,
    }
}

/// The raw number of a wire `request_error`, known or not.
fn reason_code(reason: wayland_client::WEnum<inox_window_list_v1::RequestError>) -> u32 {
    match reason {
        wayland_client::WEnum::Value(reason) => reason.into(),
        wayland_client::WEnum::Unknown(raw) => raw,
    }
}

/// The window key of a wire handle, in the generation the list is on.
fn handle_to_key(state: &ListState, window: u32) -> Option<WindowKey> {
    let generation = state.generation()?;

    Some(WindowKey::new(WindowId::new(u64::from(window)), generation))
}

/// The wire handle of a window key.
fn handle_of(key: WindowKey) -> u32 {
    u32::try_from(key.window.get()).unwrap_or(NO_WINDOW_HANDLE)
}

/// The protocol mode for a panel mode.
fn protocol_mode(mode: WindowMode) -> inox_window_list_v1::WindowMode {
    use inox_window_list_v1::WindowMode as Wire;

    match mode {
        WindowMode::Tiled => Wire::Tiled,
        WindowMode::Floating => Wire::Floating,
        WindowMode::Maximized => Wire::Maximized,
        WindowMode::Fullscreen => Wire::Fullscreen,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::RequestId;
    use crate::tasks::WorkspaceView;

    fn list() -> ListState {
        let mut state = ListState::new();
        let _ = state.session_generation(7);

        state
    }

    fn record(list: &mut ListState, window: u32) {
        let _ = list.window(
            window,
            "app".to_owned(),
            "title".to_owned(),
            0,
            "eDP-1".to_owned(),
            WindowMode::Tiled.code(),
            false,
            true,
        );
    }

    #[test]
    fn the_generation_is_reported_once_and_is_what_keys_carry() {
        let mut list = ListState::new();

        assert_eq!(
            list.session_generation(7),
            Some(Message::ServiceReady {
                generation: SessionGeneration::new(7)
            }),
            "the generation precedes every handle, so it is its own message"
        );
        assert_eq!(
            list.session_generation(7),
            None,
            "the list answering the panel's own snapshot request is not a new session"
        );

        let _ = list.window(
            42,
            "app".to_owned(),
            "title".to_owned(),
            0,
            String::new(),
            WindowMode::Tiled.code(),
            false,
            true,
        );

        let message = list.snapshot_done().expect("a snapshot after a generation");
        let Message::Snapshot { windows, .. } = message else {
            panic!("a completed collection is a snapshot");
        };

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].0.window, WindowId::new(42));
        assert_eq!(windows[0].0.generation, SessionGeneration::new(7));
    }

    #[test]
    fn a_window_before_the_generation_is_not_decoded() {
        // A handle has no meaning before the generation that issued it, so a
        // `window` that somehow preceded it would produce no key rather than a
        // key with a default generation.
        let mut list = ListState::new();

        assert_eq!(
            list.window(
                42,
                "app".to_owned(),
                "title".to_owned(),
                0,
                String::new(),
                WindowMode::Tiled.code(),
                false,
                true,
            ),
            None
        );
        assert_eq!(list.snapshot_done(), None);
        assert_eq!(list.window_removed(42), None);
    }

    #[test]
    fn a_snapshot_is_the_windows_in_the_order_they_arrived() {
        let mut list = ListState::new();
        let _ = list.session_generation(3);
        let _ = list.workspace_state(1, 4);
        let _ = list.window(
            42,
            "org.example.Editor".to_owned(),
            "notes.txt".to_owned(),
            1,
            "eDP-1".to_owned(),
            WindowMode::Maximized.code(),
            false,
            true,
        );
        let _ = list.window(
            43,
            String::new(),
            String::new(),
            0,
            String::new(),
            WindowMode::Tiled.code(),
            true,
            false,
        );

        let message = list.snapshot_done().expect("a snapshot after a generation");

        let Message::Snapshot {
            generation,
            windows,
        } = message
        else {
            panic!("the snapshot is a Message::Snapshot, got {message:?}");
        };

        assert_eq!(generation, SessionGeneration::new(3));
        assert_eq!(windows.len(), 2);

        let (first, first_state) = &windows[0];
        assert_eq!(first.window, WindowId::new(42));
        assert_eq!(first.generation, SessionGeneration::new(3));
        assert_eq!(first_state.app_id, "org.example.Editor");
        assert_eq!(first_state.title, "notes.txt");
        assert_eq!(first_state.workspace, WorkspaceId::new(1));
        assert_eq!(first_state.output, "eDP-1".into());
        assert_eq!(first_state.mode, WindowMode::Maximized);
        assert!(first_state.maximized);
        assert!(!first_state.fullscreen);
        assert!(!first_state.minimized);

        assert_eq!(windows[1].0.window, WindowId::new(43));
        assert_eq!(windows[1].1.app_id, "", "an empty app id is a value");
        assert!(windows[1].1.minimized);
        assert_eq!(windows[1].1.output, "".into());
    }

    #[test]
    fn every_window_event_is_also_an_update() {
        // The event carries the complete state both during the snapshot and after
        // it, so the same path applies: a panel that missed an event converges on
        // the next one.
        let mut list = list();

        let message = list
            .window(
                42,
                "org.example.Editor".to_owned(),
                "notes.txt".to_owned(),
                0,
                "eDP-1".to_owned(),
                WindowMode::Tiled.code(),
                false,
                true,
            )
            .expect("a known generation");

        assert_eq!(
            message,
            Message::WindowUpdated {
                key: WindowKey::new(WindowId::new(42), SessionGeneration::new(7)),
                window: WindowState {
                    app_id: "org.example.Editor".to_owned(),
                    title: "notes.txt".to_owned(),
                    output: crate::tasks::OutputId::new("eDP-1"),
                    ..WindowState::of_mode(WindowMode::Tiled)
                },
            }
        );
    }

    #[test]
    fn a_window_mode_this_build_does_not_know_is_tiled_rather_than_wrong() {
        let mut list = list();

        let message = list
            .window(
                1,
                "app".to_owned(),
                "title".to_owned(),
                0,
                String::new(),
                99,
                false,
                true,
            )
            .expect("a known generation");

        let Message::WindowUpdated { window, .. } = message else {
            panic!("a window event is an update");
        };

        assert_eq!(window.mode, WindowMode::Tiled);
    }

    #[test]
    fn a_removed_window_is_named_in_the_generation_the_list_is_on() {
        let mut list = list();
        record(&mut list, 42);

        assert_eq!(
            list.window_removed(42),
            Some(Message::WindowRemoved {
                key: WindowKey::new(WindowId::new(42), SessionGeneration::new(7))
            })
        );
        assert!(
            list.windows().is_empty(),
            "the snapshot collection must not resurrect a window that is gone"
        );
    }

    #[test]
    fn a_workspace_report_is_a_message_and_switching_is_confirmed_by_it() {
        let mut list = list();

        assert_eq!(
            list.workspace_state(2, 4),
            Message::WorkspacesChanged {
                active: WorkspaceId::new(2),
                count: 4
            }
        );

        let reported = WorkspaceView::new(2, 4);
        assert_eq!(reported.active(), Some(WorkspaceId::new(2)));
        assert_eq!(reported.count(), Some(4));
    }

    #[test]
    fn the_three_wire_reasons_are_the_contracts_three() {
        assert_eq!(
            rejection(0),
            RejectionReason::UnknownWindow,
            "unknown_window is not exceptional: the user clicked a button for a \
             window that closed a moment ago"
        );
        assert_eq!(rejection(1), RejectionReason::NotPermitted);
        assert_eq!(
            rejection(2),
            RejectionReason::Protocol {
                detail: "unsupported".to_owned()
            },
            "the protocol has no dedicated variant, so the word is the detail"
        );

        // A value this build does not know is the service speaking a later
        // revision, which is the same shape.
        assert!(matches!(rejection(7), RejectionReason::Protocol { .. }));
    }

    #[test]
    fn a_refused_window_request_settles_its_own_identifier() {
        let list = list();
        let key = WindowKey::new(WindowId::new(42), SessionGeneration::new(7));
        let request = RequestId::new(9);
        let in_flight = BTreeMap::from([(key, request)]);

        let failure = list
            .request_failed(42, 0, &in_flight, None)
            .expect("the request is in flight");

        assert_eq!(
            failure.message(),
            Message::CommandSettled {
                request,
                outcome: CommandState::Rejected {
                    reason: RejectionReason::UnknownWindow
                },
            }
        );
    }

    #[test]
    fn a_refused_workspace_switch_is_not_a_window_command() {
        // The protocol's error event always names a window; a workspace switch
        // names none, so the compositor reports the reserved handle. That must not
        // be decoded as a window — there is no window to settle.
        let list = list();
        let request = RequestId::new(3);
        let in_flight = BTreeMap::from([(
            WindowKey::new(WindowId::new(42), SessionGeneration::new(7)),
            RequestId::new(9),
        )]);

        let failure = list
            .request_failed(NO_WINDOW_HANDLE, 1, &in_flight, Some(request))
            .expect("the workspace switch is in flight");

        assert_eq!(
            failure.message(),
            Message::WorkspaceSwitchFailed {
                reason: RejectionReason::NotPermitted
            },
            "a workspace failure is its own message: it settles the switch, not a \
             window's request"
        );
        assert_eq!(
            list.request_failed(NO_WINDOW_HANDLE, 1, &in_flight, None),
            None,
            "a failure for a switch the panel never sent changes nothing"
        );
    }

    #[test]
    fn a_failure_for_a_window_with_no_request_in_flight_changes_nothing() {
        let list = list();

        assert_eq!(
            list.request_failed(42, 0, &BTreeMap::new(), None),
            None,
            "a failure for a request the panel never sent is not ours"
        );
    }

    #[test]
    fn handles_round_trip_and_the_no_window_handle_is_reserved() {
        let key = WindowKey::new(WindowId::new(42), SessionGeneration::new(7));
        assert_eq!(handle_of(key), 42);
        assert_eq!(
            handle_of(WindowKey::new(
                WindowId::new(u64::from(NO_WINDOW_HANDLE) + 1),
                SessionGeneration::new(7)
            )),
            NO_WINDOW_HANDLE,
            "a handle too large for the wire is clamped to the reserved value, not \
             silently truncated into a different window"
        );

        for (mode, wire) in [
            (WindowMode::Tiled, inox_window_list_v1::WindowMode::Tiled),
            (
                WindowMode::Floating,
                inox_window_list_v1::WindowMode::Floating,
            ),
            (
                WindowMode::Maximized,
                inox_window_list_v1::WindowMode::Maximized,
            ),
            (
                WindowMode::Fullscreen,
                inox_window_list_v1::WindowMode::Fullscreen,
            ),
        ] {
            assert_eq!(protocol_mode(mode), wire);
        }
    }
}
