//! The settings window's widgets.
//!
//! Everything here is drawing. Which pages exist, which fields a page shows, the
//! cards those fields are grouped into, what each control is set to, which one
//! the keyboard is on, which actions are on offer and why not when they are not,
//! and what the status line says all arrive already decided in a
//! [`SettingsViewModel`]; this module turns that into iced widgets and takes
//! every colour, size, radius and padding from the tokens through
//! [`crate::theme`] — never from a number written here
//! (`docs/quality/ui-guidelines.md` §1).
//!
//! Four layout decisions are worth naming, because they are what acceptance D04,
//! D05, Q01 and Q02 are about:
//!
//! - **One title, cards under it.** A screen has a single `title` with a
//!   `caption` explaining it; each card under that has a `heading`. A user
//!   should be able to tell what to read first by looking, not by reading
//!   (ui-guidelines §2, §9).
//! - **Width carries meaning.** A number is drawn as wide as a number, a
//!   connector or a path as wide as a connector or a path, and a list as wide as
//!   the window. A short field stretched across a 1280-pixel window tells a user
//!   to type a paragraph (ui-guidelines §4, §9).
//! - **The controls keep their width.** The navigation list is a fixed column
//!   and every field's label is a fixed column, so a long language tag or a long
//!   reason cannot push the control a user needs off the row.
//! - **The status line is always there.** Saved, waiting, in effect and refused
//!   are four different sentences in four different colours, and the one that
//!   says *in effect* is only ever drawn from [`crate::apply`]'s confirmation
//!   (ADR 0014, decision 4). A disabled control says why it is disabled, on the
//!   screen and not only in the log (ui-guidelines §5, §6).
//!
//! The focus is a colour, never a widget: the window's own focus is drawn on the
//! control it is on (see [`crate::theme::control`]), so no control has to hold on
//! to the toolkit's focus to be reachable by keyboard.

use iced::widget::text::Wrapping;
use iced::widget::{Column, Row, button, container, scrollable, text, text_input};
use iced::{Element, Length, Padding};
use inox_config::Theme as Tokens;

use crate::app::Message;
use crate::draft::{Field, Value, ValueKind};
use crate::theme::{self, Emphasis, Role};
use crate::view_model::{
    Action, ActionView, ControlView, FieldView, GroupView, PageView, SettingsViewModel,
    TrialChoice, TrialChoiceView, TrialView,
};

/// How many extra-large spacing steps wide the navigation list is.
const NAV_WIDTH_STEPS: f32 = 5.0;

/// How many extra-large spacing steps wide a field's label column is.
const LABEL_WIDTH_STEPS: f32 = 7.0;

/// The width the display trial's card is drawn at.
///
/// Two wide values and the card's own padding: the dialog holds the longest
/// sentences in the window and has to wrap them rather than grow past the
/// screen at a large font scale.
const DIALOG_WIDTH_STEPS: f32 = 1.0;

/// The font scale above which a list of several columns stops being a table.
///
/// A column can only name itself while it is wider than the longest word in its
/// name; past this scale the names break in the middle of a word, so the list is
/// drawn one labelled field per row instead (ui-guidelines §7).
const TABLE_SCALE_LIMIT: f32 = 1.5;

/// The geometry of the window, derived from the tokens.
///
/// Every padding, gap, size and bound the window draws with is a field here, so
/// a reviewer can read the layout without following a widget tree and so a test
/// can assert the bounds the guidelines set (ui-guidelines §4).
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsGeometry {
    /// The size of a page's title.
    pub title_size: f32,
    /// The size of a card's heading.
    pub heading_size: f32,
    /// The size of body text, labels and button text.
    pub body_size: f32,
    /// The size of an explanation, a unit or a state sentence.
    pub caption_size: f32,
    /// The window's own padding: `xl`.
    pub page_padding: Padding,
    /// The padding inside a card: `lg`.
    pub card_padding: Padding,
    /// The padding inside a status or notice strip: `md`.
    pub notice_padding: Padding,
    /// The padding inside a button or a field: `sm` above and below, `md` at the
    /// sides.
    pub control_padding: Padding,
    /// The padding inside a list entry, which is what makes a one-line entry a
    /// full control tall without ever cutting a wrapped one off.
    pub list_padding: Padding,
    /// The gap between two cards and between the window's blocks: `lg`.
    pub section_gap: f32,
    /// The gap between two entries of one card: `sm`.
    pub row_gap: f32,
    /// The gap between a value and its unit, or a label and its control: `sm`.
    pub inline_gap: f32,
    /// The width of the navigation list.
    pub nav_width: f32,
    /// The width of a field's label column, so the controls line up down the
    /// page and across its cards.
    pub label_width: f32,
    /// The width of a short value: a number with its unit, never more than six
    /// extra-large steps.
    pub short_width: f32,
    /// The width of a long value: a connector, a format string or a path, never
    /// more than twelve extra-large steps.
    pub wide_width: f32,
    /// The width of the actions a list row carries, which is a column of its own
    /// so the column names stay over the fields they name.
    pub list_action_width: f32,
    /// The smallest height a control a user can act on is drawn at.
    pub control_height: f32,
    /// The line box a field's own text sits in.
    pub line_height: iced::Pixels,
    /// Whether a list of several columns still has room to be a table.
    pub list_as_table: bool,
    /// The width of the display trial's card.
    pub dialog_width: f32,
}

