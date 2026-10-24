//! The shell configuration, `shell.toml`.
//!
//! PRD §9.1 gives the file three jobs — the panel, the launcher and the
//! wallpaper — and those are what this module models. The theme is not repeated
//! here: it is selected once in `config.toml` and resolved by [`crate::theme`],
//! so the components read one theme rather than three.
//!
//! The panel is one of three surfaces — the taskbar, the menu bar and the dock —
//! and `panel.surface` says which one an instance draws ([`PanelSurface`]). The
//! menu bar and the dock have their own sections ([`MenubarConfig`],
//! [`DockConfig`]) because they are different surfaces with different sizes and
//! different groups, while the task, tray and clock data behind them is the
//! same.
//!
//! Unlike `compositor.toml`, these defaults have no earlier implementation to
//! agree with: they are the user-facing defaults the L3 components start from,
//! and what backs them is the schema itself. Each range is enforced while the
//! file is parsed, so a value outside it is a parse error naming the field
//! rather than a silent clamp (ADR 0009, decision 4).
//!
//! `panel.clock_format` and `menubar.clock_format` are the one fields with a
//! grammar of their own: the format is the explicitly supported subset in
//! [`CLOCK_DIRECTIVES`], not a `strftime` string, so a directive the panel cannot
//! render is rejected at load time instead of reaching the panel as literal `%Q`
//! text.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer, Serialize};

/// Default height of the panel, in logical pixels.
const DEFAULT_PANEL_HEIGHT: u16 = 32;

/// Smallest accepted panel height, in logical pixels.
const PANEL_HEIGHT_MIN: u16 = 8;

/// Largest accepted panel height, in logical pixels.
const PANEL_HEIGHT_MAX: u16 = 256;

/// Default distance between the panel and the output edge, in logical pixels.
const DEFAULT_PANEL_MARGIN: u16 = 0;

/// Smallest accepted panel margin, in logical pixels.
const PANEL_MARGIN_MIN: u16 = 0;

/// Largest accepted panel margin, in logical pixels.
const PANEL_MARGIN_MAX: u16 = 64;

/// Default number of task buttons the panel shows.
const DEFAULT_PANEL_MAX_TASK_BUTTONS: u16 = 12;

/// Smallest accepted number of task buttons.
const PANEL_MAX_TASK_BUTTONS_MIN: u16 = 1;

/// Largest accepted number of task buttons.
const PANEL_MAX_TASK_BUTTONS_MAX: u16 = 64;

/// Default size of the panel's icons, in logical pixels.
const DEFAULT_PANEL_ICON_SIZE: u16 = 16;

/// Smallest accepted panel icon size, in logical pixels.
const PANEL_ICON_SIZE_MIN: u16 = 8;

/// Largest accepted panel icon size, in logical pixels.
const PANEL_ICON_SIZE_MAX: u16 = 128;

/// Default clock format: hours and minutes on a 24-hour clock.
const DEFAULT_CLOCK_FORMAT: &str = "%H:%M";

/// Default height of the menu bar, in logical pixels.
const DEFAULT_MENUBAR_HEIGHT: u16 = 28;

/// Smallest accepted menu bar height, in logical pixels.
const MENUBAR_HEIGHT_MIN: u16 = 8;

/// Largest accepted menu bar height, in logical pixels.
const MENUBAR_HEIGHT_MAX: u16 = 256;

/// Default distance between the menu bar and the top edge, in logical pixels.
const DEFAULT_MENUBAR_MARGIN: u16 = 0;

/// Smallest accepted menu bar margin, in logical pixels.
const MENUBAR_MARGIN_MIN: u16 = 0;

/// Largest accepted menu bar margin, in logical pixels.
const MENUBAR_MARGIN_MAX: u16 = 64;

/// Default size of a dock tile, in logical pixels.
const DEFAULT_DOCK_ICON_SIZE: u16 = 48;

/// Smallest accepted dock tile size, in logical pixels.
const DOCK_ICON_SIZE_MIN: u16 = 8;

/// Largest accepted dock tile size, in logical pixels.
const DOCK_ICON_SIZE_MAX: u16 = 128;

/// Default distance between the dock and the bottom edge, in logical pixels.
const DEFAULT_DOCK_MARGIN: u16 = 8;

/// Smallest accepted dock margin, in logical pixels.
const DOCK_MARGIN_MIN: u16 = 0;

/// Largest accepted dock margin, in logical pixels.
const DOCK_MARGIN_MAX: u16 = 64;

/// Default number of launcher results shown at once.
const DEFAULT_LAUNCHER_MAX_RESULTS: u8 = 8;

/// Smallest accepted number of launcher results.
const LAUNCHER_MAX_RESULTS_MIN: u8 = 1;

/// Largest accepted number of launcher results.
const LAUNCHER_MAX_RESULTS_MAX: u8 = 64;

/// Default size of a launcher icon, in logical pixels.
const DEFAULT_LAUNCHER_ICON_SIZE: u16 = 48;

/// Smallest accepted launcher icon size, in logical pixels.
const LAUNCHER_ICON_SIZE_MIN: u16 = 8;

/// Largest accepted launcher icon size, in logical pixels.
const LAUNCHER_ICON_SIZE_MAX: u16 = 256;

/// Default limit on the length of a launcher query, in characters.
const DEFAULT_LAUNCHER_MAX_QUERY_CHARS: u16 = 128;

/// Smallest accepted launcher query limit, in characters.
const LAUNCHER_MAX_QUERY_CHARS_MIN: u16 = 1;

/// Largest accepted launcher query limit, in characters.
const LAUNCHER_MAX_QUERY_CHARS_MAX: u16 = 512;

