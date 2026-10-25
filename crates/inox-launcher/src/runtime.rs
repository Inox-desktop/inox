//! The launcher's own task runner, replacing what Iced's runtime used to give.
//!
//! Two things are needed and nothing more (docs/qml-migration-contract.md §5):
//!
//! - [`Queue`] is the message queue between the UI thread and the work that runs
//!   off it. The Qt host drains it in `pump`, which is what makes a result from a
//!   worker thread a state transition on the UI thread and nothing else.
//! - [`Task`] is what an update returns: "nothing", "this message", or "run this
//!   on a thread and post what it returns". There is no async runtime behind it
//!   and none is wanted: the two slow things the launcher does — reading the
//!   desktop files and waiting for a launched process — are plain blocking work,
//!   so a thread per task is the honest shape.
//!
//! [`oneshot`] is the one piece of channel plumbing Iced's futures provided that
//! `std` does not: a one-value channel whose receiving end is a `Future`, which
//! is what the D-Bus `Reload` method awaits while the UI thread finishes the
//! reload it asked for.
//!
//! None of this is Qt-aware and none of it is `unsafe`: `Task::map` shares the
//! projection closure through an `Arc`, and `oneshot` wakes its receiver through
//! the standard [`std::task::Wake`] trait.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::thread;

/// The name every task thread carries, so a thread list still says who spawned
/// it.
const THREAD_NAME: &str = "inox-launcher-task";

/// Where a task posts its result.
///
/// An `Emitter` is a cheap, cloneable handle: [`Task::perform`] moves one into
/// the thread it starts, and [`Queue::emitter`] builds one that pushes into the
/// queue the host drains.
pub struct Emitter<T>(Arc<dyn Fn(T) + Send + Sync + 'static>);

impl<T> Clone for Emitter<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Emitter<T> {
    /// An emitter that calls `send` with every value.
    pub fn new<F>(send: F) -> Self
    where
        F: Fn(T) + Send + Sync + 'static,
    {
        Self(Arc::new(send))
    }

    /// Hands `value` to the emitter.
    pub fn emit(&self, value: T) {
        (self.0)(value);
    }
}

impl<T> fmt::Debug for Emitter<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Emitter")
    }
}

/// The message queue between the UI thread and the work that runs off it.
///
/// Shared by cloning: a task thread holds one handle and the host drains through
/// another. A poisoned lock is recovered rather than propagated — a panic in one
/// task must not take the application down with it.
#[derive(Debug)]
pub struct Queue<T> {
    items: Arc<Mutex<VecDeque<T>>>,
}

impl<T> Queue<T> {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            items: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Appends `value`.
    pub fn push(&self, value: T) {
        self.items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(value);
    }

    /// Takes everything queued so far, oldest first.
    pub fn drain(&self) -> Vec<T> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        items.drain(..).collect()
    }

    /// An emitter that pushes into this queue.
    pub fn emitter(&self) -> Emitter<T>
    where
        T: Send + 'static,
    {
        let items = Arc::clone(&self.items);
        Emitter::new(move |value| {
            items
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push_back(value);
        })
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Work an update wants done.
///
/// It is a description, not a running future: [`Task::run`] is what the host
/// calls, which keeps "what `update` decided" separate from "when the host acts
/// on it" and lets the tests assert on the former without an executor.
pub struct Task<T>(Option<Box<dyn FnOnce(Emitter<T>) + Send + 'static>>);

impl<T> fmt::Debug for Task<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(if self.0.is_some() {
            "Task::Pending"
        } else {
            "Task::None"
        })
    }
}

