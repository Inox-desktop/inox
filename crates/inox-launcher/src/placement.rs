//! Where the launcher's surface belongs, as pure data.
//!
//! The Inox compositor does not advertise `zwlr_layer_shell_v1` yet, so this
//! policy cannot be *applied* by anything today — see `crate::main`, the crate
//! README and [ADR 0018]. It is written and tested anyway, because the policy is
//! the part with decisions in it (overlay layer, centred, exclusive keyboard
//! interactivity, a size that comes from configuration and tokens), and because
//! the day a layer-shell host exists it should consume this structure
//! unchanged rather than inherit a size someone guessed at a call site.
//!
//! Everything here is a plain value: [`LogicalSize`], counts and booleans. No
//! Qt type, no Wayland type, no I/O — which is what keeps it testable, and
//! what will let a future host translate it into `set_anchor`, `set_size`,
//! `set_exclusive_zone`, `set_keyboard_interactivity` and `set_margin` one
//! field at a time.
//!
//! ## Where the numbers come from
//!
//! There is no pixel size written in this file. The surface's size is built out
//! of the launcher's configured `icon_size` and `max_results` and the theme's
//! `spacing`, `radii` and `typography.scale` tokens (root AGENTS.md §4, PRD
//! §9.2); the only literals are dimensionless ratios and counts, and the output
//! size, which the host knows and this crate does not.

use inox_config::Theme;
use inox_config::shell::LauncherConfig;

/// A size in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogicalSize {
    /// Width in logical pixels.
    pub width: u32,
    /// Height in logical pixels.
    pub height: u32,
}

impl LogicalSize {
    /// A size of `width` by `height` logical pixels.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// The layer-shell layer a surface is placed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// Below every window.
    Background,
    /// Below the normal windows.
    Bottom,
    /// Above the normal windows.
    Top,
    /// Above every layer.
    Overlay,
}

/// The edges a surface is anchored to.
///
/// The launcher anchors to nothing in particular: with no anchor the compositor
/// centres the surface, which is what an overlay wants. The struct is here
/// because the host has to be told that, and "no edges" is a value rather than
/// an omission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Anchor {
    /// Anchor to the top edge.
    pub top: bool,
    /// Anchor to the bottom edge.
    pub bottom: bool,
    /// Anchor to the left edge.
    pub left: bool,
    /// Anchor to the right edge.
    pub right: bool,
}

impl Anchor {
    /// The anchor the launcher uses: none, so the surface is centred.
    pub const CENTRE: Self = Self {
        top: false,
        bottom: false,
        left: false,
        right: false,
    };

    /// Whether the surface is anchored to any edge.
    pub fn is_centred(&self) -> bool {
        !(self.top || self.bottom || self.left || self.right)
    }
}

/// Whether the surface wants the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardInteractivity {
    /// Never take the keyboard.
    None,
    /// Take the keyboard only after a click.
    OnDemand,
    /// Take the keyboard as soon as the surface is mapped.
    Exclusive,
}

/// Everything a layer-shell host needs to place the launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// The layer to put the surface in.
    pub layer: Layer,
    /// The edges to anchor it to.
    pub anchor: Anchor,
    /// Whether the surface takes the keyboard.
    pub keyboard_interactivity: KeyboardInteractivity,
    /// How many pixels the surface reserves at the screen edge.
    ///
    /// Zero: an overlay that reserved a strip would push every window aside
    /// while the user is looking for an application.
    pub exclusive_zone: i32,
    /// The distance between the surface and the anchored edge.
    pub margin: u32,
    /// The surface's size.
    pub size: LogicalSize,
    /// The output it belongs on, or `None` for the focused one.
    pub output: Option<String>,
    /// How many characters fit in a result row's label column.
    ///
    /// The view model has to truncate a name *without* knowing how wide anything
    /// is, so the estimate is made here, where the surface's width is known.
    /// It comes from the theme's own text size ([`crate::theme::average_advance`]),
    /// which means it follows `typography.scale`: at 200 % text a name has room
    /// for half as many characters, rather than overflowing the row.
    pub label_chars: usize,
}

/// The number of icon columns the overlay is at least as wide as.
const MINIMUM_ICON_COLUMNS: u32 = 6;

/// The fraction of the output's width the overlay takes.
const WIDTH_SHARE_NUMERATOR: u32 = 2;
/// The denominator of [`WIDTH_SHARE_NUMERATOR`].
const WIDTH_SHARE_DENOMINATOR: u32 = 5;