impl SettingsGeometry {
    /// Derives the window's geometry.
    ///
    /// The navigation column and the label column grow with the user's font
    /// scale, because they hold words: a label a user has doubled has to have
    /// somewhere to go (Q02). The value widths do not, because they are bounds
    /// the guidelines fix in tokens rather than in text.
    pub fn new(tokens: &Tokens) -> Self {
        let scale = tokens.typography.scale.max(1.0);

        Self {
            title_size: theme::title_size(tokens),
            heading_size: theme::heading_size(tokens),
            body_size: theme::body_size(tokens),
            caption_size: theme::caption_size(tokens),
            page_padding: theme::page_padding(tokens),
            card_padding: theme::card_padding(tokens),
            notice_padding: theme::notice_padding(tokens),
            control_padding: theme::control_padding(tokens),
            list_padding: theme::list_padding(tokens),
            section_gap: theme::section_gap(tokens),
            row_gap: theme::row_gap(tokens),
            inline_gap: theme::inline_gap(tokens),
            nav_width: f32::from(tokens.spacing.xl) * NAV_WIDTH_STEPS * scale,
            label_width: f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS * scale,
            short_width: theme::short_width(tokens),
            wide_width: theme::wide_width(tokens),
            list_action_width: theme::list_action_width(tokens),
            control_height: theme::control_height(tokens),
            line_height: theme::control_line_height(tokens),
            list_as_table: tokens.typography.scale <= TABLE_SCALE_LIMIT,
            dialog_width: (theme::wide_width(tokens) + theme::card_padding(tokens).left * 2.0)
                * DIALOG_WIDTH_STEPS
                * scale,
        }
    }
}

/// Draws the window.
///
/// The model is taken by value and its strings are copied into the widgets, so
/// the element this returns borrows the tokens and nothing else — which is what
/// lets the caller project a fresh model on every frame instead of caching one.
pub fn settings<'a>(model: SettingsViewModel, tokens: &'a Tokens) -> Element<'a, Message> {
    let geometry = SettingsGeometry::new(tokens);

    let navigation = container(
        scrollable(
            Column::with_children(
                model
                    .pages
                    .iter()
                    .map(|page| page_entry(page, tokens, &geometry))
                    .collect::<Vec<Element<'a, Message>>>(),
            )
            .spacing(geometry.row_gap),
        )
        .id("settings-navigation"),
    )
    .width(Length::Fixed(geometry.nav_width))
    .height(Length::Fill)
    .padding(geometry.card_padding)
    .style(move |_theme| theme::sidebar(tokens));

    // The page takes the height the status line and the actions do not need, so
    // the actions sit at the foot of the window whatever the page holds — which
    // is also what keeps them in the same place as the user moves between pages.
    let body = container(
        Column::with_children([
            container(
                scrollable(crate::pages::view(model.page, &model, tokens)).id("settings-content"),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
            status_line(&model, tokens, &geometry),
            action_row(&model, tokens, &geometry),
        ])
        .spacing(geometry.section_gap)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(geometry.page_padding);

    let window = container(Row::with_children([navigation.into(), body.into()]))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| theme::window(tokens));

    // A display trial is asked on top of the window rather than in a place of
    // its own: the display has already changed, so the window is exactly what
    // the user may be losing, and the question has to be answerable even when
    // the change made the rest of the screen hard to read (ADR 0028).
    match &model.trial {
        Some(trial) => iced::widget::Stack::with_children([
            window.into(),
            container(trial_dialog(trial, tokens, &geometry))
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(move |_theme| theme::scrim(tokens))
                .into(),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
        None => window.into(),
    }
}

/// The question a display trial asks, and the two answers to it.
///
/// The dialog is its own surface, so it carries its own one title; the sentence
/// under it is body text because it is the thing the user has to read, and the
/// reason an answer could not be taken is a caption in the danger colour.
fn trial_dialog<'a>(
    trial: &TrialView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let mut card: Vec<Element<'a, Message>> = vec![
        text(trial.title.clone())
            .size(geometry.title_size)
            .font(theme::emphasis_font())
            .color(theme::text(tokens))
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
        text(trial.message.clone())
            .size(geometry.body_size)
            .color(theme::text(tokens))
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
    ];

    if let Some(error) = &trial.error {
        card.push(
            text(error.clone())
                .size(geometry.caption_size)
                .color(theme::danger_text(tokens))
                .wrapping(Wrapping::WordOrGlyph)
                .into(),
        );
    }

    card.push(
        Row::with_children(
            trial
                .choices
                .iter()
                .map(|choice| trial_choice_button(choice, tokens, geometry))
                .collect::<Vec<Element<'a, Message>>>(),
        )
        .spacing(geometry.inline_gap)
        .align_y(iced::alignment::Vertical::Center)
        .wrap()
        .into(),
    );

    container(Column::with_children(card).spacing(geometry.row_gap))
        .width(Length::Fixed(geometry.dialog_width))
        .padding(geometry.card_padding)
        .style(move |_theme| theme::dialog(tokens))
        .into()
}

/// One answer of the display trial.
///
/// The two answers are drawn with the same control as the window's actions, so
/// the focus ring and the disabled look are the ones the user already knows
/// (acceptance D05, Q01).
fn trial_choice_button<'a>(
    choice: &TrialChoiceView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let focused = choice.focused;
    let message = match choice.choice {
        TrialChoice::Confirm => Message::TrialConfirm,
        TrialChoice::Revert => Message::TrialRevert,
    };
    // Confirming keeps the change; restoring it is the answer that throws the
    // change away, and it is drawn the way every other discarding action is.
    let role = match choice.choice {
        TrialChoice::Confirm => Role::Primary,
        TrialChoice::Revert => Role::Danger,
    };

    let mut column: Vec<Element<'a, Message>> = vec![
        button(
            text(choice.label.clone())
                .size(geometry.body_size)
                .wrapping(Wrapping::None),
        )
        .padding(geometry.control_padding)
        .height(Length::Fixed(geometry.control_height))
        .on_press_maybe(choice.enabled.then_some(message))
        .style(move |_theme, status| theme::action(tokens, role, focused, status))
        .into(),
    ];

    if let Some(reason) = &choice.disabled_reason {
        column.push(
            text(reason.clone())
                .size(geometry.caption_size)
                .color(theme::muted_text(tokens))
                .wrapping(Wrapping::WordOrGlyph)
                .into(),
        );
    }

    container(Column::with_children(column).spacing(geometry.row_gap))
        .id(crate::focus::target_id(
            crate::view_model::FocusTarget::Trial(choice.choice),
        ))
        .into()
}

