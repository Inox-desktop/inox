//! The panel application: state, messages and the Elm update loop.
//! The shape is iced's and the technology stack makes it mandatory: a [`State`], a
//! The shape is Elm's, and keep it while the toolkit moves:
//! a [`State`], a [`Message`] per thing that can happen, one [`update`] that is
//! the only place state changes, and a projection ([`crate::view::scene`]) that
//! only reads. Nothing in `update` performs I/O — it records what the UI decided
//! and hands it to the transport that will carry it, which is
//! [`OutgoingCommand`]. The widgets are QML now, which changes
//! who *draws* the state and nothing about who *owns* it.
//! milestone (the L3 component architecture records the scope: domain models, service contract and
//! **The transport does not exist yet.** The window service the panel subscribes
//! to is reached over `inox-ipc`/D-Bus, neither of which is implemented in this
//! milestone. So the panel starts in
//! [`ServiceState::Connecting`], queues exactly one
//! [`OutgoingCommand::Snapshot`], and shows the starting state until a transport
//! feeds it a [`Message::Snapshot`]. `main.rs` says so at startup, and the tests
//! below drive the same messages a transport would.
//!
//! Two behaviours are load-bearing and tested here rather than asserted in a
//! comment:
//!
//! - **A second click is not a second request.** [`PendingRequests`] holds one
//!   request per window; a click on a window that already has one in flight is
//!   dropped.
//! - **Explicit navigation has one focus, and Esc unwinds it.** The focus moves
//!   over the entries the view can actually activate, wraps at both ends, and is
//!   cleared by [`Message::FocusCleared`] — the panel's own focus, which is what
//!   D01 means by "显式导航有唯一焦点".

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use chrono::{DateTime, Local};
use inox_common::i18n::Localizer;
use inox_config::{
    DockConfig, MenubarConfig, PanelConfig, PanelSurface, ShellConfig, Theme as Tokens,
};
use tracing::debug;

use crate::clock::{self, ClockLabel};
use crate::contract::{CommandState, PendingRequests, RequestId, ServiceFailure, ServiceState};
use crate::tasks::{Applied, SessionGeneration, TaskList, WindowKey, WindowState};
use crate::tray::{Tray, TrayItem, TrayItemId, TrayItemUpdate};
use crate::view_model::{PanelInput, PanelViewModel};

/// How often the panel re-reads the clock.
///
/// One second, because `panel.clock_format` may ask for `%S` and a clock that
/// lags the second hand is wrong. A format without seconds could tick once a
/// minute; that is a saving worth making only when the idle-cost budget (P03)
/// says the wake-up matters, and it is listed as not-implemented in the README
/// rather than guessed at here.
pub const TICK: Duration = Duration::from_secs(1);

/// A command the panel asks the transport to carry to the window service.
///
/// The panel's own record of what it wants; neither the transport nor the
/// protocol exists in this milestone, so these are queued and one of them (the
/// opening snapshot) is logged at startup so that the intent is visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutgoingCommand {
    /// Ask for the current windows, as the first thing after connecting or
    /// retrying. The reply is a [`Message::Snapshot`] carrying the generation
    /// the service is on.
    Snapshot,
    /// Focus `window`, at the user's request.
    Activate {
        /// The request this command belongs to, so its outcome can be matched.
        request: RequestId,
        /// The window to focus.
        window: WindowKey,
    },
    /// Put `window` away, because the user clicked the task that already had
    /// focus.
    Toggle {
        /// The request this command belongs to.
        request: RequestId,
        /// The window to minimize.
        window: WindowKey,
    },
}

/// Everything the panel knows.
///
/// The localizer is shared rather than owned because the protocol leg, the
/// reload host and the state all read the same catalog, and a catalog cannot be
/// cloned — it is parsed once and read for the life of the process. A plain
/// [`Rc`] is enough: the panel is one thread, and `FluentBundle` is neither
/// `Send` nor `Sync`.
pub struct State {
    localizer: Rc<Localizer>,
    tokens: Tokens,
    shell: ShellConfig,
    tasks: TaskList,
    tray: Tray,
    service: ServiceState,
    clock: Option<ClockLabel>,
    last_tick: Option<DateTime<Local>>,
    focused: Option<WindowKey>,
    pending: PendingRequests,
    outgoing: VecDeque<OutgoingCommand>,
}

impl State {
    /// Applies validated preferences and catalogs without discarding tasks or tray items.
    ///
    /// This is the `[panel]` section alone. The menu bar's and the dock's own
    /// sections are adopted by [`State::apply_shell`], which is what a live
    /// reload of the whole file uses.
    pub fn apply_configuration(
        &mut self,
        config: PanelConfig,
        tokens: Tokens,
        localizer: Rc<Localizer>,
    ) {
        self.shell.panel = config;
        self.tokens = tokens;
        self.localizer = localizer;
        self.refresh_clock();
    }

    /// Adopts a reloaded `shell.toml` and a theme without discarding tasks or
    /// tray items.
    ///
    /// The surface itself cannot change while the process runs — a different
    /// surface is a different window with a different size and placement — so
    /// the caller rejects that change before getting here; everything else in
    /// the file, including the menu bar's and the dock's own sections, is
    /// applied.
    pub fn apply_shell(&mut self, shell: ShellConfig, tokens: Tokens, localizer: Rc<Localizer>) {
        self.shell = shell;
        self.tokens = tokens;
        self.localizer = localizer;
        self.refresh_clock();
    }

