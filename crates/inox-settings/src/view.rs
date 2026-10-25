//! The window as data: one Qt-free projection, and the actions QML may send back.
//!
//! Everything here is drawing *decisions*, not drawing. Which pages exist, which
//! fields a page shows, the cards those fields are grouped into, what each
//! control is set to, which one the keyboard is on, which actions are on offer
//! and why not when they are not, and what the status line says all arrive
//! already decided in a [`SettingsViewModel`]; this module turns that plus the
//! design tokens into one `serde`-serialisable value that the QML tree renders.
//!
//! This is the migration's central trade: **one JSON model and one narrow action
//! channel**, not a `#[qproperty]` per control and not a second UI definition in
//! QML. The projection is a pure function of the model and the tokens, so every
//! rule below is asserted without a display:
//!
//! - **No colour and no size is written here or in QML.** Colours arrive as
//!   `#AARRGGBB` strings from [`crate::theme`], sizes as numbers derived from
//!   the tokens (`docs/quality/ui-guidelines.md` §1).
//! - **No user-visible string is written here or in QML.** Every sentence is a
//!   Fluent message the projection already carries (root `AGENTS.md` §7.3).
//! - **Four layout decisions are named values**, because acceptance D04, D05,
//!   Q01 and Q02 are about them: one title with cards under it, a width that
//!   carries meaning (a short value is not drawn across the window), a fixed
//!   label column so a long reason cannot push a control off the row, and a
//!   status line that is always there and never claims more than
//!   [`crate::apply`] knows.
//! - **A control that cannot be used says why, and a status line that says
//!   "saved" never claims "in effect"** — the distinction the settings save semantics, decision 4
//!   makes visible in the window.
//!
//! The reverse direction is [`UiAction`]: a closed set of actions QML can send,
//! parsed from JSON into values, and turned into [`crate::app::Message`]s by the
//! runtime. A malformed action changes nothing and is logged at `warn` — it can
//! never panic a window.

use inox_config::Theme as Tokens;
use serde::Serialize;

use crate::draft::{Field, Value, ValueKind};
use crate::pages::PageId;
use crate::theme::{self, ThemeView};
use crate::view_model::{
    Action, ControlView, FieldView, SettingsViewModel, StatusKind, TrialChoice, TrialView,
};

/// The size the window opens at, in logical pixels.
///
/// Window geometry is a layout decision of this component rather than a theme
/// token, so it lives in Rust (never in QML) and the QML tree binds to it. It is
/// also the size the acceptance snapshots were taken at before the migration.
pub const WINDOW_WIDTH: f32 = 1280.0;

/// The height the window opens at, in logical pixels.
pub const WINDOW_HEIGHT: f32 = 720.0;

/// How many columns of a list take how much of the row, relative to each other.
///
/// A connector is roomier than a number, and every weight is positive, so no
/// column is ever drawn as wide as the window and the row always fits the card.
pub fn column_weight(field: Field, column: usize) -> u16 {
    match (field, column) {
        (Field::Outputs, 0) => 12,
        (Field::Outputs, _) => 6,
        _ => 12,
    }
}

/// The stable identifier of a field's control.
///
/// It is what the keyboard's focus target and the QML item share, so the tree
/// can be asked to reveal the item the model says is focused.
pub fn input_id(field: Field) -> String {
    format!("settings-{}-{}", field.file().name(), field.path())
}

/// The stable identifier of a focus target, as the projection carries it.
pub fn focus_id(target: crate::view_model::FocusTarget) -> String {
    crate::focus::target_id(target)
}

/// How wide a field's control is drawn, by what the value means.
///
/// The guidelines (§4) fix three widths and forbid a fourth: a number with its
/// unit is at most six extra-large steps, a connector, a format string, a path
/// or a command at most twelve, and only explanatory text and tables take the
/// window's width. QML turns these names into token-derived widths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Width {
    /// A number with its unit, or an identifier of a couple of characters.
    Short,
    /// A connector, a format string, a path or a command.
    Wide,
    /// The window's width: an explanation or a table.
    Fill,
}

/// The width a field's control is drawn at.
pub fn width_of(field: Field) -> Width {
    match field {
        // A clock format is a string a user reads in full; a layout and a
        // variant are identifiers of a couple of characters. The wallpaper
        // chooser is a row of options, so it takes the generic `Choice` arm.
        Field::PanelClockFormat => Width::Wide,
        Field::KeyboardLayout | Field::KeyboardVariant => Width::Short,
        other => match other.control().kind {
            ValueKind::Number => Width::Short,
            ValueKind::Text => Width::Wide,
            ValueKind::Collection | ValueKind::Choice | ValueKind::Flag => Width::Fill,
        },
    }
}

// -- The projection ------------------------------------------------------------

/// One row of a structured list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellView {
    /// What the cell holds.
    pub text: String,
    /// The focus identifier of the cell's input.
    pub focus_id: String,
    /// How much of the row the column takes, relative to the other columns.
    pub weight: u16,
}

/// One editable row of a list, and the action that removes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowView {
    /// The cells of the row, in column order.
    pub cells: Vec<CellView>,
    /// The focus identifier of the remove button.
    pub remove_focus_id: String,
    /// The label of the remove button.
    pub remove: String,
    /// Whether the keyboard is on the remove button of this row.
    pub remove_focused: bool,
}

