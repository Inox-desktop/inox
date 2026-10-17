//! Where the configuration files live below `$XDG_CONFIG_HOME/inox`.
//!
//! PRD §9.1 fixes the layout, and this module is the only place that knows it:
//! every loader asks [`ConfigPaths`] for a file instead of joining names itself.
//! The root directory is normally `$XDG_CONFIG_HOME/inox`, resolved through
//! [`inox_common::paths::XdgEnv`]; [`ConfigPaths::new`] wraps an explicit
//! directory instead, which is what makes the layout testable without touching
//! the process environment.

use std::path::{Component, Path, PathBuf};

use inox_common::paths::XdgEnv;

use crate::error::ConfigError;

/// Name of the top-level configuration file.
pub const ROOT_FILE: &str = "config.toml";

/// Default name of the compositor configuration file, relative to the
/// configuration directory.
pub const COMPOSITOR_FILE: &str = "compositor.toml";

/// Default name of the shell configuration file, relative to the configuration
/// directory. The file's schema arrives with the shell milestone.
pub const SHELL_FILE: &str = "shell.toml";

/// Default name of the notification daemon configuration file, relative to the
/// configuration directory. The file's schema arrives with the notification
/// milestone.
pub const NOTIFICATIONS_FILE: &str = "notifications.toml";

/// Directory holding the user themes, below the configuration directory.
pub const THEMES_DIR: &str = "themes";

/// Name of the file defining a single theme, below `themes/<name>`.
pub const THEME_FILE: &str = "theme.toml";

/// The directory the configuration is read from, normally
/// `$XDG_CONFIG_HOME/inox`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    root: PathBuf,
}

impl ConfigPaths {
    /// Wrap an explicit configuration directory.
    ///
    /// Nothing is read here: the directory may not exist at all, in which case
    /// every loader falls back to its defaults.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve the configuration directory from an environment snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Paths`] when no configuration directory can be
    /// determined, which happens when `$HOME` is unset, empty or relative and
    /// `$XDG_CONFIG_HOME` is unusable as well; see
    /// [`XdgEnv::inox_config_dir`].
    pub fn from_xdg(env: &XdgEnv) -> Result<Self, ConfigError> {
        env.inox_config_dir()
            .map(Self::new)
            .map_err(|source| ConfigError::Paths { source })
    }

    /// Resolve the configuration directory from the environment of the current
    /// process.
    ///
    /// # Errors
    ///
    /// Same as [`ConfigPaths::from_xdg`].
    pub fn discover() -> Result<Self, ConfigError> {
        Self::from_xdg(&XdgEnv::from_process())
    }

    /// The configuration directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The top-level file, `<root>/config.toml`.
    pub fn root_file(&self) -> PathBuf {
        self.root.join(ROOT_FILE)
    }

    /// Resolve a `[components]` path against the configuration directory.
    ///
    /// A relative path is joined onto the root. Empty paths, absolute paths,
    /// paths with a `..` segment and paths with a `\\`-style prefix are
    /// rejected: a configuration file must not be able to make Inox read
    /// outside of its own directory, and an empty value names no file at all.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidComponentPath`] for such a path.
    pub fn file(&self, relative: &Path) -> Result<PathBuf, ConfigError> {
        if relative.as_os_str().is_empty() || escapes_root(relative) {
            return Err(ConfigError::InvalidComponentPath {
                path: relative.to_path_buf(),
            });
        }

        Ok(self.root.join(relative))
    }

    /// The theme directory, `<root>/themes`.
    pub fn themes_dir(&self) -> PathBuf {
        self.root.join(THEMES_DIR)
    }

    /// Directory of the theme called `name`, `<root>/themes/<name>`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidThemeName`] when `name` is empty or cannot
    /// be a single plain path segment: `/`, `\\`, a NUL byte, `.` and `..` are
    /// rejected, so a theme name cannot be used to reach outside `themes/`.
    pub fn theme_dir(&self, name: &str) -> Result<PathBuf, ConfigError> {
        validate_theme_name(name)?;

        Ok(self.themes_dir().join(name))
    }

    /// File defining the theme called `name`,
    /// `<root>/themes/<name>/theme.toml`.
    ///
    /// # Errors
    ///
    /// Same as [`ConfigPaths::theme_dir`].
    pub fn theme_file(&self, name: &str) -> Result<PathBuf, ConfigError> {
        Ok(self.theme_dir(name)?.join(THEME_FILE))
    }
}

