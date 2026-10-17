//! The top-level `config.toml`.
//!
//! PRD §9.1 describes `config.toml` as the file that points at the per-component
//! configuration, carries the environment variables the session exports and
//! selects the theme. This module models exactly that: [`ComponentFiles`] holds
//! the paths, and the paths themselves are resolved against the configuration
//! directory by [`crate::paths::ConfigPaths::file`] rather than being joined
//! here.
//!
//! The shell and notification files are modelled as paths only: their schemas
//! arrive with the milestones that consume them, and inventing fields early
//! would freeze a format nobody has needed yet.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

use crate::paths::{COMPOSITOR_FILE, NOTIFICATIONS_FILE, SHELL_FILE};

/// The top-level configuration file, `<config>/config.toml`.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct RootConfig {
    /// Environment variables the session exports to the components it starts.
    ///
    /// A map rather than a struct: the set of variables is open, and a typo can
    /// only be caught by whoever reads them.
    pub environment: BTreeMap<String, String>,
    /// Where each component finds its own configuration file.
    pub components: ComponentFiles,
    /// Which theme the session should use.
    pub theme: ThemeSelection,
}

/// The files the individual components read their own configuration from.
///
/// Every path is relative to the configuration directory; absolute paths and
/// paths that escape it are rejected by
/// [`crate::paths::ConfigPaths::file`], which
/// [`crate::store::InoxConfig::load`] runs on all three of them. A file that is
/// not parsed yet is therefore still checked at startup.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ComponentFiles {
    /// Compositor configuration file.
    pub compositor: PathBuf,
    /// Shell configuration file. Its schema arrives with the shell milestone.
    pub shell: PathBuf,
    /// Notification daemon configuration file. Its schema arrives with the
    /// notification milestone.
    pub notifications: PathBuf,
}

impl Default for ComponentFiles {
    fn default() -> Self {
        Self {
            compositor: PathBuf::from(COMPOSITOR_FILE),
            shell: PathBuf::from(SHELL_FILE),
            notifications: PathBuf::from(NOTIFICATIONS_FILE),
        }
    }
}

/// The theme the session should use (PRD §9.2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(deny_unknown_fields, default)]
pub struct ThemeSelection {
    /// Name of the theme: a built-in such as `inox-light`, or a directory below
    /// `themes/`. `None` — the default — selects the built-in theme matching the
    /// effective appearance.
    pub name: Option<String>,
    /// Appearance to use, or [`Appearance::System`] to follow the session.
    pub appearance: Appearance,
}

