//! What the settings window draws, derived from what it knows.
//!
//! The view decides nothing. Which page is open, which fields it shows, which
//! one the keyboard is on, which actions are on offer **and why not when they
//! are not**, what the status line says and which keys it names — all of it is
//! settled here, once, from the draft and the application state, and then
//! drawn. That separation is what makes acceptance D04 and D05 testable without
//! a screen: "the draft and the in-effect state are distinguishable", "a
//! disabled action names its reason" and "an error names the field" are values
//! here, not pixels.
//!
//! Three properties are worth naming, because they are the ones a reviewer
//! should check against the acceptance scenarios:
//!
//! - **Saved and applied are different sentences.** [`StatusKind`] has a state
//!   for each, and [`StatusView`] carries the per-component results that say
//!   which consumer has answered.
//! - **A disabled action or field always says why** (D05). [`ActionView`] and
//!   [`FieldView`] carry a reason the user can read, never an empty control.
//! - **A field error names the key.** The sentence the user reads is
//!   `settings-error-field` with the field's dotted path and the validator's
//!   own message, so the key named here is the key `inox-config` would name in
//!   a startup failure.

use inox_common::i18n::{Arg, Localizer};

use crate::apply::{Application, ApplyState, Component, ComponentState, Generation, Rejection};
use crate::draft::{Draft, Field, Value, ValueKind};
use crate::pages::PageId;
use crate::validate::FieldError;

/// The longest a label the projection produces may be, in characters.
///
/// A configuration file can hold a language tag or a reason of any length, and
/// the window is a fixed size. The bound is on the *model*, which is what a
/// test can assert on; the layout clips as well.
pub const MAX_LABEL_CHARS: usize = 96;

/// Something the keyboard can be on.
///
/// The window owns its own focus, the way the panel does: the entries a user
/// can reach are a value ([`SettingsViewModel::focus_order`]) rather than a
/// property of the widget toolkit, so "one visible focus, no trap, reachable by
/// keyboard only" (acceptance Q01) is testable without a compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FocusTarget {
    /// An editable cell of a structured collection.
    Cell(Field, usize, usize),
    /// The remove action for a collection row.
    RemoveRow(Field, usize),
    /// An entry of the navigation list.
    Page(PageId),
    /// A field, which activating changes.
    Field(Field),
    /// The reset action of one field.
    Reset(Field),
    /// One of the actions at the foot of the window.
    Action(Action),
    /// One of the two answers a display trial is waiting for.
    ///
    /// A trial is a question the window asks before a display change becomes
    /// permanent, so while one is open these are the only entries the keyboard
    /// can reach.
    Trial(TrialChoice),
}

/// What the user can answer to a display trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TrialChoice {
    /// Keep the change: the compositor stops counting down.
    Confirm,
    /// Put the previous display settings back now.
    Revert,
}

impl TrialChoice {
    /// Every answer, in the order the dialog draws them.
    pub const ALL: [Self; 2] = [Self::Confirm, Self::Revert];

    /// The message id of the answer's label.
    pub const fn label_id(self) -> &'static str {
        match self {
            Self::Confirm => "settings-display-trial-confirm",
            Self::Revert => "settings-display-trial-revert",
        }
    }
}

/// An action the window offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Action {
    /// Write the changed fields and ask the components to apply them.
    Apply,
    /// Throw the draft away. Writes nothing at all.
    Cancel,
    /// Return every field to the schema's default.
    ResetAll,
    /// Read the files again, after one of them changed underneath the window.
    Reload,
}

impl Action {
    /// The message id of the action's label.
    pub const fn label_id(self) -> &'static str {
        match self {
            Self::Apply => "settings-action-apply",
            Self::Cancel => "settings-action-cancel",
            Self::ResetAll => "settings-action-reset-all",
            Self::Reload => "settings-action-reload",
        }
    }
}

/// How far the change has got, as the status line reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    /// The window holds no change.
    Unchanged,
    /// The window holds changes that have not been written.
    Modified,
    /// The files are being written.
    Saving,
    /// The files are written; the components have not all answered.
    Pending,
    /// Every component confirmed the change.
    Applied,
    /// The files are written but a component could not be asked.
    SavedOnly,
    /// The change did not take effect.
    Rejected,
    /// The change is in effect but a display trial has not been answered yet.
    ///
    /// Kept apart from [`StatusKind::Applied`] because the display trial refuses to call a
    /// mode change settled before the user confirms it: it is drawn in the
    /// warning colour, next to the dialog that can still take it back.
    Provisional,
}

/// The status line, and the lines under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusView {
    /// Which of the states above this is.
    pub kind: StatusKind,
    /// The sentence.
    pub message: String,
    /// What the components answered, or what went wrong, one line each.
    pub details: Vec<String>,
}

/// One entry of the navigation list, and the page it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageView {
    /// The page.
    pub page: PageId,
    /// The page's name, drawn as the screen's one title.
    pub label: String,
    /// What the page is for, drawn as a caption under the title.
    pub description: String,
    /// The cards the page's fields are drawn in, in order.
    pub groups: Vec<GroupView>,
    /// Whether this page is the open one.
    pub selected: bool,
    /// Whether the keyboard is on it.
    pub focused: bool,
}

/// One card of a page: a heading and the fields under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupView {
    /// The card's heading.
    pub title: String,
    /// The fields it holds, in the order it draws them.
    pub fields: Vec<Field>,
}

/// One option of a choice control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceView {
    /// The value activating it writes.
    pub id: String,
    /// What the option says.
    pub label: String,
    /// Whether it is the current value.
    pub selected: bool,
    /// Whether the keyboard is on it.
    pub focused: bool,
}

/// How a field is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlView {
    /// Structured rows with localized column labels and actions.
    Collection {
        /// Editable row values.
        rows: Vec<Vec<String>>,
        /// Labels for each column.
        columns: Vec<String>,
        /// The add-row button label.
        add: String,
        /// The remove-row button label.
        remove: String,
        /// Explanation of how saved values will be used.
        help: String,
        /// What an empty list means: the state the window is in, said in words
        /// rather than left as a blank space (ui-guidelines §6).
        empty: String,
        /// The active keyboard target within this collection.
        focused: Option<FocusTarget>,
    },
    /// Editable text, kept exactly as entered.
    Text(String),
    /// A row of options.
    Choices(Vec<ChoiceView>),
    /// A switch and the word it is showing.
    Flag {
        /// Whether it is on.
        on: bool,
        /// What it says.
        label: String,
    },
    /// A number between two limits, and the unit it is in.
    Number {
        /// The localized unit shown next to the numeric input.
        unit: String,
        /// The editable text, including incomplete input.
        input: String,
        /// The number itself.
        value: i64,
        /// Smallest the schema accepts.
        min: i64,
        /// Largest the schema accepts.
        max: i64,
        /// The number with its unit, as it is shown.
        display: String,
        /// What the button that lowers it says.
        decrease: String,
        /// What the button that raises it says.
        increase: String,
    },
}

/// One field of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldView {
    /// The field.
    pub field: Field,
    /// The field's name.
    pub label: String,
    /// How it is drawn.
    pub control: ControlView,
    /// Whether the value differs from the one the file holds.
    pub modified: bool,
    /// Whether the keyboard is on the field's own control.
    pub focused: bool,
    /// The label of the field's reset action, when it has one.
    pub reset: Option<String>,
    /// Whether the keyboard is on that action.
    pub reset_focused: bool,
    /// Why the schema refuses the value, naming the key (D04).
    pub error: Option<String>,
    /// Whether it can be changed.
    pub enabled: bool,
    /// Why it cannot be, when it cannot (D05).
    pub disabled_reason: Option<String>,
}

/// One action at the foot of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionView {
    /// The action.
    pub action: Action,
    /// Its label.
    pub label: String,
    /// Whether the keyboard is on it.
    pub focused: bool,
    /// Whether it can be taken.
    pub enabled: bool,
    /// Why it cannot be, when it cannot (D05).
    pub disabled_reason: Option<String>,
}

