//! Configuration model of the Inox desktop: discovery of `~/.config/inox`,
//! TOML models with complete defaults, the locale the session runs in, the
//! theme (design token) system and the reload store that keeps the last good
//! configuration.
//!
//! The layout this crate implements is the one in PRD §9.1:
//!
//! ```text
//! ~/.config/inox/
//! ├── config.toml          # top level: component files, environment, theme, locale
//! ├── compositor.toml      # outputs, input, tiling
//! ├── shell.toml           # panel, launcher, wallpaper
//! ├── notifications.toml   # display, timeouts, queue, policy
//! └── themes/
//!     └── <name>/
//!         ├── theme.toml   # colours, typography, spacing, radii, motion
//!         └── assets/
//! ```
//!
//! ```
//! use inox_config::{Appearance, ConfigPaths, InoxConfig};
//!
//! // A directory without any configuration file is not an error: every file is
//! // optional and falls back to the defaults documented on its type.
//! let paths = ConfigPaths::new("/nonexistent/inox");
//! let config = InoxConfig::load(&paths, Appearance::System).expect("defaults");
//!
//! assert_eq!(config.theme.name, inox_config::BUILTIN_DARK);
//! assert_eq!(config.compositor.layout.master_ratio, 60);
//! assert_eq!(config.shell.panel.position, inox_config::PanelPosition::Bottom);
//! assert_eq!(config.notifications.display.max_visible, 3);
//! // No language is configured, so the locale is negotiated from the
//! // environment; negotiation always lands on a catalog Inox ships.
//! assert!(config.locale.is_supported());
//! ```

#![forbid(unsafe_code)]

pub mod compositor;
pub mod error;
pub mod notifications;
pub mod paths;
pub mod root;
pub mod shell;
pub mod store;
pub mod theme;

pub use crate::compositor::{CompositorConfig, InputConfig, LayoutConfig, OutputConfig, Position};
pub use crate::error::ConfigError;
pub use crate::notifications::{
    DisplayConfig, NotificationPosition, NotificationsConfig, PolicyConfig, QueueConfig,
    QueuePolicy, TimeoutConfig,
};
pub use crate::paths::ConfigPaths;
pub use crate::root::{Appearance, ComponentFiles, LocaleConfig, RootConfig, ThemeSelection};
pub use crate::shell::{
    CLOCK_DIRECTIVES, LauncherConfig, PanelConfig, PanelPosition, ShellConfig, WallpaperConfig,
    WallpaperMode,
};
pub use crate::store::{ConfigStore, InoxConfig};
pub use crate::theme::{
    BUILTIN_DARK, BUILTIN_LIGHT, ColorError, Colors, Motion, Radii, Rgba, Spacing, Theme,
    Typography,
};

/// The crate version as reported by Cargo.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
