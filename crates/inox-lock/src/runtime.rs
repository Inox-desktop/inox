//! The crate's replacement for `iced::Task`.
//!
//! gives every shell component the same shape: a message goes into
//! `update`, the state changes, and anything slow comes back as a task. The iced
//! runtime that used to carry that type is gone (contract §5), but the shape is
//! worth keeping: the reviewed authentication boundary names will
//! arrive as work that does not belong in a state transition, and `update` is
//! where it will be queued.
//!
//! This module is deliberately the smallest thing that has that shape. There is
//! no executor, no future and no async runtime: a [`Task`] is a list of messages
//! that the caller drains and feeds back to [`crate::app::update`]. The lock
//! screen's own transitions produce none — nothing it does is slow yet — which
//! is why the type is small rather than feature-complete.
//!
//! A task is *messages*, not side effects: a lock screen must never send a
//! request from inside `update`, and a type that could only carry messages makes
//! that structural rather than a rule someone remembers.

/// Messages an update wants delivered afterwards.
///
/// The default is empty, which is what every transition of this component
/// produces today; [`Task::done`] is for the boundary's answers, and
/// [`Task::batch`] for more than one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task<T> {
    messages: Vec<T>,
}

impl<T> Default for Task<T> {
    fn default() -> Self {
        Self::none()
    }
}

impl<T> Task<T> {
    /// A task that delivers nothing.
    pub const fn none() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    /// A task that delivers one message.
    pub fn done(message: T) -> Self {
        Self {
            messages: vec![message],
        }
    }

    /// A task that delivers everything in `tasks`, in order.
    pub fn batch(tasks: impl IntoIterator<Item = Self>) -> Self {
        let mut messages = Vec::new();

        for task in tasks {
            messages.extend(task.messages);
        }

        Self { messages }
    }

    /// The same task with its messages translated by `f`.
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Task<U> {
        Task {
            messages: self.messages.into_iter().map(&mut f).collect(),
        }
    }

    /// The messages this task carries, in order.
    pub fn into_messages(self) -> Vec<T> {
        self.messages
    }

    /// Whether this task delivers nothing.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_none_task_delivers_nothing() {
        let task: Task<u8> = Task::none();

        assert!(task.is_empty());
        assert_eq!(task.into_messages(), Vec::<u8>::new());
    }

    #[test]
    fn a_batch_keeps_the_order_of_its_parts() {
        let task = Task::batch([Task::done(1u8), Task::none(), Task::done(2)]);

        assert_eq!(task.into_messages(), vec![1, 2]);
    }

    #[test]
    fn mapping_translates_every_message() {
        let task = Task::batch([Task::done(1u8), Task::done(2)]).map(u16::from);

        assert_eq!(task.into_messages(), vec![1u16, 2]);
    }
}