/// Everything the window draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsViewModel {
    /// The window's title.
    pub title: String,
    /// The navigation list.
    pub pages: Vec<PageView>,
    /// The open page.
    pub page: PageId,
    /// The open page's name.
    pub page_title: String,
    /// The fields of the open page.
    pub fields: Vec<FieldView>,
    /// The actions on offer, in the order they are drawn.
    pub actions: Vec<ActionView>,
    /// The status line.
    pub status: StatusView,
    /// The entries the keyboard can reach, in order (Q01).
    pub focus_order: Vec<FocusTarget>,
    /// The entry the keyboard is on.
    pub focused: Option<FocusTarget>,
    /// Whether a change is waiting to be written.
    pub modified: bool,
    /// The display trial the window is asking about, if one is open.
    pub trial: Option<TrialView>,
}

/// The dialog a display trial is drawn in.
///
/// The window asks a question it can answer for the user if nobody does: the
/// change is already in effect, and when [`TrialView::remaining_seconds`] reaches
/// zero the previous display settings come back. The sentence says so, because a
/// countdown the user cannot read is a race they cannot win.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialView {
    /// The request whose display change is on trial.
    pub generation: u64,
    /// The dialog's heading.
    pub title: String,
    /// What is being asked, with the time left in it.
    pub message: String,
    /// How many seconds are left before the previous setting comes back.
    pub remaining_seconds: u32,
    /// The answers, in the order the dialog draws them.
    pub choices: Vec<TrialChoiceView>,
    /// Why the last attempt to confirm failed, when one did.
    pub error: Option<String>,
}

/// One answer of the display trial dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialChoiceView {
    /// The answer.
    pub choice: TrialChoice,
    /// Its label.
    pub label: String,
    /// Whether the keyboard is on it.
    pub focused: bool,
    /// Whether it can be taken.
    pub enabled: bool,
    /// Why it cannot be, when it cannot.
    pub disabled_reason: Option<String>,
}

/// What the window last did about a display trial.
///
/// Kept as a value so the status line can say what happened to the display
/// without inventing a claim about the request's own state: a trial that was
/// confirmed leaves the request [`StatusKind::Applied`], and one that was
/// reverted goes through the ordinary save-and-apply path again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayTrialNotice {
    /// The user confirmed the change; the compositor kept it.
    Confirmed,
    /// The previous display settings were asked for and put back.
    Reverted,
    /// The previous settings were put back in the files, but the compositor
    /// could not be told — the reason is the message it failed with.
    RevertUnreachable {
        /// Why the compositor could not be asked.
        reason: String,
    },
    /// The previous settings could not even be put back in the files.
    Failed {
        /// What went wrong.
        reason: String,
    },
}

impl DisplayTrialNotice {
    /// The message id of the sentence.
    pub const fn message_id(&self) -> &'static str {
        match self {
            Self::Confirmed => "settings-display-trial-notice-confirmed",
            Self::Reverted => "settings-display-trial-notice-reverted",
            Self::RevertUnreachable { .. } => "settings-display-trial-notice-unreachable",
            Self::Failed { .. } => "settings-display-trial-notice-failed",
        }
    }
}

impl SettingsViewModel {
    /// The view of `field`, if the open page draws it.
    pub fn field(&self, field: Field) -> Option<&FieldView> {
        self.fields.iter().find(|view| view.field == field)
    }

    /// The view of `action`, if it is on offer.
    pub fn action(&self, action: Action) -> Option<&ActionView> {
        self.actions.iter().find(|view| view.action == action)
    }
}

/// What decides whether a control can be used.
///
/// Kept as one value because four questions are asked of every control — is a
/// write in flight, is a display trial waiting for an answer, does the draft
/// break the schema, is there anything to apply — and passing them as one keeps
/// the two answers (what the button draws, and whether the keyboard may land on
/// it) impossible to give inconsistently.
#[derive(Debug, Clone, Copy)]
struct Usability {
    /// A write is in flight.
    busy: bool,
    /// A display trial is open.
    trial: bool,
    /// The schema refuses something in the draft.
    invalid: bool,
    /// The draft holds a change.
    modified: bool,
}

/// The state the projection reads.
///
/// Assembled by the application so that the projection itself takes one
/// argument and stays obvious about what it depends on.
pub struct SettingsInput<'a> {
    /// The open page.
    pub page: PageId,
    /// The draft.
    pub draft: &'a Draft,
    /// How far the last request got.
    pub application: &'a Application,
    /// What the schema refuses in the draft.
    pub errors: &'a [FieldError],
    /// The entry the keyboard is on.
    pub focused: Option<FocusTarget>,
    /// The display trial waiting for an answer, if one is.
    pub trial: Option<TrialInput>,
    /// What the window last did about a display trial.
    pub notice: Option<&'a DisplayTrialNotice>,
}

/// The display trial the projection reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrialInput {
    /// The request whose display change is on trial.
    pub generation: Generation,
    /// Seconds left before the previous setting comes back.
    pub remaining_seconds: u32,
    /// Why the last attempt to confirm failed, when one did.
    pub error: Option<String>,
    /// Whether an answer is already on its way to the compositor.
    pub answering: bool,
}