/// One option of a segmented control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionView {
    /// The value the option selects.
    pub id: String,
    /// The label of the option.
    pub label: String,
    /// Whether the option holds the current value.
    pub selected: bool,
    /// Whether the keyboard is on it.
    pub focused: bool,
    /// The focus identifier the window's own focus uses for this field.
    pub focus_id: String,
}

/// How a field is drawn, as data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Control {
    /// Structured rows with localized column labels and actions.
    Collection {
        /// The column labels.
        columns: Vec<String>,
        /// The rows, each with its own remove action.
        rows: Vec<RowView>,
        /// The add-row button label.
        add: String,
        /// The focus identifier of the add-row button.
        add_focus_id: String,
        /// Whether the keyboard is on the add-row button.
        add_focused: bool,
        /// Explanation of how saved values will be used.
        help: String,
        /// What an empty list means, said in words rather than left as a blank
        /// space (ui-guidelines §6).
        empty: String,
        /// Whether the list holds no row at all.
        is_empty: bool,
        /// Whether the columns still have room for their own names.
        as_table: bool,
    },
    /// Editable text, kept exactly as entered.
    Text {
        /// The text.
        value: String,
    },
    /// A row of options.
    Choices {
        /// The options, in the order they are drawn.
        options: Vec<OptionView>,
    },
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
        /// Whether the value is already at the bottom of its range.
        at_min: bool,
        /// Whether the value is already at the top of its range.
        at_max: bool,
    },
}

/// One field of a page, as the QML tree renders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldEntry {
    /// The field's wire name, which is how an action names it back.
    pub field: &'static str,
    /// The field's name.
    pub label: String,
    /// The focus identifier of the field's control.
    pub focus_id: String,
    /// How it is drawn.
    pub control: Control,
    /// Whether the draft holds a value the file does not.
    pub modified: bool,
    /// Whether the keyboard is on the field.
    pub focused: bool,
    /// The label of the action that restores the schema's default, when there
    /// is something to restore.
    pub reset: Option<String>,
    /// The focus identifier of that action.
    pub reset_focus_id: String,
    /// Whether the keyboard is on it.
    pub reset_focused: bool,
    /// What the schema says is wrong, when it says something.
    pub error: Option<String>,
    /// Whether the field can be edited.
    pub enabled: bool,
    /// Why it cannot be, when it cannot.
    pub disabled_reason: Option<String>,
    /// How wide the control is drawn.
    pub width: Width,
}

/// One card of a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupEntry {
    /// The card's heading.
    pub title: String,
    /// The fields under it, in the order they are drawn.
    pub fields: Vec<FieldEntry>,
}

/// One entry of the navigation list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageEntry {
    /// The page's wire name.
    pub id: &'static str,
    /// Its name.
    pub label: String,
    /// The sentence under its title.
    pub description: String,
    /// Whether it is the open page.
    pub selected: bool,
    /// Whether the keyboard is on it.
    pub focused: bool,
    /// The focus identifier of the entry.
    pub focus_id: String,
}

/// The open page: its title, its explanation, and why it cannot be worked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageHeader {
    /// The page's wire name.
    pub id: &'static str,
    /// The one title the screen carries.
    pub title: String,
    /// What the page is for.
    pub description: String,
    /// Why the page's controls cannot be used, said once above the cards
    /// rather than beside every control (ui-guidelines §5, §6).
    pub notices: Vec<String>,
}

/// One action at the foot of the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionEntry {
    /// The action's wire name.
    pub id: &'static str,
    /// Its label.
    pub label: String,
    /// Whether the keyboard is on it.
    pub focused: bool,
    /// Whether it can be taken.
    pub enabled: bool,
    /// Why it cannot be, when it cannot (acceptance D05).
    pub disabled_reason: Option<String>,
    /// The focus identifier of the button.
    pub focus_id: String,
    /// Whether it is the one accent action of the screen.
    pub primary: bool,
    /// Whether it throws work away, and is drawn in the danger colour.
    pub danger: bool,
}

/// The status line, and the lines under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    /// How far the change has got.
    pub kind: &'static str,
    /// The sentence a user reads.
    pub message: String,
    /// What each consumer answered, and what went wrong, as sentences.
    pub details: Vec<String>,
    /// The border the strip is drawn with, from the tokens.
    pub border: String,
}

/// One answer of the display trial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialChoiceEntry {
    /// The answer's wire name.
    pub id: &'static str,
    /// Its label.
    pub label: String,
    /// Whether the keyboard is on it.
    pub focused: bool,
    /// Whether it can be taken.
    pub enabled: bool,
    /// Why it cannot be, when it cannot.
    pub disabled_reason: Option<String>,
    /// The focus identifier of the button.
    pub focus_id: String,
    /// Whether it keeps the change.
    pub confirm: bool,
}

/// The question a display trial asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrialEntry {
    /// The one title the dialog carries.
    pub title: String,
    /// The countdown, as the sentence the user reads.
    pub message: String,
    /// Seconds left before the previous settings come back.
    pub remaining_seconds: u32,
    /// The two answers.
    pub choices: Vec<TrialChoiceEntry>,
    /// Why the last answer could not be taken.
    pub error: Option<String>,
}

