//! What the panel draws, derived from what the panel knows.
//!
//! The view is not allowed to make decisions. Every choice a user could notice —
//! which entries exist, in what order, which one is emphasized, which one is
//! greyed out **and why**, whether a retry is on offer — is made here, once, from
//! state, and then drawn. That separation is what makes D01 and D05 testable
//! without a compositor: the focus order and the disabled reasons are values, not
//! pixels.
//!
//! Three rules come from the interaction contracts of the interaction contract and the states
//! acceptance D05 asks for:
//!
//! - **A disabled entry always says why.** [`TaskButton::disabled_reason`] is the
//!   localized sentence a user reads, never a blank or a tooltip nobody can reach.
//! - **The panel-level state is explicit.** Starting, empty and unavailable are
//!   distinct values, and the unavailable one carries both a message and a retry
//!   action, so the panel can never be an endless spinner.
//! - **Long titles do not hide key actions.** Labels are bounded in the projection
//!   ([`TaskButton::label`] is trimmed to the widest the layout can show), so the
//!   clock and the tray keep their space whatever an application calls itself.

use inox_common::i18n::Localizer;
use inox_config::PanelConfig;

use crate::clock::ClockLabel;
use crate::contract::ServiceState;
use crate::tasks::{TaskList, WindowKey, WindowMode, WorkspaceId, WorkspaceView};
use crate::tray::{Tray, TrayItemId, TrayStatus};

/// The longest a label the projection produces may be, in characters.
///
/// A panel is a strip: an entry that is longer than this cannot be drawn whole
/// at any realistic panel width, so trimming here is what keeps a 400-character
/// window title from pushing the clock off the end of the strip. The layout
/// clips as well; this bound is the one the *model* guarantees, which is what a
/// test can assert on.
pub const MAX_LABEL_CHARS: usize = 64;

/// What the panel as a whole is doing (D05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelStatus {
    /// The window service has not answered yet.
    Starting,
    /// The service is up and there is nothing to show.
    Empty,
    /// There is something to show.
    Ready,
    /// The window service is not available; the panel offers a retry.
    Unavailable,
}

/// One task button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskButton {
    /// The window an activation of this button applies to: the group's focused
    /// window, or its first one when nothing of the group has focus.
    pub key: WindowKey,
    /// The application the button groups.
    pub app_id: String,
    /// How many windows the group holds.
    pub windows: usize,
    /// Label the button shows.
    pub label: String,
    /// Whether the compositor reports one of the group's windows as focused.
    pub active: bool,
    /// Whether every window of the group is minimized.
    pub minimized: bool,
    /// How the compositor is showing the window the button acts on.
    ///
    /// The button's `key` is the group's focused window, so this is the mode of
    /// *that* window: it is what the right click's cycle advances from, and what
    /// the tooltip says the window is doing.
    pub mode: WindowMode,
    /// Whether the button can be clicked.
    pub enabled: bool,
    /// Why it cannot be, in the user's language (D05). `None` when it can.
    pub disabled_reason: Option<String>,
    /// What a hover shows.
    pub tooltip: String,
}

/// One tray item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayItemView {
    /// Registration the item was drawn from.
    pub id: TrayItemId,
    /// Label the panel shows.
    pub label: String,
    /// The item's status, in the user's language.
    pub status: String,
    /// Whether the item is asking for attention.
    pub attention: bool,
    /// What a hover shows.
    pub tooltip: String,
}

/// The workspace indicator, drawn at the left of the task area.
///
/// It says which workspace is shown out of how many, and clicking it asks for
/// the next one. It is **one** entry rather than a row of one per workspace:
/// `panel.max_task_buttons` bounds the strip's controls, and a nine-workspace
/// desktop would otherwise spend the task area on workspace numbers (ADR 0045
/// §4, "工作区指示器画在任务区左侧，点击切换工作区").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceIndicator {
    /// The label the indicator shows: the active workspace of the count.
    pub label: String,
    /// The workspace currently shown.
    pub active: WorkspaceId,
    /// How many workspaces there are.
    pub count: u32,
    /// The workspace a click asks for.
    pub next: WorkspaceId,
    /// Whether the indicator can be clicked.
    pub enabled: bool,
    /// Why it cannot be, in the user's language. `None` when it can.
    pub disabled_reason: Option<String>,
    /// What a hover shows.
    pub tooltip: String,
}

