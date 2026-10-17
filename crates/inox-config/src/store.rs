//! Loading every configuration file at once, and keeping the last good one.
//!
//! [`InoxConfig`] is the unit a component starts with: the top-level
//! `config.toml`, the theme it selects and the compositor file it points at.
//! [`ConfigStore`] wraps it for a running session, which is what SIGHUP and
//! `inoxctl reload` act on: a reload either replaces the whole configuration or
//! changes nothing at all (PRD §9.1, root AGENTS.md §7.1).

use std::path::Path;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::de::DeserializeOwned;
use tracing::{debug, info, warn};

use crate::compositor::CompositorConfig;
use crate::error::{ConfigError, parse, read_text};
use crate::paths::ConfigPaths;
use crate::root::{Appearance, RootConfig};
use crate::theme::Theme;

/// Everything a component needs from the configuration to start up.
#[derive(Debug, Clone, PartialEq)]
pub struct InoxConfig {
    /// The top-level `config.toml`.
    pub root: RootConfig,
    /// The theme the top-level file selected.
    pub theme: Theme,
    /// The compositor configuration the top-level file points at.
    pub compositor: CompositorConfig,
}

impl InoxConfig {
    /// Load `config.toml`, resolve the theme it selects and load the compositor
    /// file it points at.
    ///
    /// A file that does not exist falls back to its defaults and is reported at
    /// debug level; a file that exists but cannot be read or parsed is an error,
    /// so a typo never quietly changes the session.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Paths`] when no configuration directory exists,
    /// [`ConfigError::InvalidComponentPath`] for any of the three `[components]`
    /// paths that escapes it, [`ConfigError::Read`] or [`ConfigError::Parse`]
    /// for a file that cannot be used (a [`LayoutConfig`](crate::compositor::LayoutConfig)
    /// value outside its range included), and [`ConfigError::InvalidThemeName`],
    /// [`ConfigError::ThemeNotFound`] for the selected theme.
    pub fn load(paths: &ConfigPaths, system: Appearance) -> Result<Self, ConfigError> {
        let root: RootConfig = load_or_default(&paths.root_file())?;
        let theme = root.theme.resolve(paths, system)?;

        let compositor_file = paths.file(&root.components.compositor)?;
        // The shell and notification files are not parsed yet — their schemas
        // arrive with the milestones that consume them — but their paths are
        // validated all the same: a path that escapes the configuration
        // directory has to fail when Inox starts, not the first time that
        // component starts.
        paths.file(&root.components.shell)?;
        paths.file(&root.components.notifications)?;

        let compositor: CompositorConfig = load_or_default(&compositor_file)?;

        Ok(Self {
            root,
            theme,
            compositor,
        })
    }
}

/// Read `path`, falling back to `T::default()` when the file is absent.
fn load_or_default<T>(path: &Path) -> Result<T, ConfigError>
where
    T: DeserializeOwned + Default,
{
    match read_text(path)? {
        Some(text) => parse(path, &text),
        None => {
            debug!(
                path = %path.display(),
                "configuration file is absent; using its defaults"
            );

            Ok(T::default())
        }
    }
}

/// The configuration a running session serves, and its reloads.
///
/// [`ConfigStore::current`] hands out an `Arc`, so a reload never invalidates a
/// reference a component is already holding: the holder keeps the configuration
/// it started with until it asks for the new one.
#[derive(Debug)]
pub struct ConfigStore {
    paths: ConfigPaths,
    system: Appearance,
    current: RwLock<Arc<InoxConfig>>,
}

impl ConfigStore {
    /// Load the configuration from `paths` and start serving it.
    ///
    /// # Errors
    ///
    /// Returns whatever [`InoxConfig::load`] returns: a configuration that is
    /// already broken is reported at startup rather than at the first reload.
    pub fn new(paths: ConfigPaths, system: Appearance) -> Result<Self, ConfigError> {
        let config = Arc::new(InoxConfig::load(&paths, system)?);

        Ok(Self {
            paths,
            system,
            current: RwLock::new(config),
        })
    }