impl SettingsInput<'_> {
    /// Projects the input into what the window draws.
    pub fn project(&self, localizer: &Localizer) -> SettingsViewModel {
        let state = self.application.state();
        let usability = Usability {
            busy: state.is_busy(),
            trial: self.trial.is_some(),
            invalid: !self.errors.is_empty(),
            modified: self.draft.is_modified(),
        };
        let trial = self.trial_view(localizer);

        // What the keyboard may be on does not depend on where it is, so the
        // walk is built first and the focus is then read against it: one place
        // answers where the focus is, and the walk and the controls cannot
        // disagree about it.
        let focus_order = match &trial {
            Some(trial) => trial_focus_order(trial),
            None => self.focus_order(usability),
        };
        let focused = self.focused.filter(|target| focus_order.contains(target));

        let pages: Vec<PageView> = PageId::ALL
            .iter()
            .map(|page| PageView {
                page: *page,
                label: localizer.message(page.label_id()),
                description: localizer.message(page.description_id()),
                groups: page
                    .groups()
                    .iter()
                    .map(|group| GroupView {
                        title: localizer.message(group.title_id),
                        fields: group.fields.to_vec(),
                    })
                    .collect(),
                selected: *page == self.page,
                focused: focused == Some(FocusTarget::Page(*page)),
            })
            .collect();

        let fields: Vec<FieldView> = self
            .page
            .fields()
            .iter()
            .map(|field| self.field_view(*field, localizer, usability, focused))
            .collect();

        let actions: Vec<ActionView> = self
            .action_order()
            .into_iter()
            .map(|action| self.action_view(action, localizer, usability, focused))
            .collect();

        SettingsViewModel {
            title: localizer.message("settings-window-title"),
            pages,
            page: self.page,
            page_title: localizer.message(self.page.label_id()),
            fields,
            actions,
            status: self.status_view(localizer),
            focused,
            focus_order,
            modified: usability.modified,
            trial,
        }
    }

    /// The entries the keyboard can reach on this page.
    ///
    /// The pages first, then the fields of the open page, then each modified
    /// field's reset action directly after it, then the actions that are on
    /// offer — so the walk follows the window down the screen.
    ///
    /// A control the window draws as unusable is not an entry of it. The focus
    /// is a colour drawn on the control it is on ([`crate::theme::control_colors`]), so
    /// an entry that cannot be drawn as focused would be an entry where the
    /// keyboard is on nothing at all, and an activation there would be a keystroke
    /// with no answer (Q01).
    ///
    /// The walk is therefore as long as what the window can do: while a save is
    /// in flight the page's fields are not in it, and the window draws no focus
    /// ring until the save lands. What the keyboard was on is still there — the
    /// focus is read against the walk again on the next frame, so it comes back
    /// to the same control rather than somewhere else.
    fn focus_order(&self, usability: Usability) -> Vec<FocusTarget> {
        let mut order: Vec<FocusTarget> = PageId::ALL
            .iter()
            .map(|page| FocusTarget::Page(*page))
            .collect();

        if !usability.busy {
            for field in self.page.fields() {
                if let Value::Collection(rows) = self.draft.field(*field) {
                    for (row, cells) in rows.iter().enumerate() {
                        for column in 0..cells.len() {
                            order.push(FocusTarget::Cell(*field, row, column));
                        }
                        order.push(FocusTarget::RemoveRow(*field, row));
                    }
                }
                order.push(FocusTarget::Field(*field));

                if self.draft.origin(*field).is_some() {
                    order.push(FocusTarget::Reset(*field));
                }
            }
        }

        order.extend(
            self.action_order()
                .into_iter()
                .filter(|action| {
                    action_reason(
                        *action,
                        usability.busy,
                        usability.modified
                            || (*action == Action::Apply && self.application.can_retry_apply()),
                        usability.invalid,
                    )
                    .is_none()
                })
                .map(FocusTarget::Action),
        );

        order
    }

    /// The actions the window offers right now.
    ///
    /// Reload is only there when it is the way out of the state the window is
    /// in: after the files changed underneath it (检测外部编辑冲突并提示重读).
    fn action_order(&self) -> Vec<Action> {
        let mut actions = vec![Action::Apply, Action::Cancel, Action::ResetAll];

        if matches!(
            self.application.state().rejection(),
            Some(Rejection::Conflict { .. })
        ) {
            actions.push(Action::Reload);
        }

        actions
    }

    /// One field.
    fn field_view(
        &self,
        field: Field,
        localizer: &Localizer,
        usability: Usability,
        focused: Option<FocusTarget>,
    ) -> FieldView {
        let enabled = !usability.busy && !usability.trial;
        let value = self.draft.field(field);
        let origin = self.draft.origin(field);

        FieldView {
            field,
            label: localizer.message(field.label_id()),
            control: {
                let mut control = control_view(field, &value, focused, localizer);
                if let ControlView::Number { input, .. } = &mut control {
                    *input = self.draft.number_input(field);
                }
                control
            },
            modified: origin.is_some(),
            focused: focused == Some(FocusTarget::Field(field)),
            reset: origin
                .is_some()
                .then(|| localizer.message("settings-action-reset")),
            reset_focused: focused == Some(FocusTarget::Reset(field)),
            error: self
                .errors
                .iter()
                .find(|error| error.path == field.path())
                .map(|error| {
                    localizer.message_with(
                        "settings-error-field",
                        &[
                            ("path", error.path.into()),
                            ("message", error.render(localizer).into()),
                        ],
                    )
                }),
            enabled,
            disabled_reason: (!enabled).then(|| {
                let id = if usability.trial {
                    "settings-display-trial-active"
                } else {
                    "settings-disabled-in-flight"
                };

                localizer.message(id)
            }),
        }
    }

    /// One action.
    fn action_view(
        &self,
        action: Action,
        localizer: &Localizer,
        usability: Usability,
        focused: Option<FocusTarget>,
    ) -> ActionView {
        // A display trial is a question the window is waiting on: every other
        // action would either act behind the question or write a second change
        // the user has not answered for yet.
        let reason = if usability.trial {
            Some("settings-display-trial-active")
        } else {
            action_reason(
                action,
                usability.busy,
                usability.modified
                    || (action == Action::Apply && self.application.can_retry_apply()),
                usability.invalid,
            )
        };

        ActionView {
            action,
            label: localizer.message(action.label_id()),
            focused: focused == Some(FocusTarget::Action(action)),
            enabled: reason.is_none(),
            disabled_reason: reason.map(|id| localizer.message(id)),
        }
    }

    /// The display trial dialog, when one is open.
    ///
    /// The countdown is drawn as a number of seconds rather than as a progress
    /// bar: the value the state machine decrements is exactly the value the user
    /// has to be able to read.
    fn trial_view(&self, localizer: &Localizer) -> Option<TrialView> {
        let trial = self.trial.as_ref()?;
        let choices = TrialChoice::ALL
            .iter()
            .map(|choice| {
                let enabled = !trial.answering;
                let focused = enabled && self.focused == Some(FocusTarget::Trial(*choice));

                TrialChoiceView {
                    choice: *choice,
                    label: localizer.message(choice.label_id()),
                    focused,
                    enabled,
                    disabled_reason: (!enabled)
                        .then(|| localizer.message("settings-display-trial-answering")),
                }
            })
            .collect();

        Some(TrialView {
            generation: trial.generation.get(),
            title: localizer.message("settings-display-trial-title"),
            message: localizer.message_with(
                "settings-display-trial-message",
                &[("seconds", trial.remaining_seconds.to_string().into())],
            ),
            remaining_seconds: trial.remaining_seconds,
            choices,
            error: trial.error.clone(),
        })
    }

    /// The status line.
    fn status_view(&self, localizer: &Localizer) -> StatusView {
        let state = self.application.state();
        let details: Vec<String> = self
            .application
            .results()
            .iter()
            .map(|(component, state)| {
                localizer.message_with(
                    "settings-detail-component-state",
                    &[
                        ("component", localizer.message(component.label_id()).into()),
                        (
                            "state",
                            component_state(*component, state, localizer).into(),
                        ),
                    ],
                )
            })
            .collect();

        let mut status = match state {
            ApplyState::Draft if self.draft.is_modified() => StatusView {
                kind: StatusKind::Modified,
                message: localizer.message("settings-status-modified"),
                details: Vec::new(),
            },
            ApplyState::Draft => StatusView {
                kind: StatusKind::Unchanged,
                message: localizer.message("settings-status-unchanged"),
                details: Vec::new(),
            },
            ApplyState::Saving { .. } => StatusView {
                kind: StatusKind::Saving,
                message: localizer.message("settings-status-saving"),
                details: Vec::new(),
            },
            ApplyState::Pending { .. } => StatusView {
                kind: StatusKind::Pending,
                message: localizer.message("settings-status-pending"),
                details,
            },
            ApplyState::Applied { .. } => StatusView {
                kind: StatusKind::Applied,
                message: localizer.message("settings-status-applied"),
                details,
            },
            ApplyState::SavedOnly { unreachable, .. } => StatusView {
                kind: StatusKind::SavedOnly,
                message: localizer.message_with(
                    "settings-status-saved-only",
                    &[(
                        "component",
                        // A list rather than a sentence: the names are joined the
                        // way the other component lists in this window are, and
                        // each name is itself a catalog message.
                        unreachable
                            .iter()
                            .map(|component| localizer.message(component.label_id()))
                            .collect::<Vec<String>>()
                            .join(", ")
                            .into(),
                    )],
                ),
                details,
            },
            ApplyState::Rejected { reason, .. } => StatusView {
                kind: StatusKind::Rejected,
                message: rejection_message(reason, localizer),
                details: match reason {
                    Rejection::Component {
                        component,
                        reason,
                        fields,
                    } => {
                        let mut detail = vec![
                            localizer.message_with(
                                "settings-detail-component-state",
                                &[
                                    ("component", localizer.message(component.label_id()).into()),
                                    (
                                        "state",
                                        localizer
                                            .message_with(
                                                "settings-component-state-rejected",
                                                &[("reason", reason.as_str().into())],
                                            )
                                            .into(),
                                    ),
                                ],
                            ),
                        ];

                        // The keys the consumer objects to, named the way the
                        // validator names them, so the user can find them.
                        if !fields.is_empty() {
                            detail.push(
                                localizer.message_with(
                                    "settings-detail-fields",
                                    &[(
                                        "fields",
                                        fields
                                            .iter()
                                            .map(|field| field.path())
                                            .collect::<Vec<&str>>()
                                            .join(", ")
                                            .into(),
                                    )],
                                ),
                            );
                        }

                        detail
                    }
                    Rejection::Unreachable {
                        component, reason, ..
                    } => vec![
                        localizer.message_with(
                            "settings-detail-component-state",
                            &[
                                ("component", localizer.message(component.label_id()).into()),
                                (
                                    "state",
                                    localizer
                                        .message_with(
                                            "settings-component-state-unreachable",
                                            &[("reason", reason.as_str().into())],
                                        )
                                        .into(),
                                ),
                            ],
                        ),
                    ],
                    Rejection::Conflict {
                        file,
                        current_digest,
                        ..
                    } => vec![localizer.message_with(
                        "settings-detail-conflict-digest",
                        &[
                            ("file", file.name().into()),
                            ("current", current_digest.to_string().into()),
                        ],
                    )],
                    Rejection::Reload { reason } => vec![reason.clone()],
                    Rejection::Save { file, reason } => vec![localizer.message_with(
                        "settings-detail-save-failed",
                        &[
                            ("file", file.name().into()),
                            ("reason", reason.as_str().into()),
                        ],
                    )],
                },
            },
        };

        // A change a consumer refused or could not be asked about is not stuck:
        // Apply is offered again for exactly that state, and the status line says
        // so rather than leaving the user to guess what a button would do
        // (ui-guidelines §6: 错误要可重试).
        if (status.kind == StatusKind::Rejected || status.kind == StatusKind::SavedOnly)
            && self.application.can_retry_apply()
        {
            status
                .details
                .push(localizer.message("settings-status-retry-hint"));
        }

        // What happened to the last display trial is a line of its own: it is a
        // fact about the display, not a claim about the request's own state, so
        // it is added beside the sentence rather than replacing it.
        if let Some(notice) = self.notice {
            status.details.push(match notice {
                DisplayTrialNotice::RevertUnreachable { reason }
                | DisplayTrialNotice::Failed { reason } => localizer
                    .message_with(notice.message_id(), &[("reason", reason.as_str().into())]),
                notice => localizer.message(notice.message_id()),
            });
        }

        // An unanswered trial is never reported as settled: the sentence says the
        // display is on trial, in the colour that says "still waiting".
        if self.trial.is_some() {
            status.kind = StatusKind::Provisional;
            status.message = localizer.message("settings-status-display-trial");
        }

        status
    }
}