/// Everything the view draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelViewModel {
    /// Panel-level state (D05).
    pub status: PanelStatus,
    /// Task buttons, in the order they are drawn and navigated.
    pub tasks: Vec<TaskButton>,
    /// The workspace indicator, when the service reports more than one
    /// workspace and the surface draws a task area.
    pub workspace: Option<WorkspaceIndicator>,
    /// Tray items, display-only.
    pub tray: Vec<TrayItemView>,
    /// The clock, when the configuration shows one and it could be rendered.
    pub clock: Option<ClockLabel>,
    /// The sentence the panel shows about itself: starting, empty or unavailable.
    pub notice: Option<String>,
    /// The label of the retry action, when the notice offers one (D05: never an
    /// endless spinner, always a way out).
    pub retry: Option<String>,
    /// The entries explicit navigation can reach, in order (D01).
    pub focus_order: Vec<WindowKey>,
    /// The entry that currently has the panel's navigation focus.
    pub focused: Option<WindowKey>,
}

impl PanelViewModel {
    /// The button for `key`, if the panel draws one.
    pub fn task(&self, key: WindowKey) -> Option<&TaskButton> {
        self.tasks.iter().find(|task| task.key == key)
    }

    /// Whether the panel has anything to show beyond its own state.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty() && self.tray.is_empty() && self.clock.is_none()
    }
}

/// The state the projection reads.
///
/// Assembled by the application (and by tests) so that the projection itself
/// takes one argument and stays obvious about what it depends on.
pub struct PanelInput<'a> {
    /// The windows of the session.
    pub tasks: &'a TaskList,
    /// The status notifier items.
    pub tray: &'a Tray,
    /// Where the window service is.
    pub service: &'a ServiceState,
    /// The rendered clock, if the panel has one.
    pub clock: Option<ClockLabel>,
    /// The panel's configuration.
    pub config: &'a PanelConfig,
    /// The entry explicit navigation is focused on, if any.
    pub focused: Option<WindowKey>,
    /// The workspaces the window service reports.
    pub workspace: WorkspaceView,
}

impl PanelInput<'_> {
    /// Projects the input into what the view draws.
    pub fn project(&self, localizer: &Localizer) -> PanelViewModel {
        let usable = self.service.accepts_commands();

        let tasks: Vec<TaskButton> = self
            .tasks
            .groups()
            .into_iter()
            // `panel.max_task_buttons` bounds what the strip shows: a panel is a
            // strip, and it does not scroll yet (the field's own default is
            // deliberately smaller than most sessions).
            .take(usize::from(self.config.max_task_buttons))
            .filter_map(|group| {
                let key = group
                    .active_window(self.tasks)
                    .or_else(|| group.windows.first().copied())?;
                let app = app_label(&group.app_id, localizer);
                let title = group.title(self.tasks);
                let active = group.is_active(self.tasks);
                let minimized = group.is_minimized(self.tasks);
                let mode = self
                    .tasks
                    .window(key)
                    .map(|state| state.mode)
                    .unwrap_or_default();

                Some(TaskButton {
                    key,
                    app_id: group.app_id.clone(),
                    windows: group.len(),
                    label: label(title, &app, group.len(), localizer),
                    active,
                    minimized,
                    mode,
                    enabled: usable,
                    disabled_reason: (!usable)
                        .then(|| localizer.message("panel-task-disabled-service")),
                    tooltip: tooltip(title, &app, mode, localizer),
                })
            })
            .collect();

        let workspace = workspace_indicator(self.workspace, usable, localizer);

        let tray: Vec<TrayItemView> = self
            .tray
            .visible()
            .map(|item| {
                let label = if item.title.is_empty() {
                    localizer.message("panel-tray-unknown-item")
                } else {
                    item.title.clone()
                };
                let status = status_label(item.status, localizer);

                TrayItemView {
                    id: item.id.clone(),
                    label: label.clone(),
                    status: status.clone(),
                    attention: item.wants_attention(),
                    tooltip: localizer.message_with(
                        "panel-tray-tooltip",
                        &[("title", label.into()), ("status", status.into())],
                    ),
                }
            })
            .collect();

        let (status, notice, retry) = self.summary(localizer, tasks.is_empty());

        // Explicit navigation reaches only buttons that can be clicked: moving
        // focus onto a disabled entry would strand it (Q01).
        let focus_order: Vec<WindowKey> = tasks
            .iter()
            .filter(|task| task.enabled)
            .map(|task| task.key)
            .collect();
        let focused = self.focused.filter(|key| focus_order.contains(key));

        PanelViewModel {
            status,
            tasks,
            workspace,
            tray,
            clock: self.clock.clone().filter(|_| self.config.show_clock),
            notice,
            retry,
            focus_order,
            focused,
        }
    }

    /// The panel's own state, its message and whether it offers a retry.
    fn summary(
        &self,
        localizer: &Localizer,
        no_tasks: bool,
    ) -> (PanelStatus, Option<String>, Option<String>) {
        let no_retry = None;

        match self.service {
            ServiceState::Connecting => (
                PanelStatus::Starting,
                Some(localizer.message("panel-state-starting")),
                no_retry,
            ),
            ServiceState::Unavailable { .. } => (
                PanelStatus::Unavailable,
                Some(localizer.message("panel-service-unavailable")),
                Some(localizer.message("panel-service-retry")),
            ),
            ServiceState::Ready { .. } if no_tasks => (
                PanelStatus::Empty,
                Some(localizer.message("panel-state-empty")),
                no_retry,
            ),
            ServiceState::Ready { .. } => (PanelStatus::Ready, None, no_retry),
        }
    }
}

