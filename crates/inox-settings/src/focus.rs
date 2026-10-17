//! Connects model focus to editable widgets and reveals targets using actual layout bounds.

use iced::advanced::widget::{Id, Operation, operation};
use iced::{Rectangle, Task, Vector};

use crate::app::Message;
use crate::view_model::FocusTarget;

pub(crate) fn target_id(target: FocusTarget) -> String {
    match target {
        FocusTarget::Field(field) => crate::view::input_id(field),
        FocusTarget::Cell(field, row, column) => {
            format!("{}-{row}-{column}", crate::view::input_id(field))
        }
        FocusTarget::RemoveRow(field, row) => {
            format!("{}-{row}-remove", crate::view::input_id(field))
        }
        FocusTarget::Reset(field) => format!("{}-reset", crate::view::input_id(field)),
        FocusTarget::Page(page) => format!("settings-page-{page:?}"),
        FocusTarget::Action(action) => format!("settings-action-{action:?}"),
    }
}

pub(crate) fn reveal(target: Option<FocusTarget>) -> Task<Message> {
    iced::advanced::widget::operate(Reveal {
        target: target.map(|target| Id::from(target_id(target))),
        scroll_id: if matches!(target, Some(FocusTarget::Page(_))) {
            "settings-navigation"
        } else {
            "settings-content"
        },
        scroll: matches!(
            target,
            Some(
                FocusTarget::Page(_)
                    | FocusTarget::Field(_)
                    | FocusTarget::Cell(_, _, _)
                    | FocusTarget::RemoveRow(_, _)
                    | FocusTarget::Reset(_)
            )
        ),
        bounds: None,
        viewport: None,
    })
}

struct Reveal {
    target: Option<Id>,
    scroll: bool,
    scroll_id: &'static str,
    bounds: Option<Rectangle>,
    viewport: Option<(Rectangle, Vector)>,
}

impl Operation<Message> for Reveal {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Message>)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id.is_some() && id == self.target.as_ref() {
            self.bounds = Some(bounds);
        }
    }

    fn focusable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        state: &mut dyn operation::Focusable,
    ) {
        if id.is_some() && id == self.target.as_ref() {
            state.focus();
            self.bounds = Some(bounds);
        } else {
            state.unfocus();
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&Id::new(self.scroll_id)) {
            self.viewport = Some((bounds, translation));
        }
    }

    fn finish(&self) -> operation::Outcome<Message> {
        if !self.scroll {
            return operation::Outcome::None;
        }
        let (Some(target), Some((viewport, translation))) = (self.bounds, self.viewport) else {
            return operation::Outcome::None;
        };
        let visible_top = target.y - translation.y;
        let visible_bottom = visible_top + target.height.min(viewport.height);
        let delta = if visible_top < viewport.y {
            visible_top - viewport.y
        } else if visible_bottom > viewport.y + viewport.height {
            visible_bottom - viewport.y - viewport.height
        } else {
            return operation::Outcome::None;
        };
        operation::Outcome::Chain(Box::new(operation::scrollable::scroll_to(
            Id::new(self.scroll_id),
            operation::scrollable::AbsoluteOffset {
                x: None,
                y: Some((translation.y + delta).max(0.0)),
            },
        )))
    }
}