/// Where the keyboard is, and where it may go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusEntry {
    /// The focus identifier of the entry the keyboard is on, if any.
    pub id: Option<String>,
    /// Which container the tree has to scroll so the focused entry is visible,
    /// when the focus moved onto something a long page can hide.
    pub scroll: Option<&'static str>,
    /// Every entry the keyboard can reach, in the order it walks them.
    pub order: Vec<String>,
    /// The page the content is showing, so the tree knows when to scroll back
    /// to the top.
    pub page: &'static str,
}

/// The whole window, as the QML tree renders it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Projection {
    /// The window's title.
    pub title: String,
    /// The width the window opens at, in logical pixels.
    pub width: f32,
    /// The height the window opens at, in logical pixels.
    pub height: f32,
    /// The design tokens, derived once.
    pub theme: ThemeView,
    /// The navigation list.
    pub pages: Vec<PageEntry>,
    /// The open page.
    pub page: PageHeader,
    /// The cards of the open page.
    pub groups: Vec<GroupEntry>,
    /// The actions at the foot of the window.
    pub actions: Vec<ActionEntry>,
    /// The status line.
    pub status: StatusEntry,
    /// The display trial waiting for an answer, if one is.
    pub trial: Option<TrialEntry>,
    /// The keyboard.
    pub focus: FocusEntry,
}

// -- The wire names ------------------------------------------------------------

/// The name a field travels under.
///
/// It is the field's dotted configuration path: stable, unique, and the same
/// vocabulary `inox-config` uses when it names a key in a parse error, so one
/// name covers what the window edits and what a failure names.
pub fn field_name(field: Field) -> &'static str {
    field.path()
}

/// The field a wire name names, if it names one.
pub fn field_by_name(name: &str) -> Option<Field> {
    Field::ALL
        .into_iter()
        .find(|field| field_name(*field) == name)
}

/// The name an action travels under.
pub fn action_name(action: Action) -> &'static str {
    match action {
        Action::Apply => "apply",
        Action::Cancel => "cancel",
        Action::ResetAll => "reset-all",
        Action::Reload => "reload",
    }
}

/// The name a page travels under.
///
/// The page's own catalog id: static, unique, and already the name of the
/// navigation entry a screen reader reads out.
pub fn page_name(page: PageId) -> &'static str {
    page.label_id()
}

/// The page a wire name names, if it names one.
pub fn page_by_name(name: &str) -> Option<PageId> {
    PageId::ALL
        .into_iter()
        .find(|page| page_name(*page) == name)
}

/// The name a status kind travels under.
///
/// QML does not switch on it today — the sentence and the border colour are
/// what it draws with — but a test can tell the states apart without reading
/// the translation, so it is part of the projection.
pub fn status_name(kind: StatusKind) -> &'static str {
    match kind {
        StatusKind::Unchanged => "unchanged",
        StatusKind::Modified => "modified",
        StatusKind::Saving => "saving",
        StatusKind::Pending => "pending",
        StatusKind::Applied => "applied",
        StatusKind::SavedOnly => "saved-only",
        StatusKind::Rejected => "rejected",
        StatusKind::Provisional => "provisional",
    }
}

/// The name an answer to a display trial travels under.
pub fn trial_choice_name(choice: TrialChoice) -> &'static str {
    match choice {
        TrialChoice::Confirm => "confirm",
        TrialChoice::Revert => "revert",
    }
}

// -- Building the projection ---------------------------------------------------

/// Projects the window: the model, the tokens and the focus, as one value.
pub fn project(model: SettingsViewModel, tokens: &Tokens) -> Projection {
    let groups = groups(&model, theme::list_as_table(tokens));

    Projection {
        title: model.title.clone(),
        width: WINDOW_WIDTH,
        height: WINDOW_HEIGHT,
        theme: theme::project(tokens),
        pages: model
            .pages
            .iter()
            .map(|page| PageEntry {
                id: page_name(page.page),
                label: page.label.clone(),
                description: page.description.clone(),
                selected: page.selected,
                focused: page.focused,
                focus_id: focus_id(crate::view_model::FocusTarget::Page(page.page)),
            })
            .collect(),
        page: PageHeader {
            id: page_name(model.page),
            title: model.page_title.clone(),
            description: model
                .pages
                .iter()
                .find(|page| page.page == model.page)
                .map(|page| page.description.clone())
                .unwrap_or_default(),
            notices: notices(&model),
        },
        groups,
        actions: model
            .actions
            .iter()
            .map(|action| ActionEntry {
                id: action_name(action.action),
                label: action.label.clone(),
                focused: action.focused,
                enabled: action.enabled,
                disabled_reason: action.disabled_reason.clone(),
                focus_id: focus_id(crate::view_model::FocusTarget::Action(action.action)),
                primary: action.action == Action::Apply,
                danger: matches!(action.action, Action::Cancel | Action::ResetAll),
            })
            .collect(),
        status: StatusEntry {
            kind: status_name(model.status.kind),
            message: model.status.message.clone(),
            details: model.status.details.clone(),
            border: theme::status_border(tokens, model.status.kind),
        },
        trial: model.trial.as_ref().map(|trial| trial_entry(trial, &model)),
        focus: FocusEntry {
            id: model.focused.map(focus_id),
            scroll: model
                .focused
                .and_then(crate::focus::reveal)
                .map(crate::focus::ScrollContainer::name),
            order: model.focus_order.iter().copied().map(focus_id).collect(),
            page: page_name(model.page),
        },
    }
}