/// What a hover over a task entry shows.
///
/// The application and the window's title, and — for a window the compositor is
/// not showing the ordinary way — the mode it is in. A tiled window adds nothing:
/// tiling is what a window normally does, and a tooltip that said "tiled" on
/// every button would be noise. The mode is the reason a button looks the way it
/// does, so it is the one thing worth naming (ADR 0045 §4).
fn tooltip(title: &str, app: &str, mode: WindowMode, localizer: &Localizer) -> String {
    let app = app.to_owned();

    match mode {
        WindowMode::Tiled if title.is_empty() => {
            localizer.message_with("panel-task-tooltip-empty", &[("app", app.into())])
        }
        WindowMode::Tiled => localizer.message_with(
            "panel-task-tooltip",
            &[("app", app.into()), ("title", title.into())],
        ),
        mode => {
            let mode = localizer.message(mode.message_id());

            if title.is_empty() {
                localizer.message_with(
                    "panel-task-tooltip-mode-empty",
                    &[("app", app.into()), ("mode", mode.into())],
                )
            } else {
                localizer.message_with(
                    "panel-task-tooltip-mode",
                    &[
                        ("app", app.into()),
                        ("title", title.into()),
                        ("mode", mode.into()),
                    ],
                )
            }
        }
    }
}

/// The workspace indicator, when the service reports more than one workspace.
///
/// The indicator is a control, so it obeys the same two rules the task buttons
/// do: it says why it cannot be clicked when the service is away (D05), and it
/// is drawn only when it has something to change — a one-workspace desktop has
/// no indicator, because a control that cannot change anything is not a control.
fn workspace_indicator(
    workspace: WorkspaceView,
    usable: bool,
    localizer: &Localizer,
) -> Option<WorkspaceIndicator> {
    if !workspace.is_shown() {
        return None;
    }

    let (Some(active), Some(count), Some(next)) =
        (workspace.active(), workspace.count(), workspace.next())
    else {
        return None;
    };

    // The label is the active workspace of the count, one-based as a user
    // counts them: the protocol's numbering is zero-based and that is a fact
    // about the wire, not about what a person reads.
    let label = localizer.message_with(
        "panel-workspace-indicator",
        &[
            ("active", i64::from(active.get() + 1).into()),
            ("count", i64::from(count).into()),
        ],
    );

    Some(WorkspaceIndicator {
        label: label.clone(),
        active,
        count,
        next,
        enabled: usable,
        disabled_reason: (!usable).then(|| localizer.message("panel-workspace-disabled-service")),
        tooltip: localizer.message_with(
            "panel-workspace-tooltip",
            &[
                ("active", i64::from(active.get() + 1).into()),
                ("count", i64::from(count).into()),
                ("next", i64::from(next.get() + 1).into()),
            ],
        ),
    })
}

/// The name of an application, or a localized placeholder for one that has none.
///
/// Visible to the rest of the crate because the menu bar and the dock name the
/// application too: they must not invent a second spelling for "an application
/// with no name".
pub(crate) fn app_label(app_id: &str, localizer: &Localizer) -> String {
    if app_id.is_empty() {
        localizer.message("panel-task-unknown-app")
    } else {
        app_id.to_owned()
    }
}