/// The directives `panel.clock_format` accepts — and the only ones.
///
/// The panel renders this subset itself, so a format the panel cannot honour is
/// a parse error rather than literal text on the panel. `%` is the escape
/// character, and `%%` is a literal percent sign.
pub const CLOCK_DIRECTIVES: [char; 15] = [
    'H', 'M', 'S', 'I', 'p', 'Y', 'm', 'd', 'a', 'A', 'b', 'B', 'e', 'Z', '%',
];

/// Everything `shell.toml` describes.
///
/// The default of a section is the default of each of its fields, so this type
/// derives it: an empty `shell.toml` is the same configuration as no
/// `shell.toml` at all.
#[derive(Debug, Clone, PartialEq, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct ShellConfig {
    /// The panel: a strip on one edge of every output.
    pub panel: PanelConfig,
    /// The menu bar: the panel drawn as a full-width bar along the top edge.
    #[serde(default)]
    pub menubar: MenubarConfig,
    /// The dock: the panel drawn as a floating pane above the bottom edge.
    #[serde(default)]
    pub dock: DockConfig,
    /// The launcher: the application search overlay.
    pub launcher: LauncherConfig,
    /// The wallpaper: what is drawn below the windows.
    pub wallpaper: WallpaperConfig,
}

/// The panel, a strip along one edge of every output.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct PanelConfig {
    /// Which surface this panel instance draws: the taskbar, the menu bar or the
    /// dock. All three present the same tasks, tray items and clock.
    pub surface: PanelSurface,
    /// Edge the panel sits on.
    pub position: PanelPosition,
    /// Height of the panel, between 8 and 256 logical pixels.
    #[serde(deserialize_with = "validate_height")]
    pub height: u16,
    /// Distance between the panel and the edge it sits on, between 0 and 64
    /// logical pixels.
    #[serde(deserialize_with = "validate_margin")]
    pub margin: u16,
    /// Whether the panel reserves its strip: the work area the compositor
    /// hands to windows stops at the panel instead of running underneath it.
    pub exclusive: bool,
    /// Whether the panel hides itself until the pointer reaches its edge.
    pub autohide: bool,
    /// Whether the panel shows the clock.
    pub show_clock: bool,
    /// Format of the clock, written with the directives of
    /// [`CLOCK_DIRECTIVES`].
    #[serde(deserialize_with = "validate_clock_format")]
    pub clock_format: String,
    /// Number of task buttons the panel shows, between 1 and 64.
    #[serde(deserialize_with = "validate_max_task_buttons")]
    pub max_task_buttons: u16,
    /// Size of the panel's icons, between 8 and 128 logical pixels.
    #[serde(deserialize_with = "validate_icon_size")]
    pub icon_size: u16,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            surface: PanelSurface::Taskbar,
            position: PanelPosition::Bottom,
            height: DEFAULT_PANEL_HEIGHT,
            margin: DEFAULT_PANEL_MARGIN,
            exclusive: true,
            autohide: false,
            show_clock: true,
            clock_format: DEFAULT_CLOCK_FORMAT.to_owned(),
            max_task_buttons: DEFAULT_PANEL_MAX_TASK_BUTTONS,
            icon_size: DEFAULT_PANEL_ICON_SIZE,
        }
    }
}

/// Which surface a panel instance draws.
///
/// The three surfaces are three *presentations of the same data*: the tasks the
/// compositor reports, the status notifier items and the clock. Which one a
/// process draws is part of its configuration rather than something it can
/// change while it runs, because a different surface is a different window with
/// a different size and a different placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelSurface {
    /// A strip along one edge: the buttons the user clicks to switch windows.
    #[default]
    Taskbar,
    /// A full-width bar along the top edge: the focused application's name, the
    /// status notifier items and the clock.
    Menubar,
    /// A floating pane above the bottom edge: one tile per running application.
    Dock,
}

/// The menu bar: the panel drawn as a full-width bar along the top edge.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct MenubarConfig {
    /// Height of the bar, between 8 and 256 logical pixels.
    #[serde(deserialize_with = "validate_menubar_height")]
    pub height: u16,
    /// Distance between the bar and the top edge, between 0 and 64 logical
    /// pixels.
    #[serde(deserialize_with = "validate_menubar_margin")]
    pub margin: u16,
    /// Whether the bar reserves its strip: the work area stops below the bar
    /// instead of running underneath it.
    pub exclusive: bool,
    /// Whether the bar shows the focused application's name at its left end.
    ///
    /// Inox has no application-menu protocol, so the name is all the bar shows
    /// there: it never invents menu titles such as "File" or "Edit".
    pub show_app_name: bool,
    /// Whether the bar shows the clock.
    pub show_clock: bool,
    /// Format of the clock, written with the directives of
    /// [`CLOCK_DIRECTIVES`].
    #[serde(deserialize_with = "validate_menubar_clock_format")]
    pub clock_format: String,
    /// Whether the bar shows the status notifier items.
    pub show_tray: bool,
}

impl Default for MenubarConfig {
    fn default() -> Self {
        Self {
            height: DEFAULT_MENUBAR_HEIGHT,
            margin: DEFAULT_MENUBAR_MARGIN,
            exclusive: true,
            show_app_name: true,
            show_clock: true,
            clock_format: DEFAULT_CLOCK_FORMAT.to_owned(),
            show_tray: true,
        }
    }
}