impl<T: Send + 'static> Task<T> {
    /// Nothing to do.
    pub fn none() -> Self {
        Self(None)
    }

    /// Post `value` immediately, on the thread that runs the task.
    pub fn done(value: T) -> Self {
        Self(Some(Box::new(move |emit| emit.emit(value))))
    }

    /// Run `work` on a thread of its own and post what it returns.
    ///
    /// A thread that cannot be started is logged and produces no message; use
    /// [`Task::attempt`] where the caller has something to say about that.
    pub fn perform<F>(work: F) -> Self
    where
        F: FnOnce() -> T + Send + 'static,
    {
        Self(Some(Box::new(move |emit: Emitter<T>| {
            let worker = emit.clone();
            match thread::Builder::new()
                .name(THREAD_NAME.to_owned())
                .spawn(move || worker.emit(work()))
            {
                Ok(handle) => drop(handle),
                Err(error) => tracing::error!(%error, "a task thread could not be started"),
            }
        })))
    }

    /// Run `work` on a thread of its own, and post `on_error(error)` instead when
    /// no thread could be started for it.
    ///
    /// The launcher's launch probe uses this: "the process could not be started"
    /// is a state the user is shown, so it needs a message of its own rather than
    /// a log line and a silent nothing.
    pub fn attempt<F, E>(work: F, on_error: E) -> Self
    where
        F: FnOnce() -> T + Send + 'static,
        E: FnOnce(std::io::Error) -> T + Send + 'static,
    {
        Self(Some(Box::new(move |emit: Emitter<T>| {
            let worker = emit.clone();
            match thread::Builder::new()
                .name(THREAD_NAME.to_owned())
                .spawn(move || worker.emit(work()))
            {
                Ok(handle) => drop(handle),
                Err(error) => {
                    tracing::error!(%error, "a task thread could not be started");
                    emit.emit(on_error(error));
                }
            }
        })))
    }

    /// Translate every message the task produces.
    ///
    /// The mapping runs on whichever thread produced the value, so it has to be
    /// shareable; the closure is held behind an `Arc` for that reason.
    pub fn map<U, F>(self, translate: F) -> Task<U>
    where
        U: Send + 'static,
        F: Fn(T) -> U + Send + Sync + 'static,
    {
        let Self(inner) = self;
        let Some(inner) = inner else {
            return Task::none();
        };
        let translate = Arc::new(translate);
        Task(Some(Box::new(move |emit: Emitter<U>| {
            let translate = Arc::clone(&translate);
            let outer = emit;
            let inner_emit = Emitter::new(move |value: T| {
                let translated = translate(value);
                outer.emit(translated);
            });
            inner(inner_emit);
        })))
    }

    /// Execute the task, posting what it produces to `emit`.
    pub fn run(self, emit: Emitter<T>) {
        if let Some(work) = self.0 {
            work(emit);
        }
    }
}

