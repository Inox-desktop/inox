//! The panel application: state, messages and the Elm update loop.
//!
//! The shape is iced's and ADR 0003 makes it mandatory: a [`State`], a
//! [`Message`] per thing that can happen, one [`update`] that is the only place
//! state changes, and a [`view()`] that only reads. Nothing in `update` performs
//! I/O — it records what the UI decided and hands it to the transport that will
//! carry it, which is [`OutgoingCommand`].
//!
//! **The transport does not exist yet.** The window service the panel subscribes
//! to is reached over `inox-ipc`/D-Bus, neither of which is implemented in this
//! milestone (ADR 0018 records the scope: domain models, service contract and
//! Iced UI, no protocol legs). So the panel starts in
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
//!   dropped (ADR 0013 decision 4, acceptance D05).
//! - **Explicit navigation has one focus, and Esc unwinds it.** The focus moves
//!   over the entries the view can actually activate, wraps at both ends, and is
//!   cleared by [`Message::FocusCleared`] — the panel's own focus, which is what
//!   D01 means by "显式导航有唯一焦点".

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use chrono::{DateTime, Local};
use iced::{Element, Subscription};
use inox_common::i18n::Localizer;
use inox_config::{PanelConfig, Theme as Tokens};
use tracing::debug;

use crate::clock::{self, ClockLabel};
use crate::contract::{CommandState, PendingRequests, RequestId, ServiceFailure, ServiceState};
use crate::tasks::{Applied, SessionGeneration, TaskList, WindowKey, WindowState};
use crate::tray::{Tray, TrayItem, TrayItemId, TrayItemUpdate};
use crate::view;
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
/// The localizer is shared rather than owned because iced asks for the initial
/// state through a factory it may call more than once, and a catalog cannot be
/// cloned — it is parsed once and read for the life of the process. A plain
/// [`Rc`] is enough: the panel is one thread, and `FluentBundle` is neither
/// `Send` nor `Sync`.
pub struct State {
    localizer: Rc<Localizer>,
    tokens: Tokens,
    config: PanelConfig,
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
    /// A panel with no data yet.
    ///
    /// It starts connected to nothing, and asks for a snapshot before anything
    /// can be shown: the first thing a window service will hear from the panel
    /// is the request for its current state (ADR 0013 decision 1).
    pub fn new(config: PanelConfig, tokens: Tokens, localizer: Rc<Localizer>) -> Self {
        let mut outgoing = VecDeque::new();
        outgoing.push_back(OutgoingCommand::Snapshot);

        Self {
            localizer,
            tokens,
            config,
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

    /// The panel's configuration.
    pub fn config(&self) -> &PanelConfig {
        &self.config
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
        PanelInput {
            tasks: &self.tasks,
            tray: &self.tray,
            service: &self.service,
            clock: self.clock.clone(),
            config: &self.config,
            focused: self.focused,
        }
        .project(&self.localizer)
    }

    /// Re-renders the clock from the last reading.
    fn refresh_clock(&mut self) {
        self.clock = match (&self.last_tick, self.config.show_clock) {
            (Some(now), true) => {
                clock::label_or_none(&self.config.clock_format, now, &self.localizer)
            }
            _ => None,
        };
    }

    /// Adopts a snapshot, unless it describes a session the panel has left.
    ///
    /// ADR 0013 decision 1: a client's view only ever moves forward. A snapshot
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
    /// only ever moves forward (ADR 0013 decision 1).
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
pub fn update(state: &mut State, message: Message) -> iced::Task<Message> {
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
        Message::ConfigReloaded(config) => {
            let show_clock_changed = state.config.show_clock != config.show_clock;
            state.config = config;

            if show_clock_changed || state.clock.is_none() {
                state.refresh_clock();
            }
        }
        Message::Tick(now) => {
            state.last_tick = Some(now);
            state.refresh_clock();
        }
    }

    iced::Task::none()
}

/// Draws the panel.
pub fn view(state: &State) -> Element<'_, Message> {
    view::panel(state.model(), &state.tokens, &state.config)
}

/// The title of the panel's own surface.
pub fn title(state: &State) -> String {
    state.localizer.message("panel-window-title")
}

/// The panel's subscriptions: the clock, and nothing else yet.
///
/// The window service is reached by a transport this milestone does not have, so
/// there is no subscription for it — the panel would otherwise be listening to a
/// connection nobody opened.
pub fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::run(tick)
}

/// A stream that produces one [`Message::Tick`] a second, for as long as the
/// subscription is alive.
///
/// The thread ends when the runtime drops the receiver, which is what stops the
/// timer when the panel exits.
fn tick() -> iced::futures::channel::mpsc::Receiver<Message> {
    let (mut sender, receiver) = iced::futures::channel::mpsc::channel(1);

    std::thread::spawn(move || {
        loop {
            match sender.try_send(Message::Tick(Local::now())) {
                Ok(()) => {}
                // The runtime has not read the last tick yet; skipping this one
                // is better than blocking the timer thread.
                Err(error) if error.is_full() => {}
                Err(_) => break,
            }

            std::thread::sleep(TICK);
        }
    });

    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::RejectionReason;
    use crate::tasks::WindowId;

    /// Feeds a message to the update loop, dropping the task the runtime would
    /// run: this crate's `update` never returns anything but `Task::none()`.
    fn send(state: &mut State, message: Message) {
        let _ = update(state, message);
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
        // is still whatever the service reports (ADR 0013 decision 2).
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

    #[test]
    fn a_tick_stream_carries_tick_messages() {
        use iced::futures::StreamExt;

        let mut stream = Box::pin(tick());
        let candidate = iced::futures::executor::block_on(stream.next());

        assert!(
            matches!(candidate, Some(Message::Tick(_))),
            "the clock subscription produces readings"
        );
    }
}