/// Draws one page: its title, its explanation and its cards.
///
/// The page modules of [`crate::pages`] call this, which is what makes a page's
/// own `view` the place its drawing is described rather than a second copy of
/// the same card layout in four files.
pub fn page_section<'a>(
    page: &PageView,
    fields: &[FieldView],
    tokens: &'a Tokens,
) -> Element<'a, Message> {
    let geometry = SettingsGeometry::new(tokens);

    let mut blocks: Vec<Element<'a, Message>> = vec![page_header(page, tokens, &geometry)];

    // Why the page cannot be changed is said once, above the cards, rather than
    // beside every control: the sentence is the same one for all of them and a
    // form that repeats it thirty times is a form nobody reads (ui-guidelines
    // §5, §6).
    if let Some(notice) = disabled_notice(fields, tokens, &geometry) {
        blocks.push(notice);
    }

    for group in &page.groups {
        blocks.push(group_card(group, fields, tokens, &geometry));
    }

    Column::with_children(blocks)
        .spacing(geometry.section_gap)
        .width(Length::Fill)
        .into()
}

/// The one title a screen carries, and what it is for.
fn page_header<'a>(
    page: &PageView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    Column::with_children([
        text(page.label.clone())
            .size(geometry.title_size)
            .font(theme::emphasis_font())
            .color(theme::text(tokens))
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
        text(page.description.clone())
            .size(geometry.caption_size)
            .color(theme::muted_text(tokens))
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
    ])
    .spacing(geometry.row_gap)
    .width(Length::Fill)
    .into()
}

/// What the window says when the page's controls cannot be used.
///
/// A disabled control has to explain itself (acceptance D05); every field of a
/// page is disabled for the same reason — a write in flight, or a display trial
/// waiting for an answer — so the reason is said once, as a caption, and the
/// controls below it are drawn the way [`crate::theme::control`] draws a
/// disabled control.
fn disabled_notice<'a>(
    fields: &[FieldView],
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Option<Element<'a, Message>> {
    let reasons = distinct_reasons(fields.iter().filter(|field| !field.enabled))?;

    Some(
        container(
            Column::with_children(
                reasons
                    .into_iter()
                    .map(|reason| {
                        text(reason)
                            .size(geometry.caption_size)
                            .color(theme::muted_text(tokens))
                            .wrapping(Wrapping::WordOrGlyph)
                            .into()
                    })
                    .collect::<Vec<Element<'a, Message>>>(),
            )
            .spacing(geometry.row_gap),
        )
        .width(Length::Fill)
        .padding(geometry.notice_padding)
        .style(move |_theme| theme::card(tokens))
        .into(),
    )
}

/// The reasons among `fields`, each once, in the order they appear.
fn distinct_reasons<'a>(fields: impl Iterator<Item = &'a FieldView>) -> Option<Vec<String>> {
    let mut reasons: Vec<String> = Vec::new();

    for field in fields {
        if let Some(reason) = &field.disabled_reason
            && !reasons.contains(reason)
        {
            reasons.push(reason.clone());
        }
    }

    (!reasons.is_empty()).then_some(reasons)
}

/// One card: a heading and the fields of one topic.
fn group_card<'a>(
    group: &GroupView,
    fields: &[FieldView],
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let mut children: Vec<Element<'a, Message>> = vec![
        text(group.title.clone())
            .size(geometry.heading_size)
            .font(theme::emphasis_font())
            .color(theme::text(tokens))
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
    ];

    for field in &group.fields {
        if let Some(view) = fields.iter().find(|view| view.field == *field) {
            children.push(field_row(view, tokens, geometry));
        }
    }

    container(Column::with_children(children).spacing(geometry.row_gap))
        .width(Length::Fill)
        .padding(geometry.card_padding)
        .style(move |_theme| theme::card(tokens))
        .into()
}

/// One entry of the navigation list.
///
/// The three states a user has to tell apart are the one holding the open page,
/// the one the pointer is over, and the one the keyboard is on. They are drawn
/// by [`crate::theme::control`], which is where the screenshot test checks them.
fn page_entry<'a>(
    page: &PageView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let emphasis = Emphasis {
        selected: page.selected,
        focused: page.focused,
    };
    let page_id = page.page;

    container(
        button(
            text(page.label.clone())
                .size(geometry.body_size)
                .wrapping(Wrapping::WordOrGlyph),
        )
        .width(Length::Fill)
        .padding(geometry.list_padding)
        .on_press(Message::PageSelected(page_id))
        .style(move |_theme, status| theme::control(tokens, emphasis, status)),
    )
    .id(crate::focus::target_id(
        crate::view_model::FocusTarget::Page(page_id),
    ))
    .into()
}