/// A one-value channel whose receiving half is a [`Future`].
///
/// `std` has the synchronous channel and the `Future` trait but not the bridge
/// between them, which is exactly what the D-Bus `Reload` method needs: it
/// submits a request to the UI thread and awaits the outcome without blocking
/// the connection's executor.
///
/// [`Future`]: std::future::Future
pub mod oneshot {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};

    /// The sender was dropped without sending, or the value was already taken.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Closed;

    impl std::fmt::Display for Closed {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("the one-shot sender is gone")
        }
    }

    impl std::error::Error for Closed {}

    #[derive(Debug)]
    struct Shared<T> {
        state: Mutex<State<T>>,
    }

    #[derive(Debug)]
    struct State<T> {
        value: Option<T>,
        waker: Option<Waker>,
        receiver_dropped: bool,
    }

    /// The sending half.
    #[derive(Debug)]
    pub struct Sender<T>(Arc<Shared<T>>);

    /// The receiving half, which is a future.
    #[derive(Debug)]
    pub struct Receiver<T>(Arc<Shared<T>>);

    /// A new channel.
    pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                value: None,
                waker: None,
                receiver_dropped: false,
            }),
        });
        (Sender(Arc::clone(&shared)), Receiver(shared))
    }

    impl<T> Sender<T> {
        /// Hands `value` to the receiver.
        ///
        /// # Errors
        ///
        /// Returns the value back when the receiver has already been dropped:
        /// nothing will ever read it, and silently discarding it would hide the
        /// caller's bug.
        pub fn send(self, value: T) -> Result<(), T> {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.receiver_dropped {
                return Err(value);
            }
            state.value = Some(value);
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
            Ok(())
        }
    }

    impl<T> Future for Receiver<T> {
        type Output = Result<T, Closed>;

        fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(value) = state.value.take() {
                return Poll::Ready(Ok(value));
            }
            if state.receiver_dropped {
                return Poll::Ready(Err(Closed));
            }
            state.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }

    impl<T> Drop for Receiver<T> {
        fn drop(&mut self) {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.receiver_dropped = true;
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_queue_hands_out_what_was_pushed_in_order() {
        let queue = Queue::new();
        queue.push(1);
        queue.push(2);

        assert_eq!(queue.drain(), vec![1, 2]);
        assert!(queue.drain().is_empty(), "draining twice yields nothing");
    }

    #[test]
    fn a_queue_emitter_pushes_into_the_queue_it_came_from() {
        let queue = Queue::new();
        let emitter = queue.emitter();
        emitter.emit("from a task");
        assert_eq!(queue.drain(), vec!["from a task"]);
    }

    #[test]
    fn a_done_task_posts_its_message() {
        let queue = Queue::new();
        Task::done(7).run(queue.emitter());
        assert_eq!(queue.drain(), vec![7]);
    }

    #[test]
    fn a_skipped_task_posts_nothing() {
        let queue: Queue<u8> = Queue::new();
        Task::none().run(queue.emitter());
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn a_performed_task_posts_what_its_thread_produced() {
        let queue = Queue::new();
        Task::perform(|| {
            std::thread::sleep(std::time::Duration::from_millis(5));
            "finished"
        })
        .run(queue.emitter());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let drained = queue.drain();
            if !drained.is_empty() {
                assert_eq!(drained, vec!["finished"]);
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the task never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn a_mapped_task_translates_every_message_it_produces() {
        let queue = Queue::new();
        Task::perform(|| 20)
            .map(|value| value * 2)
            .run(queue.emitter());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let drained = queue.drain();
            if !drained.is_empty() {
                assert_eq!(drained, vec![40]);
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the task never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn a_mapping_runs_once_per_message() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let queue = Queue::new();
        Task::done(1)
            .map(move |value| {
                counted.fetch_add(1, Ordering::SeqCst);
                value + 1
            })
            .run(queue.emitter());

        assert_eq!(queue.drain(), vec![2]);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    mod one_shot {
        use super::super::oneshot;
        use std::future::Future;
        use std::task::{Context, Poll, Waker};

        #[test]
        fn a_value_sent_before_the_poll_is_ready_immediately() {
            let (sender, mut receiver) = oneshot::channel();
            sender.send(5).expect("the receiver is alive");
            let mut context = Context::from_waker(Waker::noop());
            match std::pin::Pin::new(&mut receiver).poll(&mut context) {
                Poll::Ready(value) => assert_eq!(value, Ok(5)),
                Poll::Pending => panic!("a sent value must be ready"),
            }
        }

        #[test]
        fn a_receiver_waits_until_the_value_arrives() {
            let (sender, mut receiver) = oneshot::channel();
            let mut context = Context::from_waker(Waker::noop());
            assert!(matches!(
                std::pin::Pin::new(&mut receiver).poll(&mut context),
                Poll::Pending
            ));
            sender.send("late").expect("the receiver is alive");
            match std::pin::Pin::new(&mut receiver).poll(&mut context) {
                Poll::Ready(value) => assert_eq!(value, Ok("late")),
                Poll::Pending => panic!("the value is there, so the future is ready"),
            }
        }

        #[test]
        fn sending_to_a_dropped_receiver_hands_the_value_back() {
            let (sender, receiver) = oneshot::channel::<u8>();
            drop(receiver);
            assert_eq!(sender.send(1), Err(1));
        }
    }
}
