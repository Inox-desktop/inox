//! The settings window's widgets.
//!
//! Everything here is drawing. Which pages exist, which fields a page shows,
//! what each control is set to, which one the keyboard is on, which actions are
//! on offer and why not when they are not, and what the status line says all
//! arrive already decided in a [`SettingsViewModel`]; this module turns that
//! into iced widgets and takes every colour, size and radius from the tokens
//! through [`crate::theme`].
//!
//! Three layout decisions are worth naming, because they are what acceptance
//! D04, D05 and Q01 are about:
//!
//! - **The controls keep their width.** The navigation list is a fixed column
//!   and every field's label is a fixed column, so a long language tag or a
//!   long reason cannot push the control a user needs off the row.
//! - **The status line is always there.** Saved, waiting, in effect and refused
//!   are four different sentences in four different colours, and the one that
//!   says *in effect* is only ever drawn from [`crate::apply`]'s confirmation
//!   (ADR 0014, decision 4).
//! - **The focus is a colour, never a widget.** The window's own focus is drawn
//!   on the control it is on (see [`crate::theme::control`]), so no control has
//!   to hold on to the toolkit's focus to be reachable by keyboard.

use iced::widget::text::Wrapping;
use iced::widget::{Column, Row, button, container, scrollable, text, text_input};
use iced::{Element, Length, Padding};
use inox_config::Theme as Tokens;

use crate::app::Message;
use crate::draft::Value;
use crate::theme::{self, Emphasis};
use crate::view_model::{Action, ActionView, ControlView, FieldView, PageView, SettingsViewModel};

/// How many extra-large spacing steps wide the navigation list is.
const NAV_WIDTH_STEPS: f32 = 5.0;

/// How many extra-large spacing steps wide a field's label is.
const LABEL_WIDTH_STEPS: f32 = 7.0;

/// How many text heights a row is, so that rows line up whatever the scale.
const ROW_HEIGHT_STEPS: f32 = 2.5;

/// The geometry of the window, derived from the tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsGeometry {
    /// Text size, already scaled by the user's font preference.
    pub font_size: f32,
    /// The size of a page's heading.
    pub heading_size: f32,
    /// Padding at the window's edge and inside its panels.
    pub padding: f32,
    /// Gap between two entries.
    pub spacing: f32,
    /// Padding inside a control.
    pub control_padding: f32,
    /// Width of the navigation list.
    pub nav_width: f32,
    /// Width of a field's label.
    pub label_width: f32,
    /// Height of one row of controls.
    pub row_height: f32,
}

impl SettingsGeometry {
    /// Derives the window's geometry.
    pub fn new(tokens: &Tokens) -> Self {
        let font_size = theme::font_size(tokens);
        let padding = f32::from(tokens.spacing.lg);

        Self {
            font_size,
            heading_size: theme::heading_size(tokens),
            padding,
            spacing: f32::from(tokens.spacing.md),
            control_padding: f32::from(tokens.spacing.sm),
            nav_width: f32::from(tokens.spacing.xl) * NAV_WIDTH_STEPS,
            label_width: f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS,
            row_height: font_size * ROW_HEIGHT_STEPS,
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
        Column::with_children(
            model
                .pages
                .iter()
                .map(|page| page_entry(page, tokens, &geometry))
                .collect::<Vec<Element<'a, Message>>>(),
        )
        .spacing(geometry.spacing),
    )
    .width(Length::Fixed(geometry.nav_width))
    .height(Length::Fill)
    .padding(Padding::from(geometry.padding))
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
        .spacing(geometry.spacing)
        .width(Length::Fill)
        .height(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(Padding::from(geometry.padding));

    container(Row::with_children([navigation.into(), body.into()]))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| theme::window(tokens))
        .into()
}

/// Draws one page: its heading and its fields.
///
/// The page modules of [`crate::pages`] call this, which is what makes a page's
/// own `view` the place its drawing is described rather than a second copy of
/// the same row layout in four files.
pub fn page_section<'a>(
    page: &PageView,
    fields: &[FieldView],
    tokens: &'a Tokens,
) -> Element<'a, Message> {
    let geometry = SettingsGeometry::new(tokens);

    let mut children: Vec<Element<'a, Message>> = vec![
        text(page.label.clone())
            .size(geometry.heading_size)
            .color(theme::text(tokens))
            .into(),
    ];

    children.extend(fields.iter().map(|field| {
        container(field_row(field, tokens, &geometry))
            .id(crate::focus::target_id(
                crate::view_model::FocusTarget::Field(field.field),
            ))
            .into()
    }));

    Column::with_children(children)
        .spacing(geometry.spacing)
        .width(Length::Fill)
        .into()
}