/// The dock: the panel drawn as a floating pane above the bottom edge.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct DockConfig {
    /// Size of one tile, between 8 and 128 logical pixels.
    #[serde(deserialize_with = "validate_dock_icon_size")]
    pub icon_size: u16,
    /// Distance between the pane and the bottom edge, between 0 and 64 logical
    /// pixels.
    #[serde(deserialize_with = "validate_dock_margin")]
    pub margin: u16,
    /// Whether the dock reserves its strip. A dock floats *over* windows, so
    /// this is off by default.
    pub exclusive: bool,
    /// Whether the dock hides itself until the pointer reaches the bottom edge.
    pub autohide: bool,
    /// Whether the dock shows the status notifier items beside its tiles.
    pub show_tray: bool,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            icon_size: DEFAULT_DOCK_ICON_SIZE,
            margin: DEFAULT_DOCK_MARGIN,
            exclusive: false,
            autohide: false,
            show_tray: true,
        }
    }
}

/// The edge a [`PanelConfig`] sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelPosition {
    /// Along the top edge.
    Top,
    /// Along the bottom edge.
    #[default]
    Bottom,
}

/// The launcher: the application search overlay.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct LauncherConfig {
    /// Number of results shown at once, between 1 and 64.
    #[serde(deserialize_with = "validate_max_results")]
    pub max_results: u8,
    /// Size of a result icon, between 8 and 256 logical pixels.
    #[serde(deserialize_with = "validate_launcher_icon_size")]
    pub icon_size: u16,
    /// argv prefix that launches a program asking for a terminal, for example
    /// `["alacritty", "-e"]`.
    ///
    /// Deliberately a list rather than a shell command: an entry with
    /// `Terminal=true` is launched by appending its own argv to this prefix, so
    /// nothing anywhere has to parse or quote a string as a shell would.
    pub terminal: Vec<String>,
    /// Whether entries hidden by `Hidden=true` or `NoDisplay=true` are listed
    /// as well.
    pub include_hidden: bool,
    /// Longest accepted query, between 1 and 512 characters. Longer input is
    /// bounded rather than searched in full, so a paste cannot make the search
    /// walk an unbounded string.
    #[serde(deserialize_with = "validate_max_query_chars")]
    pub max_query_chars: u16,
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            max_results: DEFAULT_LAUNCHER_MAX_RESULTS,
            icon_size: DEFAULT_LAUNCHER_ICON_SIZE,
            terminal: Vec::new(),
            include_hidden: false,
            max_query_chars: DEFAULT_LAUNCHER_MAX_QUERY_CHARS,
        }
    }
}

/// The wallpaper: what is drawn below the windows.
///
/// Every field's default is its type's default — [`WallpaperMode::Fill`], no
/// image and no per-output entry — so this type derives [`Default`].
#[derive(Debug, Clone, PartialEq, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct WallpaperConfig {
    /// How [`WallpaperConfig::image`] is fitted to an output.
    pub mode: WallpaperMode,
    /// Image to draw. `None` — the default — leaves the output to the theme
    /// background.
    pub image: Option<PathBuf>,
    /// Image per output name, for example `HDMI-A-1 = "wide.png"`; the key `*`
    /// applies to every output that is not named. An output with no entry here
    /// falls back to [`WallpaperConfig::image`].
    pub per_output: BTreeMap<String, PathBuf>,
}

/// How a wallpaper image is fitted to an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperMode {
    /// Scale the image to cover the output, cropping the overflow.
    #[default]
    Fill,
    /// Scale the image so it is fully visible, leaving the theme background
    /// around it.
    Fit,
    /// Scale the image to the output's aspect ratio, distorting it.
    Stretch,
    /// Draw the image at its own size, centred.
    Center,
    /// Repeat the image from the top-left corner.
    Tile,
}

/// Reject a panel height outside its accepted range.
fn validate_height<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let height = u16::deserialize(deserializer)?;

    if !(PANEL_HEIGHT_MIN..=PANEL_HEIGHT_MAX).contains(&height) {
        return Err(serde::de::Error::custom(format!(
            "panel.height must be between {PANEL_HEIGHT_MIN} and {PANEL_HEIGHT_MAX} logical pixels, found {height}"
        )));
    }

    Ok(height)
}

/// Reject a panel margin outside its accepted range.
fn validate_margin<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let margin = u16::deserialize(deserializer)?;

    if !(PANEL_MARGIN_MIN..=PANEL_MARGIN_MAX).contains(&margin) {
        return Err(serde::de::Error::custom(format!(
            "panel.margin must be between {PANEL_MARGIN_MIN} and {PANEL_MARGIN_MAX} logical pixels, found {margin}"
        )));
    }

    Ok(margin)
}

/// Reject a task button count outside its accepted range.
fn validate_max_task_buttons<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let buttons = u16::deserialize(deserializer)?;

    if !(PANEL_MAX_TASK_BUTTONS_MIN..=PANEL_MAX_TASK_BUTTONS_MAX).contains(&buttons) {
        return Err(serde::de::Error::custom(format!(
            "panel.max_task_buttons must be between {PANEL_MAX_TASK_BUTTONS_MIN} and {PANEL_MAX_TASK_BUTTONS_MAX} buttons, found {buttons}"
        )));
    }

    Ok(buttons)
}

/// Reject a panel icon size outside its accepted range.
fn validate_icon_size<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let size = u16::deserialize(deserializer)?;

    if !(PANEL_ICON_SIZE_MIN..=PANEL_ICON_SIZE_MAX).contains(&size) {
        return Err(serde::de::Error::custom(format!(
            "panel.icon_size must be between {PANEL_ICON_SIZE_MIN} and {PANEL_ICON_SIZE_MAX} logical pixels, found {size}"
        )));
    }

    Ok(size)
}

/// Reject a menu bar height outside its accepted range.
fn validate_menubar_height<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let height = u16::deserialize(deserializer)?;

    if !(MENUBAR_HEIGHT_MIN..=MENUBAR_HEIGHT_MAX).contains(&height) {
        return Err(serde::de::Error::custom(format!(
            "menubar.height must be between {MENUBAR_HEIGHT_MIN} and {MENUBAR_HEIGHT_MAX} logical pixels, found {height}"
        )));
    }

    Ok(height)
}

