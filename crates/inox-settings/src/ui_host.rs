//! The Qt-free contract between the cxx-qt bridge and the running window.
//!
//! The bridge (`src/cxxqt_object.rs`) is a thin shell: it holds a JSON string,
//! forwards the actions QML sends, and asks for a fresh projection when the tree
//! is pumped. *What* the projection is, and what an action means, lives here in
//! the application — which is what keeps the QML boundary free of domain types
//! (the domain model stays in Rust) and what lets the second binary of
//! this package compile the bridge without compiling the whole window (see
//! `AGENTS.md`, "the second binary").
//!
//! The host is installed on the thread that owns the window, which is the thread
//! Qt creates and renders on. It is deliberately not `Send`: the window's state
//! holds a Fluent catalog, which is neither `Send` nor `Sync`, and the bridge is
//! only ever called from that one thread.
//!
//! A bridge that is constructed with no host installed — which only happens in a
//! unit test that instantiates the backing struct directly, or in the control
//! binary that includes the bridge solely to resolve its link — answers with an
//! empty projection and accepts no action. It never panics: a QObject's
//! constructor may not abort a process.

use std::cell::RefCell;

/// What a loaded QML tree is checked against before the window calls itself up.
///
/// A tree that loads but reports a different title, a different number of
/// navigation entries, or too few accessible items is a tree that silently lost
/// something — a `Component.onCompleted` that never ran, an item that lost its
/// `Accessible.role` (root `AGENTS.md` §7.4), a projection that did not arrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readiness {
    /// The title the projection carries, which the tree must have read.
    pub title: String,
    /// How many navigation entries the projection has.
    pub pages: i32,
    /// The fewest accessible items that tree can have.
    ///
    /// The navigation list, the page's fields, the actions, and the status line:
    /// a tree that reports fewer than this has lost part of its semantic tree.
    pub accessible_items: i32,
}

/// What the bridge can ask of the window behind it.
pub trait Host {
    /// The projection to render, as JSON.
    fn model_json(&mut self) -> String;

    /// Applies one action the QML tree sent, and returns what happened.
    ///
    /// `Unsupported` means the action was understood but cannot be taken (a key
    /// the window leaves to a text field). `Unknown` means it was not an action
    /// of the closed set at all; the caller logs it once and drops it.
    fn apply(&mut self, action: &str) -> Applied;

    /// Drains work that finished off the window's thread, and moves the display
    /// trial's clock on when one is open.
    ///
    /// Returns whether anything happened, so the bridge only re-reads the
    /// projection when there is a reason to.
    fn pump(&mut self) -> bool;

    /// The contract a loaded QML tree is checked against.
    fn readiness(&mut self) -> Readiness;

    /// The window asked to be closed.
    fn quit(&mut self);
}

/// What became of an action the bridge handed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applied {
    /// It changed the window; the projection should be re-read.
    Changed,
    /// It was understood and did nothing: a key a text field owns, or a command
    /// the state machine refused because the control is not usable.
    Unsupported,
    /// It was not an action of the closed set.
    Unknown,
}

thread_local! {
    /// The window this thread is driving, if one is installed.
    static HOST: RefCell<Option<Box<dyn Host>>> = const { RefCell::new(None) };
}

/// Installs `host` as the window this thread drives.
///
/// Returns `false` when one was already installed: a process drives exactly one
/// window, and a second install is a programming error rather than something to
/// overwrite silently.
pub fn install(host: Box<dyn Host>) -> bool {
    HOST.with(|slot| {
        let mut slot = slot.borrow_mut();

        if slot.is_some() {
            return false;
        }

        *slot = Some(host);
        true
    })
}

/// Whether a host is installed on this thread.
pub fn installed() -> bool {
    HOST.with(|slot| slot.borrow().is_some())
}

/// Calls `f` with the installed host, and returns its answer.
///
/// `None` means no window is installed, which the bridge treats as "there is
/// nothing to project" rather than as a failure.
pub fn with<R>(f: impl FnOnce(&mut dyn Host) -> R) -> Option<R> {
    HOST.with(|slot| {
        let mut slot = slot.borrow_mut();
        let host = slot.as_mut()?;

        Some(f(host.as_mut()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that answers everything with a fixed value.
    struct Stub {
        pumped: bool,
        quit: bool,
    }

    impl Host for Stub {
        fn model_json(&mut self) -> String {
            "{\"title\":\"stub\"}".to_owned()
        }

        fn apply(&mut self, action: &str) -> Applied {
            if action == "known" {
                Applied::Changed
            } else {
                Applied::Unknown
            }
        }

        fn pump(&mut self) -> bool {
            self.pumped = true;
            self.pumped
        }

        fn readiness(&mut self) -> Readiness {
            Readiness {
                title: "stub".to_owned(),
                pages: 1,
                accessible_items: 2,
            }
        }

        fn quit(&mut self) {
            self.quit = true;
        }
    }

    #[test]
    fn a_host_answers_the_bridge_through_the_thread_it_was_installed_on() {
        assert!(install(Box::new(Stub {
            pumped: false,
            quit: false,
        })));
        assert!(installed());

        assert_eq!(
            with(|host| host.model_json()),
            Some("{\"title\":\"stub\"}".to_owned())
        );
        assert_eq!(with(|host| host.apply("known")), Some(Applied::Changed));
        assert_eq!(with(|host| host.apply("nonsense")), Some(Applied::Unknown));
        assert_eq!(
            with(|host| {
                host.quit();
                host.readiness()
            }),
            Some(Readiness {
                title: "stub".to_owned(),
                pages: 1,
                accessible_items: 2,
            })
        );
    }

    #[test]
    fn only_one_window_is_installed_per_thread() {
        assert!(install(Box::new(Stub {
            pumped: false,
            quit: false,
        })));
        assert!(
            !install(Box::new(Stub {
                pumped: false,
                quit: false,
            })),
            "a second window would silently replace the first"
        );
    }

    #[test]
    fn a_thread_without_a_window_answers_nothing_and_does_not_panic() {
        let answer = std::thread::spawn(|| with(|host| host.model_json()))
            .join()
            .expect("the thread runs");

        assert_eq!(answer, None);
    }
}