    /// A panel with no data yet.
    ///
    /// It starts connected to nothing, and asks for a snapshot before anything
    /// can be shown: the first thing a window service will hear from the panel
    /// is the request for its current state.
    pub fn new(config: PanelConfig, tokens: Tokens, localizer: Rc<Localizer>) -> Self {
        Self::with_shell(
            ShellConfig {
                panel: config,
                ..ShellConfig::default()
            },
            tokens,
            localizer,
        )
    }

    /// A panel of the surface `shell.panel.surface` names.
    pub fn with_shell(shell: ShellConfig, tokens: Tokens, localizer: Rc<Localizer>) -> Self {
        let mut outgoing = VecDeque::new();
        outgoing.push_back(OutgoingCommand::Snapshot);

        Self {
            localizer,
            tokens,
            shell,
            tasks: TaskList::new(SessionGeneration::default()),
            tray: Tray::new(),
            service: ServiceState::Connecting,
            clock: None,
            last_tick: None,
            focused: None,
            pending: PendingRequests::new(),
            outgoing,
        }
    }

    /// The localizer the panel renders text with.
    pub fn localizer(&self) -> &Localizer {
        &self.localizer
    }

    /// The theme tokens the panel draws with.
    pub fn tokens(&self) -> &Tokens {
        &self.tokens
    }

    /// The whole shell configuration, of which [`State::config`] is one section.
    pub fn shell(&self) -> &ShellConfig {
        &self.shell
    }

    /// The panel's configuration.
    pub fn config(&self) -> &PanelConfig {
        &self.shell.panel
    }

    /// The menu bar's configuration.
    pub fn menubar(&self) -> &MenubarConfig {
        &self.shell.menubar
    }

    /// The dock's configuration.
    pub fn dock(&self) -> &DockConfig {
        &self.shell.dock
    }

    /// Which surface this panel draws.
    pub fn surface(&self) -> PanelSurface {
        self.shell.panel.surface
    }

    /// The windows the panel currently believes in.
    pub fn tasks(&self) -> &TaskList {
        &self.tasks
    }

    /// The status notifier items.
    pub fn tray(&self) -> &Tray {
        &self.tray
    }

    /// Where the window service is.
    pub fn service(&self) -> &ServiceState {
        &self.service
    }

    /// The last rendered clock, if any.
    pub fn clock(&self) -> Option<&ClockLabel> {
        self.clock.as_ref()
    }

    /// The entry explicit navigation is focused on.
    pub fn focused(&self) -> Option<WindowKey> {
        self.focused
    }

    /// Takes the commands waiting for a transport.
    ///
    /// Whatever is returned is the caller's to send; an empty queue is the
    /// normal state of a panel nobody has clicked.
    pub fn take_outgoing(&mut self) -> Vec<OutgoingCommand> {
        self.outgoing.drain(..).collect()
    }

    /// Projects the state into what the view draws.
    pub fn model(&self) -> PanelViewModel {
        let config = self.projection_config();

        PanelInput {
            tasks: &self.tasks,
            tray: &self.tray,
            service: &self.service,
            clock: self.clock.clone(),
            config: &config,
            focused: self.focused,
        }
        .project(&self.localizer)
    }

    /// The panel configuration as *this surface* reads it.
    ///
    /// `show_clock` lives in `[panel]`, but a menu bar has one of its own and a
    /// dock has no clock at all. The shared projection is handed the surface's
    /// answer, so all three surfaces draw the same clock the same way and the
    /// model never carries a clock the surface will not show.
    fn projection_config(&self) -> PanelConfig {
        let mut config = self.shell.panel.clone();
        config.show_clock = self.shows_clock();

        config
    }

    /// Whether the surface shows a clock at all.
    fn shows_clock(&self) -> bool {
        match self.shell.panel.surface {
            PanelSurface::Taskbar => self.shell.panel.show_clock,
            PanelSurface::Menubar => self.shell.menubar.show_clock,
            PanelSurface::Dock => false,
        }
    }

    /// The clock format this surface renders with, when it shows a clock.
    fn clock_setting(&self) -> Option<&str> {
        match self.shell.panel.surface {
            PanelSurface::Taskbar if self.shell.panel.show_clock => {
                Some(self.shell.panel.clock_format.as_str())
            }
            PanelSurface::Menubar if self.shell.menubar.show_clock => {
                Some(self.shell.menubar.clock_format.as_str())
            }
            PanelSurface::Taskbar | PanelSurface::Menubar | PanelSurface::Dock => None,
        }
    }

    /// Re-renders the clock from the last reading.
    fn refresh_clock(&mut self) {
        let label = match (&self.last_tick, self.clock_setting()) {
            (Some(now), Some(format)) => clock::label_or_none(format, now, &self.localizer),
            _ => None,
        };

        self.clock = label;
    }

    /// Adopts a snapshot, unless it describes a session the panel has left.
    ///
    /// the interaction contract, rule 1: a client's view only ever moves forward. A snapshot
    /// that arrives late — from a connection that has since been replaced, say —
    /// must not roll the panel back onto a session it has already dropped, or the
    /// panel would show a dead session's windows, and send activation requests
    /// for them, while the service is serving another one. A snapshot of the
    /// generation in hand is still the service's truth and is applied: applying
    /// one again is how a view is rebuilt after events were missed.
    fn apply_snapshot(
        &mut self,
        generation: SessionGeneration,
        windows: Vec<(WindowKey, WindowState)>,
    ) {
        let current = self.tasks.generation();
        if generation < current {
            debug!(
                generation = generation.get(),
                current = current.get(),
                "dropped a snapshot from a session the panel has already left"
            );
            return;
        }

        let discarded = self.tasks.resync(generation);
        for (key, window) in windows {
            self.tasks.apply(key, window);
        }

        let count = self.tasks.len();
        self.service = ServiceState::Ready {
            generation,
            windows: count,
        };
        // A snapshot is the answer to whatever was in flight, and anything the
        // old service never answered will not be answered now.
        self.pending.clear();
        self.focused = None;
        debug!(
            generation = generation.get(),
            windows = count,
            discarded,
            "applied a window snapshot"
        );
    }

