//! Where the launcher's surface belongs, as pure data.
//!
//! The Inox compositor does not advertise `zwlr_layer_shell_v1` yet, so this
//! policy cannot be *applied* by anything today — see `crate::main`, the crate
//! README and [the L3 component architecture]. It is written and tested anyway, because the policy is
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
//! There is no pixel size written in this file. The surface asks for a *share* of
//! the output (four fifths of the width, nine tenths of the height), and every
//! band inside it — the top row, the chips, the card, the hint strip — is built
//! out of the launcher's configured `icon_size` and the theme's `spacing`,
//! `radii` and `typography.scale` tokens (root AGENTS.md §4, PRD §9.2). The
//! shares, the floors and the ratio between a tile and its label are the only
//! literals; the output size itself, the host knows and this crate does not.

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
///
/// `PartialEq` and not `Eq`: the grid the surface holds is measured in logical
/// pixels, and a distance is a float here like everywhere else in
/// [`crate::theme`].
#[derive(Debug, Clone, PartialEq)]
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
    /// The application grid the surface holds.
    ///
    /// The card's tiles are laid out by this value and nothing else: the
    /// projection publishes it, and the keyboard moves by `columns`, so a tile
    /// never moves by a number the window is not drawing.
    pub grid: crate::theme::Grid,
    /// How many characters fit in a tile's label.
    ///
    /// The view model has to truncate a name *without* knowing how wide anything
    /// is, so the estimate is made here, where the surface's width is known. It
    /// is the tile's width at the theme's own text size times the number of lines
    /// a label wraps to ([`crate::theme::TILE_LABEL_LINES`]): a tile grows with
    /// `typography.scale` rather than cutting the name shorter.
    pub label_chars: usize,
}

/// The number of icon columns the overlay is at least as wide as.
const MINIMUM_ICON_COLUMNS: u32 = 6;

/// The fewest whole grid rows the overlay keeps visible.
///
/// A launcher whose grid shows two rows is a menu; the reference shows a wall of
/// applications. Four is the floor the height is grown to when the output is
/// tall enough — and the output wins when it is not.
const MINIMUM_GRID_ROWS: f32 = 4.0;

/// The fraction of the output's width the overlay takes.
///
/// The surface is a place you look through, not a dialog: it asks for most of
/// the output so the grid can be scanned at a glance, and the ratio keeps the
/// margin an overlay rather than a full-screen takeover.
const WIDTH_SHARE_NUMERATOR: u32 = 4;
/// The denominator of [`WIDTH_SHARE_NUMERATOR`].
const WIDTH_SHARE_DENOMINATOR: u32 = 5;

/// The fraction of the output's height the overlay takes.
const HEIGHT_SHARE_NUMERATOR: u32 = 9;
/// The denominator of [`HEIGHT_SHARE_NUMERATOR`].
const HEIGHT_SHARE_DENOMINATOR: u32 = 10;

