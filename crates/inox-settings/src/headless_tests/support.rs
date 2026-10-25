//! The harness the settings window's tests run through.
//!
//! It builds the window's real state over a real configuration directory —
//! `Documents` reads files, so the tests use a temporary directory rather than an
//! in-memory stand-in — and drives the same update loop a user drives. Nothing is
//! mocked except the two boundaries that are not available in a test process:
//! the consumers that would answer over D-Bus (a window with no transport reports
//! `SavedOnly`, which is what the binary does too) and the clock (a test sends
//! the messages a second of the trial's countdown would).
//!
//! What the iced version of this harness did beyond that — laying the real widget
//! tree out and drawing it with tiny-skia — has no equivalent after the migration:
//! the window is drawn by Qt. Its assertions moved to the projection, where the
//! same facts (a colour is this token, a control is disabled with this reason, an
//! error names this field) are values a test reads without a display. See
//! [`crate::headless_tests::projection`].

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;

use inox_common::i18n::Locale;
use inox_config::{Appearance, ConfigPaths, InoxConfig, Theme as Tokens};

use crate::app::{self, Message, OutgoingCommand, SaveRequest, State};
use crate::apply::Component;
use crate::document::Documents;

/// The settings window, over a real configuration directory.
pub struct Window {
    directory: tempfile::TempDir,
    paths: ConfigPaths,
    state: State,
    /// Whether a compositor answers, so the display-trial workflow can be driven
    /// without a session bus. Off by default: the ordinary tests exercise the
    /// window in the state this milestone ships it in, where nothing answers.
    compositor: bool,
}

impl Window {
    /// A window over a directory with no configuration files at all.
    pub fn new() -> Self {
        Self::with_files(&[])
    }

    /// A window over a directory holding `files`, as `(name, content)` pairs.
    pub fn with_files(files: &[(&str, &str)]) -> Self {
        Self::with_locale(files, Locale::default_locale(), Tokens::builtin_dark())
    }

    /// A window with a chosen locale and theme.
    pub fn with_locale(files: &[(&str, &str)], locale: Locale, tokens: Tokens) -> Self {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let paths = ConfigPaths::new(directory.path());

        for (name, content) in files {
            std::fs::write(directory.path().join(name), content).expect("the file is written");
        }

        let config = InoxConfig::load(&paths, Appearance::System).expect("the configuration loads");
        let documents = Documents::open(&config, &paths).expect("the files are read");
        let localizer = Rc::new(crate::i18n::load(&locale).expect("the catalogs parse"));

        let state = State::new(&config, documents, tokens, localizer);

        Self {
            directory,
            paths,
            state,
            compositor: false,
        }
    }

    /// The same window, but with a compositor that answers display trials.
    pub fn with_compositor(mut self) -> Self {
        self.compositor = true;
        self
    }

    /// The directory the window's files live in.
    pub fn directory(&self) -> &Path {
        self.directory.path()
    }

    /// The paths the window resolved.
    pub fn paths(&self) -> &ConfigPaths {
        &self.paths
    }

    /// The window's state.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// The window's state, for a test that drives an edit by hand.
    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    /// Feeds a message to the update loop, the way the runtime would.
    pub fn update(&mut self, message: Message) {
        app::update(&mut self.state, message);
    }

    /// Feeds several messages.
    pub fn update_all(&mut self, messages: impl IntoIterator<Item = Message>) {
        for message in messages {
            self.update(message);
        }
    }

    /// Asks for the change to be written, and performs the writes.
    ///
    /// This is the loop `runtime.rs` runs: [`app::update`] queues, the runtime
    /// performs and the outcome comes back as a message. Returns how many files
    /// were written.
    pub fn apply_and_drain(&mut self) -> usize {
        self.update(Message::Apply);

        self.drain()
    }

    /// Performs every command the update loop has queued.
    pub fn drain(&mut self) -> usize {
        let mut written = 0;

        loop {
            let commands = self.state.take_outgoing();

            if commands.is_empty() {
                return written;
            }

            for command in commands {
                match command {
                    OutgoingCommand::Save(request) => {
                        written += 1;
                        let SaveRequest {
                            generation,
                            file,
                            path,
                            content,
                            guard,
                            ..
                        } = *request;

                        let outcome = crate::persist::save(&path, content.as_bytes(), Some(&guard));

                        self.update(Message::SaveSettled {
                            generation,
                            file,
                            outcome,
                        });
                    }
                    OutgoingCommand::Restore(request) => {
                        let SaveRequest {
                            generation,
                            file,
                            path,
                            content,
                            guard,
                            ..
                        } = *request;

                        let outcome = crate::persist::save(&path, content.as_bytes(), Some(&guard));

                        self.update(Message::DisplayRestored {
                            generation,
                            file,
                            outcome,
                        });
                    }
                    OutgoingCommand::Apply {
                        generation,
                        component,
                        trial,
                    } => {
                        if trial && component == Component::Compositor && self.compositor {
                            let config = InoxConfig::load(&self.paths, Appearance::System)
                                .expect("the files load");
                            self.update(Message::DisplayTrialArmed {
                                generation,
                                config: Box::new(config),
                            });
                        } else {
                            // No transport exists in this milestone, which is what
                            // the binary reports as well.
                            self.update(Message::ComponentUnreachable {
                                generation,
                                component,
                                reason: "no transport is implemented in this milestone".to_owned(),
                            });
                        }
                    }
                    OutgoingCommand::ConfirmDisplay { generation } => {
                        if self.compositor {
                            self.update(Message::DisplayConfirmed { generation });
                        } else {
                            self.update(Message::DisplayConfirmFailed {
                                generation,
                                reason: "no transport is implemented in this milestone".to_owned(),
                            });
                        }
                    }
                    OutgoingCommand::RollbackDisplay { generation } => {
                        if self.compositor {
                            self.update(Message::DisplayReverted { generation });
                        } else {
                            self.update(Message::DisplayRevertFailed {
                                generation,
                                reason: "no transport is implemented in this milestone".to_owned(),
                            });
                        }
                    }
                    OutgoingCommand::Reload => {
                        let config = InoxConfig::load(&self.paths, Appearance::System)
                            .expect("the files load");
                        let documents = Documents::open(&config, &self.paths)
                            .expect("the files are read again");

                        self.update(Message::Reloaded {
                            config: Box::new(config),
                            documents: Box::new(documents),
                        });
                    }
                }
            }
        }
    }

    /// Reads the configuration back from the directory, the way a restart does.
    pub fn reload_config(&self) -> InoxConfig {
        InoxConfig::load(&self.paths, Appearance::System).expect("the files load")
    }

    /// The bytes of one of the window's files.
    pub fn file(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.directory.path().join(name)).unwrap_or_default()
    }

    /// The path of one of the window's files.
    pub fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}