    /// Takes the window service's readiness report.
    ///
    /// The report names the session the service is serving. It carries no
    /// windows, so it never puts the panel into [`ServiceState::Ready`] — that
    /// state is entered by a snapshot, and by nothing else (see
    /// [`crate::contract`]). What it does settle is which session the panel may
    /// still believe in: a report for another generation means everything the
    /// panel holds belongs to a session that has ended, and cross-generation
    /// state is dropped rather than shown beside the new one (crate rule:
    /// 代次变化必须先 `TaskList::resync`). The panel then asks for the snapshot
    /// it is now missing — including when the report arrives after a
    /// disconnect, where the view that survived may have missed events.
    fn service_ready(&mut self, generation: SessionGeneration) {
        if self.service.generation() == Some(generation) {
            debug!(
                generation = generation.get(),
                "the window service confirmed the session already in hand"
            );
            return;
        }

        let discarded = self.tasks.resync(generation);
        self.service = ServiceState::Connecting;
        self.focused = None;
        self.pending.clear();
        self.outgoing.push_back(OutgoingCommand::Snapshot);
        debug!(
            generation = generation.get(),
            discarded,
            "the window service is on another session; dropped the old view and asked for a snapshot"
        );
    }

    /// Moves the panel's own focus `delta` entries along.
    ///
    /// Only entries the view can activate are reachable, and the movement wraps,
    /// so a keyboard user can never be left with focus on something they cannot
    /// act on (Q01).
    fn step_focus(&mut self, delta: isize) {
        let order = self.model().focus_order;

        let Some(last) = order.len().checked_sub(1) else {
            self.focused = None;
            return;
        };

        let current = self
            .focused
            .and_then(|key| order.iter().position(|candidate| *candidate == key));

        let next = match current {
            Some(index) => (index as isize + delta).rem_euclid(order.len() as isize) as usize,
            None if delta >= 0 => 0,
            None => last,
        };

        self.focused = Some(order[next]);
    }

    /// Asks the service to act on `window`, unless it already has been asked.
    fn request(&mut self, window: WindowKey, action: Action) {
        if !self.service.accepts_commands() {
            debug!(
                %window,
                "ignoring an activation: the window service is not available"
            );
            return;
        }

        let Some(request) = self.pending.begin(window) else {
            debug!(
                %window,
                "a request for this window is already in flight; not sending another"
            );
            return;
        };

        self.outgoing.push_back(match action {
            Action::Activate => OutgoingCommand::Activate { request, window },
            Action::Toggle => OutgoingCommand::Toggle { request, window },
        });
    }
}

/// What a task button asks the window service to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Bring the window forward.
    Activate,
    /// Put the window away.
    Toggle,
}

/// Everything the panel reacts to.
///
/// These are the messages a transport, a timer or a widget produces; nothing
/// else mutates [`State`].
#[derive(Debug, Clone)]
pub enum Message {
    /// The window service answered with the windows of a session generation.
    ///
    /// A snapshot of a session the panel has already left is dropped: the view
    /// only ever moves forward.
    Snapshot {
        /// The generation the service is on.
        generation: SessionGeneration,
        /// Every window it reports, with the state it is in.
        windows: Vec<(WindowKey, WindowState)>,
    },
    /// The service reported a window's state.
    WindowUpdated {
        /// The window.
        key: WindowKey,
        /// Its new state.
        window: WindowState,
    },
    /// The service reported that a window is gone.
    WindowRemoved {
        /// The window that went away.
        key: WindowKey,
    },
    /// The service answered with the complete set of status notifier items.
    TraySnapshot {
        /// The items it reports.
        items: Vec<TrayItem>,
    },
    /// A status notifier item registered.
    TrayRegistered {
        /// The item's registration.
        id: TrayItemId,
    },
    /// A status notifier item changed.
    TrayUpdated {
        /// The item's registration.
        id: TrayItemId,
        /// What changed.
        update: TrayItemUpdate,
    },
    /// A status notifier item unregistered, or its application exited.
    TrayUnregistered {
        /// The item's registration.
        id: TrayItemId,
    },
    /// The window service reported the session generation it is serving.
    ///
    /// A readiness report carries no windows, so it never makes the panel
    /// `Ready`: the panel drops whatever belonged to another session and asks
    /// for this one's snapshot.
    ServiceReady {
        /// The generation it is serving.
        generation: SessionGeneration,
    },
    /// The window service went away.
    ServiceUnavailable {
        /// Why.
        reason: ServiceFailure,
    },
    /// A request the panel sent reached an outcome.
    CommandSettled {
        /// The request.
        request: RequestId,
        /// What became of it.
        outcome: CommandState,
    },
    /// The user clicked a task button whose window is not focused.
    TaskActivated {
        /// The window to bring forward.
        key: WindowKey,
    },
    /// The user clicked the task button of the window that already has focus.
    TaskToggled {
        /// The window to put away.
        key: WindowKey,
    },
    /// Explicit navigation asked for the next entry.
    FocusNext,
    /// Explicit navigation asked for the previous entry.
    FocusPrevious,
    /// Explicit navigation was unwound: Esc.
    FocusCleared,
    /// The user asked to reach the window service again.
    Retry,
    /// The configuration was reloaded.
    ConfigReloaded(PanelConfig),
    /// The clock reached a new reading.
    Tick(DateTime<Local>),
}

