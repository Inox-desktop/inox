//! XDG base directory resolution shared by every Inox component.
//!
//! Components store their files below the standard XDG base directories (XDG
//! Base Directory Specification), each time in an `inox` subdirectory; PRD §9.1
//! describes the resulting configuration layout. Reading the environment is
//! funnelled through [`XdgEnv`], a snapshot that can also be built from an
//! explicit set of variables, which keeps resolution testable without touching
//! the process environment.

use std::path::PathBuf;

/// Environment variable holding the user's home directory.
pub const ENV_HOME: &str = "HOME";

/// Environment variable holding the user's configuration directory.
pub const ENV_CONFIG_HOME: &str = "XDG_CONFIG_HOME";

/// Environment variable holding the user's data directory.
pub const ENV_DATA_HOME: &str = "XDG_DATA_HOME";

/// Environment variable holding the user's state directory.
pub const ENV_STATE_HOME: &str = "XDG_STATE_HOME";

/// Environment variable holding the user's cache directory.
pub const ENV_CACHE_HOME: &str = "XDG_CACHE_HOME";

/// Environment variable holding the user's runtime directory.
pub const ENV_RUNTIME_DIR: &str = "XDG_RUNTIME_DIR";

/// Directory Inox uses below each XDG base directory.
pub const APP_DIR: &str = "inox";

/// Errors returned while resolving XDG base directories.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathsError {
    /// The home directory could not be determined because [`ENV_HOME`] is
    /// unset, empty or not an absolute path.
    #[error("cannot determine the user home directory: $HOME is unset, empty or not absolute")]
    HomeUnavailable,

    /// The runtime directory could not be determined because
    /// [`ENV_RUNTIME_DIR`] is unset, empty or not an absolute path. The
    /// specification defines no fallback for this directory.
    #[error(
        "cannot determine the runtime directory: $XDG_RUNTIME_DIR is unset, empty or not absolute"
    )]
    RuntimeDirUnavailable,
}

/// Snapshot of the environment variables the XDG base directory specification
/// uses.
///
/// Values are kept as they were read; the rules of the specification (empty and
/// relative values are invalid and therefore ignored) are applied when a
/// directory is requested. Build a snapshot with [`XdgEnv::from_process`] or,
/// for tests and sandboxed sessions, with [`XdgEnv::from_pairs`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XdgEnv {
    home: Option<PathBuf>,
    config_home: Option<PathBuf>,
    data_home: Option<PathBuf>,
    state_home: Option<PathBuf>,
    cache_home: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
}