/// The cards of the open page, with their fields.
fn groups(model: &SettingsViewModel, as_table: bool) -> Vec<GroupEntry> {
    let page = model
        .pages
        .iter()
        .find(|page| page.page == model.page && page.selected)
        .or_else(|| model.pages.iter().find(|page| page.page == model.page));

    let Some(page) = page else {
        return Vec::new();
    };

    page.groups
        .iter()
        .map(|group| GroupEntry {
            title: group.title.clone(),
            fields: group
                .fields
                .iter()
                .filter_map(|field| model.field(*field))
                .map(|field| field_entry(field, as_table))
                .collect(),
        })
        .collect()
}

/// Why the page's controls cannot be used, when they all cannot.
///
/// A disabled control has to explain itself (acceptance D05); every field of a
/// page is disabled for the same reason — a write in flight, or a display trial
/// waiting for an answer — so the reason is said once, above the cards, rather
/// than repeated beside thirty controls (ui-guidelines §5, §6).
fn notices(model: &SettingsViewModel) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();

    for field in model.fields.iter().filter(|field| !field.enabled) {
        if let Some(reason) = &field.disabled_reason
            && !reasons.contains(reason)
        {
            reasons.push(reason.clone());
        }
    }

    reasons
}

/// One field.
fn field_entry(field: &FieldView, as_table: bool) -> FieldEntry {
    let key = field.field;

    FieldEntry {
        field: field_name(key),
        label: field.label.clone(),
        focus_id: input_id(key),
        control: control(&field.control, field, as_table),
        modified: field.modified,
        focused: field.focused,
        reset: field.reset.clone(),
        reset_focus_id: focus_id(crate::view_model::FocusTarget::Reset(key)),
        reset_focused: field.reset_focused,
        error: field.error.clone(),
        enabled: field.enabled,
        disabled_reason: field.disabled_reason.clone(),
        width: width_of(key),
    }
}

/// One field's control.
fn control(control: &ControlView, field: &FieldView, as_table: bool) -> Control {
    let key = field.field;

    match control {
        ControlView::Collection {
            rows,
            columns,
            add,
            remove,
            help,
            empty,
            focused,
        } => Control::Collection {
            columns: columns.clone(),
            rows: rows
                .iter()
                .enumerate()
                .map(|(row, cells)| RowView {
                    cells: cells
                        .iter()
                        .enumerate()
                        .map(|(column, text)| CellView {
                            text: text.clone(),
                            focus_id: focus_id(crate::view_model::FocusTarget::Cell(
                                key, row, column,
                            )),
                            weight: column_weight(key, column),
                        })
                        .collect(),
                    remove_focus_id: focus_id(crate::view_model::FocusTarget::RemoveRow(key, row)),
                    remove: remove.clone(),
                    remove_focused: *focused
                        == Some(crate::view_model::FocusTarget::RemoveRow(key, row)),
                })
                .collect(),
            add: add.clone(),
            add_focus_id: focus_id(crate::view_model::FocusTarget::Field(key)),
            add_focused: *focused == Some(crate::view_model::FocusTarget::Field(key)),
            help: help.clone(),
            empty: empty.clone(),
            is_empty: rows.is_empty(),
            as_table: columns.len() <= 1 || as_table,
        },
        ControlView::Text(value) => Control::Text {
            value: value.clone(),
        },
        ControlView::Choices(options) => Control::Choices {
            options: options
                .iter()
                .map(|option| OptionView {
                    id: option.id.clone(),
                    label: option.label.clone(),
                    selected: option.selected,
                    focused: option.focused,
                    focus_id: input_id(key),
                })
                .collect(),
        },
        ControlView::Flag { on, label } => Control::Flag {
            on: *on,
            label: label.clone(),
        },
        ControlView::Number {
            unit,
            input,
            value,
            min,
            max,
            display,
            decrease,
            increase,
        } => Control::Number {
            unit: unit.clone(),
            input: input.clone(),
            value: *value,
            min: *min,
            max: *max,
            display: display.clone(),
            decrease: decrease.clone(),
            increase: increase.clone(),
            at_min: *value <= *min,
            at_max: *value >= *max,
        },
    }
}

/// One display trial.
fn trial_entry(trial: &TrialView, _model: &SettingsViewModel) -> TrialEntry {
    TrialEntry {
        title: trial.title.clone(),
        message: trial.message.clone(),
        remaining_seconds: trial.remaining_seconds,
        choices: trial
            .choices
            .iter()
            .map(|choice| TrialChoiceEntry {
                id: trial_choice_name(choice.choice),
                label: choice.label.clone(),
                focused: choice.focused,
                enabled: choice.enabled,
                disabled_reason: choice.disabled_reason.clone(),
                focus_id: focus_id(crate::view_model::FocusTarget::Trial(choice.choice)),
                confirm: choice.choice == TrialChoice::Confirm,
            })
            .collect(),
        error: trial.error.clone(),
    }
}

// -- Actions coming back from QML ---------------------------------------------