/// The entries the keyboard can reach while a display trial is open.
///
/// Only the two answers: the trial is a question, and a field or another action
/// would let the user act behind it — the same reason those controls are drawn
/// disabled.
fn trial_focus_order(trial: &TrialView) -> Vec<FocusTarget> {
    trial
        .choices
        .iter()
        .filter(|choice| choice.enabled)
        .map(|choice| FocusTarget::Trial(choice.choice))
        .collect()
}

/// Why an action cannot be taken, when it cannot.
///
/// The one answer to both questions the window asks about an action — what the
/// button draws, and whether the keyboard may land on it — so a control can
/// never be drawn as usable and be missing from the walk, or the other way
/// round.
fn action_reason(
    action: Action,
    busy: bool,
    modified: bool,
    invalid: bool,
) -> Option<&'static str> {
    match action {
        Action::Apply if busy => Some("settings-disabled-in-flight"),
        Action::Apply if invalid => Some("settings-disabled-invalid"),
        Action::Apply if !modified => Some("settings-disabled-no-changes"),
        // A save that is on its way cannot be stopped by throwing the draft
        // away: the file keeps the change the user withdrew, and the window
        // would no longer know what the file holds (D04: 取消不写盘).
        Action::Cancel if busy => Some("settings-disabled-in-flight"),
        Action::Cancel if !modified => Some("settings-disabled-no-changes"),
        Action::ResetAll if busy => Some("settings-disabled-in-flight"),
        _ => None,
    }
}

/// How a field is drawn, from its control and its value.
///
/// Reachable from [`crate::view`]'s own tests, which draw every field of every
/// page rather than trusting that each kind of control has a drawing.
pub(crate) fn control_view(
    field: Field,
    value: &Value,
    focused: Option<FocusTarget>,
    localizer: &Localizer,
) -> ControlView {
    let control = field.control();

    match control.kind {
        ValueKind::Collection => ControlView::Collection {
            focused,
            rows: match value {
                Value::Collection(rows) => rows.clone(),
                _ => vec![],
            },
            columns: crate::collections::columns(field)
                .iter()
                .map(|id| localizer.message(id))
                .collect(),
            add: localizer.message("settings-list-add"),
            remove: localizer.message("settings-list-remove"),
            help: localizer.message(match field {
                Field::InputMethodCommand => "settings-input-method-help",
                Field::Outputs => "settings-outputs-help",
                Field::LauncherTerminal => "settings-terminal-help",
                _ => "settings-wallpapers-help",
            }),
            empty: localizer.message(empty_state_id(field)),
        },
        ValueKind::Text => ControlView::Text(match value {
            Value::Text(text) => text.clone(),
            _ => String::new(),
        }),
        ValueKind::Flag => ControlView::Flag {
            on: matches!(value, Value::Flag(true)),
            label: localizer.message(if matches!(value, Value::Flag(true)) {
                "settings-toggle-on"
            } else {
                "settings-toggle-off"
            }),
        },
        ValueKind::Number => {
            let number = match value {
                Value::Number(number) => *number,
                _ => control.range.map_or(0, |range| range.min),
            };
            let range = control.range.unwrap_or(crate::draft::Range {
                min: number,
                max: number,
                step: 1,
                unit_id: "settings-unit-px",
            });

            ControlView::Number {
                unit: localizer.message(range.unit_id),
                input: number.to_string(),
                value: number,
                min: range.min,
                max: range.max,
                display: localizer.message_with(
                    "settings-number-with-unit",
                    &[
                        ("value", Arg::from(number)),
                        ("unit", localizer.message(range.unit_id).into()),
                    ],
                ),
                decrease: localizer.message("settings-number-decrease"),
                increase: localizer.message("settings-number-increase"),
            }
        }
        ValueKind::Choice => {
            let current = value.choice_id();
            let mut options: Vec<ChoiceView> = control
                .choices
                .iter()
                .map(|choice| ChoiceView {
                    id: choice.id.to_owned(),
                    label: localizer.message(choice.label_id),
                    selected: current == Some(choice.id),
                    focused: focused == Some(FocusTarget::Field(field)),
                })
                .collect();

            // A value the control does not offer — a language tag written by
            // hand, say — is shown as it stands rather than being dropped or
            // silently replaced by one of the options. The tag is data, not
            // copy, so it is not in a catalog.
            if let Some(current) = current
                && !control.choices.iter().any(|choice| choice.id == current)
            {
                options.push(ChoiceView {
                    id: current.to_owned(),
                    label: bounded(current),
                    selected: true,
                    focused: focused == Some(FocusTarget::Field(field)),
                });
            }

            ControlView::Choices(options)
        }
    }
}

/// What an empty list says it will do, as a message id.
///
/// Kept beside the projection because it is a sentence a user reads, not a
/// drawing decision: [`crate::view`] only draws the string it is given.
fn empty_state_id(field: Field) -> &'static str {
    match field {
        Field::Outputs => "settings-empty-outputs",
        Field::InputMethodCommand => "settings-empty-input-method",
        Field::LauncherTerminal => "settings-empty-launcher-terminal",
        _ => "settings-empty-wallpaper-per-output",
    }
}

/// What one component answered, as a sentence.
fn component_state(_component: Component, state: &ComponentState, localizer: &Localizer) -> String {
    match state {
        ComponentState::Pending => localizer.message("settings-component-state-pending"),
        ComponentState::Applied => localizer.message("settings-component-state-applied"),
        ComponentState::Rejected { reason, .. } => localizer.message_with(
            "settings-component-state-rejected",
            &[("reason", reason.as_str().into())],
        ),
        ComponentState::Unreachable { reason } => localizer.message_with(
            "settings-component-state-unreachable",
            &[("reason", reason.as_str().into())],
        ),
    }
}

