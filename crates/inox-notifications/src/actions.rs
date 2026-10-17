//! Action identity, and the at-most-once rule that goes with it.
//!
//! A notification may offer actions — buttons a user can press, sent to the
//! client as an `ActionInvoked` signal. Two things about them are contract
//! rather than convenience, and both are decided here:
//!
//! - **An action is identified by its key.** `Notify` carries a flat array of
//!   keys and labels; [`Actions::from_flat`] turns it into pairs and refuses the
//!   shapes that cannot be acted on — an odd number of strings, a key with
//!   nothing to invoke it by, two actions sharing one key (which would make "at
//!   most once" undecidable), or more actions than a popup can show.
//! - **An action fires at most once per notification instance.** The client on
//!   the other side of the bus does something when it receives `ActionInvoked`,
//!   so a second signal for the same click — a bouncing double click, a replayed
//!   event — is not a duplicate to be forgotten, it is a mistake to be reported:
//!   [`Actions::invoke`] answers [`ActionError::AlreadyInvoked`] and the daemon
//!   counts it (acceptance D06).
//!
//! Invoking an action also closes the notification, which is the specification's
//! rule; [`crate::app`] applies that, and this module keeps the record that makes
//! a second invocation a rejection rather than a repeat.

use std::fmt;

use crate::protocol::DEFAULT_ACTION_KEY;

/// The most actions one notification may offer.
///
/// A popup is a small surface. A client that sends more than this is refused
/// with a diagnosable error rather than having the excess silently dropped,
/// which is the difference between a client that can be fixed and one that
/// cannot.
pub const MAX_ACTIONS: usize = 16;

/// The identifier of an action, as `ActionInvoked` carries it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActionId(String);

impl ActionId {
    /// Names an action.
    pub fn new(value: &str) -> Self {
        Self(value.to_owned())
    }

    /// The key itself.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the reserved `default` action.
    ///
    /// The specification reserves that key for "the user activated the
    /// notification itself", which is what a click on the body means; it is not
    /// drawn as a button.
    pub fn is_default(&self) -> bool {
        self.0 == DEFAULT_ACTION_KEY
    }

    /// Whether the key is empty, which nothing can be invoked by.
    fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

/// One action a notification offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// The key the client receives back when the action is invoked.
    pub id: ActionId,
    /// The text of the button, as the client wrote it.
    pub label: String,
}

impl Action {
    /// Whether this action can be drawn as a button.
    ///
    /// The reserved `default` action is not one: it is what a click on the
    /// notification body invokes. An action whose label is empty is not drawn
    /// either — a button with nothing to read is not an action a user can take.
    pub fn is_button(&self) -> bool {
        !self.id.is_default() && !self.label.trim().is_empty()
    }
}

/// An action that was invoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invoked {
    /// The key the client is told about.
    pub id: ActionId,
    /// Whether it was the reserved `default` action, meaning the user activated
    /// the notification rather than pressing a button.
    pub is_default: bool,
}

/// Why an action could not be read or invoked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    /// The flat array `Notify` carries did not hold pairs.
    #[error(
        "the actions array holds {count} strings, which is not a whole number of key/label pairs"
    )]
    MalformedPairs {
        /// How many strings were sent.
        count: usize,
    },
    /// The notification offered more actions than a popup can show.
    #[error(
        "the notification offers {count} actions, more than the {MAX_ACTIONS} a popup can show"
    )]
    TooManyActions {
        /// How many were sent.
        count: usize,
    },
    /// Two actions shared one key.
    #[error("two actions share the key `{key}`, so an invocation of it would be ambiguous")]
    DuplicateKey {
        /// The repeated key.
        key: ActionId,
    },
    /// An action had no key at all.
    #[error("an action without a key cannot be invoked")]
    EmptyKey,
    /// The action is not one this notification offers.
    #[error("the notification offers no action `{id}`")]
    UnknownAction {
        /// The key that was invoked.
        id: ActionId,
    },
    /// An action of this notification instance was already invoked.
    #[error(
        "`{invoked}` of this notification was already invoked; `{requested}` is ignored rather than repeated"
    )]
    AlreadyInvoked {
        /// The action that was invoked first.
        invoked: ActionId,
        /// The action that arrived afterwards.
        requested: ActionId,
    },
}