/// Whether joining `path` onto a directory could leave it.
///
/// An absolute path starts with a root or prefix component, so it is covered
/// here as well.
fn escapes_root(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

/// Reject theme names that are not a single plain directory name.
fn validate_theme_name(name: &str) -> Result<(), ConfigError> {
    let rejected = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0');

    if rejected {
        return Err(ConfigError::InvalidThemeName {
            name: name.to_owned(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "/cfg/inox";

    fn paths() -> ConfigPaths {
        ConfigPaths::new(ROOT)
    }

    #[test]
    fn the_root_follows_xdg_config_home() {
        let env = XdgEnv::from_pairs([("HOME", "/home/inox")]);

        assert_eq!(
            ConfigPaths::from_xdg(&env)
                .expect("home is absolute")
                .root(),
            Path::new("/home/inox/.config/inox")
        );

        let env = XdgEnv::from_pairs([("HOME", "/home/inox"), ("XDG_CONFIG_HOME", "/cfg")]);

        assert_eq!(
            ConfigPaths::from_xdg(&env).expect("absolute").root(),
            Path::new("/cfg/inox")
        );
    }

    #[test]
    fn a_missing_home_is_reported_as_a_paths_error_keeping_the_source() {
        let error = ConfigPaths::from_xdg(&XdgEnv::default()).expect_err("no home");

        assert!(error.to_string().contains("$HOME"));
        // The XDG resolution failure stays reachable as the source.
        assert!(std::error::Error::source(&error).is_some());

        match error {
            ConfigError::Paths { source } => {
                assert_eq!(source, inox_common::paths::PathsError::HomeUnavailable);
            }
            other => panic!("expected a paths error, got {other:?}"),
        }
    }

    #[test]
    fn files_are_named_after_the_prd_layout() {
        assert_eq!(paths().root_file(), Path::new("/cfg/inox/config.toml"));
        assert_eq!(paths().themes_dir(), Path::new("/cfg/inox/themes"));
        assert_eq!(
            paths().theme_dir("mine").expect("plain name"),
            Path::new("/cfg/inox/themes/mine")
        );
        assert_eq!(
            paths().theme_file("mine").expect("plain name"),
            Path::new("/cfg/inox/themes/mine/theme.toml")
        );
    }

    #[test]
    fn relative_component_paths_are_joined_onto_the_root() {
        for (relative, expected) in [
            ("compositor.toml", "/cfg/inox/compositor.toml"),
            ("components/c.toml", "/cfg/inox/components/c.toml"),
            ("./compositor.toml", "/cfg/inox/./compositor.toml"),
        ] {
            assert_eq!(
                paths().file(Path::new(relative)).expect("relative"),
                Path::new(expected),
                "expected `{relative}` to be joined onto the root"
            );
        }
    }

    #[test]
    fn an_empty_component_path_is_rejected() {
        match paths().file(Path::new("")) {
            Err(ConfigError::InvalidComponentPath { path }) => {
                assert!(path.as_os_str().is_empty(), "got {path:?}");
            }
            other => panic!("expected an empty path to be rejected, got {other:?}"),
        }
    }

    #[test]
    fn absolute_and_escaping_component_paths_are_rejected() {
        for relative in [
            "/etc/inox/compositor.toml",
            "..",
            "../compositor.toml",
            "components/../../compositor.toml",
            "components/..",
        ] {
            match paths().file(Path::new(relative)) {
                Err(ConfigError::InvalidComponentPath { path }) => {
                    assert_eq!(path, Path::new(relative));
                }
                other => panic!("expected `{relative}` to be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn theme_names_that_are_not_plain_segments_are_rejected() {
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "../escape",
            "a\\b",
            "with\0nul",
            "/etc",
        ] {
            match paths().theme_file(name) {
                Err(ConfigError::InvalidThemeName { name: rejected }) => {
                    assert_eq!(rejected, name);
                }
                other => panic!("expected `{name}` to be rejected, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_rejected_theme_name_keeps_the_name_it_rejected() {
        let error = paths().theme_dir("../escape").expect_err("traversal");

        assert!(matches!(&error, ConfigError::InvalidThemeName { name } if name == "../escape"));
        assert!(error.to_string().contains("../escape"));
    }
}