impl Placement {
    /// The launcher's placement on an output of `output` logical pixels.
    ///
    /// `output` is the size of the output the launcher was invoked on, which
    /// the host knows and this crate does not; `output_name` is that output's
    /// name, or `None` when the launcher should follow the compositor's focused
    /// output.
    ///
    /// The height covers the three groups the view draws — the search field, the
    /// result card with `max_results` rows and the status bar — and is clamped to
    /// the output, so that a small screen shows fewer rows rather than a surface
    /// taller than the screen.
    pub fn overlay(
        config: &LauncherConfig,
        theme: &Theme,
        output: LogicalSize,
        output_name: Option<&str>,
    ) -> Self {
        let metrics = crate::theme::Metrics::from_tokens(theme);
        let gutter = u32::from(theme.spacing.xl);
        let icon = u32::from(config.icon_size);

        let width = (output.width * WIDTH_SHARE_NUMERATOR / WIDTH_SHARE_DENOMINATOR)
            .max(gutter * 2 + icon * MINIMUM_ICON_COLUMNS);
        let width = width.min(output.width.max(1));

        let height = (chrome(metrics, theme, config).ceil() as u32).min(output.height.max(1));

        Self {
            layer: Layer::Overlay,
            anchor: Anchor::CENTRE,
            // The launcher is a place to type: it takes the keyboard as soon as
            // it appears, and gives it back when it closes.
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            exclusive_zone: 0,
            margin: gutter,
            size: LogicalSize::new(width, height),
            output: output_name.map(str::to_owned),
            label_chars: label_budget(width, icon, theme),
        }
    }

    /// The placement as a description, for logs and for a host that has to be
    /// told about it in words.
    pub fn describe(&self) -> String {
        format!(
            "{:?} layer, {}x{} logical pixels, {} margin, {}{}",
            self.layer,
            self.size.width,
            self.size.height,
            self.margin,
            match self.output {
                Some(ref name) => format!("on output {name}, "),
                None => "on the focused output, ".to_owned(),
            },
            match self.keyboard_interactivity {
                KeyboardInteractivity::Exclusive => "exclusive keyboard",
                KeyboardInteractivity::OnDemand => "keyboard on demand",
                KeyboardInteractivity::None => "no keyboard",
            }
        )
    }
}

/// How tall the launcher's content is, in logical pixels.
///
/// The three groups of `view.rs` in order — the search field, the result card
/// and the status bar — separated by `spacing.lg` inside a window padded with
/// `spacing.xl`. Every term is a token or the configured `icon_size`; the text
/// heights come from the theme's roles through `crate::theme::line_height`, so a
/// 200 % font scale grows the surface instead of clipping it.
fn chrome(metrics: crate::theme::Metrics, theme: &Theme, config: &LauncherConfig) -> f32 {
    let heading = crate::theme::line_height(theme, crate::theme::TextRole::Heading);
    let caption = crate::theme::line_height(theme, crate::theme::TextRole::Caption);
    let field = crate::theme::line_height(theme, crate::theme::TextRole::Title)
        + metrics.space_sm * 2.0
        + metrics.hairline * 2.0;

    // One row is as tall as the icon it shows; the label beside it is shorter.
    let row = (f32::from(config.icon_size)).max(metrics.space_xl) + metrics.space_sm * 2.0;
    let rows = f32::from(config.max_results);
    let gaps = (rows - 1.0).max(0.0) * metrics.space_sm;
    let list = metrics.space_lg * 2.0 + heading + metrics.space_md + rows * row + gaps;

    let status = metrics.space_lg * 2.0 + caption;

    metrics.space_xl * 2.0 + field + metrics.space_lg + list + metrics.space_lg + status
}