/// One key the window answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyName {
    /// Tab: the next entry, or with shift the previous one.
    Tab,
    /// Return or Enter: activate what the keyboard is on.
    Enter,
    /// Escape: unwind one level.
    Escape,
    /// Arrow down: the next entry.
    ArrowDown,
    /// Arrow up: the previous entry.
    ArrowUp,
    /// A key the window leaves to the control that has the text cursor.
    Other,
}

impl KeyName {
    /// The key a QML name maps to.
    pub fn from_name(name: &str) -> Self {
        match name {
            "Tab" => Self::Tab,
            "Enter" | "Return" => Self::Enter,
            "Escape" => Self::Escape,
            "ArrowDown" | "Down" => Self::ArrowDown,
            "ArrowUp" | "Up" => Self::ArrowUp,
            _ => Self::Other,
        }
    }
}

/// Everything QML can ask the window to do.
///
/// It is a closed set: the QML tree renders, the Rust state machine decides. An
/// action is parsed from the JSON string the bridge hands over; anything
/// unrecognised is logged and dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    /// Open a page.
    PageSelected(PageId),
    /// Set a field to a value.
    FieldSet(Field, Value),
    /// Replace the text of a number field, exactly as typed.
    NumberEdited(Field, String),
    /// Walk a number by one step.
    FieldStepped(Field, i64),
    /// Return one field to the schema's default.
    FieldReset(Field),
    /// Replace one cell of a list.
    CellEdited {
        /// The list.
        field: Field,
        /// The row.
        row: usize,
        /// The column.
        column: usize,
        /// What the cell now holds.
        text: String,
    },
    /// Append an empty row to a list.
    RowAdded(Field),
    /// Remove one row of a list.
    RowRemoved {
        /// The list.
        field: Field,
        /// The row.
        row: usize,
    },
    /// Return every field to the schema's default.
    AllReset,
    /// Write the changed fields.
    Apply,
    /// Throw the draft away.
    Cancel,
    /// Read the files again.
    Reload,
    /// The keyboard asked for the next entry.
    FocusNext,
    /// The keyboard asked for the previous entry.
    FocusPrevious,
    /// The keyboard activated the focused entry.
    Activate,
    /// The keyboard asked to unwind a level.
    Back,
    /// Keep the display change on trial.
    TrialConfirm,
    /// Put the previous display settings back.
    TrialRevert,
    /// One key press the window itself answers.
    Key {
        /// Which key.
        key: KeyName,
        /// Whether shift was held.
        shift: bool,
        /// Whether the key is being held down.
        repeat: bool,
    },
}

/// Parses one action the QML tree sent.
///
/// Returns `None` for anything that is not a well-formed action of the closed
/// set above; the caller logs it. A window that cannot parse a click is a
/// window that does nothing, never one that panics.
pub fn parse_action(json: &str) -> Option<UiAction> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let object = value.as_object()?;
    let kind = object.get("type")?.as_str()?;

    Some(match kind {
        "page-selected" => UiAction::PageSelected(page_by_name(object.get("page")?.as_str()?)?),
        "field-set" => {
            let field = field_by_name(object.get("field")?.as_str()?)?;
            let value = object.get("value")?;
            UiAction::FieldSet(field, parse_value(value)?)
        }
        "number-edited" => UiAction::NumberEdited(
            field_by_name(object.get("field")?.as_str()?)?,
            object.get("text")?.as_str()?.to_owned(),
        ),
        "field-stepped" => UiAction::FieldStepped(
            field_by_name(object.get("field")?.as_str()?)?,
            object.get("delta")?.as_i64()?,
        ),
        "field-reset" => UiAction::FieldReset(field_by_name(object.get("field")?.as_str()?)?),
        "cell-edited" => UiAction::CellEdited {
            field: field_by_name(object.get("field")?.as_str()?)?,
            row: usize::try_from(object.get("row")?.as_u64()?).ok()?,
            column: usize::try_from(object.get("column")?.as_u64()?).ok()?,
            text: object.get("text")?.as_str()?.to_owned(),
        },
        "row-added" => UiAction::RowAdded(field_by_name(object.get("field")?.as_str()?)?),
        "row-removed" => UiAction::RowRemoved {
            field: field_by_name(object.get("field")?.as_str()?)?,
            row: usize::try_from(object.get("row")?.as_u64()?).ok()?,
        },
        // The wire name is the one the projection carries
        // ([`action_name`]), because that is what the tree sends back.
        "reset-all" => UiAction::AllReset,
        "apply" => UiAction::Apply,
        "cancel" => UiAction::Cancel,
        "reload" => UiAction::Reload,
        "focus-next" => UiAction::FocusNext,
        "focus-previous" => UiAction::FocusPrevious,
        "activate" => UiAction::Activate,
        "back" => UiAction::Back,
        "trial-confirm" => UiAction::TrialConfirm,
        "trial-revert" => UiAction::TrialRevert,
        "key" => UiAction::Key {
            key: KeyName::from_name(object.get("key")?.as_str()?),
            shift: object
                .get("shift")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            repeat: object
                .get("repeat")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        },
        _ => return None,
    })
}

/// The value a `field-set` action carries.
///
/// Only the shapes a control can produce are accepted: a list is edited cell by
/// cell and row by row, never replaced wholesale by the tree.
fn parse_value(value: &serde_json::Value) -> Option<Value> {
    let object = value.as_object()?;

    match object.get("kind")?.as_str()? {
        "text" => Some(Value::Text(object.get("value")?.as_str()?.to_owned())),
        "number" => Some(Value::Number(object.get("value")?.as_i64()?)),
        "flag" => Some(Value::Flag(object.get("on")?.as_bool()?)),
        "choice" => Some(Value::Choice(object.get("id")?.as_str()?.to_owned())),
        _ => None,
    }
}