/// The actions of one notification instance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Actions {
    /// The actions, in the order the client sent them.
    actions: Vec<Action>,
    /// The action that was invoked, if one has been.
    invoked: Option<ActionId>,
}

impl Actions {
    /// No actions at all.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the flat key/label array a `Notify` call carries.
    ///
    /// # Errors
    ///
    /// Returns the reasons in [`ActionError`]: an odd number of strings, an
    /// empty or repeated key, or more actions than [`MAX_ACTIONS`].
    pub fn from_flat(flat: &[String]) -> Result<Self, ActionError> {
        if !flat.len().is_multiple_of(2) {
            return Err(ActionError::MalformedPairs { count: flat.len() });
        }

        let count = flat.len() / 2;
        if count > MAX_ACTIONS {
            return Err(ActionError::TooManyActions { count });
        }

        let mut actions: Vec<Action> = Vec::with_capacity(count);

        for pair in flat.chunks(2) {
            let [key, label] = pair else {
                // `chunks(2)` on an even-length slice always yields pairs.
                return Err(ActionError::MalformedPairs { count: flat.len() });
            };
            let id = ActionId::new(key);

            if id.is_empty() {
                return Err(ActionError::EmptyKey);
            }
            if actions.iter().any(|action| action.id == id) {
                return Err(ActionError::DuplicateKey { key: id });
            }

            actions.push(Action {
                id,
                label: label.clone(),
            });
        }

        Ok(Self {
            actions,
            invoked: None,
        })
    }

    /// How many actions the notification offers, the reserved one included.
    pub fn len(&self) -> usize {
        self.actions.len()
    }

    /// Whether the notification offers no action at all.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Every action, in the order the client sent them.
    pub fn iter(&self) -> impl Iterator<Item = &Action> {
        self.actions.iter()
    }

    /// The actions a popup draws buttons for.
    pub fn buttons(&self) -> impl Iterator<Item = &Action> {
        self.actions.iter().filter(|action| action.is_button())
    }

    /// The action called `id`, if the notification offers it.
    pub fn get(&self, id: &ActionId) -> Option<&Action> {
        self.actions.iter().find(|action| action.id == *id)
    }

    /// Whether the notification offers an action called `id`.
    pub fn contains(&self, id: &ActionId) -> bool {
        self.get(id).is_some()
    }

    /// The reserved default action, if the client sent one.
    ///
    /// This is what a click on the notification body invokes.
    pub fn default_action(&self) -> Option<&Action> {
        self.actions.iter().find(|action| action.id.is_default())
    }

    /// The action that was invoked, if one has been.
    pub fn invoked(&self) -> Option<&ActionId> {
        self.invoked.as_ref()
    }