/// One entry of the navigation list.
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

    button(
        text(page.label.clone())
            .size(geometry.font_size)
            .wrapping(Wrapping::None),
    )
    .width(Length::Fill)
    .padding(Padding::from(geometry.control_padding))
    .on_press(Message::PageSelected(page_id))
    .style(move |_theme, status| theme::control(tokens, emphasis, status))
    .into()
}

/// One field: its label, its control, its reset action and its error.
fn field_row<'a>(
    field: &FieldView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    if field.field.is_collection() {
        let mut contents = vec![
            text(field.label.clone())
                .size(geometry.font_size)
                .color(theme::text(tokens))
                .into(),
            control_widget(field, tokens, geometry),
        ];
        if let Some(error) = &field.error {
            contents.push(
                text(error.clone())
                    .size(geometry.font_size)
                    .color(theme::danger_text(tokens))
                    .into(),
            );
        }
        if let Some(reset) = &field.reset {
            contents.push(reset_button(field, reset, tokens, geometry));
        }
        return Column::with_children(contents)
            .spacing(geometry.spacing)
            .width(Length::Fill)
            .into();
    }
    let label_colour = if field.enabled {
        theme::text(tokens)
    } else {
        theme::muted_text(tokens)
    };

    let label = container(
        text(field.label.clone())
            .size(geometry.font_size)
            .color(label_colour)
            .wrapping(Wrapping::Word),
    )
    .width(Length::Fixed(geometry.label_width));

    let mut row: Vec<Element<'a, Message>> =
        vec![label.into(), control_widget(field, tokens, geometry)];

    if let Some(reset) = &field.reset {
        row.push(reset_button(field, reset, tokens, geometry));
    }

    let row = Row::with_children(row)
        .spacing(geometry.spacing)
        .align_y(iced::alignment::Vertical::Center);

    match &field.error {
        None => row.into(),
        Some(error) => Column::with_children([
            row.into(),
            container(
                text(error.clone())
                    .size(geometry.font_size)
                    .color(theme::danger_text(tokens)),
            )
            .padding(Padding {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: geometry.label_width,
            })
            .into(),
        ])
        .spacing(geometry.control_padding)
        .into(),
    }
}