/// Serialises the projection into the string the bridge hands to QML.
///
/// A projection that cannot be serialised is a programming error, not a runtime
/// condition: every value in it is a string, a number or a boolean. Returning
/// the error keeps the caller from having to unwrap.
pub fn to_json(projection: &Projection) -> Result<String, serde_json::Error> {
    serde_json::to_string(projection)
}

/// The projection of a window whose model has not been built yet.
///
/// The QML tree is only ever loaded after the binary installs a real projection,
/// so this exists for the tests that construct the bridge object directly; it
/// says so by being empty rather than by inventing a window.
pub fn empty() -> &'static str {
    "{}"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::Application;
    use crate::draft::{Draft, Settings};
    use crate::view_model::{SettingsInput, StatusKind};
    use inox_common::i18n::{Locale, Localizer};

    fn localizer(tag: &str) -> Localizer {
        let locale = Locale::parse(tag).expect("locale");
        crate::i18n::load(&locale).expect("the catalogs parse")
    }

    fn model(page: PageId, tag: &str) -> SettingsViewModel {
        let draft = Draft::new(Settings::defaults());
        let application = Application::new();
        let errors = crate::validate::validate(draft.settings());

        SettingsInput {
            page,
            draft: &draft,
            application: &application,
            errors: &errors,
            focused: None,
            trial: None,
            notice: None,
        }
        .project(&localizer(tag))
    }

    fn projection(page: PageId, tag: &str, tokens: &Tokens) -> Projection {
        project(model(page, tag), tokens)
    }

    #[test]
    fn the_projection_serialises_to_a_json_object() {
        let tokens = Tokens::builtin_dark();
        let window = projection(PageId::Panel, "en-US", &tokens);
        let json = to_json(&window).expect("the projection serialises");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert!(parsed.get("title").is_some());
        assert!(parsed["theme"]["colors"]["window"].is_string());
        assert!(parsed["theme"]["geometry"]["bodySize"].is_number());
        assert_eq!(parsed["pages"].as_array().map(Vec::len), Some(10));
        assert_eq!(parsed["page"]["id"], page_name(PageId::Panel));
        assert!(!parsed["groups"].as_array().expect("groups").is_empty());
        assert!(parsed["trial"].is_null());
        assert!(parsed["status"]["border"].is_string());
    }

    #[test]
    fn the_window_opens_at_the_size_the_snapshots_used() {
        let tokens = Tokens::builtin_dark();
        let window = projection(PageId::Panel, "en-US", &tokens);

        assert!((window.width - 1280.0).abs() < f32::EPSILON);
        assert!((window.height - 720.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_projection_carries_the_tokens_rather_than_writing_its_own() {
        let tokens = Tokens::builtin_dark();
        let window = projection(PageId::Panel, "en-US", &tokens);

        assert_eq!(
            window.theme.colors.window,
            theme::qml_color(tokens.colors.background)
        );
        assert_eq!(window.theme.geometry.body_size, theme::body_size(&tokens));
        assert_eq!(window.theme.geometry.nav_width, theme::nav_width(&tokens));

        // The light theme differs in the projection as it does in the tokens.
        let light = projection(PageId::Panel, "en-US", &Tokens::builtin_light());
        assert_ne!(window.theme.colors.window, light.theme.colors.window);
    }

    #[test]
    fn the_projection_carries_the_negotiated_language() {
        let tokens = Tokens::builtin_dark();
        let english = projection(PageId::Appearance, "en-US", &tokens);
        let chinese = projection(PageId::Appearance, "zh-CN", &tokens);

        assert_eq!(english.page.title, "Appearance");
        assert_eq!(chinese.page.title, "外观");
        assert_ne!(english.status.message, chinese.status.message);
        assert_eq!(english.pages.len(), chinese.pages.len());
        // A user-visible string is text, never a message id.
        for entry in &chinese.pages {
            assert!(!entry.label.starts_with("settings-"), "{}", entry.label);
            assert!(!entry.description.starts_with("settings-"));
        }
    }

    #[test]
    fn every_field_of_every_page_is_projected_with_its_control() {
        let tokens = Tokens::builtin_dark();

        for page in PageId::ALL {
            let window = projection(page, "en-US", &tokens);
            let projected: Vec<&str> = window
                .groups
                .iter()
                .flat_map(|group| group.fields.iter())
                .map(|field| field.field)
                .collect();
            let expected: Vec<&str> = page
                .fields()
                .iter()
                .map(|field| field_name(*field))
                .collect();

            assert_eq!(projected, expected, "{page:?}");
            for group in &window.groups {
                assert!(!group.title.is_empty());
                for field in &group.fields {
                    assert!(!field.label.is_empty(), "{}", field.field);
                    assert!(!field.focus_id.is_empty());
                }
            }
        }
    }

    #[test]
    fn a_number_control_carries_its_range_and_its_unit() {
        let tokens = Tokens::builtin_dark();
        let window = projection(PageId::Panel, "en-US", &tokens);
        let field = window
            .groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .find(|field| field.field == field_name(Field::PanelHeight))
            .expect("the panel height is on the panel page");

        let Control::Number {
            unit,
            min,
            max,
            value,
            at_min,
            at_max,
            ..
        } = &field.control
        else {
            panic!("the panel height is a number");
        };

        assert!(!unit.is_empty());
        assert!(min < max);
        assert!(min <= value && value <= max);
        assert!(!at_min && !at_max);
        assert_eq!(field.width, Width::Short);
    }

    #[test]
    fn a_short_field_is_never_drawn_as_wide_as_the_window() {
        let tokens = Tokens::builtin_dark();
        let window = projection(PageId::Panel, "en-US", &tokens);

        for group in &window.groups {
            for field in &group.fields {
                assert_eq!(
                    field.width,
                    width_of(field_by_name(field.field).expect("a known field"))
                );
            }
        }

        assert_eq!(width_of(Field::PanelHeight), Width::Short);
        assert_eq!(width_of(Field::PanelClockFormat), Width::Wide);
        assert_eq!(width_of(Field::Outputs), Width::Fill);
    }

    #[test]
    fn a_list_is_projected_row_by_row_with_a_focus_identifier() {
        let tokens = Tokens::builtin_dark();
        let mut draft = Draft::new(Settings::defaults());
        draft
            .set(
                Field::Outputs,
                Value::Collection(vec![
                    vec!["DP-1".to_owned(), "1920".to_owned()],
                    vec!["DP-2".to_owned(), "2560".to_owned()],
                ]),
            )
            .expect("a list is settable");

        let application = Application::new();
        let errors = crate::validate::validate(draft.settings());
        let model = SettingsInput {
            page: PageId::Displays,
            draft: &draft,
            application: &application,
            errors: &errors,
            focused: None,
            trial: None,
            notice: None,
        }
        .project(&localizer("en-US"));

        let window = project(model, &tokens);
        let field = window
            .groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .find(|field| field.field == field_name(Field::Outputs))
            .expect("the outputs are on the displays page");

        let Control::Collection {
            rows,
            columns,
            empty,
            ..
        } = &field.control
        else {
            panic!("the outputs are a collection");
        };

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].cells[0].text, "DP-1");
        assert_eq!(rows[1].cells[1].text, "2560");
        assert!(!columns.is_empty());
        assert!(!empty.is_empty());
        assert!(rows[0].cells[0].focus_id.ends_with("-0-0"));
        assert!(rows[1].remove_focus_id.ends_with("-1-remove"));
        assert!(field.width == Width::Fill);
    }

    #[test]
    fn an_empty_list_says_what_it_will_do() {
        let tokens = Tokens::builtin_dark();
        let window = projection(PageId::InputMethod, "en-US", &tokens);
        let field = window
            .groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .next()
            .expect("the input method page has a field");

        let Control::Collection {
            is_empty, empty, ..
        } = &field.control
        else {
            panic!("the input method is a list");
        };

        assert!(is_empty);
        assert!(!empty.is_empty());
    }

    #[test]
    fn the_actions_carry_their_reason_and_the_primary_is_marked() {
        let tokens = Tokens::builtin_dark();
        let window = projection(PageId::Panel, "en-US", &tokens);

        let apply = window
            .actions
            .iter()
            .find(|action| action.id == "apply")
            .expect("apply is offered");
        assert!(apply.primary);
        assert!(!apply.enabled, "nothing has changed yet");
        // The sentence is the catalog's, not a copy of it written here: a window
        // that names a reason has to name the one the user reads.
        let reason = localizer("en-US").message("settings-disabled-no-changes");
        assert_eq!(
            apply.disabled_reason.as_deref(),
            Some(reason.as_str()),
            "a disabled action names its reason"
        );

        let cancel = window
            .actions
            .iter()
            .find(|action| action.id == "cancel")
            .expect("cancel is offered");
        assert!(cancel.danger, "cancel throws work away");
        assert!(!cancel.primary);
    }

    #[test]
    fn the_status_line_says_what_the_change_has_done() {
        let tokens = Tokens::builtin_dark();
        let window = projection(PageId::Panel, "en-US", &tokens);

        assert_eq!(window.status.kind, "unchanged");
        assert_eq!(window.status.message, "The files are unchanged");
        assert_eq!(
            window.status.border,
            theme::status_border(&tokens, StatusKind::Unchanged)
        );
    }

    #[test]
    fn the_keyboard_projection_names_where_the_focus_is_and_where_it_may_go() {
        let tokens = Tokens::builtin_dark();
        let draft = Draft::new(Settings::defaults());
        let application = Application::new();
        let errors = crate::validate::validate(draft.settings());
        let focused = Some(crate::view_model::FocusTarget::Page(PageId::Panel));
        let model = SettingsInput {
            page: PageId::Panel,
            draft: &draft,
            application: &application,
            errors: &errors,
            focused,
            trial: None,
            notice: None,
        }
        .project(&localizer("en-US"));

        let window = project(model, &tokens);

        assert_eq!(
            window.focus.id.as_deref(),
            Some(window.pages[1].focus_id.as_str())
        );
        assert!(window.focus.order.len() > window.pages.len());
        assert_eq!(window.focus.order[1].as_str(), window.pages[1].focus_id);
        assert_eq!(window.focus.page, page_name(PageId::Panel));
        assert_eq!(
            window.focus.scroll,
            Some("navigation"),
            "the keyboard is on a page entry, which the navigation list may have scrolled"
        );
        assert!(window.pages[1].focused);
    }

    #[test]
    fn a_display_trial_projects_its_two_answers() {
        let tokens = Tokens::builtin_dark();
        let draft = Draft::new(Settings::defaults());
        let application = Application::new();
        let errors = crate::validate::validate(draft.settings());
        let model = SettingsInput {
            page: PageId::Displays,
            draft: &draft,
            application: &application,
            errors: &errors,
            focused: Some(crate::view_model::FocusTarget::Trial(TrialChoice::Confirm)),
            trial: Some(crate::view_model::TrialInput {
                generation: crate::apply::Generation::new(1),
                remaining_seconds: 29,
                error: None,
                answering: false,
            }),
            notice: None,
        }
        .project(&localizer("en-US"));

        let window = project(model, &tokens);
        let trial = window.trial.expect("the trial is projected");

        assert_eq!(trial.remaining_seconds, 29);
        assert!(trial.message.contains("29"));
        assert_eq!(trial.choices.len(), 2);
        assert!(trial.choices[0].confirm);
        assert!(trial.choices[0].focused);
        assert!(!trial.choices[1].confirm);
        assert_eq!(window.status.kind, "provisional");
    }

    #[test]
    fn every_action_of_the_wire_round_trips() {
        for action in [
            Action::Apply,
            Action::Cancel,
            Action::ResetAll,
            Action::Reload,
        ] {
            let json = format!("{{\"type\":\"{}\"}}", action_name(action));
            let parsed = parse_action(&json).expect("a known action parses");
            assert_eq!(
                parsed,
                match action {
                    Action::Apply => UiAction::Apply,
                    Action::Cancel => UiAction::Cancel,
                    Action::ResetAll => UiAction::AllReset,
                    Action::Reload => UiAction::Reload,
                }
            );
        }

        for page in PageId::ALL {
            let json = format!(
                "{{\"type\":\"page-selected\",\"page\":\"{}\"}}",
                page_name(page)
            );
            assert_eq!(parse_action(&json), Some(UiAction::PageSelected(page)));
        }

        for field in Field::ALL {
            let json = format!(
                "{{\"type\":\"field-reset\",\"field\":\"{}\"}}",
                field_name(field)
            );
            assert_eq!(parse_action(&json), Some(UiAction::FieldReset(field)));
        }
    }

    #[test]
    fn a_value_of_every_shape_a_control_produces_parses() {
        assert_eq!(
            parse_action(&format!(
                "{{\"type\":\"field-set\",\"field\":\"{}\",\"value\":{{\"kind\":\"flag\",\"on\":true}}}}",
                field_name(Field::PanelAutohide)
            )),
            Some(UiAction::FieldSet(Field::PanelAutohide, Value::Flag(true)))
        );
        assert_eq!(
            parse_action(&format!(
                "{{\"type\":\"field-set\",\"field\":\"{}\",\"value\":{{\"kind\":\"choice\",\"id\":\"dark\"}}}}",
                field_name(Field::ThemeAppearance)
            )),
            Some(UiAction::FieldSet(
                Field::ThemeAppearance,
                Value::Choice("dark".to_owned())
            ))
        );
        assert_eq!(
            parse_action(&format!(
                "{{\"type\":\"field-set\",\"field\":\"{}\",\"value\":{{\"kind\":\"number\",\"value\":48}}}}",
                field_name(Field::PanelHeight)
            )),
            Some(UiAction::FieldSet(Field::PanelHeight, Value::Number(48)))
        );
    }

    #[test]
    fn a_malformed_action_is_dropped_rather_than_fatal() {
        for json in [
            "",
            "not json",
            "[]",
            "{}",
            "{\"type\":\"unknown\"}",
            "{\"type\":\"page-selected\"}",
            "{\"type\":\"page-selected\",\"page\":\"Nope\"}",
            "{\"type\":\"field-reset\",\"field\":\"Nope\"}",
            "{\"type\":\"field-stepped\",\"field\":\"PanelHeight\"}",
        ] {
            assert_eq!(parse_action(json), None, "`{json}` must not parse");
        }
    }

    #[test]
    fn a_key_press_becomes_the_key_the_window_knows() {
        assert_eq!(KeyName::from_name("Tab"), KeyName::Tab);
        assert_eq!(KeyName::from_name("Return"), KeyName::Enter);
        assert_eq!(KeyName::from_name("Escape"), KeyName::Escape);
        assert_eq!(KeyName::from_name("ArrowDown"), KeyName::ArrowDown);
        assert_eq!(KeyName::from_name("x"), KeyName::Other);

        let json = "{\"type\":\"key\",\"key\":\"Tab\",\"shift\":true,\"repeat\":false}";
        assert_eq!(
            parse_action(json),
            Some(UiAction::Key {
                key: KeyName::Tab,
                shift: true,
                repeat: false
            })
        );
    }

    #[test]
    fn an_empty_projection_is_an_empty_object() {
        let parsed: serde_json::Value = serde_json::from_str(empty()).expect("valid JSON");
        assert!(parsed.as_object().expect("an object").is_empty());
    }
}