impl XdgEnv {
    /// Snapshot the environment of the current process.
    ///
    /// Variables that are unset are recorded as absent; everything else is
    /// validated when a directory is resolved.
    #[must_use]
    pub fn from_process() -> Self {
        Self::from_pairs(
            [
                ENV_HOME,
                ENV_CONFIG_HOME,
                ENV_DATA_HOME,
                ENV_STATE_HOME,
                ENV_CACHE_HOME,
                ENV_RUNTIME_DIR,
            ]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name, value))),
        )
    }

    /// Build a snapshot from explicit `name = value` pairs.
    ///
    /// Variables other than the ones named by this module are ignored, so a
    /// whole environment can be handed over. Later pairs win over earlier ones.
    ///
    /// # Examples
    ///
    /// ```
    /// use inox_common::paths::XdgEnv;
    ///
    /// let env = XdgEnv::from_pairs([("HOME", "/home/inox")]);
    ///
    /// assert_eq!(
    ///     env.inox_config_dir().expect("home is absolute"),
    ///     std::path::Path::new("/home/inox/.config/inox"),
    /// );
    /// ```
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut env = Self::default();

        for (name, value) in pairs {
            let value = PathBuf::from(value.as_ref());

            match name.as_ref() {
                ENV_HOME => env.home = Some(value),
                ENV_CONFIG_HOME => env.config_home = Some(value),
                ENV_DATA_HOME => env.data_home = Some(value),
                ENV_STATE_HOME => env.state_home = Some(value),
                ENV_CACHE_HOME => env.cache_home = Some(value),
                ENV_RUNTIME_DIR => env.runtime_dir = Some(value),
                _ => {}
            }
        }

        env
    }

    /// Resolve `$XDG_CONFIG_HOME`, defaulting to `$HOME/.config`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::HomeUnavailable`] when the variable is unusable
    /// and `$HOME` cannot stand in for it.
    pub fn config_home(&self) -> Result<PathBuf, PathsError> {
        self.base_dir(self.config_home.as_ref(), ".config")
    }

    /// Resolve `$XDG_DATA_HOME`, defaulting to `$HOME/.local/share`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::HomeUnavailable`] when the variable is unusable
    /// and `$HOME` cannot stand in for it.
    pub fn data_home(&self) -> Result<PathBuf, PathsError> {
        self.base_dir(self.data_home.as_ref(), ".local/share")
    }

    /// Resolve `$XDG_STATE_HOME`, defaulting to `$HOME/.local/state`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::HomeUnavailable`] when the variable is unusable
    /// and `$HOME` cannot stand in for it.
    pub fn state_home(&self) -> Result<PathBuf, PathsError> {
        self.base_dir(self.state_home.as_ref(), ".local/state")
    }

    /// Resolve `$XDG_CACHE_HOME`, defaulting to `$HOME/.cache`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::HomeUnavailable`] when the variable is unusable
    /// and `$HOME` cannot stand in for it.
    pub fn cache_home(&self) -> Result<PathBuf, PathsError> {
        self.base_dir(self.cache_home.as_ref(), ".cache")
    }

    /// Resolve `$XDG_RUNTIME_DIR`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::RuntimeDirUnavailable`] when the variable is
    /// unset, empty or relative: the specification defines no fallback for the
    /// runtime directory.
    pub fn runtime_dir(&self) -> Result<PathBuf, PathsError> {
        self.absolute(self.runtime_dir.as_ref())
            .ok_or(PathsError::RuntimeDirUnavailable)
    }

    /// Resolve the Inox configuration directory, `$XDG_CONFIG_HOME/inox`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::HomeUnavailable`] when no configuration directory
    /// can be determined.
    pub fn inox_config_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.config_home()?.join(APP_DIR))
    }

    /// Resolve the Inox data directory, `$XDG_DATA_HOME/inox`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::HomeUnavailable`] when no data directory can be
    /// determined.
    pub fn inox_data_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.data_home()?.join(APP_DIR))
    }

    /// Resolve the Inox state directory, `$XDG_STATE_HOME/inox`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::HomeUnavailable`] when no state directory can be
    /// determined.
    pub fn inox_state_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.state_home()?.join(APP_DIR))
    }

    /// Resolve the Inox cache directory, `$XDG_CACHE_HOME/inox`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::HomeUnavailable`] when no cache directory can be
    /// determined.
    pub fn inox_cache_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.cache_home()?.join(APP_DIR))
    }

    /// Resolve the Inox runtime directory, `$XDG_RUNTIME_DIR/inox`.
    ///
    /// # Errors
    ///
    /// Returns [`PathsError::RuntimeDirUnavailable`] when the runtime directory
    /// cannot be determined.
    pub fn inox_runtime_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.runtime_dir()?.join(APP_DIR))
    }

    /// Resolve the home directory, which every default below the base
    /// directories is built from.
    fn home(&self) -> Result<PathBuf, PathsError> {
        self.absolute(self.home.as_ref())
            .ok_or(PathsError::HomeUnavailable)
    }

    /// Resolve a base directory: the configured value when it is usable, the
    /// `$HOME`-relative `fallback` otherwise.
    fn base_dir(&self, value: Option<&PathBuf>, fallback: &str) -> Result<PathBuf, PathsError> {
        match self.absolute(value) {
            Some(dir) => Ok(dir),
            None => Ok(self.home()?.join(fallback)),
        }
    }

    /// The value as a directory, or `None` when the specification forbids
    /// using it: unset, empty or relative paths are invalid.
    fn absolute(&self, value: Option<&PathBuf>) -> Option<PathBuf> {
        value.filter(|dir| dir.is_absolute()).cloned()
    }
}