/// The Elm update: the only place [`State`] changes.
pub fn update(state: &mut State, message: Message) {
    match message {
        Message::Snapshot {
            generation,
            windows,
        } => state.apply_snapshot(generation, windows),
        Message::WindowUpdated { key, window } => {
            if state.tasks.apply(key, window) == Applied::Stale {
                debug!(
                    %key,
                    "dropped a window state from another session generation"
                );
            }
        }
        Message::WindowRemoved { key } => {
            if state.tasks.remove(key).is_some() {
                state.pending.forget(key);
                debug!(%key, "a window went away");
            }
        }
        Message::TraySnapshot { items } => {
            let diff = state.tray.apply_snapshot(items);
            debug!(
                added = diff.added.len(),
                removed = diff.removed.len(),
                updated = diff.updated.len(),
                "rebuilt the status notifier items from a snapshot"
            );
        }
        Message::TrayRegistered { id } => {
            if !state.tray.register(&id) {
                debug!(%id, "the status notifier item was already registered");
            }
        }
        Message::TrayUpdated { id, update } => {
            if let Err(error) = state.tray.update(&id, update) {
                debug!(%error, "ignored a status notifier event");
            }
        }
        Message::TrayUnregistered { id } => {
            if state.tray.unregister(&id).is_none() {
                debug!(%id, "a status notifier item that was not registered left");
            }
        }
        Message::ServiceReady { generation } => state.service_ready(generation),
        Message::ServiceUnavailable { reason } => {
            debug!(?reason, "the window service is not available");
            state.service = ServiceState::Unavailable { reason };
            // Answers to requests that were in flight cannot come any more, and
            // the panel must not stay disabled waiting for them (D05).
            state.pending.clear();
        }
        Message::CommandSettled { request, outcome } => {
            if outcome.is_settled() {
                if let Some(window) = state.pending.settle(request) {
                    debug!(%window, ?outcome, "a request finished");
                }
            } else {
                debug!(
                    request = request.get(),
                    ?outcome,
                    "a request is being worked on"
                );
            }
        }
        Message::TaskActivated { key } => state.request(key, Action::Activate),
        Message::TaskToggled { key } => state.request(key, Action::Toggle),
        Message::FocusNext => state.step_focus(1),
        Message::FocusPrevious => state.step_focus(-1),
        Message::FocusCleared => {
            // Esc unwinds the panel's navigation and hands the user back to the
            // window the compositor has focused; the panel never held the real
            // keyboard focus in the first place (D01).
            state.focused = None;
        }
        Message::Retry => {
            state.service = ServiceState::Connecting;
            state.pending.clear();
            state.outgoing.push_back(OutgoingCommand::Snapshot);
        }
        Message::ConfigReloaded(mut config) => {
            // A different surface is a different window, not a live restyle, so
            // a reload of the `[panel]` section can never change it — the
            // running surface wins whatever the candidate document says.
            config.surface = state.surface();
            let show_clock_changed = state.config().show_clock != config.show_clock;
            state.shell.panel = config;

            if show_clock_changed || state.clock.is_none() {
                state.refresh_clock();
            }
        }
        Message::Tick(now) => {
            state.last_tick = Some(now);
            state.refresh_clock();
        }
    }
}

/// The vertical extent the surface draws itself in: the bar's own height, or the
/// dock's pane plus the margin below it.
///
/// The real surface is a layer surface, and `src/layer.rs` takes its size from
/// [`crate::placement::PanelPlacement`]; this is the same extent as a plain
/// number, which is what the offscreen render harness lays the widget tree out
/// at (and what the placement's `height` is derived from).
pub fn window_height(shell: &ShellConfig, tokens: &Tokens) -> f32 {
    match shell.panel.surface {
        PanelSurface::Taskbar => f32::from(shell.panel.height),
        PanelSurface::Menubar => f32::from(shell.menubar.height) + f32::from(shell.menubar.margin),
        PanelSurface::Dock => crate::dock::DockGeometry::new(tokens, &shell.dock).window_height(),
    }
}