/// Light, dark, or whatever the session reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    /// Light colours.
    Light,
    /// Dark colours.
    Dark,
    /// Follow the appearance the session reports. When the session reports
    /// `system` as well there is nothing to follow, which
    /// [`ThemeSelection::resolve`] documents as falling back to
    /// [`Appearance::Dark`].
    ///
    /// Only this selection may ask for it: a theme file describes a single
    /// palette and has to pick [`Appearance::Light`] or [`Appearance::Dark`].
    #[default]
    System,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ConfigError;
    use std::path::Path;

    /// The file name used in assertions on rendered errors.
    const ROOT_PATH: &str = "/cfg/inox/config.toml";

    fn parse_root(text: &str) -> Result<RootConfig, ConfigError> {
        crate::error::parse(Path::new(ROOT_PATH), text)
    }

    #[test]
    fn an_empty_document_uses_every_default() {
        let root = parse_root("").expect("empty document");

        assert!(root.environment.is_empty());
        assert_eq!(root.components.compositor, PathBuf::from("compositor.toml"));
        assert_eq!(root.components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            root.components.notifications,
            PathBuf::from("notifications.toml")
        );
        assert_eq!(root.theme.name, None);
        assert_eq!(root.theme.appearance, Appearance::System);
    }

    #[test]
    fn the_component_defaults_are_the_file_names_of_the_prd_layout() {
        let components = ComponentFiles::default();

        assert_eq!(components.compositor, PathBuf::from("compositor.toml"));
        assert_eq!(components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            components.notifications,
            PathBuf::from("notifications.toml")
        );
    }

    #[test]
    fn environment_components_and_theme_are_parsed() {
        let root = parse_root(
            "[environment]\n\
             MOZ_ENABLE_WAYLAND = \"1\"\n\
             WINIT_UNIX_BACKEND = \"wayland\"\n\
             \n\
             [components]\n\
             compositor = \"c.toml\"\n\
             shell = \"s.toml\"\n\
             notifications = \"n.toml\"\n\
             \n\
             [theme]\n\
             name = \"mine\"\n\
             appearance = \"light\"\n",
        )
        .expect("document");

        assert_eq!(
            root.environment
                .get("WINIT_UNIX_BACKEND")
                .map(String::as_str),
            Some("wayland")
        );
        assert_eq!(root.environment.len(), 2);
        assert_eq!(root.components.compositor, PathBuf::from("c.toml"));
        assert_eq!(root.components.shell, PathBuf::from("s.toml"));
        assert_eq!(root.components.notifications, PathBuf::from("n.toml"));
        assert_eq!(root.theme.name.as_deref(), Some("mine"));
        assert_eq!(root.theme.appearance, Appearance::Light);
    }

    #[test]
    fn a_partial_section_keeps_the_defaults_of_the_fields_it_leaves_out() {
        let root = parse_root("[theme]\nappearance = \"dark\"\n").expect("document");

        assert_eq!(root.theme.appearance, Appearance::Dark);
        assert_eq!(root.theme.name, None);
        assert_eq!(root.components.compositor, PathBuf::from("compositor.toml"));
        assert_eq!(root.components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            root.components.notifications,
            PathBuf::from("notifications.toml")
        );
        assert!(root.environment.is_empty());
    }

    #[test]
    fn every_appearance_spelling_is_parsed() {
        for (text, expected) in [
            ("light", Appearance::Light),
            ("dark", Appearance::Dark),
            ("system", Appearance::System),
        ] {
            let root =
                parse_root(&format!("[theme]\nappearance = \"{text}\"\n")).expect("document");

            assert_eq!(root.theme.appearance, expected);
        }
    }

    #[test]
    fn a_misspelled_appearance_is_rejected() {
        let error = parse_root("[theme]\nappearance = \"purple\"\n").expect_err("unknown variant");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("theme.appearance"));
                assert_eq!(line, Some(2));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_key_is_rejected_with_its_field_path() {
        let error = parse_root("nonsense = 1\n").expect_err("unknown key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("nonsense"));
                assert_eq!(line, Some(1));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_key_inside_a_section_reports_the_dotted_path() {
        let error = parse_root("[components]\nshell = \"s.toml\"\nnonsense = 1\n")
            .expect_err("unknown key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("components.nonsense"));
                assert_eq!(line, Some(3));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_parse_error_names_the_file_the_position_and_the_field() {
        // The offending key is the first thing on the first line of the document.
        let error = parse_root("nonsense = 1\n").expect_err("unknown key");
        let rendered = error.to_string();

        assert!(
            rendered.contains("/cfg/inox/config.toml:1:1"),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains("(field `nonsense`)"),
            "rendered `{rendered}`"
        );
        assert!(rendered.contains("nonsense"), "rendered `{rendered}`");
    }

    #[test]
    fn a_document_level_error_is_reported_without_a_field_path() {
        // The TOML parser rejects the duplicated key before any field is being
        // deserialized, so there is no field path to report.
        let error = parse_root("[components]\ncompositor = \"a.toml\"\ncompositor = \"b.toml\"\n")
            .expect_err("duplicate key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field, None);
                assert!(
                    line.is_some(),
                    "expected the parser to locate the duplicate key"
                );
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }
}