/// Resolve `$XDG_CONFIG_HOME` from the environment of the current process.
///
/// # Errors
///
/// Returns [`PathsError::HomeUnavailable`] when no configuration directory can
/// be determined; see [`XdgEnv::config_home`].
pub fn config_home() -> Result<PathBuf, PathsError> {
    XdgEnv::from_process().config_home()
}

/// Resolve `$XDG_DATA_HOME` from the environment of the current process.
///
/// # Errors
///
/// Returns [`PathsError::HomeUnavailable`] when no data directory can be
/// determined; see [`XdgEnv::data_home`].
pub fn data_home() -> Result<PathBuf, PathsError> {
    XdgEnv::from_process().data_home()
}

/// Resolve `$XDG_STATE_HOME` from the environment of the current process.
///
/// # Errors
///
/// Returns [`PathsError::HomeUnavailable`] when no state directory can be
/// determined; see [`XdgEnv::state_home`].
pub fn state_home() -> Result<PathBuf, PathsError> {
    XdgEnv::from_process().state_home()
}

/// Resolve `$XDG_CACHE_HOME` from the environment of the current process.
///
/// # Errors
///
/// Returns [`PathsError::HomeUnavailable`] when no cache directory can be
/// determined; see [`XdgEnv::cache_home`].
pub fn cache_home() -> Result<PathBuf, PathsError> {
    XdgEnv::from_process().cache_home()
}

/// Resolve `$XDG_RUNTIME_DIR` from the environment of the current process.
///
/// # Errors
///
/// Returns [`PathsError::RuntimeDirUnavailable`] when no runtime directory can
/// be determined; see [`XdgEnv::runtime_dir`].
pub fn runtime_dir() -> Result<PathBuf, PathsError> {
    XdgEnv::from_process().runtime_dir()
}