/// The title of the panel's own surface.
pub fn title(state: &State) -> String {
    state.localizer.message("panel-window-title")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::RejectionReason;
    use crate::tasks::WindowId;

    /// Feeds a message to the update loop. `update` returns nothing: the clock
    /// is the `calloop` timer in [`crate::layer`], and no message starts
    /// asynchronous work.
    fn send(state: &mut State, message: Message) {
        update(state, message);
    }

    fn panel() -> State {
        let locale = inox_common::i18n::Locale::default_locale();

        State::new(
            PanelConfig::default(),
            Tokens::builtin_dark(),
            Rc::new(crate::i18n::load(&locale).expect("the catalogs parse")),
        )
    }

    fn generation() -> SessionGeneration {
        SessionGeneration::new(7)
    }

    fn key(window: u64) -> WindowKey {
        WindowKey::new(WindowId::new(window), generation())
    }

    fn window(app: &str, title: &str) -> WindowState {
        WindowState {
            app_id: app.to_owned(),
            title: title.to_owned(),
            ..WindowState::default()
        }
    }

    /// A panel that has just been handed a session with two applications.
    ///
    /// The opening snapshot request is taken off the queue, so a test only sees
    /// the commands its own clicks produced.
    fn connected() -> State {
        let mut state = panel();
        state.take_outgoing();
        send(
            &mut state,
            Message::Snapshot {
                generation: generation(),
                windows: vec![
                    (key(1), window("firefox", "Inox")),
                    (key(2), window("terminal", "~/src")),
                ],
            },
        );

        state
    }

    #[test]
    fn live_configuration_preserves_windows_and_refreshes_clock_theme_and_locale() {
        let mut state = connected();
        let before = state.tasks().windows().count();
        let locale = inox_common::i18n::Locale::parse("zh-CN").expect("locale");
        let mut config = state.config().clone();
        config.height = 48;
        config.show_clock = false;
        let tokens = Tokens::builtin_light();
        state.apply_configuration(
            config.clone(),
            tokens.clone(),
            Rc::new(crate::i18n::load(&locale).expect("catalog")),
        );
        assert_eq!(state.config(), &config);
        assert_eq!(state.tokens(), &tokens);
        assert_eq!(state.tasks().windows().count(), before);
        assert!(state.clock().is_none());
        assert_eq!(state.localizer().locale(), &locale);
    }

    #[test]
    fn a_new_panel_asks_for_a_snapshot_before_anything_else() {
        let mut state = panel();

        assert_eq!(state.service(), &ServiceState::Connecting);
        assert_eq!(state.take_outgoing(), vec![OutgoingCommand::Snapshot]);
        assert!(state.take_outgoing().is_empty(), "it is asked for once");
        assert_eq!(
            state.model().status,
            crate::view_model::PanelStatus::Starting
        );
    }

    #[test]
    fn a_snapshot_replaces_the_whole_view() {
        let mut state = panel();
        let old = WindowKey::new(WindowId::new(9), SessionGeneration::new(1));
        send(
            &mut state,
            Message::Snapshot {
                generation: SessionGeneration::new(1),
                windows: vec![(old, window("stale", "old session"))],
            },
        );

        send(
            &mut state,
            Message::Snapshot {
                generation: generation(),
                windows: vec![(key(1), window("firefox", "Inox"))],
            },
        );

        assert_eq!(state.tasks().len(), 1);
        assert!(!state.tasks().contains(old), "the old session is gone");
        assert_eq!(
            state.service(),
            &ServiceState::Ready {
                generation: generation(),
                windows: 1
            }
        );
    }

    #[test]
    fn a_snapshot_from_a_session_the_panel_has_left_is_dropped() {
        let mut state = connected();
        let late = WindowKey::new(WindowId::new(3), SessionGeneration::new(1));

        send(
            &mut state,
            Message::Snapshot {
                generation: SessionGeneration::new(1),
                windows: vec![(late, window("stale", "old session"))],
            },
        );

        assert_eq!(
            state.tasks().len(),
            2,
            "the newer session is still the one on screen"
        );
        assert!(!state.tasks().contains(late));
        assert_eq!(
            state.service(),
            &ServiceState::Ready {
                generation: generation(),
                windows: 2
            },
            "and is still the session the service is on"
        );
    }

    #[test]
    fn a_readiness_report_for_another_session_drops_the_old_view() {
        let mut state = connected();
        let new_session = SessionGeneration::new(9);

        send(
            &mut state,
            Message::ServiceReady {
                generation: new_session,
            },
        );

        assert_eq!(state.tasks().generation(), new_session);
        assert_eq!(
            state.tasks().len(),
            0,
            "the windows of the session that ended are gone, not shown beside it"
        );
        assert_eq!(state.service(), &ServiceState::Connecting);
        assert_eq!(
            state.take_outgoing(),
            vec![OutgoingCommand::Snapshot],
            "the panel asks for the session it now knows it is missing"
        );

        let model = state.model();
        assert_eq!(model.status, crate::view_model::PanelStatus::Starting);
        assert!(
            model.tasks.is_empty(),
            "no button of a session that ended is left clickable"
        );
    }

    #[test]
    fn a_readiness_report_alone_never_claims_a_windows_snapshot() {
        let mut state = panel();
        state.take_outgoing();

        send(
            &mut state,
            Message::ServiceReady {
                generation: SessionGeneration::new(4),
            },
        );

        // The service says which session it is on, not what is in it: a panel
        // that has been handed no windows is starting, not empty (D05).
        assert_eq!(state.service(), &ServiceState::Connecting);
        assert_eq!(
            state.model().status,
            crate::view_model::PanelStatus::Starting
        );
        assert_eq!(state.take_outgoing(), vec![OutgoingCommand::Snapshot]);
    }

    #[test]
    fn a_readiness_report_for_the_session_in_hand_confirms_it() {
        let mut state = connected();

        send(
            &mut state,
            Message::ServiceReady {
                generation: generation(),
            },
        );

        assert_eq!(
            state.service(),
            &ServiceState::Ready {
                generation: generation(),
                windows: 2
            },
            "the snapshot in hand is the session it reports"
        );
        assert!(
            state.take_outgoing().is_empty(),
            "so there is nothing to ask for again"
        );
    }

    #[test]
    fn a_window_state_from_another_session_is_dropped() {
        let mut state = connected();
        let other = WindowKey::new(WindowId::new(1), SessionGeneration::new(99));

        send(
            &mut state,
            Message::WindowUpdated {
                key: other,
                window: window("late", "another session"),
            },
        );

        assert_eq!(state.tasks().len(), 2);
        assert!(!state.tasks().contains(other));
    }

    #[test]
    fn clicking_a_task_asks_the_service_to_focus_it_once() {
        let mut state = connected();

        send(&mut state, Message::TaskActivated { key: key(1) });

        let commands = state.take_outgoing();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands[0],
            OutgoingCommand::Activate { window, .. } if window == key(1)
        ));

        // An impatient second click is not a second request (D05).
        send(&mut state, Message::TaskActivated { key: key(1) });
        assert!(state.take_outgoing().is_empty());

        // ... and a click on another window still is one.
        send(&mut state, Message::TaskActivated { key: key(2) });
        assert_eq!(state.take_outgoing().len(), 1);
    }

    #[test]
    fn a_settled_request_frees_its_window_again() {
        let mut state = connected();
        send(&mut state, Message::TaskActivated { key: key(1) });

        let request = match state.take_outgoing().as_slice() {
            [OutgoingCommand::Activate { request, .. }] => *request,
            other => panic!("expected one activation, got {other:?}"),
        };

        send(
            &mut state,
            Message::CommandSettled {
                request,
                outcome: CommandState::Applied,
            },
        );

        send(&mut state, Message::TaskActivated { key: key(1) });
        assert_eq!(
            state.take_outgoing().len(),
            1,
            "the window is free to be asked about again"
        );
    }

    #[test]
    fn an_unsettled_request_does_not_free_its_window() {
        let mut state = connected();
        send(&mut state, Message::TaskActivated { key: key(1) });
        let request = match state.take_outgoing().as_slice() {
            [OutgoingCommand::Activate { request, .. }] => *request,
            other => panic!("expected one activation, got {other:?}"),
        };

        send(
            &mut state,
            Message::CommandSettled {
                request,
                outcome: CommandState::Accepted,
            },
        );
        send(&mut state, Message::TaskActivated { key: key(1) });

        assert!(
            state.take_outgoing().is_empty(),
            "an accepted request is still in flight"
        );
    }

    #[test]
    fn a_rejected_request_is_reported_and_forgotten() {
        let mut state = connected();
        send(&mut state, Message::TaskActivated { key: key(1) });
        let request = match state.take_outgoing().as_slice() {
            [OutgoingCommand::Activate { request, .. }] => *request,
            other => panic!("expected one activation, got {other:?}"),
        };

        send(
            &mut state,
            Message::CommandSettled {
                request,
                outcome: CommandState::Rejected {
                    reason: RejectionReason::UnknownWindow,
                },
            },
        );

        // The rejection is not treated as an achieved action: the panel's truth
        // is still whatever the service reports.
        assert!(state.tasks().contains(key(1)));
    }

    #[test]
    fn a_service_that_is_down_disables_activation_and_asks_nothing() {
        let mut state = connected();
        send(
            &mut state,
            Message::ServiceUnavailable {
                reason: ServiceFailure::Disconnected,
            },
        );

        send(&mut state, Message::TaskActivated { key: key(1) });

        assert!(state.take_outgoing().is_empty());
        let model = state.model();
        assert_eq!(model.status, crate::view_model::PanelStatus::Unavailable);
        assert!(model.retry.is_some());
        assert!(model.tasks.iter().all(|task| !task.enabled));
    }

    #[test]
    fn a_retry_goes_back_to_waiting_and_asks_for_a_fresh_snapshot() {
        let mut state = connected();
        send(
            &mut state,
            Message::ServiceUnavailable {
                reason: ServiceFailure::Timeout,
            },
        );

        send(&mut state, Message::Retry);

        assert_eq!(state.service(), &ServiceState::Connecting);
        assert_eq!(state.take_outgoing(), vec![OutgoingCommand::Snapshot]);
    }

    #[test]
    fn navigation_has_one_focus_and_wraps() {
        let mut state = connected();

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focused(), Some(key(1)));
        assert_eq!(
            state.model().focused,
            Some(key(1)),
            "the entry the user is on is the one the view draws focused"
        );

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focused(), Some(key(2)));

        send(&mut state, Message::FocusNext);
        assert_eq!(state.focused(), Some(key(1)), "the last entry wraps round");

        send(&mut state, Message::FocusPrevious);
        assert_eq!(state.focused(), Some(key(2)), "and back the other way");
    }

    #[test]
    fn escape_unwinds_the_navigation() {
        let mut state = connected();
        send(&mut state, Message::FocusNext);

        send(&mut state, Message::FocusCleared);

        assert_eq!(state.focused(), None);
        assert_eq!(state.model().focused, None);
    }

    #[test]
    fn navigation_over_nothing_clears_the_focus() {
        let mut state = panel();
        send(
            &mut state,
            Message::Snapshot {
                generation: generation(),
                windows: Vec::new(),
            },
        );

        send(&mut state, Message::FocusNext);

        assert_eq!(state.focused(), None);
    }

    #[test]
    fn a_click_on_the_focused_window_puts_it_away() {
        let mut state = connected();
        let mut focused = state.tasks().window(key(2)).cloned().expect("known");
        focused.focused = true;
        send(
            &mut state,
            Message::WindowUpdated {
                key: key(2),
                window: focused,
            },
        );

        send(&mut state, Message::TaskToggled { key: key(2) });

        assert!(matches!(
            state.take_outgoing().as_slice(),
            [OutgoingCommand::Toggle { window, .. }] if *window == key(2)
        ));
    }

    #[test]
    fn an_application_exiting_takes_its_window_and_its_button() {
        let mut state = connected();

        send(&mut state, Message::WindowRemoved { key: key(1) });

        assert_eq!(state.tasks().len(), 1);
        let model = state.model();
        assert_eq!(model.tasks.len(), 1);
        assert!(model.task(key(1)).is_none());
    }

    #[test]
    fn the_clock_is_rendered_from_the_configured_format() {
        let mut state = panel();
        send(
            &mut state,
            Message::ConfigReloaded(PanelConfig {
                clock_format: "%H:%M".to_owned(),
                ..PanelConfig::default()
            }),
        );

        let now = Local::now();
        send(&mut state, Message::Tick(now));

        let clock = state.clock().expect("the clock is shown by default");
        assert_eq!(clock.text.len(), 5, "{:?}", clock.text);
        assert_eq!(clock.text.chars().nth(2), Some(':'));
        assert!(!clock.tooltip.is_empty());
    }

    #[test]
    fn a_reloaded_configuration_stops_showing_the_clock() {
        let mut state = panel();
        send(&mut state, Message::Tick(Local::now()));
        assert!(state.clock().is_some());

        send(
            &mut state,
            Message::ConfigReloaded(PanelConfig {
                show_clock: false,
                ..PanelConfig::default()
            }),
        );

        assert_eq!(state.clock(), None);
        assert_eq!(state.model().clock, None);
    }

    #[test]
    fn a_tray_item_is_drawn_and_can_leave() {
        let mut state = connected();
        let id = TrayItemId::new(":1.7", "/StatusNotifierItem");

        send(&mut state, Message::TrayRegistered { id: id.clone() });
        send(
            &mut state,
            Message::TrayUpdated {
                id: id.clone(),
                update: TrayItemUpdate::Title("Mail".to_owned()),
            },
        );

        assert_eq!(state.model().tray.len(), 1);
        assert_eq!(state.model().tray[0].label, "Mail");

        send(&mut state, Message::TrayUnregistered { id });

        assert!(state.model().tray.is_empty());
    }

    #[test]
    fn a_tray_snapshot_rebuilds_the_items_without_duplicating_them() {
        let mut state = connected();
        let items = vec![TrayItem {
            title: "Mail".to_owned(),
            ..TrayItem::new(TrayItemId::new(":1.8", "/StatusNotifierItem"))
        }];

        send(
            &mut state,
            Message::TraySnapshot {
                items: items.clone(),
            },
        );
        send(&mut state, Message::TraySnapshot { items });

        assert_eq!(state.model().tray.len(), 1);
    }

    #[test]
    fn the_title_comes_from_the_catalog() {
        let state = panel();

        assert_eq!(title(&state), "Inox Panel");
    }

    /// A panel of `mode` with the default sections.
    fn surface_panel(mode: PanelSurface) -> State {
        let locale = inox_common::i18n::Locale::default_locale();

        State::with_shell(
            ShellConfig {
                panel: PanelConfig {
                    surface: mode,
                    ..PanelConfig::default()
                },
                ..ShellConfig::default()
            },
            Tokens::builtin_dark(),
            Rc::new(crate::i18n::load(&locale).expect("the catalogs parse")),
        )
    }

    fn catalog() -> Rc<Localizer> {
        let locale = inox_common::i18n::Locale::default_locale();

        Rc::new(crate::i18n::load(&locale).expect("the catalogs parse"))
    }

    #[test]
    fn the_state_draws_the_surface_the_shell_configuration_names() {
        for mode in [
            PanelSurface::Taskbar,
            PanelSurface::Menubar,
            PanelSurface::Dock,
        ] {
            let state = surface_panel(mode);

            assert_eq!(state.surface(), mode);
            assert_eq!(state.config().surface, mode);
        }
    }

    #[test]
    fn the_three_sections_are_reachable_from_the_state() {
        let mut shell = ShellConfig::default();
        shell.menubar.height = 36;
        shell.dock.icon_size = 64;
        let state = State::with_shell(shell, Tokens::builtin_dark(), catalog());

        assert_eq!(state.menubar().height, 36);
        assert_eq!(state.dock().icon_size, 64);
        assert_eq!(state.shell().dock, state.dock().clone());
    }

    #[test]
    fn the_window_height_is_the_surfaces_own() {
        let tokens = Tokens::builtin_dark();

        let mut shell = ShellConfig::default();
        assert!(
            (window_height(&shell, &tokens) - f32::from(PanelConfig::default().height)).abs()
                < f32::EPSILON,
            "the taskbar's window is as tall as the strip"
        );

        shell.panel.surface = PanelSurface::Menubar;
        assert!(
            (window_height(&shell, &tokens)
                - f32::from(shell.menubar.height)
                - f32::from(shell.menubar.margin))
            .abs()
                < f32::EPSILON,
            "the menu bar's window is its height plus the margin above it"
        );

        shell.panel.surface = PanelSurface::Dock;
        let dock = crate::dock::DockGeometry::new(&tokens, &shell.dock);
        assert!(
            (window_height(&shell, &tokens) - dock.window_height()).abs() < f32::EPSILON,
            "the dock's window is the pane plus the margin below it"
        );
        assert!(
            window_height(&shell, &tokens) > f32::from(shell.dock.icon_size),
            "and it is tall enough for a tile"
        );
    }

    #[test]
    fn each_surface_shows_the_clock_its_own_section_asks_for() {
        let now = Local::now();

        let mut taskbar = surface_panel(PanelSurface::Taskbar);
        send(&mut taskbar, Message::Tick(now));
        assert!(taskbar.model().clock.is_some());

        let mut menubar = surface_panel(PanelSurface::Menubar);
        send(&mut menubar, Message::Tick(now));
        assert!(
            menubar.model().clock.is_some(),
            "a menu bar shows a clock by default"
        );

        // The menu bar's clock is `[menubar] show_clock`, not the panel's.
        menubar.apply_shell(
            ShellConfig {
                panel: PanelConfig {
                    surface: PanelSurface::Menubar,
                    show_clock: false,
                    ..PanelConfig::default()
                },
                menubar: MenubarConfig {
                    show_clock: true,
                    ..MenubarConfig::default()
                },
                ..ShellConfig::default()
            },
            Tokens::builtin_dark(),
            catalog(),
        );
        assert!(
            menubar.model().clock.is_some(),
            "the panel section does not govern the menu bar's clock"
        );

        // ... and switching the menu bar's own clock off hides it.
        menubar.apply_shell(
            ShellConfig {
                panel: PanelConfig {
                    surface: PanelSurface::Menubar,
                    ..PanelConfig::default()
                },
                menubar: MenubarConfig {
                    show_clock: false,
                    ..MenubarConfig::default()
                },
                ..ShellConfig::default()
            },
            Tokens::builtin_dark(),
            catalog(),
        );
        assert_eq!(menubar.model().clock, None);
        assert_eq!(menubar.clock(), None);

        let mut dock = surface_panel(PanelSurface::Dock);
        send(&mut dock, Message::Tick(now));
        assert_eq!(dock.model().clock, None, "a dock has no clock");
        assert_eq!(dock.clock(), None);
    }

    #[test]
    fn the_menu_bar_renders_the_clock_with_its_own_format() {
        let mut shell = ShellConfig::default();
        shell.panel.surface = PanelSurface::Menubar;
        shell.menubar.clock_format = "%Y".to_owned();
        let mut state = State::with_shell(shell, Tokens::builtin_dark(), catalog());

        send(&mut state, Message::Tick(Local::now()));

        let clock = state
            .clock()
            .expect("the menu bar shows a clock by default");
        assert_eq!(clock.text.len(), 4, "{:?}", clock.text);
    }

    #[test]
    fn a_reloaded_panel_section_cannot_change_the_surface() {
        let mut state = surface_panel(PanelSurface::Menubar);

        send(
            &mut state,
            Message::ConfigReloaded(PanelConfig {
                surface: PanelSurface::Dock,
                height: 48,
                ..PanelConfig::default()
            }),
        );

        assert_eq!(
            state.config().height,
            48,
            "the rest of the section is applied"
        );
        assert_eq!(
            state.surface(),
            PanelSurface::Menubar,
            "a different surface is a different window, not a live restyle"
        );
    }

    #[test]
    fn a_reloaded_panel_section_keeps_the_other_surfaces_sections() {
        let mut state = surface_panel(PanelSurface::Menubar);

        send(
            &mut state,
            Message::ConfigReloaded(PanelConfig {
                surface: PanelSurface::Menubar,
                height: 40,
                ..PanelConfig::default()
            }),
        );

        assert_eq!(state.config().height, 40);
        assert_eq!(state.menubar(), &MenubarConfig::default());
        assert_eq!(state.dock(), &DockConfig::default());
    }

    #[test]
    fn a_new_shell_configuration_reaches_the_menu_bar_and_the_dock() {
        let mut state = panel();

        state.apply_shell(
            ShellConfig {
                menubar: MenubarConfig {
                    height: 36,
                    ..MenubarConfig::default()
                },
                dock: DockConfig {
                    icon_size: 64,
                    ..DockConfig::default()
                },
                ..ShellConfig::default()
            },
            Tokens::builtin_dark(),
            catalog(),
        );

        assert_eq!(state.menubar().height, 36);
        assert_eq!(state.dock().icon_size, 64);
    }

    #[test]
    fn a_dock_panel_still_reports_the_same_tasks_and_tray_as_a_taskbar() {
        let mut dock = surface_panel(PanelSurface::Dock);
        let mut taskbar = surface_panel(PanelSurface::Taskbar);
        let windows = vec![
            (key(1), window("firefox", "Inox")),
            (key(2), window("terminal", "~/src")),
        ];

        for state in [&mut dock, &mut taskbar] {
            send(
                state,
                Message::Snapshot {
                    generation: generation(),
                    windows: windows.clone(),
                },
            );
        }

        assert_eq!(
            dock.model().tasks,
            taskbar.model().tasks,
            "the surfaces are presentations of one model, not two"
        );
        assert_eq!(dock.model().tray, taskbar.model().tray);
        assert_eq!(dock.model().status, taskbar.model().status);
        assert_eq!(dock.tasks().len(), taskbar.tasks().len());
        assert_eq!(
            dock.tray().visible().count(),
            taskbar.tray().visible().count()
        );
    }

    #[test]
    fn the_clock_ticks_once_a_second_and_a_tick_changes_the_projection() {
        // The tick is the `calloop` timer's `Message::Tick` (see
        // `crate::layer`), not a subscription. What matters here is that the
        // interval is the second hand's and that a tick is what moves it.
        assert_eq!(TICK.as_secs(), 1);

        let mut state = panel();
        let before = state.model();
        send(&mut state, Message::Tick(Local::now()));

        assert!(
            state.clock().is_some(),
            "the clock is projected after a tick"
        );
        assert!(
            state.model() != before,
            "a tick is a reason to draw, not a silent update"
        );
    }
}