    /// The configuration in force.
    pub fn current(&self) -> Arc<InoxConfig> {
        Arc::clone(&self.read())
    }

    /// Re-read every configuration file; this is what a reload signal runs.
    ///
    /// The new configuration replaces the old one only when it loaded
    /// completely. On any failure the configuration already in force stays, the
    /// reason is logged at warn level, and the error is returned as well so that
    /// a caller can tell the user what to fix.
    ///
    /// # Errors
    ///
    /// Returns whatever [`InoxConfig::load`] returns.
    pub fn reload(&self) -> Result<(), ConfigError> {
        match InoxConfig::load(&self.paths, self.system) {
            Ok(config) => {
                *self.write() = Arc::new(config);
                info!("configuration reloaded");

                Ok(())
            }
            Err(error) => {
                warn!(
                    error = %error,
                    "configuration reload failed; keeping the configuration in force"
                );

                Err(error)
            }
        }
    }

    /// Take the read lock, ignoring poisoning.
    ///
    /// The lock only guards an `Arc` swap, so a thread that panicked while
    /// holding it cannot have left the configuration inconsistent; refusing to
    /// serve it would be the bigger failure.
    fn read(&self) -> RwLockReadGuard<'_, Arc<InoxConfig>> {
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Take the write lock, ignoring poisoning; see [`ConfigStore::read`].
    fn write(&self) -> RwLockWriteGuard<'_, Arc<InoxConfig>> {
        self.current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::test_support::{column_of, line_of};
    use crate::theme::{BUILTIN_DARK, Rgba};
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Replace `relative` inside `dir`, creating the directories it needs.
    fn write(dir: &TempDir, relative: &str, contents: &str) {
        let path = dir.path().join(relative);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent directory");
        }

