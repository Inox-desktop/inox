//! Where the window's own focus lives, and which part of the tree has to show it.
//!
//! The window owns its focus: the entries a user can reach are a value
//! ([`crate::view_model::SettingsViewModel::focus_order`]) rather than a property
//! of the toolkit, so "one visible focus, no trap, reachable by keyboard only"
//! (acceptance Q01) is testable without a compositor.
//!
//! Two things are needed to *draw* that focus in a declarative tree:
//!
//! 1. **A stable identifier per target**, which the projection carries as the
//!    `focusId` of the matching item and as `focus.id` for the entry the keyboard
//!    is on. The QML tree compares the two; it never decides where the focus is.
//! 2. **Which scroll container has to reveal it**, so that walking a long page
//!    with Tab brings the control into view. The rule is a value
//!    ([`ScrollContainer`]), computed here from the target rather than guessed by
//!    the tree, exactly as the widget bounds used to be read from the layout.
//!
//! Both are pure functions of a [`FocusTarget`], so this module has no runtime
//! state at all — which is what lets a unit test walk every target the window can
//! produce and assert the identifiers stay distinct.

use serde::Serialize;

use crate::view_model::{FocusTarget, TrialChoice};

/// A part of the window that scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrollContainer {
    /// The navigation list down the left-hand side.
    Navigation,
    /// The page beside it.
    Content,
}

impl ScrollContainer {
    /// The name the QML tree knows the container by.
    pub fn name(self) -> &'static str {
        match self {
            Self::Navigation => "navigation",
            Self::Content => "content",
        }
    }
}

/// The identifier of one focus target.
///
/// The same identifier names the QML item that draws it, so a highlight and the
/// model's focus are the same fact seen twice rather than two facts that could
/// disagree.
pub fn target_id(target: FocusTarget) -> String {
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
        FocusTarget::Trial(choice) => format!("settings-display-trial-{choice:?}"),
    }
}

/// Which container has to reveal `target`, when the focus moves onto it.
///
/// The pages are in the navigation list and everything a page holds is in the
/// content; the actions at the foot of the window and the trial's two answers
/// are always on screen, so they ask for nothing. The previous widget tree
/// scrolled exactly these two containers for exactly these targets.
pub fn reveal(target: FocusTarget) -> Option<ScrollContainer> {
    match target {
        FocusTarget::Page(_) => Some(ScrollContainer::Navigation),
        FocusTarget::Field(_)
        | FocusTarget::Cell(_, _, _)
        | FocusTarget::RemoveRow(_, _)
        | FocusTarget::Reset(_) => Some(ScrollContainer::Content),
        FocusTarget::Action(_) | FocusTarget::Trial(_) => None,
    }
}

/// The message id of an answer to a display trial, so a test can name it.
pub fn trial_choice_id(choice: TrialChoice) -> &'static str {
    choice.label_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::Field;
    use crate::pages::PageId;
    use crate::view_model::Action;
    use std::collections::BTreeSet;

    /// Every target the window can produce for the fields of one page.
    fn targets() -> Vec<FocusTarget> {
        let mut targets = Vec::new();

        for page in PageId::ALL {
            targets.push(FocusTarget::Page(page));
        }

        for field in Field::ALL {
            targets.push(FocusTarget::Field(field));
            targets.push(FocusTarget::Reset(field));
            for row in 0..2 {
                targets.push(FocusTarget::RemoveRow(field, row));
                for column in 0..2 {
                    targets.push(FocusTarget::Cell(field, row, column));
                }
            }
        }

        for action in [
            Action::Apply,
            Action::Cancel,
            Action::ResetAll,
            Action::Reload,
        ] {
            targets.push(FocusTarget::Action(action));
        }

        for choice in TrialChoice::ALL {
            targets.push(FocusTarget::Trial(choice));
        }

        targets
    }

    #[test]
    fn every_target_has_an_identifier_of_its_own() {
        let ids: BTreeSet<String> = targets().into_iter().map(target_id).collect();

        assert_eq!(
            ids.len(),
            targets().len(),
            "two focus targets share an identifier, so one highlight would show the other"
        );
    }

    #[test]
    fn an_identifier_names_the_file_and_the_key_it_belongs_to() {
        let id = target_id(FocusTarget::Field(Field::PanelHeight));

        assert!(id.contains("shell"), "`{id}` does not name its file");
        assert!(id.ends_with("panel.height"), "`{id}` does not name its key");

        assert_eq!(
            target_id(FocusTarget::Reset(Field::PanelHeight)),
            format!("{id}-reset")
        );
        assert_eq!(
            target_id(FocusTarget::RemoveRow(Field::Outputs, 3)),
            format!("{}-3-remove", crate::view::input_id(Field::Outputs))
        );
        assert_eq!(
            target_id(FocusTarget::Cell(Field::Outputs, 3, 1)),
            format!("{}-3-1", crate::view::input_id(Field::Outputs))
        );
    }

    #[test]
    fn the_pages_are_in_the_navigation_list_and_their_fields_are_in_the_content() {
        assert_eq!(
            reveal(FocusTarget::Page(PageId::Panel)),
            Some(ScrollContainer::Navigation)
        );
        assert_eq!(
            reveal(FocusTarget::Field(Field::PanelHeight)),
            Some(ScrollContainer::Content)
        );
        assert_eq!(
            reveal(FocusTarget::Cell(Field::Outputs, 0, 0)),
            Some(ScrollContainer::Content)
        );
        assert_eq!(
            reveal(FocusTarget::Action(Action::Apply)),
            None,
            "the actions are always on screen"
        );
        assert_eq!(
            reveal(FocusTarget::Trial(TrialChoice::Confirm)),
            None,
            "the trial's answers are always on screen"
        );
    }

    #[test]
    fn the_containers_are_named_the_way_qml_knows_them() {
        assert_eq!(ScrollContainer::Navigation.name(), "navigation");
        assert_eq!(ScrollContainer::Content.name(), "content");
    }

    #[test]
    fn an_answers_identifier_is_its_label_id() {
        assert_eq!(
            trial_choice_id(TrialChoice::Confirm),
            TrialChoice::Confirm.label_id()
        );
    }
}