/// One field: its label, its control, its reset action and its error.
fn field_row<'a>(
    field: &FieldView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    if let ControlView::Collection { .. } = &field.control {
        return collection_field(field, tokens, geometry);
    }

    let label_colour = if field.enabled {
        theme::text(tokens)
    } else {
        theme::muted_text(tokens)
    };

    let label = container(
        text(field.label.clone())
            .size(geometry.body_size)
            .color(label_colour)
            .wrapping(Wrapping::WordOrGlyph),
    )
    .width(Length::Fixed(geometry.label_width));

    let mut row: Vec<Element<'a, Message>> =
        vec![label.into(), control_widget(field, tokens, geometry)];

    if let Some(reset) = &field.reset {
        row.push(reset_button(field, reset, tokens, geometry));
    }

    let row = Row::with_children(row)
        .spacing(geometry.inline_gap)
        .align_y(iced::alignment::Vertical::Center);

    // A refused value says which key and which rule, as a caption under the
    // control it belongs to, so the two are read together (ui-guidelines §6).
    match &field.error {
        None => container(row)
            .id(crate::focus::target_id(
                crate::view_model::FocusTarget::Field(field.field),
            ))
            .into(),
        Some(error) => Column::with_children([
            container(row)
                .id(crate::focus::target_id(
                    crate::view_model::FocusTarget::Field(field.field),
                ))
                .into(),
            container(
                text(error.clone())
                    .size(geometry.caption_size)
                    .color(theme::danger_text(tokens))
                    .wrapping(Wrapping::WordOrGlyph),
            )
            .padding(Padding {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: geometry.label_width + geometry.inline_gap,
            })
            .into(),
        ])
        .spacing(geometry.row_gap)
        .into(),
    }
}

/// A structured list: what it is for, what it holds now, and how to change it.
fn collection_field<'a>(
    field: &FieldView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let ControlView::Collection {
        focused,
        rows,
        columns,
        add,
        remove,
        help,
        empty,
    } = &field.control
    else {
        return text("").into();
    };

    let key = field.field;
    let refused = field.error.is_some();
    let focused = *focused;
    let field_focused = field.focused;
    let label_colour = if field.enabled {
        theme::text(tokens)
    } else {
        theme::muted_text(tokens)
    };

    let mut entries: Vec<Element<'a, Message>> = vec![
        text(field.label.clone())
            .size(geometry.body_size)
            .color(label_colour)
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
        text(help.clone())
            .size(geometry.caption_size)
            .color(theme::muted_text(tokens))
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
    ];

    // An empty list is a state of its own: it says what the window will do, so a
    // user who sees no rows knows that is a decision rather than a failure
    // (ui-guidelines §6). A header over no rows would only be noise.
    if rows.is_empty() {
        entries.push(
            text(empty.clone())
                .size(geometry.caption_size)
                .color(theme::muted_text(tokens))
                .wrapping(Wrapping::WordOrGlyph)
                .into(),
        );

        entries.push(column_actions(
            field,
            add,
            key,
            field_focused,
            tokens,
            geometry,
        ));

        return Column::with_children(entries)
            .spacing(geometry.row_gap)
            .width(Length::Fill)
            .into();
    }

    // A list of several columns is a table while its columns can hold their own
    // names. At a doubled font scale a seven-column table gives each column less
    // room than the longest word in it, and a header broken in the middle of a
    // word stops naming the field under it — so the list is drawn one labelled
    // field per row instead (ui-guidelines §7).
    let as_a_table = columns.len() <= 1 || geometry.list_as_table;

    if as_a_table {
        let mut header: Vec<Element<'a, Message>> = columns
            .iter()
            .enumerate()
            .map(|(column, label)| {
                container(
                    text(label.clone())
                        .size(geometry.caption_size)
                        .color(theme::muted_text(tokens))
                        .wrapping(Wrapping::WordOrGlyph),
                )
                .width(Length::FillPortion(column_weight(key, column)))
                .into()
            })
            .collect();

        // The actions of a row are their own column, so the header keeps a cell
        // for them: without it the column names would drift off the fields they
        // name.
        header.push(
            container(text(""))
                .width(Length::Fixed(geometry.list_action_width))
                .into(),
        );

        entries.push(
            Row::with_children(header)
                .spacing(geometry.inline_gap)
                .align_y(iced::alignment::Vertical::Bottom)
                .into(),
        );
    }

    for (row, cells) in rows.iter().enumerate() {
        let mut inputs: Vec<Element<'a, Message>> = cells
            .iter()
            .enumerate()
            .map(|(column, value)| {
                if as_a_table {
                    cell_input(
                        field,
                        key,
                        row,
                        column,
                        value,
                        tokens,
                        geometry,
                        refused,
                        Length::FillPortion(column_weight(key, column)),
                    )
                } else {
                    Row::with_children([
                        container(
                            text(columns[column].clone())
                                .size(geometry.caption_size)
                                .color(theme::muted_text(tokens))
                                .wrapping(Wrapping::WordOrGlyph),
                        )
                        .width(Length::Fixed(geometry.label_width))
                        .into(),
                        cell_input(
                            field,
                            key,
                            row,
                            column,
                            value,
                            tokens,
                            geometry,
                            refused,
                            Length::Fill,
                        ),
                    ])
                    .spacing(geometry.inline_gap)
                    .align_y(iced::alignment::Vertical::Center)
                    .into()
                }
            })
            .collect();

        inputs.push(remove_row_button(
            field, remove, key, row, focused, tokens, geometry,
        ));

        entries.push(if as_a_table {
            Row::with_children(inputs)
                .spacing(geometry.inline_gap)
                .align_y(iced::alignment::Vertical::Center)
                .into()
        } else {
            Column::with_children(inputs)
                .spacing(geometry.row_gap)
                .into()
        });
    }

    entries.push(column_actions(
        field,
        add,
        key,
        field_focused,
        tokens,
        geometry,
    ));

    Column::with_children(entries)
        .spacing(geometry.row_gap)
        .width(Length::Fill)
        .into()
}

/// One editable cell of a list, at the width its column gets.
#[allow(clippy::too_many_arguments)]
fn cell_input<'a>(
    field: &FieldView,
    key: Field,
    row: usize,
    column: usize,
    value: &str,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
    refused: bool,
    width: Length,
) -> Element<'a, Message> {
    text_input("", value)
        .id(crate::focus::target_id(
            crate::view_model::FocusTarget::Cell(key, row, column),
        ))
        .on_submit(Message::FocusNext)
        .on_input_maybe(field.enabled.then_some(move |text| Message::CellEdited {
            field: key,
            row,
            column,
            text,
        }))
        .size(geometry.body_size)
        .line_height(geometry.line_height)
        .padding(geometry.control_padding)
        .width(width)
        .style(move |_theme, status| theme::input(tokens, status, refused))
        .into()
}