/// Reject a menu bar margin outside its accepted range.
fn validate_menubar_margin<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let margin = u16::deserialize(deserializer)?;

    if !(MENUBAR_MARGIN_MIN..=MENUBAR_MARGIN_MAX).contains(&margin) {
        return Err(serde::de::Error::custom(format!(
            "menubar.margin must be between {MENUBAR_MARGIN_MIN} and {MENUBAR_MARGIN_MAX} logical pixels, found {margin}"
        )));
    }

    Ok(margin)
}

/// Reject a dock tile size outside its accepted range.
fn validate_dock_icon_size<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let size = u16::deserialize(deserializer)?;

    if !(DOCK_ICON_SIZE_MIN..=DOCK_ICON_SIZE_MAX).contains(&size) {
        return Err(serde::de::Error::custom(format!(
            "dock.icon_size must be between {DOCK_ICON_SIZE_MIN} and {DOCK_ICON_SIZE_MAX} logical pixels, found {size}"
        )));
    }

    Ok(size)
}

/// Reject a dock margin outside its accepted range.
fn validate_dock_margin<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let margin = u16::deserialize(deserializer)?;

    if !(DOCK_MARGIN_MIN..=DOCK_MARGIN_MAX).contains(&margin) {
        return Err(serde::de::Error::custom(format!(
            "dock.margin must be between {DOCK_MARGIN_MIN} and {DOCK_MARGIN_MAX} logical pixels, found {margin}"
        )));
    }

    Ok(margin)
}

/// Reject a result count outside its accepted range.
fn validate_max_results<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let results = u8::deserialize(deserializer)?;

    if !(LAUNCHER_MAX_RESULTS_MIN..=LAUNCHER_MAX_RESULTS_MAX).contains(&results) {
        return Err(serde::de::Error::custom(format!(
            "launcher.max_results must be between {LAUNCHER_MAX_RESULTS_MIN} and {LAUNCHER_MAX_RESULTS_MAX} results, found {results}"
        )));
    }

    Ok(results)
}

/// Reject a launcher icon size outside its accepted range.
fn validate_launcher_icon_size<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let size = u16::deserialize(deserializer)?;

    if !(LAUNCHER_ICON_SIZE_MIN..=LAUNCHER_ICON_SIZE_MAX).contains(&size) {
        return Err(serde::de::Error::custom(format!(
            "launcher.icon_size must be between {LAUNCHER_ICON_SIZE_MIN} and {LAUNCHER_ICON_SIZE_MAX} logical pixels, found {size}"
        )));
    }

    Ok(size)
}

/// Reject a query limit outside its accepted range.
fn validate_max_query_chars<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: Deserializer<'de>,
{
    let characters = u16::deserialize(deserializer)?;

    if !(LAUNCHER_MAX_QUERY_CHARS_MIN..=LAUNCHER_MAX_QUERY_CHARS_MAX).contains(&characters) {
        return Err(serde::de::Error::custom(format!(
            "launcher.max_query_chars must be between {LAUNCHER_MAX_QUERY_CHARS_MIN} and {LAUNCHER_MAX_QUERY_CHARS_MAX} characters, found {characters}"
        )));
    }

    Ok(characters)
}

/// Reject a clock format that uses a directive the panel cannot render.
fn validate_clock_format<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let format = String::deserialize(deserializer)?;

    check_clock_format(&format, "panel.clock_format").map_err(serde::de::Error::custom)?;

    Ok(format)
}

/// Reject a menu bar clock format that uses a directive the panel cannot render.
fn validate_menubar_clock_format<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let format = String::deserialize(deserializer)?;

    check_clock_format(&format, "menubar.clock_format").map_err(serde::de::Error::custom)?;

    Ok(format)
}

/// Check `format` against [`CLOCK_DIRECTIVES`].
///
/// `%` introduces a directive and `%%` is a literal percent sign; every other
/// `%x`, and a `%` that ends the format, is rejected with the offending
/// directive in the message. `field` is the dotted field the format came from,
/// so the message names `menubar.clock_format` and not `panel.clock_format`
/// when it was the menu bar that was misconfigured.
fn check_clock_format(format: &str, field: &str) -> Result<(), String> {
    let mut characters = format.chars();

    while let Some(character) = characters.next() {
        if character != '%' {
            continue;
        }

        let Some(directive) = characters.next() else {
            return Err(format!(
                "{field} must not end with a lone `%`; supported directives are {}",
                supported_directives()
            ));
        };

        if !CLOCK_DIRECTIVES.contains(&directive) {
            return Err(format!(
                "`%{directive}` is not a supported clock directive; supported directives are {}",
                supported_directives()
            ));
        }
    }

    Ok(())
}