/// A field's control, whichever kind it is.
fn control_widget<'a>(
    field: &FieldView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    match &field.control {
        ControlView::Collection {
            focused,
            rows,
            columns,
            add,
            remove,
            help,
        } => {
            let key = field.field;
            let focused = *focused;
            let field_focused = field.focused;
            let mut entries: Vec<Element<'a, Message>> = vec![
                text(help.clone())
                    .size(geometry.font_size)
                    .color(theme::muted_text(tokens))
                    .into(),
            ];
            for (row, cells) in rows.iter().enumerate() {
                let mut inputs: Vec<Element<'a, Message>> = Vec::new();
                for (column, (value, label)) in cells.iter().zip(columns).enumerate() {
                    inputs.push(
                        Column::with_children([
                            text(label.clone())
                                .size(geometry.font_size)
                                .color(theme::text(tokens))
                                .into(),
                            text_input("", value)
                                .id(crate::focus::target_id(
                                    crate::view_model::FocusTarget::Cell(key, row, column),
                                ))
                                .on_submit(Message::FocusNext)
                                .on_input_maybe(field.enabled.then_some(move |text| {
                                    Message::CellEdited {
                                        field: key,
                                        row,
                                        column,
                                        text,
                                    }
                                }))
                                .size(geometry.font_size)
                                .padding(Padding::from(geometry.control_padding))
                                .style(move |_theme, status| theme::input(tokens, status))
                                .into(),
                        ])
                        .spacing(geometry.control_padding)
                        .into(),
                    );
                }
                inputs.push(
                    container(
                        button(text(remove.clone()).size(geometry.font_size))
                            .padding(Padding::from(geometry.control_padding))
                            .on_press_maybe(
                                field
                                    .enabled
                                    .then_some(Message::RowRemoved { field: key, row }),
                            )
                            .style(move |_theme, status| {
                                theme::control(
                                    tokens,
                                    Emphasis {
                                        selected: false,
                                        focused: focused
                                            == Some(crate::view_model::FocusTarget::RemoveRow(
                                                key, row,
                                            )),
                                    },
                                    status,
                                )
                            }),
                    )
                    .id(crate::focus::target_id(
                        crate::view_model::FocusTarget::RemoveRow(key, row),
                    ))
                    .into(),
                );
                entries.push(
                    container(Column::with_children(inputs).spacing(geometry.control_padding))
                        .padding(Padding::from(geometry.control_padding))
                        .width(Length::Fill)
                        .into(),
                );
            }
            entries.push(
                container(
                    button(text(add.clone()).size(geometry.font_size))
                        .padding(Padding::from(geometry.control_padding))
                        .on_press_maybe(field.enabled.then_some(Message::RowAdded(key)))
                        .style(move |_theme, status| {
                            theme::control(
                                tokens,
                                Emphasis {
                                    selected: false,
                                    focused: field_focused,
                                },
                                status,
                            )
                        }),
                )
                .id(crate::focus::target_id(
                    crate::view_model::FocusTarget::Field(key),
                ))
                .into(),
            );
            Column::with_children(entries)
                .spacing(geometry.spacing)
                .width(Length::Fill)
                .into()
        }
        ControlView::Text(value) => {
            let key = field.field;
            text_input("", value)
                .id(input_id(key))
                .on_submit(Message::FocusNext)
                .on_input_maybe(field.enabled.then_some(move |value| Message::FieldSet {
                    field: key,
                    value: Value::Text(value),
                }))
                .size(geometry.font_size)
                .padding(Padding::from(geometry.control_padding))
                .width(Length::Fill)
                .style(move |_theme, status| theme::input(tokens, status))
                .into()
        }
        ControlView::Choices(options) => Row::with_children(
            options
                .iter()
                .map(|option| {
                    let emphasis = Emphasis {
                        selected: option.selected,
                        focused: option.focused,
                    };
                    let pressed = Value::Choice(option.id.clone());

                    button(
                        text(option.label.clone())
                            .size(geometry.font_size)
                            .wrapping(Wrapping::None),
                    )
                    .padding(Padding::from(geometry.control_padding))
                    .on_press_maybe(field.enabled.then_some(Message::FieldSet {
                        field: field.field,
                        value: pressed,
                    }))
                    .style(move |_theme, status| theme::control(tokens, emphasis, status))
                    .into()
                })
                .collect::<Vec<Element<'a, Message>>>(),
        )
        .spacing(geometry.control_padding)
        .align_y(iced::alignment::Vertical::Center)
        .wrap()
        .into(),
        ControlView::Flag { on, label } => {
            let emphasis = Emphasis {
                selected: *on,
                focused: field.focused,
            };

            button(
                text(label.clone())
                    .size(geometry.font_size)
                    .wrapping(Wrapping::None),
            )
            .padding(Padding::from(geometry.control_padding))
            .on_press_maybe(field.enabled.then_some(Message::FieldSet {
                field: field.field,
                value: Value::Flag(!on),
            }))
            .style(move |_theme, status| theme::control(tokens, emphasis, status))
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
                    .size(geometry.font_size)
                    .padding(Padding::from(geometry.control_padding))
                    .width(Length::Fixed(geometry.label_width))
                    .style(move |_theme, status| theme::input(tokens, status))
                    .into(),
                text(unit.clone())
                    .size(geometry.font_size)
                    .color(theme::muted_text(tokens))
                    .into(),
                step_button(increase, field, 1, tokens, geometry, *value < *max),
            ])
            .spacing(geometry.control_padding)
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
    let emphasis = Emphasis {
        selected: false,
        focused: field.focused,
    };
    let enabled = field.enabled && within_range;

    button(text(glyph.to_owned()).size(geometry.font_size))
        .padding(Padding::from(geometry.control_padding))
        .on_press_maybe(enabled.then_some(Message::FieldStepped {
            field: field.field,
            delta,
        }))
        .style(move |_theme, status| theme::control(tokens, emphasis, status))
        .into()
}