/// How many characters fit into a result row's label column.
///
/// The insets are the ones the view actually draws: the window's padding, the
/// result card's padding, the row's own control padding, the icon tile and the
/// gap after it. Keeping the estimate in step with the widget tree is what makes
/// "a truncated label is exactly as wide as the row's column" true rather than
/// hopeful. The character width is [`crate::theme::average_advance`], so the
/// estimate follows the font scale: at 200 % text a name has room for half as
/// many characters, rather than overflowing the row (acceptance Q02).
fn label_budget(width: u32, icon: u32, theme: &Theme) -> usize {
    let metrics = crate::theme::Metrics::from_tokens(theme);
    let chrome = metrics.space_xl * 2.0
        + metrics.space_lg * 2.0
        + metrics.space_md * 2.0
        + icon as f32
        + metrics.space_sm;
    let available = (width as f32 - chrome).max(0.0);

    // At least one character: a row with no room for even an ellipsis would be
    // a row the user cannot identify.
    ((available / crate::theme::average_advance(theme)) as usize).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::LauncherConfig;

    const OUTPUT: LogicalSize = LogicalSize::new(1920, 1080);

    fn theme() -> Theme {
        Theme::builtin_dark()
    }

    fn placement(config: &LauncherConfig) -> Placement {
        Placement::overlay(config, &theme(), OUTPUT, None)
    }

    #[test]
    fn the_overlay_is_centred_on_the_top_layer_with_the_keyboard() {
        let placement = placement(&LauncherConfig::default());
        assert_eq!(placement.layer, Layer::Overlay);
        assert!(placement.anchor.is_centred());
        assert_eq!(
            placement.keyboard_interactivity,
            KeyboardInteractivity::Exclusive
        );
        assert_eq!(placement.exclusive_zone, 0, "an overlay reserves no strip");
        assert_eq!(placement.output, None);
    }

    #[test]
    fn the_size_comes_from_the_configuration_and_the_theme() {
        let config = LauncherConfig::default();
        let theme = theme();
        let placement = placement(&config);

        assert!(placement.size.width > 0);
        assert!(placement.size.width <= OUTPUT.width);
        assert!(placement.size.height <= OUTPUT.height);

        // A larger icon makes every row taller, which makes the surface taller.
        let mut bigger = config.clone();
        bigger.icon_size = config.icon_size * 2;
        let taller = Placement::overlay(&bigger, &theme, OUTPUT, None);
        assert!(taller.size.height > placement.size.height);

        // A different output width gives a different surface width.
        let narrow = Placement::overlay(&config, &theme, LogicalSize::new(800, 600), None);
        assert!(narrow.size.width < placement.size.width);
    }

    #[test]
    fn the_surface_never_exceeds_a_small_output() {
        let config = LauncherConfig {
            max_results: 64,
            ..LauncherConfig::default()
        };
        let tiny = LogicalSize::new(320, 200);
        let placement = Placement::overlay(&config, &theme(), tiny, None);
        assert!(placement.size.width <= tiny.width);
        assert!(placement.size.height <= tiny.height);
    }

    #[test]
    fn the_size_is_never_zero_for_a_nonsensical_output() {
        let placement = Placement::overlay(
            &LauncherConfig::default(),
            &theme(),
            LogicalSize::new(0, 0),
            None,
        );
        assert!(placement.size.width >= 1);
        assert!(placement.size.height >= 1);
    }

    #[test]
    fn the_margin_is_a_theme_token() {
        let theme = theme();
        let placement = placement(&LauncherConfig::default());
        assert_eq!(placement.margin, u32::from(theme.spacing.xl));
    }

    #[test]
    fn the_label_budget_shrinks_when_the_font_scales_up() {
        let config = LauncherConfig::default();
        let theme = theme();
        let normal = Placement::overlay(&config, &theme, OUTPUT, None).label_chars;

        let mut scaled = theme.clone();
        scaled.typography.scale *= 2.0;
        let enlarged = Placement::overlay(&config, &scaled, OUTPUT, None).label_chars;

        assert!(
            enlarged < normal,
            "at 200 % text a name has room for fewer characters, not the same number",
        );
        assert!(enlarged >= 1);
    }

    #[test]
    fn the_output_name_is_carried_through_unchanged() {
        let placement = Placement::overlay(
            &LauncherConfig::default(),
            &theme(),
            OUTPUT,
            Some("HDMI-A-1"),
        );
        assert_eq!(placement.output.as_deref(), Some("HDMI-A-1"));
        assert!(placement.describe().contains("HDMI-A-1"));
    }

    #[test]
    fn the_description_names_the_layer_the_size_and_the_keyboard() {
        let described = placement(&LauncherConfig::default()).describe();
        assert!(described.contains("Overlay"), "{described}");
        assert!(described.contains("exclusive keyboard"), "{described}");
        assert!(described.contains('x'), "{described}");
    }
}