/// The supported clock directives as `%H, %M, ...`, for an error message.
fn supported_directives() -> String {
    CLOCK_DIRECTIVES
        .iter()
        .map(|directive| format!("%{directive}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ConfigError;
    use crate::error::test_support::{column_of, line_of};
    use std::path::Path;

    const SHELL_PATH: &str = "/cfg/inox/shell.toml";

    fn parse_shell(text: &str) -> Result<ShellConfig, ConfigError> {
        crate::error::parse(Path::new(SHELL_PATH), text)
    }

    #[test]
    fn an_empty_document_uses_every_default() {
        let config = parse_shell("").expect("empty document");

        assert_eq!(config.panel.surface, PanelSurface::Taskbar);
        assert_eq!(config.panel.position, PanelPosition::Bottom);
        assert_eq!(config.panel.height, 32);
        assert_eq!(config.panel.margin, 0);
        assert!(config.panel.exclusive);
        assert!(!config.panel.autohide);
        assert!(config.panel.show_clock);
        assert_eq!(config.panel.clock_format, "%H:%M");
        assert_eq!(config.panel.max_task_buttons, 12);
        assert_eq!(config.panel.icon_size, 16);
        assert_eq!(config.menubar.height, 28);
        assert_eq!(config.menubar.margin, 0);
        assert!(config.menubar.exclusive);
        assert!(config.menubar.show_app_name);
        assert!(config.menubar.show_clock);
        assert_eq!(config.menubar.clock_format, "%H:%M");
        assert!(config.menubar.show_tray);
        assert_eq!(config.dock.icon_size, 48);
        assert_eq!(config.dock.margin, 8);
        assert!(!config.dock.exclusive);
        assert!(!config.dock.autohide);
        assert!(config.dock.show_tray);
        assert_eq!(config.launcher.max_results, 8);
        assert_eq!(config.launcher.icon_size, 48);
        assert!(config.launcher.terminal.is_empty());
        assert!(!config.launcher.include_hidden);
        assert_eq!(config.launcher.max_query_chars, 128);
        assert_eq!(config.wallpaper.mode, WallpaperMode::Fill);
        assert_eq!(config.wallpaper.image, None);
        assert!(config.wallpaper.per_output.is_empty());
    }

    #[test]
    fn the_defaults_of_the_three_sections_are_the_ones_this_schema_declares() {
        assert_eq!(
            ShellConfig::default(),
            parse_shell("").expect("empty document")
        );
        assert_eq!(PanelConfig::default().clock_format, DEFAULT_CLOCK_FORMAT);
        assert_eq!(MenubarConfig::default(), ShellConfig::default().menubar);
        assert_eq!(DockConfig::default(), ShellConfig::default().dock);
        assert_eq!(
            MenubarConfig::default().clock_format,
            DEFAULT_CLOCK_FORMAT,
            "the menu bar reuses the one documented clock default"
        );
        assert_eq!(LauncherConfig::default(), ShellConfig::default().launcher);
        assert_eq!(WallpaperConfig::default(), ShellConfig::default().wallpaper);
    }

    #[test]
    fn the_menu_bar_and_the_dock_are_parsed() {
        let config = parse_shell(
            "[menubar]\n\
             height = 40\n\
             margin = 4\n\
             exclusive = false\n\
             show_app_name = false\n\
             show_clock = false\n\
             clock_format = \"%Y-%m-%d %H:%M\"\n\
             show_tray = false\n\
             \n\
             [dock]\n\
             icon_size = 64\n\
             margin = 16\n\
             exclusive = true\n\
             autohide = true\n\
             show_tray = false\n",
        )
        .expect("document");

        assert_eq!(config.menubar.height, 40);
        assert_eq!(config.menubar.margin, 4);
        assert!(!config.menubar.exclusive);
        assert!(!config.menubar.show_app_name);
        assert!(!config.menubar.show_clock);
        assert_eq!(config.menubar.clock_format, "%Y-%m-%d %H:%M");
        assert!(!config.menubar.show_tray);
        assert_eq!(config.dock.icon_size, 64);
        assert_eq!(config.dock.margin, 16);
        assert!(config.dock.exclusive);
        assert!(config.dock.autohide);
        assert!(!config.dock.show_tray);
    }

    #[test]
    fn a_partial_menubar_and_dock_keep_the_defaults_of_the_fields_they_leave_out() {
        let config =
            parse_shell("[menubar]\nheight = 36\n\n[dock]\nicon_size = 32\n").expect("document");

        assert_eq!(config.menubar.height, 36);
        assert_eq!(
            config.menubar,
            MenubarConfig {
                height: 36,
                ..MenubarConfig::default()
            }
        );
        assert_eq!(
            config.dock,
            DockConfig {
                icon_size: 32,
                ..DockConfig::default()
            }
        );
    }

    #[test]
    fn every_panel_surface_spelling_is_parsed() {
        for (text, expected) in [
            ("taskbar", PanelSurface::Taskbar),
            ("menubar", PanelSurface::Menubar),
            ("dock", PanelSurface::Dock),
        ] {
            let config =
                parse_shell(&format!("[panel]\nsurface = \"{text}\"\n")).expect("document");

            assert_eq!(config.panel.surface, expected);
        }
    }

    #[test]
    fn a_bare_panel_surface_line_parses_and_leaves_the_rest_at_their_defaults() {
        let config = parse_shell("[panel]\nsurface = \"dock\"\n").expect("document");

        assert_eq!(config.panel.surface, PanelSurface::Dock);
        assert_eq!(
            config,
            ShellConfig {
                panel: PanelConfig {
                    surface: PanelSurface::Dock,
                    ..PanelConfig::default()
                },
                ..ShellConfig::default()
            }
        );
    }

    #[test]
    fn a_misspelled_panel_surface_is_rejected() {
        let error = parse_shell("[panel]\nsurface = \"task\"\n").expect_err("unknown variant");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("panel.surface"));
                assert_eq!(line, Some(2));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_keys_in_the_new_sections_are_rejected_with_their_field_paths() {
        for (document, field) in [
            ("[menubar]\nheigth = 40\n", "menubar.heigth"),
            ("[dock]\nicon = 48\n", "dock.icon"),
        ] {
            let error = parse_shell(document).expect_err("unknown key");

            match error {
                ConfigError::Parse {
                    field: reported,
                    line,
                    ..
                } => {
                    assert_eq!(reported.as_deref(), Some(field), "document `{document:?}`");
                    assert_eq!(line, Some(2));
                }
                other => panic!("expected a parse error, got {other:?}"),
            }
        }
    }

    #[test]
    fn panel_launcher_and_wallpaper_are_parsed() {
        let config = parse_shell(
            "[panel]\n\
             surface = \"menubar\"\n\
             position = \"top\"\n\
             height = 40\n\
             margin = 4\n\
             exclusive = false\n\
             autohide = true\n\
             show_clock = false\n\
             clock_format = \"%Y-%m-%d %H:%M\"\n\
             max_task_buttons = 24\n\
             icon_size = 24\n\
             \n\
             [launcher]\n\
             max_results = 12\n\
             icon_size = 64\n\
             terminal = [\"alacritty\", \"-e\"]\n\
             include_hidden = true\n\
             max_query_chars = 64\n\
             \n\
             [wallpaper]\n\
             mode = \"tile\"\n\
             image = \"/usr/share/wallpapers/grain.png\"\n\
             \n\
             [wallpaper.per_output]\n\
             \"*\" = \"fallback.png\"\n\
             \"HDMI-A-1\" = \"wide.png\"\n",
        )
        .expect("document");

        assert_eq!(config.panel.surface, PanelSurface::Menubar);
        assert_eq!(config.panel.position, PanelPosition::Top);
        assert_eq!(config.panel.height, 40);
        assert_eq!(config.panel.margin, 4);
        assert!(!config.panel.exclusive);
        assert!(config.panel.autohide);
        assert!(!config.panel.show_clock);
        assert_eq!(config.panel.clock_format, "%Y-%m-%d %H:%M");
        assert_eq!(config.panel.max_task_buttons, 24);
        assert_eq!(config.panel.icon_size, 24);
        assert_eq!(config.launcher.max_results, 12);
        assert_eq!(config.launcher.icon_size, 64);
        assert_eq!(config.launcher.terminal, ["alacritty", "-e"]);
        assert!(config.launcher.include_hidden);
        assert_eq!(config.launcher.max_query_chars, 64);
        assert_eq!(config.wallpaper.mode, WallpaperMode::Tile);
        assert_eq!(
            config.wallpaper.image,
            Some(PathBuf::from("/usr/share/wallpapers/grain.png"))
        );
        assert_eq!(
            config.wallpaper.per_output.get("HDMI-A-1"),
            Some(&PathBuf::from("wide.png"))
        );
        assert_eq!(
            config.wallpaper.per_output.get("*"),
            Some(&PathBuf::from("fallback.png"))
        );
        assert_eq!(config.wallpaper.per_output.len(), 2);
    }

    #[test]
    fn a_partial_section_keeps_the_defaults_of_the_fields_it_leaves_out() {
        let config = parse_shell("[panel]\nheight = 48\n").expect("document");

        assert_eq!(config.panel.height, 48);
        assert_eq!(config.panel.position, PanelPosition::Bottom);
        assert_eq!(config.panel.margin, 0);
        assert!(config.panel.exclusive);
        assert!(!config.panel.autohide);
        assert!(config.panel.show_clock);
        assert_eq!(config.panel.clock_format, "%H:%M");
        assert_eq!(config.panel.max_task_buttons, 12);
        assert_eq!(config.panel.icon_size, 16);
        assert_eq!(config.launcher, LauncherConfig::default());
        assert_eq!(config.wallpaper, WallpaperConfig::default());
    }

    #[test]
    fn an_empty_section_is_a_section_of_defaults() {
        // Every section is `#[serde(default)]` as well, so naming one without
        // any key in it is not an error.
        let config = parse_shell("[panel]\n\n[launcher]\n\n[wallpaper]\n").expect("document");

        assert_eq!(config, ShellConfig::default());
    }

    #[test]
    fn every_panel_position_spelling_is_parsed() {
        for (text, expected) in [
            ("top", PanelPosition::Top),
            ("bottom", PanelPosition::Bottom),
        ] {
            let config =
                parse_shell(&format!("[panel]\nposition = \"{text}\"\n")).expect("document");

            assert_eq!(config.panel.position, expected);
        }
    }

    #[test]
    fn a_misspelled_panel_position_is_rejected() {
        let error = parse_shell("[panel]\nposition = \"left\"\n").expect_err("unknown variant");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("panel.position"));
                assert_eq!(line, Some(2));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn every_wallpaper_mode_spelling_is_parsed() {
        for (text, expected) in [
            ("fill", WallpaperMode::Fill),
            ("fit", WallpaperMode::Fit),
            ("stretch", WallpaperMode::Stretch),
            ("center", WallpaperMode::Center),
            ("tile", WallpaperMode::Tile),
        ] {
            let config =
                parse_shell(&format!("[wallpaper]\nmode = \"{text}\"\n")).expect("document");

            assert_eq!(config.wallpaper.mode, expected);
        }
    }

    #[test]
    fn a_misspelled_wallpaper_mode_is_rejected() {
        let error = parse_shell("[wallpaper]\nmode = \"cover\"\n").expect_err("unknown variant");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("wallpaper.mode"));
                assert_eq!(line, Some(2));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn out_of_range_values_are_rejected_while_parsing() {
        // Each row is a document whose second line is out of range; the reason
        // is what the rendered error has to say about the accepted range.
        for (document, field, needle, reason) in [
            (
                "[panel]\nheight = 4\n",
                "panel.height",
                "4",
                "between 8 and 256",
            ),
            (
                "[panel]\nheight = 257\n",
                "panel.height",
                "257",
                "between 8 and 256",
            ),
            (
                "[panel]\nmargin = 65\n",
                "panel.margin",
                "65",
                "between 0 and 64",
            ),
            (
                "[panel]\nmax_task_buttons = 0\n",
                "panel.max_task_buttons",
                "0",
                "between 1 and 64",
            ),
            (
                "[panel]\nicon_size = 4\n",
                "panel.icon_size",
                "4",
                "between 8 and 128",
            ),
            (
                "[launcher]\nmax_results = 65\n",
                "launcher.max_results",
                "65",
                "between 1 and 64",
            ),
            (
                "[launcher]\nicon_size = 257\n",
                "launcher.icon_size",
                "257",
                "between 8 and 256",
            ),
            (
                "[launcher]\nmax_query_chars = 0\n",
                "launcher.max_query_chars",
                "0",
                "between 1 and 512",
            ),
            (
                "[menubar]\nheight = 4\n",
                "menubar.height",
                "4",
                "between 8 and 256",
            ),
            (
                "[menubar]\nheight = 257\n",
                "menubar.height",
                "257",
                "between 8 and 256",
            ),
            (
                "[menubar]\nmargin = 65\n",
                "menubar.margin",
                "65",
                "between 0 and 64",
            ),
            (
                "[dock]\nicon_size = 4\n",
                "dock.icon_size",
                "4",
                "between 8 and 128",
            ),
            (
                "[dock]\nicon_size = 129\n",
                "dock.icon_size",
                "129",
                "between 8 and 128",
            ),
            (
                "[dock]\nmargin = 65\n",
                "dock.margin",
                "65",
                "between 0 and 64",
            ),
        ] {
            let outcome = parse_shell(document);
            assert!(
                outcome.is_err(),
                "expected `{document:?}` to be rejected, got {outcome:?}"
            );

            let error = outcome.expect_err("checked above");
            let expected_line = line_of(document, needle);
            let rendered = error.to_string();

            assert!(
                rendered.contains(&format!("shell.toml:{expected_line}:")),
                "rendered `{rendered}`"
            );
            assert!(
                rendered.contains(&format!("(field `{field}`)")),
                "rendered `{rendered}`"
            );
            assert!(rendered.contains(reason), "rendered `{rendered}`");

            match error {
                ConfigError::Parse {
                    field: reported,
                    line,
                    column,
                    ..
                } => {
                    assert_eq!(reported.as_deref(), Some(field));
                    assert_eq!(line, Some(expected_line));
                    assert_eq!(column, Some(column_of(document, needle)));
                }
                other => panic!("expected a parse error, got {other:?}"),
            }
        }
    }

    #[test]
    fn values_at_the_edge_of_their_range_are_accepted() {
        let lowest = parse_shell(
            "[panel]\n\
             height = 8\n\
             margin = 0\n\
             max_task_buttons = 1\n\
             icon_size = 8\n\
             \n\
             [menubar]\n\
             height = 8\n\
             margin = 0\n\
             \n\
             [dock]\n\
             icon_size = 8\n\
             margin = 0\n\
             \n\
             [launcher]\n\
             max_results = 1\n\
             icon_size = 8\n\
             max_query_chars = 1\n",
        )
        .expect("values at the lower edge of their ranges");

        assert_eq!(
            (
                lowest.panel.height,
                lowest.panel.margin,
                lowest.panel.max_task_buttons,
                lowest.panel.icon_size
            ),
            (8, 0, 1, 8)
        );
        assert_eq!((lowest.menubar.height, lowest.menubar.margin), (8, 0));
        assert_eq!((lowest.dock.icon_size, lowest.dock.margin), (8, 0));
        assert_eq!(
            (
                lowest.launcher.max_results,
                lowest.launcher.icon_size,
                lowest.launcher.max_query_chars
            ),
            (1, 8, 1)
        );

        let highest = parse_shell(
            "[panel]\n\
             height = 256\n\
             margin = 64\n\
             max_task_buttons = 64\n\
             icon_size = 128\n\
             \n\
             [menubar]\n\
             height = 256\n\
             margin = 64\n\
             \n\
             [dock]\n\
             icon_size = 128\n\
             margin = 64\n\
             \n\
             [launcher]\n\
             max_results = 64\n\
             icon_size = 256\n\
             max_query_chars = 512\n",
        )
        .expect("values at the upper edge of their ranges");

        assert_eq!(
            (
                highest.panel.height,
                highest.panel.margin,
                highest.panel.max_task_buttons,
                highest.panel.icon_size
            ),
            (256, 64, 64, 128)
        );
        assert_eq!((highest.menubar.height, highest.menubar.margin), (256, 64));
        assert_eq!((highest.dock.icon_size, highest.dock.margin), (128, 64));
        assert_eq!(
            (
                highest.launcher.max_results,
                highest.launcher.icon_size,
                highest.launcher.max_query_chars
            ),
            (64, 256, 512)
        );
    }

    #[test]
    fn every_default_survives_the_range_checks() {
        for document in [
            "",
            "[panel]\n",
            "[menubar]\n",
            "[dock]\n",
            "[launcher]\n",
            "[wallpaper]\n",
        ] {
            let config = parse_shell(document).expect("defaults");

            assert_eq!(config, ShellConfig::default(), "document `{document:?}`");
        }
    }

    #[test]
    fn the_menu_bar_clock_format_is_checked_like_the_panels() {
        for directive in CLOCK_DIRECTIVES {
            let config = parse_shell(&format!("[menubar]\nclock_format = \"%{directive}\"\n"))
                .unwrap_or_else(|error| panic!("`%{directive}` was rejected: {error}"));

            assert_eq!(config.menubar.clock_format, format!("%{directive}"));
        }

        let document = "[menubar]\nclock_format = \"%Q\"\n";
        let error = parse_shell(document).expect_err("`%Q` is not a directive the panel renders");
        let rendered = error.to_string();

        assert!(
            rendered.contains("(field `menubar.clock_format`)"),
            "rendered `{rendered}`"
        );
        assert!(rendered.contains("%Q"), "rendered `{rendered}`");

        let document = "[menubar]\nclock_format = \"%H:%M %\"\n";
        let error = parse_shell(document).expect_err("a format cannot end with a lone `%`");
        let rendered = error.to_string();

        assert!(
            rendered.contains("menubar.clock_format"),
            "the message names the field it came from: `{rendered}`"
        );
        assert!(rendered.contains("lone `%`"), "rendered `{rendered}`");
        assert!(
            !rendered.contains("panel.clock_format"),
            "the menu bar's error must not name the panel's field: `{rendered}`"
        );
    }

    #[test]
    fn every_supported_clock_directive_is_accepted() {
        for directive in CLOCK_DIRECTIVES {
            let config = parse_shell(&format!("[panel]\nclock_format = \"%{directive}\"\n"))
                .unwrap_or_else(|error| panic!("`%{directive}` was rejected: {error}"));

            assert_eq!(config.panel.clock_format, format!("%{directive}"));
        }

        // A directive behind literal text, and a literal percent sign, which is
        // the escape character rather than a directive of its own.
        let config = parse_shell("[panel]\nclock_format = \"100%% %Y-%m-%d %H:%M:%S\"\n")
            .expect("a format mixing literals with directives");

        assert_eq!(config.panel.clock_format, "100%% %Y-%m-%d %H:%M:%S");
    }

    #[test]
    fn the_supported_directives_are_exactly_the_documented_ones() {
        assert_eq!(CLOCK_DIRECTIVES.len(), 15);
        assert_eq!(
            CLOCK_DIRECTIVES,
            [
                'H', 'M', 'S', 'I', 'p', 'Y', 'm', 'd', 'a', 'A', 'b', 'B', 'e', 'Z', '%'
            ]
        );
    }

    #[test]
    fn an_unsupported_clock_directive_is_rejected() {
        let document = "[panel]\nclock_format = \"%Q\"\n";
        let error = parse_shell(document).expect_err("`%Q` is not a directive the panel renders");

        let rendered = error.to_string();
        assert!(
            rendered.contains(&format!("shell.toml:{}:", line_of(document, "%Q"))),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains("(field `panel.clock_format`)"),
            "rendered `{rendered}`"
        );
        assert!(rendered.contains("%Q"), "rendered `{rendered}`");
        assert!(rendered.contains("%H"), "rendered `{rendered}`");

        match error {
            ConfigError::Parse { field, message, .. } => {
                assert_eq!(field.as_deref(), Some("panel.clock_format"));
                assert!(message.contains("%Q"), "message `{message}`");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn an_unsupported_directive_inside_otherwise_valid_text_is_rejected() {
        let document = "[panel]\nclock_format = \"%Y-%m-%d %Q\"\n";
        let error = parse_shell(document).expect_err("`%Q` is not a directive the panel renders");

        let rendered = error.to_string();
        assert!(rendered.contains("%Q"), "rendered `{rendered}`");
        assert!(
            rendered.contains("(field `panel.clock_format`)"),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains(&format!(
                "shell.toml:{}:",
                line_of(document, "clock_format")
            )),
            "rendered `{rendered}`"
        );
    }

    #[test]
    fn a_trailing_percent_is_rejected() {
        let document = "[panel]\nclock_format = \"%H:%M %\"\n";
        let error = parse_shell(document).expect_err("a format cannot end with a lone `%`");

        let rendered = error.to_string();
        assert!(
            rendered.contains("(field `panel.clock_format`)"),
            "rendered `{rendered}`"
        );
        assert!(rendered.contains("lone `%`"), "rendered `{rendered}`");

        match error {
            ConfigError::Parse { line, .. } => {
                assert_eq!(line, Some(line_of(document, "clock_format")));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_clock_format_without_directives_is_accepted() {
        let config = parse_shell("[panel]\nclock_format = \"\"\n").expect("an empty format");
        assert_eq!(config.panel.clock_format, "");

        let config = parse_shell("[panel]\nclock_format = \"wall clock\"\n").expect("literal text");
        assert_eq!(config.panel.clock_format, "wall clock");
    }

    #[test]
    fn an_unknown_key_is_rejected_with_its_field_path() {
        let error = parse_shell("[panel]\nheight = 40\nnonsense = 1\n").expect_err("unknown key");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("panel.nonsense"));
                assert_eq!(line, Some(3));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_section_is_rejected() {
        // `[menubar]` and `[dock]` are sections of this schema now; anything
        // else is still a typo rather than a silently ignored table.
        let error = parse_shell("[taskbar]\nheight = 40\n").expect_err("unknown section");

        match error {
            ConfigError::Parse { field, line, .. } => {
                assert_eq!(field.as_deref(), Some("taskbar"));
                assert_eq!(line, Some(1));
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_value_of_the_wrong_type_names_the_file_the_line_and_the_field() {
        let document = "[launcher]\nterminal = \"alacritty -e\"\n";
        let error = parse_shell(document).expect_err("the terminal prefix is a list, not a string");

        let rendered = error.to_string();
        assert!(
            rendered.contains(&format!("shell.toml:{}:", line_of(document, "terminal"))),
            "rendered `{rendered}`"
        );
        assert!(
            rendered.contains("(field `launcher.terminal`)"),
            "rendered `{rendered}`"
        );
        assert!(rendered.contains("invalid type"), "rendered `{rendered}`");

        match error {
            ConfigError::Parse { field, message, .. } => {
                assert_eq!(field.as_deref(), Some("launcher.terminal"));
                assert!(message.contains("sequence"), "message `{message}`");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_wallpaper_image_may_be_absent_or_named() {
        let config = parse_shell("[wallpaper]\n").expect("document");
        assert_eq!(config.wallpaper.image, None);

        let config = parse_shell("[wallpaper]\nimage = \"wall.png\"\n").expect("document");
        assert_eq!(config.wallpaper.image, Some(PathBuf::from("wall.png")));
    }
}