/// The headline of a rejection.
fn rejection_message(reason: &Rejection, localizer: &Localizer) -> String {
    match reason {
        Rejection::Save { file, .. } => localizer.message_with(
            "settings-status-save-failed",
            &[("file", file.name().into())],
        ),
        Rejection::Conflict { file, .. } => {
            localizer.message_with("settings-status-conflict", &[("file", file.name().into())])
        }
        Rejection::Component { component, .. } => localizer.message_with(
            "settings-status-component-rejected",
            &[("component", localizer.message(component.label_id()).into())],
        ),
        Rejection::Unreachable { component, .. } => localizer.message_with(
            "settings-status-unreachable",
            &[("component", localizer.message(component.label_id()).into())],
        ),
        Rejection::Reload { .. } => localizer.message("settings-status-reload-failed"),
    }
}

/// Trims a label to [`MAX_LABEL_CHARS`] characters, on a character boundary.
pub fn bounded(label: &str) -> String {
    if label.chars().count() <= MAX_LABEL_CHARS {
        return label.to_owned();
    }

    let mut trimmed: String = label.chars().take(MAX_LABEL_CHARS - 1).collect();
    trimmed.push('…');

    trimmed
}

/// Every message id the projection can ask for.
///
/// `tests/catalog.rs` walks this, so an id the window can show but a catalog
/// does not define fails there rather than rendering as a raw id in front of a
/// user (acceptance Q03).
pub fn message_ids() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = vec![
        "settings-window-title",
        "settings-toggle-on",
        "settings-toggle-off",
        "settings-number-with-unit",
        "settings-number-decrease",
        "settings-number-increase",
        "settings-error-field",
        "settings-action-apply",
        "settings-action-cancel",
        "settings-action-reset",
        "settings-action-reset-all",
        "settings-action-reload",
        "settings-disabled-in-flight",
        "settings-disabled-invalid",
        "settings-disabled-no-changes",
        "settings-status-unchanged",
        "settings-status-modified",
        "settings-status-saving",
        "settings-status-pending",
        "settings-status-applied",
        "settings-status-saved-only",
        "settings-status-conflict",
        "settings-status-save-failed",
        "settings-status-reload-failed",
        "settings-status-component-rejected",
        "settings-status-unreachable",
        "settings-status-retry-hint",
        "settings-detail-component-state",
        "settings-detail-conflict-digest",
        "settings-detail-save-failed",
        "settings-detail-fields",
        "settings-component-state-pending",
        "settings-component-state-applied",
        "settings-component-state-rejected",
        "settings-component-state-unreachable",
        "settings-error-height-range",
        "settings-error-timeout-range",
        "settings-error-locale",
        "settings-input-method-help",
        "settings-error-collection",
        "settings-error-input-method-command",
        "settings-error-wallpaper-entry",
        "settings-output-name",
        "settings-output-width",
        "settings-output-height",
        "settings-output-refresh",
        "settings-output-scale",
        "settings-output-x",
        "settings-output-y",
        "settings-terminal-argument",
        "settings-wallpaper-path",
        "settings-list-add",
        "settings-list-remove",
        "settings-outputs-help",
        "settings-terminal-help",
        "settings-wallpapers-help",
        "settings-empty-outputs",
        "settings-empty-input-method",
        "settings-empty-launcher-terminal",
        "settings-empty-wallpaper-per-output",
        "settings-error-integer",
        "settings-error-number-range",
        "settings-error-clock-format",
        "settings-status-display-trial",
        "settings-display-trial-title",
        "settings-display-trial-message",
        "settings-display-trial-confirm",
        "settings-display-trial-revert",
        "settings-display-trial-active",
        "settings-display-trial-answering",
        "settings-display-trial-notice-confirmed",
        "settings-display-trial-notice-reverted",
        "settings-display-trial-notice-unreachable",
        "settings-display-trial-notice-failed",
    ];

    for page in PageId::ALL {
        ids.extend(page.message_ids());
    }

    for component in Component::ALL {
        ids.push(component.label_id());
    }

    for action in [
        Action::Apply,
        Action::Cancel,
        Action::ResetAll,
        Action::Reload,
    ] {
        ids.push(action.label_id());
    }

    for choice in TrialChoice::ALL {
        ids.push(choice.label_id());
    }

    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use crate::apply::Generation;
    use crate::draft::{ENVIRONMENT, Settings};
    use inox_common::i18n::Locale;

    fn localizer() -> Localizer {
        crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse")
    }

    fn draft() -> Draft {
        Draft::new(Settings::defaults())
    }

    fn project(
        page: PageId,
        draft: &Draft,
        application: &Application,
        focused: Option<FocusTarget>,
    ) -> SettingsViewModel {
        let errors = crate::validate::validate(draft.settings());

        SettingsInput {
            page,
            draft,
            application,
            errors: &errors,
            focused,
            trial: None,
            notice: None,
        }
        .project(&localizer())
    }

    /// The same projection, with a display trial waiting for an answer.
    fn project_trial(
        page: PageId,
        draft: &Draft,
        application: &Application,
        focused: Option<FocusTarget>,
        remaining_seconds: u32,
        answering: bool,
    ) -> SettingsViewModel {
        let errors = crate::validate::validate(draft.settings());

        SettingsInput {
            page,
            draft,
            application,
            errors: &errors,
            focused,
            trial: Some(TrialInput {
                generation: Generation::new(7),
                remaining_seconds,
                error: None,
                answering,
            }),
            notice: None,
        }
        .project(&localizer())
    }

    fn open(page: PageId) -> SettingsViewModel {
        project(page, &draft(), &Application::new(), None)
    }

    #[test]
    fn every_page_is_in_the_navigation_list_and_one_of_them_is_open() {
        let model = open(PageId::Panel);

        assert_eq!(model.pages.len(), PageId::ALL.len());
        assert_eq!(model.page, PageId::Panel);
        assert_eq!(model.page_title, "Panel");

        let selected: Vec<PageId> = model
            .pages
            .iter()
            .filter(|page| page.selected)
            .map(|page| page.page)
            .collect();
        assert_eq!(selected, vec![PageId::Panel]);
    }

    #[test]
    fn a_page_draws_its_own_fields_and_no_others() {
        for page in PageId::ALL {
            let model = open(page);

            assert_eq!(model.fields.len(), page.fields().len(), "{page:?}");

            for (field, view) in page.fields().iter().zip(&model.fields) {
                assert_eq!(view.field, *field);
                assert!(!view.label.is_empty());
            }
        }
    }

    #[test]
    fn the_actions_are_offered_with_their_reasons() {
        let model = open(PageId::Panel);

        let apply = model.action(Action::Apply).expect("Apply is offered");
        assert!(!apply.enabled);
        assert_eq!(
            apply.disabled_reason.as_deref(),
            Some("Nothing has been changed yet")
        );

        let cancel = model.action(Action::Cancel).expect("Cancel is offered");
        assert!(!cancel.enabled, "there is nothing to cancel");
        assert!(cancel.disabled_reason.is_some());

        assert!(
            model
                .action(Action::ResetAll)
                .expect("Reset is offered")
                .enabled
        );
        assert!(
            model.action(Action::Reload).is_none(),
            "the files have not changed underneath the window"
        );
    }

    #[test]
    fn a_change_turns_apply_on() {
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let model = project(PageId::Panel, &draft, &Application::new(), None);

        assert!(model.modified);
        assert!(model.action(Action::Apply).expect("Apply").enabled);
        assert!(model.action(Action::Cancel).expect("Cancel").enabled);
        assert_eq!(
            model.status.kind,
            StatusKind::Modified,
            "the draft has unwritten changes"
        );
    }

    #[test]
    fn only_the_fields_the_user_changed_are_marked_as_changes() {
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let model = project(PageId::Panel, &draft, &Application::new(), None);

        let modified: Vec<Field> = model
            .fields
            .iter()
            .filter(|field| field.modified)
            .map(|field| field.field)
            .collect();
        assert_eq!(modified, vec![Field::PanelHeight]);

        let with_reset: Vec<Field> = model
            .fields
            .iter()
            .filter(|field| field.reset.is_some())
            .map(|field| field.field)
            .collect();
        assert_eq!(with_reset, vec![Field::PanelHeight]);
    }

    #[test]
    fn every_field_the_schema_refuses_carries_a_sentence_that_names_it() {
        let mut settings = Settings::defaults();
        settings.panel_height = 9_000;
        let draft = Draft::new(settings);

        let model = project(PageId::Panel, &draft, &Application::new(), None);
        let field = model.field(Field::PanelHeight).expect("the height");

        let error = field.error.as_deref().expect("the value is out of range");
        assert!(error.contains("panel.height"), "sentence `{error}`");
        assert!(error.contains("256"), "sentence `{error}`");

        assert!(
            !model.action(Action::Apply).expect("Apply").enabled,
            "an invalid draft cannot be applied"
        );
        assert_eq!(
            model
                .action(Action::Apply)
                .expect("Apply")
                .disabled_reason
                .as_deref(),
            Some("Some fields need fixing first")
        );
    }

    #[test]
    fn a_field_with_no_error_carries_no_sentence() {
        let model = open(PageId::Panel);

        for field in &model.fields {
            assert_eq!(field.error, None, "`{}`", field.field);
        }
    }

    #[test]
    fn a_choice_control_shows_which_option_is_current() {
        let model = project(PageId::Appearance, &draft(), &Application::new(), None);

        let Some(ControlView::Choices(options)) = model
            .field(Field::ThemeAppearance)
            .map(|field| &field.control)
        else {
            panic!("the appearance is a choice");
        };

        assert_eq!(options.len(), 3);
        let selected: Vec<&str> = options
            .iter()
            .filter(|option| option.selected)
            .map(|option| option.id.as_str())
            .collect();
        assert_eq!(selected, vec!["system"], "the schema's default");
    }

    #[test]
    fn a_language_tag_the_control_does_not_offer_is_shown_as_it_stands() {
        let mut settings = Settings::defaults();
        settings.locale = crate::draft::LocaleChoice::Tag("fr-FR".to_owned());
        let draft = Draft::new(settings);

        let model = project(PageId::Locale, &draft, &Application::new(), None);
        let Some(ControlView::Choices(options)) = model
            .field(Field::LocaleLanguage)
            .map(|field| &field.control)
        else {
            panic!("the language is a choice");
        };

        assert_eq!(
            options.len(),
            4,
            "the three Inox ships and the file's own tag"
        );
        assert_eq!(options[3].id, "fr-FR");
        assert_eq!(options[3].label, "fr-FR");
        assert!(options[3].selected);
        assert!(options[..3].iter().all(|option| !option.selected));
    }

    #[test]
    fn the_wallpaper_chooser_offers_the_catalogue_and_selects_the_default() {
        let model = project(PageId::Wallpaper, &draft(), &Application::new(), None);

        let Some(ControlView::Choices(options)) = model
            .field(Field::WallpaperImage)
            .map(|field| &field.control)
        else {
            panic!("the wallpaper is a choice");
        };

        let ids: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();
        let mut expected: Vec<&str> = vec![""];
        expected.extend(
            inox_config::wallpapers::BUNDLED
                .iter()
                .map(|wallpaper| wallpaper.id),
        );
        expected.push(inox_config::BACKGROUND_ID);

        assert_eq!(ids, expected, "the chooser is the catalogue in its order");
        let selected: Vec<&str> = options
            .iter()
            .filter(|option| option.selected)
            .map(|option| option.id.as_str())
            .collect();
        assert_eq!(
            selected,
            vec![""],
            "a configuration that names nothing shows the default option"
        );
    }

    #[test]
    fn a_wallpaper_path_written_by_hand_is_shown_as_it_stands() {
        let mut settings = Settings::defaults();
        settings.wallpaper_image = "/images/主壁纸.png".to_owned();

        let model = project(
            PageId::Wallpaper,
            &Draft::new(settings),
            &Application::new(),
            None,
        );
        let Some(ControlView::Choices(options)) = model
            .field(Field::WallpaperImage)
            .map(|field| &field.control)
        else {
            panic!("the wallpaper is a choice");
        };

        let last = options.last().expect("the appended option");
        assert_eq!(last.id, "/images/主壁纸.png");
        assert_eq!(last.label, "/images/主壁纸.png", "a path is data, not copy");
        assert!(last.selected);
        assert_eq!(
            options.iter().filter(|option| option.selected).count(),
            1,
            "exactly one option is the current one"
        );
    }

    #[test]
    fn a_switch_control_says_which_way_it_is() {
        let mut draft = draft();
        draft
            .set(Field::PanelAutohide, Value::Flag(true))
            .expect("a switch");

        let model = project(PageId::Panel, &draft, &Application::new(), None);
        let field = model.field(Field::PanelAutohide).expect("the switch");

        assert_eq!(
            field.control,
            ControlView::Flag {
                on: true,
                label: "On".to_owned(),
            }
        );
    }

    #[test]
    fn a_number_control_shows_the_value_with_its_unit() {
        let model = open(PageId::Panel);
        let field = model.field(Field::PanelHeight).expect("the height");

        match &field.control {
            ControlView::Number {
                value,
                min,
                max,
                display,
                decrease,
                increase,
                ..
            } => {
                assert_eq!(*value, 32);
                assert_eq!((*min, *max), (8, 256));
                assert!(display.contains("px"), "display `{display}`");
                assert!(!decrease.is_empty() && !increase.is_empty());
            }
            other => panic!("expected a number, got {other:?}"),
        }
    }

    #[test]
    fn the_keyboard_walks_the_window_from_the_top_down() {
        // Nothing has been changed, so Apply and Cancel have nothing to do and
        // are not entries of the walk; Reset still has.
        let model = open(PageId::Panel);

        let expected: Vec<FocusTarget> = PageId::ALL
            .iter()
            .map(|page| FocusTarget::Page(*page))
            .chain(
                PageId::Panel
                    .fields()
                    .iter()
                    .map(|field| FocusTarget::Field(*field)),
            )
            .chain([FocusTarget::Action(Action::ResetAll)])
            .collect();

        assert_eq!(model.focus_order, expected);

        // A change turns the two of them into entries, after the fields.
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let model = project(PageId::Panel, &draft, &Application::new(), None);
        let actions: Vec<FocusTarget> = model
            .focus_order
            .iter()
            .filter(|target| matches!(target, FocusTarget::Action(_)))
            .copied()
            .collect();

        assert_eq!(
            actions,
            vec![
                FocusTarget::Action(Action::Apply),
                FocusTarget::Action(Action::Cancel),
                FocusTarget::Action(Action::ResetAll),
            ],
            "the actions the window draws as usable are the ones after the fields"
        );
    }

    #[test]
    fn the_walk_holds_only_controls_the_window_draws_as_usable() {
        // The focus is a colour on the control it is on, so anything the walk
        // could land on that is drawn as unusable would leave the keyboard on
        // nothing (Q01). The three states a reviewer names: a window nobody has
        // touched, one holding a change, and one whose save is in flight.
        let mut changed = draft();
        changed
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let mut busy = Application::new();
        let files = BTreeSet::from([crate::document::SettingFile::Shell]);
        let _ = busy
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");

        for (state, model) in [
            ("untouched", open(PageId::Panel)),
            (
                "changed",
                project(PageId::Panel, &changed, &Application::new(), None),
            ),
            ("in flight", project(PageId::Panel, &draft(), &busy, None)),
        ] {
            for target in &model.focus_order {
                match target {
                    FocusTarget::Cell(field, _, _) | FocusTarget::RemoveRow(field, _) => {
                        assert!(model.field(*field).expect("collection field").enabled)
                    }
                    FocusTarget::Page(page) => assert!(
                        model.pages.iter().any(|view| view.page == *page),
                        "{state}: the walk holds a page that is not drawn"
                    ),
                    FocusTarget::Field(field) => assert!(
                        model
                            .field(*field)
                            .expect("a field of the open page")
                            .enabled,
                        "{state}: the walk holds `{field}`, which is disabled"
                    ),
                    FocusTarget::Reset(field) => assert!(
                        model
                            .field(*field)
                            .expect("a field of the open page")
                            .enabled,
                        "{state}: the walk holds the reset of `{field}`, which is disabled"
                    ),
                    FocusTarget::Action(action) => assert!(
                        model.action(*action).expect("an action on offer").enabled,
                        "{state}: the walk holds {action:?}, which is disabled"
                    ),
                    FocusTarget::Trial(choice) => assert!(
                        model
                            .trial
                            .as_ref()
                            .expect("a trial dialog")
                            .choices
                            .iter()
                            .any(|view| view.choice == *choice && view.enabled),
                        "{state}: the walk holds {choice:?}, which is disabled"
                    ),
                }
            }

            // And the other way round: every field the page draws as usable is
            // one the keyboard can reach.
            for field in &model.fields {
                assert_eq!(
                    field.enabled,
                    model.focus_order.contains(&FocusTarget::Field(field.field)),
                    "{state}: `{}` is {} but is {} the walk",
                    field.field,
                    if field.enabled { "usable" } else { "disabled" },
                    if field.enabled { "not in" } else { "in" }
                );
            }

            // The focus the walk holds is the focus the window draws, and the
            // other way round.
            let drawn: Vec<FocusTarget> = model
                .fields
                .iter()
                .flat_map(|field| {
                    [
                        (field.focused, FocusTarget::Field(field.field)),
                        (field.reset_focused, FocusTarget::Reset(field.field)),
                    ]
                })
                .filter(|(focused, _)| *focused)
                .map(|(_, target)| target)
                .chain(
                    model
                        .actions
                        .iter()
                        .filter(|action| action.focused)
                        .map(|action| FocusTarget::Action(action.action)),
                )
                .chain(
                    model
                        .pages
                        .iter()
                        .filter(|page| page.focused)
                        .map(|page| FocusTarget::Page(page.page)),
                )
                .collect();

            assert_eq!(
                drawn,
                model.focused.into_iter().collect::<Vec<FocusTarget>>(),
                "{state}: the window draws a focus the walk does not hold"
            );
        }
    }

    #[test]
    fn cancelling_is_not_offered_while_a_save_is_in_flight() {
        // Cancel writes nothing, and a write that is already on its way cannot be
        // called back: the honest answer is that the action is not available,
        // with the reason named (D04, D05).
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let modified = project(PageId::Panel, &draft, &Application::new(), None);
        assert!(modified.action(Action::Cancel).expect("Cancel").enabled);

        let mut application = Application::new();
        let files = BTreeSet::from([crate::document::SettingFile::Shell]);
        let _ = application
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");

        let saving = project(PageId::Panel, &draft, &application, None);
        let cancel = saving.action(Action::Cancel).expect("Cancel");

        assert!(!cancel.enabled, "Cancel cannot stop a write in flight");
        assert_eq!(
            cancel.disabled_reason.as_deref(),
            Some("A save is in progress"),
            "and it says why"
        );
        assert!(
            !saving
                .focus_order
                .contains(&FocusTarget::Action(Action::Cancel)),
            "a disabled action is not an entry of the walk"
        );
    }

    #[test]
    fn a_modified_field_puts_its_reset_action_in_the_walk() {
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let model = project(PageId::Panel, &draft, &Application::new(), None);
        let position = model
            .focus_order
            .iter()
            .position(|target| *target == FocusTarget::Reset(Field::PanelHeight))
            .expect("the reset action is reachable");
        let field = model
            .focus_order
            .iter()
            .position(|target| *target == FocusTarget::Field(Field::PanelHeight))
            .expect("the field is reachable");

        assert_eq!(position, field + 1, "it follows its own field");
    }

    #[test]
    fn the_focus_is_only_ever_one_of_the_reachable_entries() {
        let model = project(
            PageId::Panel,
            &draft(),
            &Application::new(),
            Some(FocusTarget::Field(Field::PanelPosition)),
        );

        assert_eq!(
            model.focused,
            Some(FocusTarget::Field(Field::PanelPosition))
        );
        assert!(model.focus_order.contains(&model.focused.expect("focused")));

        // A field of another page is not reachable from this one.
        let model = project(
            PageId::Panel,
            &draft(),
            &Application::new(),
            Some(FocusTarget::Field(Field::LocaleLanguage)),
        );
        assert_eq!(model.focused, None);
    }

    #[test]
    fn the_focused_field_says_so_once() {
        let model = project(
            PageId::Panel,
            &draft(),
            &Application::new(),
            Some(FocusTarget::Field(Field::PanelHeight)),
        );

        let focused: Vec<Field> = model
            .fields
            .iter()
            .filter(|field| field.focused)
            .map(|field| field.field)
            .collect();
        assert_eq!(focused, vec![Field::PanelHeight]);
    }

    #[test]
    fn the_status_line_separates_saved_from_in_effect() {
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        // Not written yet.
        let model = project(PageId::Panel, &draft, &Application::new(), None);
        assert_eq!(model.status.kind, StatusKind::Modified);

        // Written, waiting for the shell.
        let mut application = Application::new();
        let files = draft.files();
        let generation = application
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");
        application.file_written(generation, crate::document::SettingFile::Shell);

        let model = project(PageId::Panel, &draft, &application, None);
        assert_eq!(model.status.kind, StatusKind::Pending);
        assert_eq!(model.status.details.len(), 1);
        assert!(
            model.status.details[0].contains("not confirmed"),
            "details {:?}",
            model.status.details
        );

        // Confirmed.
        application.component_applied(crate::apply::Component::Shell, generation);

        let model = project(PageId::Panel, &draft, &application, None);
        assert_eq!(model.status.kind, StatusKind::Applied);
        assert!(model.status.details[0].contains("applied"));
    }

    #[test]
    fn a_change_that_was_saved_but_cannot_be_asked_for_is_not_called_in_effect() {
        let mut draft = draft();
        draft
            .set(Field::PanelHeight, Value::Number(48))
            .expect("a number");

        let mut application = Application::new();
        let files = draft.files();
        let generation = application
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");
        application.file_written(generation, crate::document::SettingFile::Shell);
        application.component_unreachable(
            crate::apply::Component::Shell,
            generation,
            "no transport is implemented in this milestone".to_owned(),
        );

        let model = project(PageId::Panel, &draft, &application, None);

        assert_eq!(model.status.kind, StatusKind::SavedOnly);
        assert!(
            model.status.message.contains("the shell"),
            "message `{}`",
            model.status.message
        );
    }

    #[test]
    fn a_save_that_failed_says_so_and_offers_no_reload() {
        let mut application = Application::new();
        let files = draft().files();
        let generation = application
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");
        application.fail(
            generation,
            Rejection::Save {
                file: crate::document::SettingFile::Shell,
                reason: "read-only file system".to_owned(),
            },
        );

        let model = project(PageId::Panel, &draft(), &application, None);

        assert_eq!(model.status.kind, StatusKind::Rejected);
        assert!(
            model.status.message.contains("shell.toml"),
            "{}",
            model.status.message
        );
        assert!(
            model.status.details[0].contains("read-only file system"),
            "{:?}",
            model.status.details
        );
        assert!(model.action(Action::Reload).is_none());
    }

    #[test]
    fn an_external_edit_offers_the_way_out() {
        let mut application = Application::new();
        let files = BTreeSet::from([crate::document::SettingFile::Shell]);
        let generation = application
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");
        application.fail(
            generation,
            Rejection::Conflict {
                file: crate::document::SettingFile::Shell,
                current_digest: crate::persist::Digest::of(b"# edited\n"),
            },
        );

        let model = project(PageId::Panel, &draft(), &application, None);

        assert_eq!(model.status.kind, StatusKind::Rejected);
        let reload = model.action(Action::Reload).expect("Reload is offered");
        assert!(reload.enabled);
        assert!(
            model.status.details[0]
                .contains(&crate::persist::Digest::of(b"# edited\n").to_string()),
            "{:?}",
            model.status.details
        );
    }

    #[test]
    fn a_component_that_refused_is_named_and_its_fields_are_listed() {
        let application = Application::new();
        let mut application = application;
        let files = BTreeSet::from([crate::document::SettingFile::Shell]);
        let generation = application
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");
        application.file_written(generation, crate::document::SettingFile::Shell);
        application.component_rejected(
            crate::apply::Component::Shell,
            generation,
            "no output is connected".to_owned(),
            vec![Field::PanelHeight],
        );

        let model = project(PageId::Panel, &draft(), &application, None);

        assert_eq!(model.status.kind, StatusKind::Rejected);
        assert!(
            model.status.message.contains("the shell"),
            "{}",
            model.status.message
        );
        assert!(
            model.status.details[0].contains("no output is connected"),
            "{:?}",
            model.status.details
        );
        assert!(
            model
                .status
                .details
                .iter()
                .any(|detail| detail.contains(Field::PanelHeight.path())),
            "the key the consumer objects to is named: {:?}",
            model.status.details
        );
    }

    #[test]
    fn a_control_is_disabled_while_a_save_is_in_flight() {
        let mut application = Application::new();
        let files = BTreeSet::from([crate::document::SettingFile::Shell]);
        let _ = application
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");

        let model = project(PageId::Panel, &draft(), &application, None);

        assert_eq!(model.status.kind, StatusKind::Saving);
        for field in &model.fields {
            assert!(!field.enabled, "`{}`", field.field);
            assert_eq!(
                field.disabled_reason.as_deref(),
                Some("A save is in progress")
            );
        }
        assert!(!model.action(Action::Apply).expect("Apply").enabled);
    }

    #[test]
    fn a_generation_is_not_part_of_what_is_drawn() {
        // The generation orders answers; a user reads sentences. This checks the
        // projection does not leak it into the status line.
        let mut application = Application::new();
        let files = BTreeSet::from([crate::document::SettingFile::Shell]);
        let generation = application
            .begin_save(files.clone(), crate::apply::Component::for_files(&files))
            .expect("nothing is in flight");
        application.file_written(generation, crate::document::SettingFile::Shell);

        let model = project(PageId::Panel, &draft(), &application, None);

        assert_eq!(generation, Generation::new(1));
        assert!(
            !model.status.message.contains("1"),
            "{}",
            model.status.message
        );
    }

    #[test]
    fn a_language_followed_from_the_environment_is_shown_as_that_choice() {
        let model = open(PageId::Locale);

        let Some(ControlView::Choices(options)) = model
            .field(Field::LocaleLanguage)
            .map(|field| &field.control)
        else {
            panic!("the language is a choice");
        };

        assert_eq!(options.len(), 3);
        assert!(options[0].selected);
        assert_eq!(options[0].id, ENVIRONMENT);
    }

    #[test]
    fn a_long_value_is_bounded() {
        let long = "a".repeat(MAX_LABEL_CHARS * 2);

        assert_eq!(bounded(&long).chars().count(), MAX_LABEL_CHARS);
        assert!(bounded(&long).ends_with('…'));
        assert_eq!(bounded("short"), "short");
    }

    #[test]
    fn every_projected_field_has_a_reason_when_it_is_disabled() {
        let model = open(PageId::Notifications);

        for field in &model.fields {
            assert_eq!(
                field.disabled_reason.is_some(),
                !field.enabled,
                "`{}`",
                field.field
            );
        }

        for action in &model.actions {
            assert_eq!(
                action.disabled_reason.is_some(),
                !action.enabled,
                "{:?}",
                action.action
            );
        }
    }

    #[test]
    fn a_display_trial_offers_its_two_answers_and_counts_the_seconds_down() {
        let draft = draft();
        let model = project_trial(
            PageId::Displays,
            &draft,
            &Application::new(),
            Some(FocusTarget::Trial(TrialChoice::Confirm)),
            30,
            false,
        );

        let trial = model.trial.expect("the trial is drawn");
        assert_eq!(trial.remaining_seconds, 30);
        assert!(trial.message.contains("30"), "sentence `{}`", trial.message);
        assert_eq!(
            trial
                .choices
                .iter()
                .map(|choice| choice.choice)
                .collect::<Vec<TrialChoice>>(),
            TrialChoice::ALL
        );

        let shorter = project_trial(
            PageId::Displays,
            &draft,
            &Application::new(),
            None,
            4,
            false,
        );
        let trial = shorter.trial.expect("the trial is drawn");
        assert_eq!(trial.remaining_seconds, 4);
        assert!(trial.message.contains('4'), "sentence `{}`", trial.message);
    }

    #[test]
    fn while_a_trial_is_open_only_its_answers_are_reachable() {
        let draft = draft();
        let model = project_trial(
            PageId::Displays,
            &draft,
            &Application::new(),
            Some(FocusTarget::Trial(TrialChoice::Revert)),
            12,
            false,
        );

        assert_eq!(
            model.focus_order,
            vec![
                FocusTarget::Trial(TrialChoice::Confirm),
                FocusTarget::Trial(TrialChoice::Revert)
            ]
        );
        assert_eq!(
            model.focused,
            Some(FocusTarget::Trial(TrialChoice::Revert)),
            "the focus stays on the answer it was on"
        );
        assert!(
            model
                .trial
                .as_ref()
                .expect("the trial is drawn")
                .choices
                .iter()
                .any(|choice| choice.choice == TrialChoice::Revert && choice.focused)
        );
    }

    #[test]
    fn while_a_trial_is_open_every_other_control_names_it_as_the_reason() {
        let model = project_trial(
            PageId::Displays,
            &draft(),
            &Application::new(),
            None,
            9,
            false,
        );

        for field in &model.fields {
            assert!(!field.enabled, "`{}`", field.field);
            assert_eq!(
                field.disabled_reason.as_deref(),
                Some("The display settings are waiting for an answer"),
                "`{}`",
                field.field
            );
        }

        for action in &model.actions {
            assert!(!action.enabled, "{:?}", action.action);
            assert_eq!(
                action.disabled_reason.as_deref(),
                Some("The display settings are waiting for an answer"),
                "{:?}",
                action.action
            );
        }
    }

    #[test]
    fn an_unanswered_trial_is_not_reported_as_settled() {
        let model = project_trial(
            PageId::Displays,
            &draft(),
            &Application::new(),
            None,
            30,
            false,
        );

        assert_eq!(model.status.kind, StatusKind::Provisional);
        assert_eq!(
            model.status.message,
            "The display settings are applied, waiting for confirmation"
        );
    }

    #[test]
    fn an_answer_on_its_way_disables_both_choices_with_a_reason() {
        let model = project_trial(
            PageId::Displays,
            &draft(),
            &Application::new(),
            Some(FocusTarget::Trial(TrialChoice::Confirm)),
            2,
            true,
        );

        let trial = model.trial.expect("the trial is drawn");
        for choice in &trial.choices {
            assert!(!choice.enabled, "{:?}", choice.choice);
            assert!(choice.disabled_reason.is_some(), "{:?}", choice.choice);
        }
        assert!(
            model.focus_order.is_empty(),
            "there is nothing left to answer while the answer travels"
        );
        assert_eq!(model.focused, None);
    }
}