/// Resolve the Inox configuration directory from the environment of the
/// current process; see [`XdgEnv::inox_config_dir`] and PRD §9.1.
///
/// # Errors
///
/// Returns [`PathsError::HomeUnavailable`] when no configuration directory can
/// be determined.
pub fn inox_config_dir() -> Result<PathBuf, PathsError> {
    XdgEnv::from_process().inox_config_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const HOME: &str = "/home/inox";

    /// A snapshot with only `$HOME` set, so that every base directory is
    /// resolved through its default.
    fn home_only() -> XdgEnv {
        XdgEnv::from_pairs([(ENV_HOME, HOME)])
    }

    #[test]
    fn absolute_values_are_used_verbatim() {
        let env = XdgEnv::from_pairs([
            (ENV_HOME, HOME),
            (ENV_CONFIG_HOME, "/cfg"),
            (ENV_DATA_HOME, "/data"),
            (ENV_STATE_HOME, "/state"),
            (ENV_CACHE_HOME, "/cache"),
            (ENV_RUNTIME_DIR, "/run/user/1000"),
        ]);

        assert_eq!(env.config_home().expect("absolute"), Path::new("/cfg"));
        assert_eq!(env.data_home().expect("absolute"), Path::new("/data"));
        assert_eq!(env.state_home().expect("absolute"), Path::new("/state"));
        assert_eq!(env.cache_home().expect("absolute"), Path::new("/cache"));
        assert_eq!(
            env.runtime_dir().expect("absolute"),
            Path::new("/run/user/1000")
        );
    }

    #[test]
    fn defaults_follow_the_specification() {
        let env = home_only();

        assert_eq!(
            env.config_home().expect("home is absolute"),
            Path::new("/home/inox/.config")
        );
        assert_eq!(
            env.data_home().expect("home is absolute"),
            Path::new("/home/inox/.local/share")
        );
        assert_eq!(
            env.state_home().expect("home is absolute"),
            Path::new("/home/inox/.local/state")
        );
        assert_eq!(
            env.cache_home().expect("home is absolute"),
            Path::new("/home/inox/.cache")
        );
    }

    #[test]
    fn empty_and_relative_values_fall_back_to_the_default() {
        for invalid in ["", "relative", "./config", "../config"] {
            let env = XdgEnv::from_pairs([(ENV_HOME, HOME), (ENV_CONFIG_HOME, invalid)]);

            assert_eq!(
                env.config_home().expect("home is absolute"),
                Path::new("/home/inox/.config"),
                "expected `{invalid}` to be ignored"
            );
        }
    }

    #[test]
    fn a_missing_or_relative_home_cannot_be_defaulted_to() {
        for home in [None, Some(""), Some("relative")] {
            let env = match home {
                Some(value) => XdgEnv::from_pairs([(ENV_HOME, value)]),
                None => XdgEnv::default(),
            };

            assert_eq!(env.config_home(), Err(PathsError::HomeUnavailable));
            assert_eq!(env.data_home(), Err(PathsError::HomeUnavailable));
            assert_eq!(env.state_home(), Err(PathsError::HomeUnavailable));
            assert_eq!(env.cache_home(), Err(PathsError::HomeUnavailable));
        }
    }

    #[test]
    fn the_runtime_directory_has_no_fallback() {
        for runtime in [None, Some(""), Some("relative")] {
            let env = match runtime {
                Some(value) => XdgEnv::from_pairs([(ENV_HOME, HOME), (ENV_RUNTIME_DIR, value)]),
                None => home_only(),
            };

            assert_eq!(
                env.runtime_dir(),
                Err(PathsError::RuntimeDirUnavailable),
                "expected `{runtime:?}` to be rejected"
            );
        }

        let env = XdgEnv::from_pairs([(ENV_HOME, HOME), (ENV_RUNTIME_DIR, "/run/user/1000")]);

        assert_eq!(
            env.inox_runtime_dir().expect("absolute"),
            Path::new("/run/user/1000/inox")
        );
    }

    #[test]
    fn app_directories_are_nested_below_their_base_directory() {
        let env = XdgEnv::from_pairs([
            (ENV_HOME, HOME),
            (ENV_CONFIG_HOME, "/cfg"),
            (ENV_DATA_HOME, "/data"),
            (ENV_STATE_HOME, "/state"),
            (ENV_CACHE_HOME, "/cache"),
        ]);

        assert_eq!(
            env.inox_config_dir().expect("absolute"),
            Path::new("/cfg/inox")
        );
        assert_eq!(
            env.inox_data_dir().expect("absolute"),
            Path::new("/data/inox")
        );
        assert_eq!(
            env.inox_state_dir().expect("absolute"),
            Path::new("/state/inox")
        );
        assert_eq!(
            env.inox_cache_dir().expect("absolute"),
            Path::new("/cache/inox")
        );
    }

    #[test]
    fn app_directories_of_defaults_are_absolute_too() {
        let env = home_only();

        assert_eq!(
            env.inox_config_dir().expect("home is absolute"),
            Path::new("/home/inox/.config/inox")
        );
    }

    #[test]
    fn variables_outside_the_specification_are_ignored() {
        let env = XdgEnv::from_pairs([("PATH", "/usr/bin"), ("WAYLAND_DISPLAY", "wayland-0")]);

        assert_eq!(env, XdgEnv::default());
        assert_eq!(env.config_home(), Err(PathsError::HomeUnavailable));
        assert_eq!(env.runtime_dir(), Err(PathsError::RuntimeDirUnavailable));
    }

    #[test]
    fn later_pairs_win_over_earlier_ones() {
        let env = XdgEnv::from_pairs([
            (ENV_HOME, HOME),
            (ENV_CONFIG_HOME, "/first"),
            (ENV_CONFIG_HOME, "/second"),
        ]);

        assert_eq!(env.config_home().expect("absolute"), Path::new("/second"));
    }
    #[test]
    fn errors_name_the_variable_they_are_about() {
        assert!(PathsError::HomeUnavailable.to_string().contains("$HOME"));
        assert!(
            PathsError::RuntimeDirUnavailable
                .to_string()
                .contains("$XDG_RUNTIME_DIR")
        );
    }
}
