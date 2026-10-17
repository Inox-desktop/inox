//! User-readable explanations, as Fluent messages rather than rendered prose.
//!
//! A [`Diagnostic`] is a message id plus its placeholders; it becomes a string
//! only when a [`Localizer`] renders it. That indirection is the whole point of
//! this module: the states in [`crate::navigation`] and [`crate::contract`] can
//! say *why* something happened — a window that no longer exists, a command that
//! was only accepted — without a component hard-coding a sentence, and without
//! deciding the locale at the moment the state is produced. Root `AGENTS.md`
//! §7.3 and PRD §11.2 require every user-visible string to come from a catalog;
//! a type that can only carry a message id cannot break that rule by accident.
//!
//! Diagnostics nest: [`Diagnostic::with_message`] puts one explanation inside
//! another's placeholder. It is how a state whose reason is itself a diagnostic
//! (`ServiceState::Unavailable { actionable }`) stays readable without the
//! producer having to render anything.

use std::fmt;

use inox_common::i18n::{Arg, Localizer};

/// A user-readable explanation, waiting for a locale.
///
/// The id must be one of the ids in [`crate::i18n::id`]; `tests/catalog.rs`
/// renders every diagnostic the crate can produce in both shipped locales, so
/// an id that no catalog defines fails the test suite rather than reaching a
/// user as a bare identifier.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    id: &'static str,
    values: Vec<(&'static str, Value)>,
}

/// One placeholder of a [`Diagnostic`].
#[derive(Debug, Clone, PartialEq)]
enum Value {
    /// A scalar, formatted for the locale.
    Scalar(Arg),
    /// Another diagnostic, rendered first and then passed as text.
    Nested(Box<Diagnostic>),
}

impl Diagnostic {
    /// A diagnostic for the message `id`.
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            values: Vec::new(),
        }
    }

    /// Adds a placeholder: `name = value`.
    pub fn with(mut self, name: &'static str, value: impl Into<Arg>) -> Self {
        self.values.push((name, Value::Scalar(value.into())));
        self
    }

    /// Adds a placeholder whose value is another diagnostic.
    ///
    /// The nested diagnostic is rendered with the renderer's localizer, so both
    /// halves of the sentence are translated.
    pub fn with_message(mut self, name: &'static str, message: Diagnostic) -> Self {
        self.values.push((name, Value::Nested(Box::new(message))));
        self
    }

    /// The message id, which is also what a test or a log record can key on.
    pub fn id(&self) -> &'static str {
        self.id
    }

    /// Renders the message in the localizer's locale.
    ///
    /// A message neither catalog defines renders as its own id, which is what
    /// [`inox_common::i18n::Localizer::message_with`] does and why the catalog
    /// test in `tests/catalog.rs` exists.
    pub fn render(&self, localizer: &Localizer) -> String {
        let placeholders: Vec<(&str, Arg)> = self
            .values
            .iter()
            .map(|(name, value)| {
                let resolved = match value {
                    Value::Scalar(arg) => arg.clone(),
                    Value::Nested(message) => Arg::Text(message.render(localizer)),
                };
                (*name, resolved)
            })
            .collect();

        localizer.message_with(self.id, &placeholders)
    }
}

impl fmt::Display for Diagnostic {
    /// Writes the message id and its placeholders, for logs that have no
    /// localizer at hand (a test failure, an `anyhow` context).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.id)?;
        for (name, value) in &self.values {
            match value {
                Value::Scalar(arg) => write!(f, " {name}={arg:?}")?,
                Value::Nested(message) => write!(f, " {name}=({message})")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{id, localizer};
    use inox_common::i18n::Locale;

    fn english() -> Localizer {
        localizer(Locale::default_locale()).expect("the shipped catalogs parse")
    }

    #[test]
    fn a_diagnostic_renders_its_placeholders() {
        let diagnostic = Diagnostic::new(id::REGISTRY_UNKNOWN_ENTRY)
            .with("name", "tray")
            .with("known", "panel, launcher");

        let rendered = diagnostic.render(&english());

        assert!(rendered.contains("tray"), "{rendered}");
        assert!(rendered.contains("panel, launcher"), "{rendered}");
        assert!(!rendered.contains("$"), "{rendered}");
    }

    #[test]
    fn a_nested_diagnostic_is_rendered_into_its_placeholder() {
        let inner = Diagnostic::new(id::SERVICE_READY);
        let outer = Diagnostic::new(id::SERVICE_UNAVAILABLE).with_message("actionable", inner);

        let rendered = outer.render(&english());

        assert!(
            rendered.contains(&english().message(id::SERVICE_READY)),
            "{rendered}"
        );
    }

    #[test]
    fn the_message_id_is_readable_without_a_localizer() {
        let diagnostic = Diagnostic::new(id::NAV_NOTHING_TO_CLOSE);

        assert_eq!(diagnostic.id(), id::NAV_NOTHING_TO_CLOSE);
        assert_eq!(diagnostic.to_string(), id::NAV_NOTHING_TO_CLOSE);
    }

    #[test]
    fn an_unknown_message_id_renders_as_the_id() {
        let diagnostic = Diagnostic::new("shell-not-a-real-id");

        assert_eq!(diagnostic.render(&english()), "shell-not-a-real-id");
    }
}