    /// Invokes an action of this notification instance.
    ///
    /// # Errors
    ///
    /// [`ActionError::AlreadyInvoked`] when an action of this instance was
    /// invoked before — at most one is ever invoked, and the second attempt is
    /// reported rather than repeated. [`ActionError::UnknownAction`] when the
    /// notification does not offer the action at all.
    pub fn invoke(&mut self, id: &ActionId) -> Result<Invoked, ActionError> {
        if let Some(invoked) = &self.invoked {
            return Err(ActionError::AlreadyInvoked {
                invoked: invoked.clone(),
                requested: id.clone(),
            });
        }

        if !self.contains(id) {
            return Err(ActionError::UnknownAction { id: id.clone() });
        }

        self.invoked = Some(id.clone());

        Ok(Invoked {
            id: id.clone(),
            is_default: id.is_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn flat(pairs: &[(&str, &str)]) -> Vec<String> {
        pairs
            .iter()
            .flat_map(|(key, label)| [(*key).to_owned(), (*label).to_owned()])
            .collect()
    }

    fn actions(pairs: &[(&str, &str)]) -> Actions {
        Actions::from_flat(&flat(pairs)).expect("well-formed actions")
    }

    #[test]
    fn a_pair_array_becomes_actions_in_the_order_it_was_sent() {
        let parsed = actions(&[("default", ""), ("open", "Open"), ("dismiss", "Dismiss")]);

        assert_eq!(parsed.len(), 3);
        let labels: Vec<&str> = parsed.iter().map(|action| action.label.as_str()).collect();
        assert_eq!(labels, ["", "Open", "Dismiss"]);
    }

    #[test]
    fn an_odd_number_of_strings_is_refused() {
        let error = Actions::from_flat(&["open".to_owned()]).expect_err("odd");

        assert_eq!(error, ActionError::MalformedPairs { count: 1 });
        assert!(error.to_string().contains("1 strings"));
    }

    #[test]
    fn more_actions_than_a_popup_can_show_is_refused() {
        let flat: Vec<String> = (0..=MAX_ACTIONS)
            .flat_map(|index| [format!("key{index}"), format!("Label {index}")])
            .collect();
        let error = Actions::from_flat(&flat).expect_err("too many");

        assert_eq!(
            error,
            ActionError::TooManyActions {
                count: MAX_ACTIONS + 1
            }
        );
    }

    #[test]
    fn the_largest_accepted_set_is_accepted() {
        let flat: Vec<String> = (0..MAX_ACTIONS)
            .flat_map(|index| [format!("key{index}"), format!("Label {index}")])
            .collect();

        assert_eq!(
            Actions::from_flat(&flat).expect("the limit itself").len(),
            MAX_ACTIONS
        );
    }

    #[test]
    fn two_actions_sharing_a_key_are_refused() {
        let error = Actions::from_flat(&flat(&[("open", "Open"), ("open", "Open again")]))
            .expect_err("duplicate");

        assert_eq!(
            error,
            ActionError::DuplicateKey {
                key: ActionId::new("open")
            }
        );
    }

    #[test]
    fn an_action_without_a_key_is_refused() {
        let error = Actions::from_flat(&flat(&[("", "Open")])).expect_err("empty key");

        assert_eq!(error, ActionError::EmptyKey);
        let error = Actions::from_flat(&flat(&[("   ", "Open")])).expect_err("blank key");
        assert_eq!(error, ActionError::EmptyKey);
    }

    #[test]
    fn no_actions_at_all_are_no_actions() {
        let parsed = Actions::from_flat(&[]).expect("an empty array is well-formed");

        assert!(parsed.is_empty());
        assert_eq!(parsed.len(), 0);
        assert!(parsed.buttons().next().is_none());
        assert!(parsed.default_action().is_none());
    }

    #[test]
    fn the_reserved_default_action_is_not_drawn_as_a_button() {
        let parsed = actions(&[("default", "Open"), ("dismiss", "Dismiss")]);

        let buttons: Vec<&ActionId> = parsed.buttons().map(|action| &action.id).collect();
        assert_eq!(buttons, [&ActionId::new("dismiss")]);
        assert_eq!(
            parsed.default_action().map(|action| action.id.as_str()),
            Some("default")
        );
        assert!(ActionId::new("default").is_default());
        assert!(!ActionId::new("Default").is_default());
    }

    #[test]
    fn an_action_with_no_label_is_not_drawn_as_a_button() {
        let parsed = actions(&[("open", ""), ("dismiss", "Dismiss")]);

        let buttons: Vec<&str> = parsed.buttons().map(|action| action.id.as_str()).collect();
        assert_eq!(buttons, ["dismiss"]);
        assert!(
            parsed.contains(&ActionId::new("open")),
            "it can still be invoked, it just has nothing to draw"
        );
    }

    #[test]
    fn invoking_an_action_records_it_once() {
        let mut parsed = actions(&[("open", "Open")]);

        let invoked = parsed.invoke(&ActionId::new("open")).expect("the action");

        assert_eq!(invoked.id, ActionId::new("open"));
        assert!(!invoked.is_default);
        assert_eq!(parsed.invoked(), Some(&ActionId::new("open")));
    }

    #[test]
    fn invoking_the_default_action_says_so() {
        let mut parsed = actions(&[("default", "")]);

        let invoked = parsed
            .invoke(&ActionId::new("default"))
            .expect("the action");

        assert!(invoked.is_default);
    }

    #[test]
    fn the_same_action_twice_is_reported_rather_than_repeated() {
        let mut parsed = actions(&[("open", "Open")]);
        parsed.invoke(&ActionId::new("open")).expect("the first");

        let error = parsed
            .invoke(&ActionId::new("open"))
            .expect_err("the second must not fire");

        assert_eq!(
            error,
            ActionError::AlreadyInvoked {
                invoked: ActionId::new("open"),
                requested: ActionId::new("open"),
            }
        );
        assert!(error.to_string().contains("already invoked"));
        assert_eq!(
            parsed.invoked(),
            Some(&ActionId::new("open")),
            "the record is unchanged"
        );
    }

    #[test]
    fn a_second_different_action_is_also_reported() {
        let mut parsed = actions(&[("open", "Open"), ("dismiss", "Dismiss")]);
        parsed.invoke(&ActionId::new("open")).expect("the first");

        assert_eq!(
            parsed.invoke(&ActionId::new("dismiss")),
            Err(ActionError::AlreadyInvoked {
                invoked: ActionId::new("open"),
                requested: ActionId::new("dismiss"),
            }),
            "one action per notification instance, not one per key"
        );
    }

    #[test]
    fn an_action_the_notification_does_not_offer_is_refused() {
        let mut parsed = actions(&[("open", "Open")]);

        assert_eq!(
            parsed.invoke(&ActionId::new("dismiss")),
            Err(ActionError::UnknownAction {
                id: ActionId::new("dismiss")
            })
        );
        assert_eq!(parsed.invoked(), None);
    }

    #[test]
    fn a_refused_invocation_does_not_block_a_real_one() {
        let mut parsed = actions(&[("open", "Open")]);

        let _ = parsed.invoke(&ActionId::new("typo"));
        assert!(
            parsed.invoke(&ActionId::new("open")).is_ok(),
            "an unknown key is not an invocation"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn reading_a_flat_array_never_panics_and_keeps_the_pairs(
            strings in proptest::collection::vec("[a-z]{0,4}", 0..20),
        ) {
            match Actions::from_flat(&strings) {
                Ok(parsed) => {
                    prop_assert_eq!(parsed.len(), strings.len() / 2);
                    let keys: Vec<&str> =
                        parsed.iter().map(|action| action.id.as_str()).collect();
                    let mut unique = keys.clone();
                    unique.sort_unstable();
                    unique.dedup();
                    prop_assert_eq!(unique.len(), keys.len(), "keys are unique");
                    prop_assert!(keys.iter().all(|key| !key.trim().is_empty()));
                }
                Err(error) => {
                    let expected = matches!(
                        error,
                        ActionError::MalformedPairs { .. }
                            | ActionError::TooManyActions { .. }
                            | ActionError::DuplicateKey { .. }
                            | ActionError::EmptyKey
                    );
                    prop_assert!(expected, "unexpected: {}", error);
                }
            }
        }

        #[test]
        fn an_action_of_a_parsed_set_can_be_invoked_exactly_once(
            strings in proptest::collection::vec("[a-z]{1,4}", 2..16),
        ) {
            // Duplicate keys are possible here, so only well-formed sets are
            // exercised; the point of the property is the at-most-once rule.
            let flat: Vec<String> = strings
                .chunks(2)
                .enumerate()
                .flat_map(|(index, chunk)| {
                    let key = chunk.first().cloned().unwrap_or_default();
                    let label = chunk.get(1).cloned().unwrap_or_default();
                    [format!("{index}-{key}"), label]
                })
                .collect();

            let mut parsed = Actions::from_flat(&flat).expect("unique keys by construction");
            let first = parsed.iter().map(|action| action.id.clone()).collect::<Vec<_>>();
            let Some(id) = first.first().cloned() else {
                return Ok(());
            };

            let first_invocation = parsed.invoke(&id);
            prop_assert!(first_invocation.is_ok(), "the first invocation fires");

            let second_invocation = parsed.invoke(&id);
            prop_assert!(second_invocation.is_err(), "the second is reported");
            assert!(matches!(
                second_invocation,
                Err(ActionError::AlreadyInvoked { .. })
            ));
            prop_assert_eq!(parsed.invoked(), Some(&id));
        }
    }
}