/// What a list can be changed by: adding a row, and what went wrong with what it
/// already holds.
fn column_actions<'a>(
    field: &FieldView,
    add: &str,
    key: Field,
    field_focused: bool,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let mut footer: Vec<Element<'a, Message>> = vec![
        container(
            button(
                text(add.to_owned())
                    .size(geometry.body_size)
                    .wrapping(Wrapping::None),
            )
            .padding(geometry.control_padding)
            .height(Length::Fixed(geometry.control_height))
            .on_press_maybe(field.enabled.then_some(Message::RowAdded(key)))
            .style(move |_theme, status| {
                theme::action(tokens, Role::Secondary, field_focused, status)
            }),
        )
        .id(crate::focus::target_id(
            crate::view_model::FocusTarget::Field(key),
        ))
        .into(),
    ];

    if let Some(error) = &field.error {
        footer.push(
            text(error.clone())
                .size(geometry.caption_size)
                .color(theme::danger_text(tokens))
                .wrapping(Wrapping::WordOrGlyph)
                .into(),
        );
    }

    Column::with_children(footer)
        .spacing(geometry.row_gap)
        .into()
}

/// The remove action of one row.
///
/// Removing an entry is destructive, so it is drawn the way every other
/// discarding action is — danger text and outline, never a filled accent
/// (ui-guidelines §5).
fn remove_row_button<'a>(
    field: &FieldView,
    label: &str,
    key: Field,
    row: usize,
    focused: Option<crate::view_model::FocusTarget>,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let on_this_button = focused == Some(crate::view_model::FocusTarget::RemoveRow(key, row));

    container(
        button(
            text(label.to_owned())
                .size(geometry.body_size)
                .wrapping(Wrapping::WordOrGlyph),
        )
        .padding(geometry.control_padding)
        .width(Length::Fixed(geometry.list_action_width))
        .on_press_maybe(
            field
                .enabled
                .then_some(Message::RowRemoved { field: key, row }),
        )
        .style(move |_theme, status| theme::action(tokens, Role::Danger, on_this_button, status)),
    )
    .id(crate::focus::target_id(
        crate::view_model::FocusTarget::RemoveRow(key, row),
    ))
    .into()
}

/// A field's control, whichever kind it is.
fn control_widget<'a>(
    field: &FieldView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    match &field.control {
        // Handled by [`collection_field`]: a list is not a row.
        ControlView::Collection { .. } => text("").into(),
        ControlView::Text(value) => {
            let key = field.field;
            let refused = field.error.is_some();

            text_input("", value)
                .id(input_id(key))
                .on_submit(Message::FocusNext)
                .on_input_maybe(field.enabled.then_some(move |value| Message::FieldSet {
                    field: key,
                    value: Value::Text(value),
                }))
                .size(geometry.body_size)
                .line_height(geometry.line_height)
                .padding(geometry.control_padding)
                .width(control_length(key, geometry))
                .style(move |_theme, status| theme::input(tokens, status, refused))
                .into()
        }
        ControlView::Choices(options) => Row::with_children(
            options
                .iter()
                .map(|option| {
                    let selected = option.selected;
                    let focused = option.focused;
                    let pressed = Value::Choice(option.id.clone());

                    button(
                        text(option.label.clone())
                            .size(geometry.body_size)
                            .wrapping(Wrapping::None),
                    )
                    .padding(geometry.control_padding)
                    .height(Length::Fixed(geometry.control_height))
                    .on_press_maybe(field.enabled.then_some(Message::FieldSet {
                        field: field.field,
                        value: pressed,
                    }))
                    .style(move |_theme, status| theme::segment(tokens, selected, focused, status))
                    .into()
                })
                .collect::<Vec<Element<'a, Message>>>(),
        )
        .spacing(geometry.inline_gap)
        .align_y(iced::alignment::Vertical::Center)
        .wrap()
        .into(),
        ControlView::Flag { on, label } => {
            let on = *on;
            let focused = field.focused;

            button(
                text(label.clone())
                    .size(geometry.body_size)
                    .wrapping(Wrapping::None),
            )
            .padding(geometry.control_padding)
            .height(Length::Fixed(geometry.control_height))
            .on_press_maybe(field.enabled.then_some(Message::FieldSet {
                field: field.field,
                value: Value::Flag(!on),
            }))
            .style(move |_theme, status| theme::switch(tokens, on, focused, status))
            .into()
        }
        ControlView::Number {
            unit,
            input,
            value,
            min,
            max,
            display: _,
            decrease,
            increase,
        } => {
            let key = field.field;
            let refused = field.error.is_some();

            // The number, its unit and its two steps are one control: they never
            // wrap away from each other, whatever the font scale (Q02).
            Row::with_children([
                step_button(decrease, field, -1, tokens, geometry, *value > *min),
                text_input("", input)
                    .id(input_id(key))
                    .on_submit(Message::FocusNext)
                    .on_input_maybe(
                        field
                            .enabled
                            .then_some(move |text| Message::NumberEdited { field: key, text }),
                    )
                    .size(geometry.body_size)
                    .line_height(geometry.line_height)
                    .padding(geometry.control_padding)
                    .width(Length::Fixed(geometry.short_width))
                    .style(move |_theme, status| theme::input(tokens, status, refused))
                    .into(),
                text(unit.clone())
                    .size(geometry.caption_size)
                    .color(theme::muted_text(tokens))
                    .wrapping(Wrapping::None)
                    .into(),
                step_button(increase, field, 1, tokens, geometry, *value < *max),
            ])
            .spacing(geometry.inline_gap)
            .align_y(iced::alignment::Vertical::Center)
            .into()
        }
    }
}