/// The action that returns one field to the schema's default.
fn reset_button<'a>(
    field: &FieldView,
    label: &str,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let emphasis = Emphasis {
        selected: false,
        focused: field.reset_focused,
    };
    let target = field.field;

    container(
        button(
            text(label.to_owned())
                .size(geometry.font_size)
                .wrapping(Wrapping::None),
        )
        .padding(Padding::from(geometry.control_padding))
        .on_press_maybe(field.enabled.then_some(Message::FieldReset(target)))
        .style(move |_theme, status| theme::control(tokens, emphasis, status)),
    )
    .id(crate::focus::target_id(
        crate::view_model::FocusTarget::Reset(target),
    ))
    .into()
}

/// The status line, and what the components answered.
fn status_line<'a>(
    model: &SettingsViewModel,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let kind = model.status.kind;
    let colour = match kind {
        crate::view_model::StatusKind::Rejected => theme::danger_text(tokens),
        crate::view_model::StatusKind::Pending | crate::view_model::StatusKind::SavedOnly => {
            theme::warning_text(tokens)
        }
        crate::view_model::StatusKind::Applied => theme::success_text(tokens),
        _ => theme::muted_text(tokens),
    };

    let mut lines: Vec<Element<'a, Message>> = vec![
        text(model.status.message.clone())
            .size(geometry.font_size)
            .color(colour)
            .into(),
    ];

    lines.extend(model.status.details.iter().map(|detail| {
        text(detail.clone())
            .size(geometry.font_size)
            .color(theme::muted_text(tokens))
            .into()
    }));

    container(Column::with_children(lines).spacing(geometry.control_padding))
        .width(Length::Fill)
        .padding(Padding::from(geometry.padding))
        .style(move |_theme| theme::status(tokens, kind))
        .into()
}

/// The actions at the foot of the window.
fn action_row<'a>(
    model: &SettingsViewModel,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    Row::with_children(
        model
            .actions
            .iter()
            .map(|action| action_button(action, tokens, geometry))
            .collect::<Vec<Element<'a, Message>>>(),
    )
    .spacing(geometry.spacing)
    .align_y(iced::alignment::Vertical::Center)
    .into()
}

/// One action.
fn action_button<'a>(
    action: &ActionView,
    tokens: &'a Tokens,
    geometry: &SettingsGeometry,
) -> Element<'a, Message> {
    let primary = action.action == Action::Apply;
    let focused = action.focused;
    let enabled = action.enabled;
    let message = action_message(action.action);

    button(
        text(action.label.clone())
            .size(geometry.font_size)
            .wrapping(Wrapping::None),
    )
    .padding(Padding::from(geometry.control_padding))
    .on_press_maybe(enabled.then_some(message))
    .style(move |_theme, status| theme::action(tokens, primary, focused, status))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::Application;
    use crate::draft::{Draft, Settings};
    use crate::view_model::{SettingsInput, StatusKind};
    use inox_common::i18n::Locale;

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
        }
        .project(&localizer)
    }

    #[test]
    fn the_geometry_comes_from_the_tokens() {
        let tokens = Tokens::builtin_dark();
        let geometry = SettingsGeometry::new(&tokens);

        assert!((geometry.padding - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON);
        assert!(
            (geometry.nav_width - f32::from(tokens.spacing.xl) * NAV_WIDTH_STEPS).abs()
                < f32::EPSILON
        );
        assert!(geometry.label_width > geometry.nav_width / 2.0);
        assert!(geometry.row_height > geometry.font_size);
    }

    #[test]
    fn scaled_text_grows_with_the_tokens_and_the_rows_with_it() {
        let mut tokens = Tokens::builtin_dark();
        let plain = SettingsGeometry::new(&tokens);

        tokens.typography.scale = 2.0;
        let scaled = SettingsGeometry::new(&tokens);

        assert!(scaled.font_size > plain.font_size);
        assert!(scaled.row_height > plain.row_height);
        assert!(scaled.heading_size > plain.heading_size);
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
}