impl Placement {
    /// The launcher's placement on an output of `output` logical pixels.
    ///
    /// `output` is the size of the output the launcher was invoked on, which
    /// the host knows and this crate does not; `output_name` is that output's
    /// name, or `None` when the launcher should follow the compositor's focused
    /// output.
    ///
    /// The width is [`WIDTH_SHARE_NUMERATOR`] of the output, grown when that is
    /// too narrow for [`MINIMUM_ICON_COLUMNS`] tiles and never wider than the
    /// output. The height is [`HEIGHT_SHARE_NUMERATOR`] of the output rounded to
    /// whole grid rows — the card never ends with half a tile — grown to
    /// [`MINIMUM_GRID_ROWS`] rows, and never taller than the output either.
    ///
    /// `launcher.max_results` does not size the surface: it bounds a *search*.
    /// The browse is the whole index, and a grid that showed only as many tiles
    /// as a search may return would be a menu pretending to be a launcher.
    pub fn overlay(
        config: &LauncherConfig,
        theme: &Theme,
        output: LogicalSize,
        output_name: Option<&str>,
    ) -> Self {
        let metrics = crate::theme::Metrics::from_tokens(theme);
        let gutter = u32::from(theme.spacing.xl);

        // A tile is sized by the tokens alone; only the column count needs the
        // surface's width. Measuring the grid at width 0 is how the floor is
        // taken without guessing the cell size twice.
        let cell = crate::theme::grid(theme, config, 0);
        let minimum_width = gutter * 2
            + u32::from(theme.spacing.lg) * 2
            + ((cell.cell_width + cell.gap).ceil() as u32) * MINIMUM_ICON_COLUMNS;

        let width = (output.width * WIDTH_SHARE_NUMERATOR / WIDTH_SHARE_DENOMINATOR)
            .max(minimum_width)
            .min(output.width.max(1));

        let grid = crate::theme::grid(theme, config, width);

        let target = output.height * HEIGHT_SHARE_NUMERATOR / HEIGHT_SHARE_DENOMINATOR;
        let height =
            (rows_height(metrics, theme, &grid, target).ceil() as u32).min(output.height.max(1));

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
            grid,
            label_chars: label_budget(theme, &grid),
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

/// The surface height that holds whole grid rows at (or just above) `target`.
///
/// Every band except the card's tiles has one height, fixed by the tokens
/// ([`bands`]); the card gets a whole number of [`crate::theme::Grid`] rows.
/// Rounding *up* keeps the surface at least as tall as the share it was asked
/// for, and [`MINIMUM_GRID_ROWS`] is the floor the acceptance criteria name. A
/// card that ended mid-tile would show a cut row, which is the defect this exists
/// to prevent.
fn rows_height(
    metrics: crate::theme::Metrics,
    theme: &Theme,
    grid: &crate::theme::Grid,
    target: u32,
) -> f32 {
    let pitch = grid.cell_height + grid.gap;
    let bands = bands(metrics, theme);
    let fitting = ((target as f32 - bands) / pitch).ceil();
    let rows = fitting.max(MINIMUM_GRID_ROWS);

    bands + rows * pitch
}

/// Every band of `qml/Main.qml` except the card's tiles, in logical pixels.
///
/// The four bands in order — the top row (the surface title beside the search
/// field), the category chips, the card and the hint strip — separated by
/// `spacing.md` inside the window and `spacing.lg` around the card, on a window
/// padded with `spacing.xl`. Every term is a token and the text heights come from
/// the theme's roles through [`crate::theme::line_height`], so a 200 % font scale
/// grows the surface instead of clipping it.
fn bands(metrics: crate::theme::Metrics, theme: &Theme) -> f32 {
    let heading = crate::theme::line_height(theme, crate::theme::TextRole::Heading);
    let caption = crate::theme::line_height(theme, crate::theme::TextRole::Caption);
    let field = crate::theme::line_height(theme, crate::theme::TextRole::Title)
        + metrics.space_sm * 2.0
        + metrics.hairline * 2.0;

    // The title and the field share the top row; the field is the taller of the
    // two, and it is the one the row is measured by.
    let top = field.max(heading);

    // One chip row. The stock category set fits a surface this wide on one row;
    // the wrapping fallback a narrow output gets takes its extra rows out of the
    // card, which scrolls rather than clipping the window.
    let chips = crate::theme::action_height(theme);

    // The card's own padding, which sits inside the card above and below the
    // grid; the grid itself is the only part `rows_height` measures in rows.
    let card_padding = metrics.space_lg * 2.0;

    // Two caption lines: the key hint, and the skipped-entries notice a real
    // installation almost always has. A launch status adds a third line and a
    // failure a fourth; those are transient, and they take their room from the
    // card, which scrolls and still draws whole rows.
    let status = metrics.space_lg * 2.0 + caption * 2.0 + metrics.space_sm;

    metrics.space_xl * 2.0
        + top
        + metrics.space_md
        + chips
        + metrics.space_md
        + card_padding
        + metrics.space_lg
        + status
}

/// How many characters fit into a tile's label.
///
/// The estimate is the tile's own width (which the grid derivation already sized
/// from the theme's average advance) times the lines the label wraps to, so a
/// truncated name is as wide as the label it is drawn in rather than as wide as
/// some other layout's column (acceptance Q02).
fn label_budget(theme: &Theme, grid: &crate::theme::Grid) -> usize {
    let per_line = grid.cell_width / crate::theme::average_advance(theme);

    // At least one character: a tile with no room for even an ellipsis would be
    // a tile the user cannot identify.
    ((per_line * crate::theme::TILE_LABEL_LINES as f32) as usize).max(1)
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
    fn the_overlay_takes_most_of_the_output_in_whole_grid_rows() {
        let config = LauncherConfig::default();
        let theme = theme();
        let placement = placement(&config);
        let grid = crate::theme::grid(&theme, &config, placement.size.width);
        let metrics = crate::theme::Metrics::from_tokens(&theme);
        let pitch = grid.cell_height + grid.gap;
        let rows = (placement.size.height as f32 - bands(metrics, &theme)) / pitch;

        assert_eq!(
            placement.size.width,
            OUTPUT.width * 4 / 5,
            "the surface asks for four fifths of the output's width",
        );
        assert!(
            placement.size.height <= OUTPUT.height,
            "and never for more than the output",
        );
        assert!(
            placement.size.height as f32 >= OUTPUT.height as f32 * 0.9,
            "the height is at least the share it asks for: {} of {}",
            placement.size.height,
            OUTPUT.height,
        );
        assert!(
            (rows - rows.round()).abs() < 0.02,
            "the card holds whole rows, not {rows}",
        );
        assert!(
            rows.round() >= 4.0,
            "at least four grid rows are visible: {rows}",
        );
        assert!(placement.grid.columns >= 6);
    }

    #[test]
    fn the_size_comes_from_the_configuration_and_the_theme() {
        let config = LauncherConfig::default();
        let theme = theme();
        let placement = placement(&config);

        assert!(placement.size.width > 0);
        assert!(placement.size.width <= OUTPUT.width);
        assert!(placement.size.height <= OUTPUT.height);

        // A larger icon makes every tile taller and wider: fewer columns fit, and
        // fewer rows fit the same share of the output, so the surface trades many
        // short rows for fewer tall ones.
        let mut bigger = config.clone();
        bigger.icon_size = config.icon_size * 2;
        let taller = Placement::overlay(&bigger, &theme, OUTPUT, None);
        assert!(taller.grid.cell_height > placement.grid.cell_height);
        assert!(taller.grid.cell_width > placement.grid.cell_width);
        assert!(taller.grid.columns < placement.grid.columns);
        assert!(taller.size.width <= OUTPUT.width && taller.size.height <= OUTPUT.height);

        // A narrower output still gets six columns of tiles, and a narrower surface.
        let narrow = Placement::overlay(&config, &theme, LogicalSize::new(800, 600), None);
        assert!(narrow.size.width < placement.size.width);
        assert!(narrow.size.width <= 800);
        assert_eq!(narrow.grid.columns, 6);
    }

    #[test]
    fn a_short_output_keeps_the_row_floor_only_as_far_as_the_output_allows() {
        let config = LauncherConfig::default();
        let theme = theme();
        let short = LogicalSize::new(1280, 600);
        let placement = Placement::overlay(&config, &theme, short, None);

        // Four rows need more than ninety percent of a 600 px output, so the
        // floor wins — and the output still caps it.
        assert_eq!(placement.size.height, 600);
        assert_eq!(placement.size.width, short.width * 4 / 5);

        // A taller output gets the share (rounded up to whole rows) instead of the
        // output height, and never more than the output.
        let taller = Placement::overlay(&config, &theme, LogicalSize::new(1280, 900), None);
        assert!(taller.size.height > placement.size.height);
        assert!(taller.size.height < 900);
        assert!(taller.size.height as f32 >= 900.0 * 0.9);
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
    fn the_label_budget_stays_usable_when_the_font_scales_up() {
        let config = LauncherConfig::default();
        let theme = theme();
        let normal = Placement::overlay(&config, &theme, OUTPUT, None);

        let mut scaled = theme.clone();
        scaled.typography.scale *= 2.0;
        let enlarged = Placement::overlay(&config, &scaled, OUTPUT, None);

        assert!(normal.label_chars > 1);
        assert!(
            enlarged.label_chars >= 1,
            "a tile always has room for a name, and grows to keep it",
        );
        assert!(
            enlarged.grid.cell_width > normal.grid.cell_width,
            "the tile grows with the type instead of cutting the label",
        );
    }

    #[test]
    fn the_placement_carries_the_grid_the_keyboard_steps_by() {
        let placement = placement(&LauncherConfig::default());

        assert!(placement.grid.columns >= 1);
        assert!(placement.grid.cell_width > 0.0);
        assert!(placement.grid.cell_height > 0.0);
        assert!(placement.grid.image > 0.0);
        assert!(
            placement.grid.gap > 0.0,
            "tiles are separated by a token, not by each other",
        );
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