/// One step button of a number control.
///
/// The button is disabled at the end of the range, in the sense a user
/// experiences: the value is drawn in the muted colour rather than stopping
/// silently. Activating it with the keyboard wraps instead, so no stop of the
/// range is unreachable by keyboard alone (Q01).
fn step_button<'a>(
    glyph: &str,
    field: &FieldView,
    delta: i64,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
    within_range: bool,
) -> Element<'a, Message> {
    let focused = field.focused;

    button(text(glyph.to_owned()).size(geometry.body_size))
        .padding(Padding::ZERO)
        .width(Length::Fixed(geometry.control_height))
        .height(Length::Fixed(geometry.control_height))
        .on_press_maybe(
            (field.enabled && within_range).then_some(Message::FieldStepped {
                field: field.field,
                delta,
            }),
        )
        .style(move |_theme, status| theme::segment(tokens, false, focused, status))
        .into()
}

/// The action that returns one field to the schema's default.
///
/// Returning one value to its default throws a change away, so it is a danger
/// action: danger text and outline, and never the accent (ui-guidelines §5).
fn reset_button<'a>(
    field: &FieldView,
    label: &str,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let focused = field.reset_focused;
    let target = field.field;

    container(
        button(
            text(label.to_owned())
                .size(geometry.body_size)
                .wrapping(Wrapping::None),
        )
        .padding(geometry.control_padding)
        .height(Length::Fixed(geometry.control_height))
        .on_press_maybe(field.enabled.then_some(Message::FieldReset(target)))
        .style(move |_theme, status| theme::action(tokens, Role::Danger, focused, status)),
    )
    .id(crate::focus::target_id(
        crate::view_model::FocusTarget::Reset(target),
    ))
    .into()
}

/// The status line, and what the components answered.
///
/// The sentence is body text because it is the thing the user has to read; what
/// each component answered and what went wrong are captions under it.
fn status_line<'a>(
    model: &SettingsViewModel,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let kind = model.status.kind;
    let colour = match kind {
        crate::view_model::StatusKind::Rejected => theme::danger_text(tokens),
        crate::view_model::StatusKind::Pending
        | crate::view_model::StatusKind::SavedOnly
        | crate::view_model::StatusKind::Provisional => theme::warning_text(tokens),
        crate::view_model::StatusKind::Applied => theme::success_text(tokens),
        _ => theme::muted_text(tokens),
    };

    let mut lines: Vec<Element<'a, Message>> = vec![
        text(model.status.message.clone())
            .size(geometry.body_size)
            .color(colour)
            .wrapping(Wrapping::WordOrGlyph)
            .into(),
    ];

    lines.extend(model.status.details.iter().map(|detail| {
        text(detail.clone())
            .size(geometry.caption_size)
            .color(theme::muted_text(tokens))
            .wrapping(Wrapping::WordOrGlyph)
            .into()
    }));

    container(Column::with_children(lines).spacing(geometry.row_gap))
        .width(Length::Fill)
        .padding(geometry.notice_padding)
        .style(move |_theme| theme::status(tokens, kind))
        .into()
}

/// The actions at the foot of the window.
///
/// Apply is the one accent action; Cancel and Reload are ordinary ones; Reset
/// all is the one that throws work away and is drawn as a danger action. What
/// cannot be done is said under them, because a grey button with no explanation
/// is the defect the guidelines name (ui-guidelines §5, §9).
fn action_row<'a>(
    model: &SettingsViewModel,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let mut rows: Vec<Element<'a, Message>> = vec![
        Row::with_children(
            model
                .actions
                .iter()
                .map(|action| action_button(action, tokens, geometry))
                .collect::<Vec<Element<'a, Message>>>(),
        )
        .spacing(geometry.inline_gap)
        .align_y(iced::alignment::Vertical::Center)
        .into(),
    ];

    let mut reasons: Vec<String> = Vec::new();

    for action in &model.actions {
        if let Some(reason) = &action.disabled_reason
            && !reasons.contains(reason)
        {
            reasons.push(reason.clone());
        }
    }

    rows.extend(reasons.into_iter().map(|reason| {
        text(reason)
            .size(geometry.caption_size)
            .color(theme::muted_text(tokens))
            .wrapping(Wrapping::WordOrGlyph)
            .into()
    }));

    Column::with_children(rows)
        .spacing(geometry.row_gap)
        .width(Length::Fill)
        .into()
}

/// One action.
fn action_button<'a>(
    action: &ActionView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    // Apply is the action that writes the files and asks the components to apply
    // them, so it is the one the accent colour is spent on. Reset all is the one
    // action here that discards work that has not been written.
    let role = match action.action {
        Action::Apply => Role::Primary,
        Action::ResetAll => Role::Danger,
        Action::Cancel | Action::Reload => Role::Secondary,
    };
    let focused = action.focused;
    let enabled = action.enabled;
    let message = action_message(action.action);

    container(
        button(
            text(action.label.clone())
                .size(geometry.body_size)
                .wrapping(Wrapping::None),
        )
        .padding(geometry.control_padding)
        .height(Length::Fixed(geometry.control_height))
        .on_press_maybe(enabled.then_some(message))
        .style(move |_theme, status| theme::action(tokens, role, focused, status)),
    )
    .id(crate::focus::target_id(
        crate::view_model::FocusTarget::Action(action.action),
    ))
    .into()
}

/// The message an action sends.
pub fn action_message(action: Action) -> Message {
    match action {
        Action::Apply => Message::Apply,
        Action::Cancel => Message::Cancel,
        Action::ResetAll => Message::AllReset,
        Action::Reload => Message::Reload,
    }
}

/// The stable widget identifier used to focus a field's text input.
pub fn input_id(field: crate::draft::Field) -> String {
    format!("settings-{}-{}", field.file().name(), field.path())
}