        std::fs::write(&path, contents).expect("write configuration file");
    }

    fn paths(dir: &TempDir) -> ConfigPaths {
        ConfigPaths::new(dir.path())
    }

    fn hex(value: &str) -> Rgba {
        Rgba::from_hex(value).expect("valid hexadecimal colour")
    }

    #[test]
    fn a_configuration_directory_that_does_not_exist_yields_every_default() {
        let dir = tempfile::tempdir().expect("temp directory");
        let paths = ConfigPaths::new(dir.path().join("absent"));

        let config = InoxConfig::load(&paths, Appearance::System).expect("defaults");

        assert!(config.root.environment.is_empty());
        assert_eq!(
            config.root.components.compositor,
            PathBuf::from("compositor.toml")
        );
        assert_eq!(config.root.components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            config.root.components.notifications,
            PathBuf::from("notifications.toml")
        );
        assert_eq!(config.root.theme.name, None);
        assert_eq!(config.root.theme.appearance, Appearance::System);
        assert_eq!(config.theme.name, BUILTIN_DARK);
        assert_eq!(config.theme.appearance, Appearance::Dark);
        assert!(config.compositor.outputs.is_empty());
        assert_eq!(config.compositor.input.keyboard_layout, "us");
        assert_eq!(config.compositor.layout.master_ratio, 60);
        assert_eq!(config.compositor.layout.master_count, 1);
        assert_eq!(config.compositor.layout.gap, 8);
    }

    #[test]
    fn a_component_file_that_does_not_exist_falls_back_to_its_defaults() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(
            &dir,
            "config.toml",
            "[components]\ncompositor = \"compositor.toml\"\n",
        );

        let config = InoxConfig::load(&paths(&dir), Appearance::System).expect("defaults");

        assert_eq!(
            config.root.components.compositor,
            PathBuf::from("compositor.toml")
        );
        assert!(config.compositor.outputs.is_empty());
        assert_eq!(config.compositor.layout.master_ratio, 60);
        assert_eq!(config.compositor.layout.master_count, 1);
        assert_eq!(config.compositor.layout.gap, 8);
    }

    #[test]
    fn a_component_file_that_appears_at_a_custom_path_is_read() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(
            &dir,
            "config.toml",
            "[components]\ncompositor = \"components/comp.toml\"\n",
        );
        write(
            &dir,
            "components/comp.toml",
            "[layout]\nmaster_ratio = 70\n",
        );

        let config = InoxConfig::load(&paths(&dir), Appearance::System).expect("config");

        assert_eq!(config.compositor.layout.master_ratio, 70);
    }

    #[test]
    fn a_components_section_that_only_names_the_compositor_keeps_the_other_names() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(
            &dir,
            "config.toml",
            "[components]\ncompositor = \"components/comp.toml\"\n",
        );

        let config = InoxConfig::load(&paths(&dir), Appearance::System).expect("config");

        assert_eq!(
            config.root.components.compositor,
            PathBuf::from("components/comp.toml")
        );
        assert_eq!(config.root.components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            config.root.components.notifications,
            PathBuf::from("notifications.toml")
        );
        // The default compositor file is absent too, so its contents are defaults.
        assert_eq!(config.compositor.layout.master_ratio, 60);
        assert!(config.compositor.outputs.is_empty());
    }

    #[test]
    fn an_empty_configuration_file_keeps_every_default() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(&dir, "config.toml", "");

        let config = InoxConfig::load(&paths(&dir), Appearance::System).expect("defaults");

        assert!(config.root.environment.is_empty());
        assert_eq!(config.root.theme.name, None);
        assert_eq!(config.root.theme.appearance, Appearance::System);
        assert_eq!(config.theme.name, BUILTIN_DARK);
        assert_eq!(config.theme.appearance, Appearance::Dark);
        assert_eq!(config.compositor.layout.master_ratio, 60);
    }

    #[test]
    fn a_malformed_root_file_reports_file_line_and_field() {
        let dir = tempfile::tempdir().expect("temp directory");
        let document = "[theme]\nappearance = \"purple\"\n";
        write(&dir, "config.toml", document);

        let error = InoxConfig::load(&paths(&dir), Appearance::System).expect_err("malformed");

        let expected_line = line_of(document, "appearance");
        match error {
            ConfigError::Parse {
                path,
                field,
                line,
                message,
                ..
            } => {
                assert!(path.ends_with("config.toml"), "got {path:?}");
                assert_eq!(field.as_deref(), Some("theme.appearance"));
                assert_eq!(line, Some(expected_line));
                assert!(message.contains("purple"), "message `{message}`");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_component_path_that_escapes_the_configuration_directory_is_rejected() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(
            &dir,
            "config.toml",
            "[components]\ncompositor = \"../compositor.toml\"\n",
        );

        let error = InoxConfig::load(&paths(&dir), Appearance::System).expect_err("escaping");

        match error {
            ConfigError::InvalidComponentPath { path } => {
                assert_eq!(path, PathBuf::from("../compositor.toml"));
            }
            other => panic!("expected an invalid component path, got {other:?}"),
        }
    }

    #[test]
    fn an_escaping_shell_or_notification_path_is_rejected_at_startup() {
        // Neither file is parsed yet, so this is the only place their paths are
        // checked: an absolute path and one climbing out of the directory.
        for (key, rejected) in [
            ("shell", "/etc/inox/shell.toml"),
            ("notifications", "../../../etc/hosts"),
            ("notifications", ""),
        ] {
            let dir = tempfile::tempdir().expect("temp directory");
            write(
                &dir,
                "config.toml",
                &format!("[components]\n{key} = \"{rejected}\"\n"),
            );

            let error = InoxConfig::load(&paths(&dir), Appearance::System).expect_err("escaping");

            assert!(
                matches!(&error, ConfigError::InvalidComponentPath { path } if path == Path::new(rejected)),
                "expected the `{key}` path to be rejected, got {error:?}"
            );
        }
    }

    #[test]
    fn valid_shell_and_notification_paths_are_accepted() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(
            &dir,
            "config.toml",
            "[components]\nshell = \"shell.toml\"\nnotifications = \"notifications.toml\"\n",
        );

        let config = InoxConfig::load(&paths(&dir), Appearance::System).expect("config");

        assert_eq!(config.root.components.shell, PathBuf::from("shell.toml"));
        assert_eq!(
            config.root.components.notifications,
            PathBuf::from("notifications.toml")
        );
    }

    #[test]
    fn a_malformed_component_file_reports_file_line_and_field() {
        let dir = tempfile::tempdir().expect("temp directory");
        let document = "[layout]\nmaster_ratio = \"wide\"\n";
        write(&dir, "compositor.toml", document);

        let error = InoxConfig::load(&paths(&dir), Appearance::System).expect_err("malformed");

        let expected_line = line_of(document, "master_ratio");
        let expected_column = column_of(document, "\"wide\"");
        let rendered = error.to_string();

        assert!(
            rendered.contains(&format!(
                "compositor.toml:{expected_line}:{expected_column}"
            )),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains("(field `layout.master_ratio`)"),
            "rendered `{rendered}`"
        );

        match error {
            ConfigError::Parse {
                path,
                field,
                line,
                column,
                message,
            } => {
                assert!(path.ends_with("compositor.toml"), "got {path:?}");
                assert_eq!(field.as_deref(), Some("layout.master_ratio"));
                assert_eq!(line, Some(expected_line));
                assert_eq!(column, Some(expected_column));
                assert!(message.contains("invalid type"), "message `{message}`");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_component_file_that_cannot_be_read_is_an_error() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(
            &dir,
            "config.toml",
            "[components]\ncompositor = \"compositor.toml\"\n",
        );
        // A directory where the file is expected is not an absent file.
        std::fs::create_dir(dir.path().join("compositor.toml")).expect("create directory");

        let error = InoxConfig::load(&paths(&dir), Appearance::System).expect_err("not a file");

        let rendered = error.to_string();
        assert!(
            rendered.contains("compositor.toml"),
            "rendered `{rendered}`"
        );
        // The I/O failure stays reachable instead of being flattened into a string.
        assert!(std::error::Error::source(&error).is_some());

        match error {
            ConfigError::Read { path, .. } => {
                assert!(path.ends_with("compositor.toml"), "got {path:?}")
            }
            other => panic!("expected a read error, got {other:?}"),
        }
    }

    #[test]
    fn a_custom_theme_is_loaded_with_the_dark_tokens_it_does_not_change() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(
            &dir,
            "config.toml",
            "[theme]\nname = \"mine\"\nappearance = \"light\"\n",
        );
        write(
            &dir,
            "themes/mine/theme.toml",
            "colors = { accent = \"#ff0000\" }\n",
        );

        let config = InoxConfig::load(&paths(&dir), Appearance::Dark).expect("custom theme");

        assert_eq!(config.theme.name, "mine");
        // The selection asked for light, so the session's dark appearance is not
        // consulted.
        assert_eq!(config.theme.appearance, Appearance::Light);
        assert_eq!(config.theme.colors.accent, hex("#ff0000"));
        assert_eq!(config.theme.colors.background, hex("#1b1d23"));
    }

    #[test]
    fn a_theme_that_cannot_be_found_is_reported() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(&dir, "config.toml", "[theme]\nname = \"absent\"\n");

        let error = InoxConfig::load(&paths(&dir), Appearance::System).expect_err("missing theme");

        assert!(
            matches!(
                &error,
                ConfigError::ThemeNotFound { name, path }
                    if name == "absent" && path.ends_with("themes/absent/theme.toml")
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn a_failed_reload_of_a_broken_theme_keeps_the_configuration_in_force() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(&dir, "config.toml", "[theme]\nname = \"mine\"\n");
        write(
            &dir,
            "themes/mine/theme.toml",
            "colors = { accent = \"#ff0000\" }\n",
        );

        let store = ConfigStore::new(paths(&dir), Appearance::System).expect("load");
        let before = store.current();
        assert_eq!(before.theme.colors.accent, hex("#ff0000"));

        write(
            &dir,
            "themes/mine/theme.toml",
            "colors = { accent = \"red\" }\n",
        );

        match store.reload() {
            Err(ConfigError::Parse { path, field, .. }) => {
                assert!(path.ends_with("themes/mine/theme.toml"), "got {path:?}");
                assert_eq!(field.as_deref(), Some("colors.accent"));
            }
            other => panic!("expected a parse error from the theme, got {other:?}"),
        }

        let after = store.current();
        assert_eq!(after.theme.colors.accent, hex("#ff0000"));
        assert!(Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn the_store_serves_the_configuration_it_loaded() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(&dir, "compositor.toml", "[layout]\nmaster_ratio = 70\n");

        let store = ConfigStore::new(paths(&dir), Appearance::System).expect("load");

        assert_eq!(store.current().compositor.layout.master_ratio, 70);
    }

    #[test]
    fn a_reload_picks_up_an_edit() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(&dir, "compositor.toml", "[layout]\nmaster_ratio = 70\n");

        let store = ConfigStore::new(paths(&dir), Appearance::System).expect("load");
        let before = store.current();

        write(&dir, "compositor.toml", "[layout]\nmaster_ratio = 75\n");
        store.reload().expect("reload");

        assert_eq!(store.current().compositor.layout.master_ratio, 75);
        // The configuration handed out earlier is untouched.
        assert_eq!(before.compositor.layout.master_ratio, 70);
    }

    #[test]
    fn a_reload_picks_up_a_file_that_did_not_exist_before() {
        let dir = tempfile::tempdir().expect("temp directory");

        let store = ConfigStore::new(paths(&dir), Appearance::System).expect("load");
        assert_eq!(store.current().compositor.layout.master_ratio, 60);
        assert!(store.current().compositor.outputs.is_empty());

        write(&dir, "compositor.toml", "[layout]\nmaster_ratio = 33\n");
        store.reload().expect("reload");

        assert_eq!(store.current().compositor.layout.master_ratio, 33);
    }

    #[test]
    fn a_failed_reload_keeps_the_configuration_in_force() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(&dir, "compositor.toml", "[layout]\nmaster_ratio = 70\n");

        let store = ConfigStore::new(paths(&dir), Appearance::System).expect("load");
        let before = store.current();

        write(
            &dir,
            "compositor.toml",
            "[layout]\nmaster_ratio = \"wide\"\n",
        );
        let error = store.reload().expect_err("broken edit");

        match error {
            ConfigError::Parse { path, field, .. } => {
                assert!(path.ends_with("compositor.toml"), "got {path:?}");
                assert_eq!(field.as_deref(), Some("layout.master_ratio"));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }

        let after = store.current();
        assert_eq!(after.compositor.layout.master_ratio, 70);
        // Not merely equal: the very same configuration is still in place.
        assert!(Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn a_failed_reload_can_be_followed_by_a_successful_one() {
        let dir = tempfile::tempdir().expect("temp directory");
        write(&dir, "compositor.toml", "[layout]\nmaster_ratio = 70\n");

        let store = ConfigStore::new(paths(&dir), Appearance::System).expect("load");

        write(
            &dir,
            "compositor.toml",
            "[layout]\nmaster_ratio = 70\nnonsense = 1\n",
        );

        match store.reload() {
            Err(ConfigError::Parse { field, .. }) => {
                assert_eq!(field.as_deref(), Some("layout.nonsense"));
            }
            other => panic!("expected a parse error for the unknown key, got {other:?}"),
        }

        write(&dir, "compositor.toml", "[layout]\nmaster_ratio = 80\n");
        store.reload().expect("reload");

        assert_eq!(store.current().compositor.layout.master_ratio, 80);
    }
}