/// The label of a task button: the group's title, bounded, with its window count.
fn label(title: &str, app: &str, windows: usize, localizer: &Localizer) -> String {
    let title = if title.is_empty() {
        app.to_owned()
    } else {
        title.to_owned()
    };

    let composed = if windows > 1 {
        localizer.message_with(
            "panel-task-title-with-count",
            &[
                ("title", title.into()),
                ("count", format!("{windows}").into()),
            ],
        )
    } else {
        title
    };

    bounded(&composed)
}

/// Trims a label to [`MAX_LABEL_CHARS`] characters, on a character boundary.
///
/// The layout clips as well, so a label that is too long for the strip is never
/// drawn past its edge; this is the bound the *model* promises, which is the one
/// a test can assert on without a renderer.
pub fn bounded(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL_CHARS {
        return label.to_owned();
    }

    let mut trimmed: String = label
        .chars()
        .take(MAX_LABEL_CHARS.saturating_sub(1))
        .collect();
    trimmed.push('…');

    trimmed
}

/// The localized name of a tray item's status.
fn status_label(status: TrayStatus, localizer: &Localizer) -> String {
    localizer.message(match status {
        TrayStatus::Passive => "panel-tray-status-passive",
        TrayStatus::Active => "panel-tray-status-active",
        TrayStatus::NeedsAttention => "panel-tray-status-attention",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;

    use crate::i18n;
    use crate::tasks::{SessionGeneration, WindowId, WindowState};
    use crate::tray::{TrayItem, TrayItemUpdate};

    const GENERATION: SessionGeneration = SessionGeneration::new(1);

    fn localizer(tag: &str) -> Localizer {
        let locale = Locale::parse(tag).expect("a well-formed tag");

        i18n::load(&locale).expect("the catalogs parse")
    }

    fn english() -> Localizer {
        localizer("en-US")
    }

    fn key(window: u64) -> WindowKey {
        WindowKey::new(WindowId::new(window), GENERATION)
    }

    fn window(app: &str, title: &str) -> WindowState {
        WindowState {
            app_id: app.to_owned(),
            title: title.to_owned(),
            ..WindowState::default()
        }
    }

    /// Two windows of one application, one of another.
    fn list() -> TaskList {
        let mut list = TaskList::new(GENERATION);
        list.apply(key(1), window("firefox", "Inox — Documentation"));
        list.apply(key(2), window("firefox", "Inox — News"));
        let mut focused = window("terminal", "~/src");
        focused.focused = true;
        list.apply(key(3), focused);
        list
    }

    fn tray() -> Tray {
        let mut tray = Tray::new();
        tray.apply_snapshot([
            TrayItem {
                title: "Mail".to_owned(),
                ..TrayItem::new(TrayItemId::new(":1.1", "/StatusNotifierItem"))
            },
            TrayItem::new(TrayItemId::new(":1.2", "/StatusNotifierItem")),
        ]);
        tray
    }

    fn project<'a>(input: &PanelInput<'a>, localizer: &Localizer) -> PanelViewModel {
        input.project(localizer)
    }

    fn ready() -> ServiceState {
        ServiceState::Ready {
            generation: GENERATION,
            windows: 3,
        }
    }

    #[test]
    fn a_session_with_windows_draws_one_button_per_application() {
        let tasks = list();
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());

        assert_eq!(model.status, PanelStatus::Ready);
        assert_eq!(model.notice, None);
        assert_eq!(model.retry, None);
        assert_eq!(model.tasks.len(), 2);

        let firefox = &model.tasks[0];
        assert_eq!(firefox.app_id, "firefox");
        assert_eq!(firefox.windows, 2);
        assert_eq!(
            firefox.label, "Inox — Documentation (2)",
            "a group without focus is named after its first window"
        );
        assert!(firefox.enabled);
        assert_eq!(firefox.disabled_reason, None);

        let terminal = &model.tasks[1];
        assert!(terminal.active, "the focused window marks its group");
        assert_eq!(terminal.label, "~/src");
        assert!(!terminal.minimized);
        assert_eq!(terminal.tooltip, "terminal: ~/src");
    }

    #[test]
    fn a_minimized_group_is_drawn_as_minimized() {
        let mut tasks = list();
        for window in [1, 2] {
            let mut state = tasks.window(key(window)).cloned().expect("known");
            state.minimized = true;
            tasks.apply(key(window), state);
        }
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());

        assert!(model.tasks[0].minimized);
        assert!(!model.tasks[0].active);
    }

    #[test]
    fn a_ready_service_without_windows_is_the_empty_state() {
        let tasks = TaskList::new(GENERATION);
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());

        assert_eq!(model.status, PanelStatus::Empty);
        assert_eq!(model.notice.as_deref(), Some("No windows open"));
        assert_eq!(model.retry, None);
        assert!(model.tasks.is_empty());
    }

    #[test]
    fn a_service_that_has_not_answered_is_the_starting_state() {
        let tasks = TaskList::new(GENERATION);
        let tray = Tray::new();
        let config = PanelConfig::default();
        let service = ServiceState::Connecting;
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &service,
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());

        assert_eq!(model.status, PanelStatus::Starting);
        assert_eq!(
            model.notice.as_deref(),
            Some("Waiting for the window service")
        );
        assert_eq!(
            model.retry, None,
            "nothing to retry while the first answer is due"
        );
    }

    #[test]
    fn a_service_that_is_down_keeps_the_list_but_disables_it_with_a_reason() {
        let tasks = list();
        let tray = Tray::new();
        let config = PanelConfig::default();
        let service = ServiceState::Unavailable {
            reason: crate::contract::ServiceFailure::Disconnected,
        };
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &service,
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());

        assert_eq!(model.status, PanelStatus::Unavailable);
        assert_eq!(
            model.notice.as_deref(),
            Some("The window service is not available")
        );
        assert_eq!(
            model.retry.as_deref(),
            Some("Retry"),
            "the panel offers a way out (D05)"
        );
        assert_eq!(model.tasks.len(), 2, "the last known list stays visible");
        for task in &model.tasks {
            assert!(!task.enabled);
            assert_eq!(
                task.disabled_reason.as_deref(),
                Some("The window service is not available"),
                "a disabled entry says why"
            );
        }
        assert!(model.focus_order.is_empty(), "nothing navigable is offered");
    }

    #[test]
    fn a_long_title_is_bounded_and_keeps_the_rest_of_the_panel_reachable() {
        let mut tasks = TaskList::new(GENERATION);
        let long = "ただいま".repeat(200);
        tasks.apply(key(1), window("app", &long));
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());

        let label = &model.tasks[0].label;
        assert!(label.chars().count() <= MAX_LABEL_CHARS, "{label}");
        assert!(label.ends_with('…'));
        assert_eq!(
            label.chars().count(),
            MAX_LABEL_CHARS,
            "the bound is the longest a label may be"
        );
    }

    #[test]
    fn an_application_without_a_name_is_labelled_as_unknown() {
        let mut tasks = TaskList::new(GENERATION);
        tasks.apply(key(1), window("", ""));
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());
        assert_eq!(model.tasks[0].label, "Unknown application");
        assert_eq!(model.tasks[0].tooltip, "Unknown application");

        let chinese = project(&input, &localizer("zh-CN"));
        assert_eq!(chinese.tasks[0].label, "未知应用");
    }

    #[test]
    fn navigation_reaches_the_enabled_entries_only() {
        let tasks = list();
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: Some(key(3)),
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());
        assert_eq!(model.focus_order, vec![key(1), key(3)]);
        assert_eq!(model.focused, Some(key(3)));
    }

    #[test]
    fn focus_on_an_entry_that_disappeared_is_dropped() {
        let tasks = list();
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: Some(key(99)),
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());
        assert_eq!(model.focused, None, "focus never points at nothing");
    }

    #[test]
    fn tray_items_are_labelled_statused_and_tooltipped() {
        let tasks = TaskList::new(GENERATION);
        let tray = tray();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());

        assert_eq!(model.tray.len(), 2);
        assert_eq!(model.tray[0].label, "Mail");
        assert_eq!(model.tray[0].status, "active");
        assert_eq!(model.tray[0].tooltip, "Mail (active)");
        assert_eq!(
            model.tray[1].label, "Status item",
            "an item that has not reported a title still has a name"
        );
        assert!(!model.tray[1].attention);
    }

    #[test]
    fn an_item_asking_for_attention_is_marked() {
        let tasks = TaskList::new(GENERATION);
        let mut tray = tray();
        let id = TrayItemId::new(":1.1", "/StatusNotifierItem");
        tray.update(&id, TrayItemUpdate::Status(TrayStatus::NeedsAttention))
            .expect("registered");
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());
        assert!(model.tray[0].attention);
        assert_eq!(model.tray[0].status, "needs attention");
    }

    #[test]
    fn a_passive_item_is_not_drawn() {
        let tasks = TaskList::new(GENERATION);
        let mut tray = tray();
        let id = TrayItemId::new(":1.1", "/StatusNotifierItem");
        tray.update(&id, TrayItemUpdate::Status(TrayStatus::Passive))
            .expect("registered");
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());
        assert_eq!(model.tray.len(), 1);
        assert_eq!(model.tray[0].label, "Status item");
    }

    #[test]
    fn the_clock_follows_the_configuration() {
        let tasks = list();
        let tray = Tray::new();
        let clock = ClockLabel {
            text: "09:05".to_owned(),
            tooltip: "Friday, October 16 2026".to_owned(),
        };
        let showing = PanelConfig {
            show_clock: true,
            ..PanelConfig::default()
        };
        let hidden = PanelConfig {
            show_clock: false,
            ..PanelConfig::default()
        };

        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: Some(clock.clone()),
            config: &showing,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };
        assert_eq!(project(&input, &english()).clock, Some(clock));

        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: Some(ClockLabel {
                text: "09:05".to_owned(),
                tooltip: String::new(),
            }),
            config: &hidden,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };
        assert_eq!(project(&input, &english()).clock, None);
    }

    #[test]
    fn a_button_is_reachable_by_its_window() {
        let tasks = list();
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());

        assert!(model.task(key(1)).is_some());
        assert!(model.task(key(99)).is_none());
        assert!(!model.is_empty());
        assert!(
            project(
                &PanelInput {
                    tasks: &TaskList::new(GENERATION),
                    tray: &Tray::new(),
                    service: &ready(),
                    clock: None,
                    config: &config,
                    focused: None,
                    workspace: WorkspaceView::Unknown,
                },
                &english()
            )
            .is_empty()
        );
    }

    #[test]
    fn a_one_workspace_desktop_draws_no_indicator() {
        let tasks = list();
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::new(0, 1),
        };

        assert_eq!(
            project(&input, &english()).workspace,
            None,
            "a control that cannot change anything is not a control"
        );
    }

    #[test]
    fn the_indicator_says_which_workspace_is_shown_of_how_many() {
        let tasks = list();
        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::new(1, 3),
        };

        let model = project(&input, &english());
        let indicator = model.workspace.as_ref().expect("three workspaces");

        assert_eq!(
            indicator.label, "2/3",
            "a reader counts workspaces from one, the wire counts from zero"
        );
        assert_eq!(indicator.active, WorkspaceId::new(1));
        assert_eq!(indicator.count, 3);
        assert_eq!(indicator.next, WorkspaceId::new(2));
        assert!(indicator.enabled);
        assert_eq!(indicator.disabled_reason, None);
        assert_eq!(
            indicator.tooltip, "Workspace 2 of 3; click for workspace 3",
            "the tooltip says what the click will do, not what it did"
        );

        let chinese = project(&input, &localizer("zh-CN"));
        assert_eq!(
            chinese.workspace.map(|indicator| indicator.label),
            Some("2/3".to_owned()),
            "the numbers are the locale's numbers through Fluent, not a literal"
        );
    }

    #[test]
    fn a_service_that_is_down_disables_the_indicator_with_a_reason() {
        let tasks = list();
        let tray = Tray::new();
        let config = PanelConfig::default();
        let service = ServiceState::Unavailable {
            reason: crate::contract::ServiceFailure::Disconnected,
        };
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &service,
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::new(0, 4),
        };

        let model = project(&input, &english());
        let indicator = model.workspace.expect("the indicator stays drawn");

        assert!(!indicator.enabled);
        assert_eq!(
            indicator.disabled_reason.as_deref(),
            Some("The window service is not available"),
            "a disabled control says why (D05)"
        );
    }

    #[test]
    fn a_task_button_reports_the_mode_its_window_is_in() {
        let mut tasks = list();
        let mut maximized = tasks.window(key(3)).cloned().expect("known");
        maximized.mode = WindowMode::Maximized;
        maximized.maximized = true;
        tasks.apply(key(3), maximized);

        let tray = Tray::new();
        let config = PanelConfig::default();
        let input = PanelInput {
            tasks: &tasks,
            tray: &tray,
            service: &ready(),
            clock: None,
            config: &config,
            focused: None,
            workspace: WorkspaceView::Unknown,
        };

        let model = project(&input, &english());
        let terminal = model.task(key(3)).expect("the focused window's button");

        assert_eq!(
            terminal.mode,
            WindowMode::Maximized,
            "the button acts on the group's focused window, so it is that window's mode"
        );
        assert_eq!(
            model.task(key(1)).map(|task| task.mode),
            Some(WindowMode::Tiled),
            "and every other button reports its own group's mode"
        );
    }
}