/// How wide a field's control is drawn, by what the value means.
///
/// The guidelines (§4) fix three widths and forbid a fourth: a number with its
/// unit is at most six extra-large steps, a connector, a format string, a path
/// or a command at most twelve, and only explanatory text and tables take the
/// window's width. Everything else sizes to its own content.
fn control_length(field: Field, geometry: &SettingsGeometry) -> Length {
    match field {
        // A clock format is a string a user reads in full; a layout and a
        // variant are identifiers of a couple of characters. The wallpaper
        // chooser is a row of options, so it takes the generic `Choice` arm.
        Field::PanelClockFormat => Length::Fixed(geometry.wide_width),
        Field::KeyboardLayout | Field::KeyboardVariant => Length::Fixed(geometry.short_width),
        other => match other.control().kind {
            ValueKind::Number => Length::Fixed(geometry.short_width),
            ValueKind::Text => Length::Fixed(geometry.wide_width),
            ValueKind::Collection | ValueKind::Choice | ValueKind::Flag => Length::Shrink,
        },
    }
}

/// How much of a row one column of a list takes, relative to the others.
///
/// The columns of a list share the card between them instead of being fixed, so
/// the row always fits and no column is ever drawn as wide as the window; the
/// weights keep the proportion the guidelines' bound describes — a connector is
/// roomier than a number.
fn column_weight(field: Field, column: usize) -> u16 {
    match (field, column) {
        (Field::Outputs, 0) => 12,
        (Field::Outputs, _) => 6,
        _ => 12,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::Application;
    use crate::draft::Draft;
    use crate::draft::Settings;
    use crate::view_model::{SettingsInput, StatusKind};
    use inox_common::i18n::Locale;

    /// The source of this file, for the checks that look for a number where a
    /// token belongs.
    const SOURCE: &str = include_str!("view.rs");

    fn model() -> SettingsViewModel {
        let draft = Draft::new(Settings::defaults());
        let application = Application::new();
        let errors = crate::validate::validate(draft.settings());
        let localizer = crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse");

        SettingsInput {
            page: crate::pages::PageId::Panel,
            draft: &draft,
            application: &application,
            errors: &errors,
            focused: None,
            trial: None,
            notice: None,
        }
        .project(&localizer)
    }

    fn fixed(length: Length) -> f32 {
        match length {
            Length::Fixed(pixels) => pixels,
            other => panic!("expected a fixed width, got {other:?}"),
        }
    }

    #[test]
    fn the_geometry_comes_from_the_tokens() {
        let tokens = Tokens::builtin_dark();
        let geometry = SettingsGeometry::new(&tokens);

        assert_eq!(geometry.page_padding, theme::page_padding(&tokens));
        assert_eq!(geometry.card_padding, theme::card_padding(&tokens));
        assert_eq!(geometry.notice_padding, theme::notice_padding(&tokens));
        assert!((geometry.section_gap - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON);
        assert!((geometry.row_gap - f32::from(tokens.spacing.sm)).abs() < f32::EPSILON);
        assert!(
            (geometry.nav_width - f32::from(tokens.spacing.xl) * NAV_WIDTH_STEPS).abs()
                < f32::EPSILON
        );
        assert!(geometry.label_width > geometry.nav_width / 2.0);
        assert!(geometry.control_height >= f32::from(tokens.spacing.xl));
        assert!(geometry.dialog_width > geometry.wide_width);

        // One screen carries one title, its cards carry headings and everything
        // that explains something is a caption (ui-guidelines §2).
        assert_eq!(geometry.title_size, theme::title_size(&tokens));
        assert_eq!(geometry.heading_size, theme::heading_size(&tokens));
        assert_eq!(geometry.body_size, theme::body_size(&tokens));
        assert_eq!(geometry.caption_size, theme::caption_size(&tokens));
        assert!(geometry.title_size > geometry.heading_size);
        assert!(geometry.heading_size > geometry.body_size);
        assert!(geometry.body_size > geometry.caption_size);
        assert_eq!(
            geometry.list_action_width,
            theme::list_action_width(&tokens)
        );
        assert!(geometry.list_as_table);
    }

    #[test]
    fn scaled_text_grows_with_the_tokens_and_the_columns_with_it() {
        let mut tokens = Tokens::builtin_dark();
        let plain = SettingsGeometry::new(&tokens);

        tokens.typography.scale = 2.0;
        let scaled = SettingsGeometry::new(&tokens);

        assert!(scaled.body_size > plain.body_size);
        assert!(scaled.nav_width > plain.nav_width);
        assert!(scaled.label_width > plain.label_width);
        assert!(scaled.control_height > plain.control_height);
        assert!(scaled.heading_size > plain.heading_size);
        assert!(
            !scaled.list_as_table,
            "a table whose columns are narrower than the words in its headings is \
             drawn one field per row instead"
        );
    }

    #[test]
    fn a_short_field_is_never_drawn_as_wide_as_a_window() {
        let tokens = Tokens::builtin_dark();
        let geometry = SettingsGeometry::new(&tokens);
        let xl = f32::from(tokens.spacing.xl);

        for field in Field::ALL {
            let width = control_length(field, &geometry);

            match (&field.control().kind, width) {
                (ValueKind::Number, Length::Fixed(pixels)) => assert!(
                    pixels <= xl * 6.0,
                    "`{field}` is a number drawn {pixels} wide, more than 6 × xl"
                ),
                (ValueKind::Text, Length::Fixed(pixels)) => assert!(
                    pixels <= xl * 12.0,
                    "`{field}` is text drawn {pixels} wide, more than 12 × xl"
                ),
                (_, Length::Fixed(pixels)) => assert!(
                    pixels <= xl * 12.0,
                    "`{field}` is drawn {pixels} wide, more than 12 × xl"
                ),
                _ => {}
            }
        }

        assert!(
            fixed(control_length(Field::PanelHeight, &geometry)) <= xl * 6.0,
            "a number is a short field"
        );
        assert!(
            fixed(control_length(Field::PanelClockFormat, &geometry))
                > fixed(control_length(Field::PanelHeight, &geometry)),
            "a format string is wider than a number"
        );
    }

    #[test]
    fn a_list_never_lets_one_column_take_the_whole_row() {
        let outputs: Vec<u16> = (0..7)
            .map(|column| column_weight(Field::Outputs, column))
            .collect();

        assert!(outputs.iter().all(|weight| *weight > 0));
        assert!(
            outputs[0] >= outputs[1],
            "a connector is roomier than a number"
        );
        assert_eq!(
            column_weight(Field::WallpaperPerOutput, 1),
            column_weight(Field::WallpaperPerOutput, 0),
            "a path and a connector are both long values"
        );
    }

    #[test]
    fn the_window_is_built_from_the_model() {
        // Building the element is the assertion: a model with nothing changed
        // must still produce a window.
        let tokens = Tokens::builtin_dark();
        let model = model();
        let _element = settings(model, &tokens);
    }

    #[test]
    fn every_page_module_draws_the_section_the_window_puts_in_its_body() {
        let tokens = Tokens::builtin_dark();
        let model = model();

        for page in &model.pages {
            let page_id = page.page;
            let fields: Vec<FieldView> = page_id
                .fields()
                .iter()
                .filter_map(|field| model.field(*field).cloned())
                .collect();

            // Building the element is the assertion.
            let _section = crate::pages::view(page_id, &model, &tokens);
            let _direct = crate::pages::panel::view(page, &fields, &tokens);
        }
    }

    #[test]
    fn every_action_has_a_message_and_they_are_all_different() {
        let messages: Vec<Message> = [
            Action::Apply,
            Action::Cancel,
            Action::ResetAll,
            Action::Reload,
        ]
        .into_iter()
        .map(action_message)
        .collect();

        assert!(matches!(messages[0], Message::Apply));
        assert!(matches!(messages[1], Message::Cancel));
        assert!(matches!(messages[2], Message::AllReset));
        assert!(matches!(messages[3], Message::Reload));
    }

    #[test]
    fn the_status_kind_of_a_window_with_nothing_changed_is_drawn_calmly() {
        let tokens = Tokens::builtin_dark();
        let model = model();
        let element = status_line(&model, &tokens, &SettingsGeometry::new(&tokens));

        let _ = element;
        assert_eq!(model.status.kind, StatusKind::Unchanged);
    }

    #[test]
    fn every_kind_of_control_is_drawn() {
        let tokens = Tokens::builtin_dark();
        let geometry = SettingsGeometry::new(&tokens);
        let draft = Draft::new(Settings::defaults());

        for field in Field::ALL {
            let control = crate::view_model::control_view(
                field,
                &draft.field(field),
                None,
                &crate::i18n::load(&Locale::default_locale()).expect("the catalogs parse"),
            );
            let view = FieldView {
                field,
                label: "label".to_owned(),
                control,
                modified: false,
                focused: false,
                reset: Some("Reset".to_owned()),
                reset_focused: false,
                error: None,
                enabled: true,
                disabled_reason: None,
            };

            // Building the element is the assertion: every kind of control has a
            // drawing, and none of them needs a live window to be laid out.
            let _row = field_row(&view, &tokens, &geometry);
        }
    }

    #[test]
    fn a_page_with_a_disabled_control_says_why() {
        let tokens = Tokens::builtin_dark();
        let geometry = SettingsGeometry::new(&tokens);
        let disabled = vec![FieldView {
            field: Field::PanelHeight,
            label: "Height".to_owned(),
            control: ControlView::Text("48".to_owned()),
            modified: false,
            focused: false,
            reset: None,
            reset_focused: false,
            error: None,
            enabled: false,
            disabled_reason: Some("A save is in progress".to_owned()),
        }];

        let notice = disabled_notice(&disabled, &tokens, &geometry);
        assert!(notice.is_some(), "a disabled control explains itself");

        let enabled = vec![FieldView {
            enabled: true,
            disabled_reason: None,
            ..disabled[0].clone()
        }];
        assert!(
            disabled_notice(&enabled, &tokens, &geometry).is_none(),
            "a page a user can work says nothing"
        );
    }

    #[test]
    fn two_controls_disabled_for_the_same_reason_say_it_once() {
        let field = |reason: &str| FieldView {
            field: Field::PanelHeight,
            label: "Height".to_owned(),
            control: ControlView::Text("48".to_owned()),
            modified: false,
            focused: false,
            reset: None,
            reset_focused: false,
            error: None,
            enabled: false,
            disabled_reason: Some(reason.to_owned()),
        };

        assert_eq!(
            distinct_reasons([field("a"), field("a"), field("b")].iter()),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(distinct_reasons([].iter()), None);
    }

    #[test]
    fn the_view_draws_no_colour_and_no_size_of_its_own() {
        // The guidelines' checklist asks for a check a reviewer can repeat:
        // `view.rs` holds no literal colour and no literal padding or size
        // (ui-guidelines §1, §10). Every pattern is assembled from two pieces
        // here so that this test's own text is not one of the matches.
        fn pattern(head: &str, tail: &str) -> String {
            format!("{head}{tail}")
        }

        let forbidden = [
            pattern("Color::", "from_rgba8"),
            pattern("Color::", "BLACK"),
            pattern("Color::", "WHITE"),
            pattern("Color::", "TRANSPARENT"),
            pattern("padding(", "1"),
            pattern("padding(", "2"),
            pattern(".size(", "1"),
            pattern(".size(", "2"),
        ];

        for pattern in &forbidden {
            assert!(
                !SOURCE.contains(pattern.as_str()),
                "`{pattern}` is a value the tokens should have supplied"
            );
        }
    }
}
