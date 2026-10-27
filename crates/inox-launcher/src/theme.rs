//! Design tokens to the values QML draws with, as pure functions.
//!
//! Every colour, distance, radius and text size the launcher draws with is
//! derived here from [`inox_config::Theme`] (root AGENTS.md §4, PRD §9.2): no
//! projection and no QML file in this crate contains a colour or a pixel size of
//! its own. The L3 component architecture explains why this small mapping is duplicated in each L3
//! component instead of being shared — a shared UI crate would be a new
//! same-layer dependency, which needs a decision of its own.
//!
//! Nothing here is Qt-aware. A colour leaves as the `#AARRGGBB` string QML's
//! `color` properties accept (`QColor::name(QColor::HexArgb)`'s format), and a
//! distance leaves as a number, which is what lets the whole mapping — and the
//! projection built on top of it — be unit-tested without a display
//! (docs/qml-migration-contract.md §4).
//!
//! Three things are derived here rather than read from a token, because the
//! token set has no field for them and the design guidelines' §1 says so:
//!
//! | derived | from | used by |
//! |---------|------|---------|
//! | the four text sizes | [`TextRole`] × the base text size | every string |
//! | the hairline | `spacing.xs / 2`, never below one pixel | outlines |
//! | the focus ring | `spacing.xs` | the search field |
//!
//! The base text size is not a token either: the theme carries a *scale*, and
//! what it scales is Qt Quick's nominal body size at 96 dpi,
//! [`BASE_TEXT_PX`] — so a theme asking for 200 % text gets 200 % of what an
//! unscaled Qt Quick window would draw, without this file naming a size the
//! user changed their mind about.

use inox_config::LauncherConfig;
use inox_config::Theme;
use inox_config::theme::Rgba;

/// The multiplier of the `title` text role (guidelines §2).
const TITLE_RATIO: f32 = 1.5;
/// The multiplier of the `heading` text role.
const HEADING_RATIO: f32 = 1.25;
/// The multiplier of the `body` text role.
const BODY_RATIO: f32 = 1.0;
/// The multiplier of the `caption` text role.
const CAPTION_RATIO: f32 = 0.875;

/// A hairline is half of the smallest spacing step (guidelines §1).
const HAIRLINE_RATIO: f32 = 0.5;

/// A hairline thinner than one pixel is a border nobody can see, so a theme that
/// zeroes `spacing.xs` still gets an outline rather than none.
const MINIMUM_HAIRLINE: f32 = 1.0;

/// The narrowest a group is ever allowed to get, for an output too small to hold
/// even the window's own padding. A width of zero draws nothing at all.
const MINIMUM_CONTENT_WIDTH: f32 = 1.0;

/// The ratio of an average glyph's advance to the font size.
///
/// A proportional sans font's average advance is about half an em. This is a
/// dimensionless ratio, not a size: the size is [`text_size`], and the ratio
/// only decides how many characters a result row's label has room for.
const AVERAGE_ADVANCE_RATIO: f32 = 0.5;

/// One line of text occupies its font size times this.
///
/// Qt Quick's own text layout uses the font's metrics, which for the token font
/// family settle at about 1.2 em; the launcher has to know the number because
/// [`crate::placement`] sizes the surface before any text exists, so it is
/// written down once here rather than guessed in two places.
const LINE_HEIGHT_RATIO: f32 = 1.2;

/// How many characters a tile's label has room for on one line.
///
/// A tile is sized so the name it carries stays readable without stretching the
/// cell: the icon plus `spacing.lg` on either side, or this many average glyphs,
/// whichever is wider. When `typography.scale` grows, the *cell* grows with the
/// type instead of the name being cut shorter.
const GRID_LABEL_CHARS: f32 = 12.0;

/// The number of lines a tile's label wraps to.
pub const TILE_LABEL_LINES: usize = 2;

/// The body text size of an unscaled Qt Quick window, in logical pixels.
///
/// Qt's default application font is 12 pt, which at 96 dpi is 16 px. The theme's
/// `typography.scale` multiplies this, so the launcher follows the user's font
/// scale rather than a size of its own.
pub const BASE_TEXT_PX: f32 = 16.0;

/// The four text roles of the guidelines' §2.
///
/// The roles are the whole type scale: a surface has at most one `title`, groups
/// and sections are headed with `heading`, text and control labels are `body`,
/// and explanations, units and secondary metadata are `caption`. Emphasis is a
/// weight, not a fifth size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    /// The one thing the surface is about; the launcher draws the search field
    /// with it.
    Title,
    /// A group or section heading.
    Heading,
    /// Body text and control labels.
    Body,
    /// Explanations, units and secondary metadata.
    Caption,
}

impl TextRole {
    /// The multiplier the role applies to the base text size (guidelines §2).
    pub const fn ratio(self) -> f32 {
        match self {
            Self::Title => TITLE_RATIO,
            Self::Heading => HEADING_RATIO,
            Self::Body => BODY_RATIO,
            Self::Caption => CAPTION_RATIO,
        }
    }

    /// The role's name, as the projection reports it to QML.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Heading => "heading",
            Self::Body => "body",
            Self::Caption => "caption",
        }
    }
}

/// The four edges of a padding or a margin, in logical pixels.
///
/// This is what a padding looks like without a GUI toolkit type: the projection
/// publishes the four numbers and QML applies them, so a view never adds two
/// tokens together by hand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edges {
    /// Distance above the content.
    pub top: f32,
    /// Distance to the right of the content.
    pub right: f32,
    /// Distance below the content.
    pub bottom: f32,
    /// Distance to the left of the content.
    pub left: f32,
}

impl Edges {
    /// The same distance on every side.
    pub const fn uniform(all: f32) -> Self {
        Self {
            top: all,
            right: all,
            bottom: all,
            left: all,
        }
    }

    /// `horizontal` on the left and right, `vertical` above and below.
    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }
    }
}

/// The theme's distances, in the form the projection needs them.
///
/// The token table has three granularities of the same numbers — a step
/// ([`Theme::spacing`]), the derived widths (hairline, focus ring) and the
/// paddings the guidelines' §3 and §4 spell out. Building them once per
/// projection keeps every gap in the QML tree a named value rather than
/// arithmetic (guidelines §1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// `spacing.xs`.
    pub space_xs: f32,
    /// `spacing.sm`.
    pub space_sm: f32,
    /// `spacing.md`.
    pub space_md: f32,
    /// `spacing.lg`.
    pub space_lg: f32,
    /// `spacing.xl`.
    pub space_xl: f32,
    /// `radii.sm`, for controls.
    pub radius_sm: f32,
    /// `radii.md`, for cards, groups and popovers.
    pub radius_md: f32,
    /// `radii.lg`, for windows and full-screen surfaces.
    pub radius_lg: f32,
    /// The border width of an outline: `spacing.xs / 2`, at least one pixel.
    pub hairline: f32,
    /// The border width of the focus ring: `spacing.xs`, at least a hairline.
    pub focus_ring: f32,
}

impl Metrics {
    /// The distances a theme implies.
    pub fn from_tokens(tokens: &Theme) -> Self {
        let hairline = (f32::from(tokens.spacing.xs) * HAIRLINE_RATIO).max(MINIMUM_HAIRLINE);

        Self {
            space_xs: f32::from(tokens.spacing.xs),
            space_sm: f32::from(tokens.spacing.sm),
            space_md: f32::from(tokens.spacing.md),
            space_lg: f32::from(tokens.spacing.lg),
            space_xl: f32::from(tokens.spacing.xl),
            radius_sm: f32::from(tokens.radii.sm),
            radius_md: f32::from(tokens.radii.md),
            radius_lg: f32::from(tokens.radii.lg),
            hairline,
            focus_ring: f32::from(tokens.spacing.xs).max(hairline),
        }
    }

    /// The padding of a control: `spacing.md` at the sides, `spacing.sm` above
    /// and below (guidelines §3).
    pub fn control_padding(&self) -> Edges {
        Edges::symmetric(self.space_sm, self.space_md)
    }

    /// The padding inside a group card: `spacing.lg` on every side (§4).
    pub fn card_padding(&self) -> Edges {
        Edges::uniform(self.space_lg)
    }

    /// The width the groups get inside a surface of `surface` logical pixels.
    ///
    /// The window's own padding is spent on both sides of the groups, which is
    /// what keeps a field from being drawn edge to edge on a screen much wider
    /// than the surface the host maps (§4: a width follows what a control is
    /// for, not how much room there happens to be).
    pub fn content_width(&self, surface: u32) -> f32 {
        (surface as f32 - self.space_xl * 2.0).max(MINIMUM_CONTENT_WIDTH)
    }
}

/// A token colour as the `#AARRGGBB` string QML accepts.
///
/// Alpha comes first because that is Qt's `QColor::HexArgb` order, which is the
/// one format a QML `color` property parses without a `Qt.rgba` call in the
/// middle of a binding. Uppercase digits keep two runs of the same colour
/// comparable as strings in a test.
pub fn qml_color(rgba: Rgba) -> String {
    format!("#{:02X}{:02X}{:02X}{:02X}", rgba.a, rgba.r, rgba.g, rgba.b)
}

/// The text size the `body` role is drawn at.
pub fn text_size(tokens: &Theme) -> f32 {
    role_size(tokens, TextRole::Body)
}

/// The size one of the four text roles is drawn at.
///
/// A nonsensical configured scale must not make text disappear, so the result is
/// floored at one pixel.
pub fn role_size(tokens: &Theme, role: TextRole) -> f32 {
    (BASE_TEXT_PX * tokens.typography.scale * role.ratio()).max(1.0)
}

/// The height one line of a text role takes.
///
/// A surface that has to be sized before any text exists (see
/// [`crate::placement`]) measures its text with this, the same number the QML
/// tree lays the text out with.
pub fn line_height(tokens: &Theme, role: TextRole) -> f32 {
    role_size(tokens, role) * LINE_HEIGHT_RATIO
}

/// The width one character of the theme's text takes, on average.
pub fn average_advance(tokens: &Theme) -> f32 {
    (text_size(tokens) * AVERAGE_ADVANCE_RATIO).max(1.0)
}

/// The geometry of the application grid, in logical pixels.
///
/// One value, computed once per placement and used by everything that has to
/// agree about the grid: the surface's height, the projection QML draws, and the
/// keyboard step (`Left`/`Right` move by one, `Up`/`Down` by a whole
/// [`Grid::columns`]). There is no second derivation to drift — `crate::app` and
/// `crate::view` both read the one `crate::placement::Placement::grid`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    /// How many tiles fit across the card.
    pub columns: usize,
    /// The width of one tile, including nothing of the gap.
    pub cell_width: f32,
    /// The height of one tile, including nothing of the gap.
    pub cell_height: f32,
    /// The gap between two tiles, horizontally and vertically.
    pub gap: f32,
    /// The square the icon is drawn in, inside a tile.
    pub image: f32,
}

/// The grid a surface of `surface_width` logical pixels holds.
///
/// The card's content is what the tiles get: the window's `xl` padding and the
/// card's `lg` padding come off the surface first (guidelines §3). A tile is as
/// wide as its label needs and as tall as its icon plus two label lines, and a
/// surface too narrow for even one column still gets one — a grid with zero
/// columns would draw nothing at all.
///
/// [`Grid::columns`] is a whole number of [`Grid::cell_width`] plus [`Grid::gap`]
/// cells: QML sizes the view to exactly that, so the columns the projection names
/// are the columns the grid draws.
pub fn grid(tokens: &Theme, config: &LauncherConfig, surface_width: u32) -> Grid {
    let metrics = Metrics::from_tokens(tokens);

    let image = f32::from(config.icon_size).max(metrics.space_xl * 2.0);
    let gap = metrics.space_md;
    let cell_width =
        (image + metrics.space_lg * 2.0).max(average_advance(tokens) * GRID_LABEL_CHARS);
    let cell_height =
        image + metrics.space_sm + line_height(tokens, TextRole::Caption) * TILE_LABEL_LINES as f32;

    let available = (surface_width as f32 - metrics.space_xl * 2.0 - metrics.space_lg * 2.0)
        .max(MINIMUM_CONTENT_WIDTH);
    // A `GridView` lays its delegates out on a `cellWidth` grid and fits
    // `floor(width / cellWidth)` of them per row. The projection publishes the
    // column count the keyboard steps by, so it has to be that same floor: a grid
    // told "nine columns" while drawing eight would move the selection by a
    // number the window is not drawing.
    let pitch = cell_width + gap;
    let columns = ((available / pitch).floor() as usize).max(1);

    Grid {
        columns,
        cell_width,
        cell_height,
        gap,
        image,
    }
}

/// The height of an interactive control: a button, a retry, an action.
///
/// The guidelines' §4 asks for an interaction target of at least `spacing.xl`
/// (32 px) that *grows* with the type; §3 says a control's vertical padding is
/// `spacing.sm`. At the default scale the two do not agree — a 16 px body line
/// plus `sm` above and below is 27.2 px — and the guidelines resolve that in
/// favour of §4, so the floor is what carries the rule until `typography.scale`
/// has grown the line box past it. The height therefore never shrinks below the
/// target and never clips the label at 200 % text.
///
/// QML binds this instead of adding a line box and a padding together, so the
/// number is derived once (root AGENTS.md §4) and a test can hold both ends of
/// it: the floor, and the growth.
pub fn action_height(tokens: &Theme) -> f32 {
    let metrics = Metrics::from_tokens(tokens);
    let label_and_padding = line_height(tokens, TextRole::Body) + metrics.space_sm * 2.0;

    label_and_padding.max(metrics.space_xl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_config::Theme as Tokens;

    fn tokens() -> Tokens {
        Tokens::builtin_dark()
    }

    #[test]
    fn a_token_colour_becomes_the_qt_argb_string() {
        let dark = tokens();
        let accent = dark.colors.accent;
        assert_eq!(
            qml_color(accent),
            format!(
                "#{:02X}{:02X}{:02X}{:02X}",
                accent.a, accent.r, accent.g, accent.b
            ),
        );

        // The shape a QML `color` property parses: `#`, eight hex digits, alpha
        // first. A three- or six-digit value would silently mean something else.
        let rendered = qml_color(accent);
        assert_eq!(rendered.len(), 9, "{rendered}");
        assert!(rendered.starts_with('#'));
        assert!(rendered[1..].chars().all(|digit| digit.is_ascii_hexdigit()));
    }

    #[test]
    fn opaque_and_transparent_colours_keep_their_alpha() {
        assert_eq!(
            qml_color(Rgba {
                r: 0x12,
                g: 0x34,
                b: 0x56,
                a: 0xff
            }),
            "#FF123456"
        );
        assert_eq!(
            qml_color(Rgba {
                r: 0x12,
                g: 0x34,
                b: 0x56,
                a: 0x00
            }),
            "#00123456"
        );
    }

    #[test]
    fn the_two_builtin_themes_project_different_colours() {
        let dark = tokens();
        let light = Tokens::builtin_light();
        assert_ne!(
            qml_color(dark.colors.background),
            qml_color(light.colors.background)
        );
        assert_ne!(qml_color(dark.colors.text), qml_color(light.colors.text));
    }

    #[test]
    fn the_four_text_roles_follow_the_guideline_ratios() {
        let tokens = tokens();
        let base = text_size(&tokens);

        assert_eq!(role_size(&tokens, TextRole::Body), base);
        assert_eq!(role_size(&tokens, TextRole::Title), base * TITLE_RATIO);
        assert_eq!(role_size(&tokens, TextRole::Heading), base * HEADING_RATIO);
        assert_eq!(role_size(&tokens, TextRole::Caption), base * CAPTION_RATIO);

        let title = role_size(&tokens, TextRole::Title);
        let heading = role_size(&tokens, TextRole::Heading);
        let caption = role_size(&tokens, TextRole::Caption);
        assert!(title > heading && heading > base && base > caption);
        assert!(
            caption >= base * 0.75,
            "a caption may not be smaller than three quarters of the body size",
        );

        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;
        for role in [
            TextRole::Title,
            TextRole::Heading,
            TextRole::Body,
            TextRole::Caption,
        ] {
            assert_eq!(
                role_size(&doubled, role),
                role_size(&tokens, role) * 2.0,
                "every role follows the font scale, so 200 % text stays uncropped",
            );
        }
    }

    #[test]
    fn the_text_size_is_the_token_scale_applied_to_the_nominal_body_size() {
        let tokens = tokens();
        assert!(text_size(&tokens) > 0.0, "a zero text size draws nothing");
        assert_eq!(text_size(&tokens), BASE_TEXT_PX * tokens.typography.scale);

        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;
        assert_eq!(text_size(&doubled), text_size(&tokens) * 2.0);
    }

    #[test]
    fn a_nonsensical_font_scale_still_yields_visible_text() {
        let mut tokens = tokens();
        for scale in [f32::NAN, f32::INFINITY, -4.0, 0.0] {
            tokens.typography.scale = scale;
            for role in [
                TextRole::Title,
                TextRole::Heading,
                TextRole::Body,
                TextRole::Caption,
            ] {
                assert!(
                    role_size(&tokens, role) >= 1.0,
                    "scale {scale} produced {} for {role:?}",
                    role_size(&tokens, role),
                );
            }
        }
    }

    #[test]
    fn the_line_height_is_the_font_size_times_the_ratio() {
        let tokens = tokens();
        assert_eq!(
            line_height(&tokens, TextRole::Caption),
            role_size(&tokens, TextRole::Caption) * LINE_HEIGHT_RATIO
        );
        assert!(line_height(&tokens, TextRole::Title) > line_height(&tokens, TextRole::Body));
    }

    #[test]
    fn the_average_advance_follows_the_font_scale() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;

        assert!(average_advance(&tokens) >= 1.0);
        assert_eq!(average_advance(&doubled), average_advance(&tokens) * 2.0);
    }

    #[test]
    fn the_metrics_are_derived_from_the_tokens() {
        let tokens = tokens();
        let metrics = Metrics::from_tokens(&tokens);

        assert_eq!(metrics.space_xs, f32::from(tokens.spacing.xs));
        assert_eq!(metrics.space_sm, f32::from(tokens.spacing.sm));
        assert_eq!(metrics.space_md, f32::from(tokens.spacing.md));
        assert_eq!(metrics.space_lg, f32::from(tokens.spacing.lg));
        assert_eq!(metrics.space_xl, f32::from(tokens.spacing.xl));
        assert_eq!(metrics.radius_sm, f32::from(tokens.radii.sm));
        assert_eq!(metrics.radius_md, f32::from(tokens.radii.md));
        assert_eq!(metrics.radius_lg, f32::from(tokens.radii.lg));
        assert_eq!(metrics.hairline, f32::from(tokens.spacing.xs) / 2.0);
        assert_eq!(metrics.focus_ring, f32::from(tokens.spacing.xs));
    }

    #[test]
    fn a_theme_that_zeroes_its_smallest_step_still_draws_an_outline() {
        let mut tokens = tokens();
        tokens.spacing.xs = 0;
        let metrics = Metrics::from_tokens(&tokens);

        assert_eq!(metrics.hairline, MINIMUM_HAIRLINE);
        assert!(metrics.focus_ring >= metrics.hairline);
    }

    #[test]
    fn the_paddings_follow_the_spacing_rhythm() {
        let tokens = tokens();
        let metrics = Metrics::from_tokens(&tokens);
        let control = metrics.control_padding();
        let card = metrics.card_padding();

        assert_eq!(control.top, metrics.space_sm);
        assert_eq!(control.bottom, metrics.space_sm);
        assert_eq!(control.left, metrics.space_md);
        assert_eq!(control.right, metrics.space_md);
        assert_eq!(card.top, metrics.space_lg);
        assert_eq!(card.right, metrics.space_lg);
        assert_eq!(card.bottom, metrics.space_lg);
        assert_eq!(card.left, metrics.space_lg);
    }

    #[test]
    fn the_grid_fills_the_card_with_whole_tiles() {
        let tokens = tokens();
        let config = LauncherConfig::default();
        let grid = grid(&tokens, &config, 512);
        let metrics = Metrics::from_tokens(&tokens);

        let available = 512.0 - metrics.space_xl * 2.0 - metrics.space_lg * 2.0;
        let used = grid.columns as f32 * grid.cell_width + (grid.columns as f32 - 1.0) * grid.gap;
        let pitch = grid.cell_width + grid.gap;
        assert!(
            used <= available,
            "the tiles leave the card: {used} of {available}",
        );
        assert!(
            used + pitch > available,
            "one more column would have fitted",
        );
        assert_eq!(grid.columns, 4, "512 px holds four default tiles");
        assert_eq!(
            super::grid(&tokens, &config, 1024).columns,
            8,
            "the launchpad width holds eight whole cells",
        );
        assert!(grid.image >= f32::from(config.icon_size));
        assert!(
            grid.cell_height > grid.image,
            "the label gets room below the icon"
        );
    }

    #[test]
    fn a_narrow_surface_still_gets_one_column() {
        let tokens = tokens();
        let config = LauncherConfig::default();

        assert_eq!(grid(&tokens, &config, 0).columns, 1);
        assert_eq!(grid(&tokens, &config, 120).columns, 1);
    }

    #[test]
    fn the_grid_grows_with_the_type_instead_of_cutting_the_label() {
        let tokens = tokens();
        let config = LauncherConfig::default();
        let normal = grid(&tokens, &config, 512);

        let mut scaled = tokens.clone();
        scaled.typography.scale *= 2.0;
        let enlarged = grid(&scaled, &config, 512);

        assert!(enlarged.cell_width > normal.cell_width);
        assert!(enlarged.cell_height > normal.cell_height);
        assert!(
            enlarged.columns < normal.columns,
            "wider tiles mean fewer columns in the same surface",
        );
        assert!(enlarged.columns >= 1);
    }

    #[test]
    fn a_bigger_configured_icon_enlarges_the_tile() {
        let tokens = tokens();
        let plain = grid(&tokens, &LauncherConfig::default(), 512);

        let mut bigger = LauncherConfig::default();
        bigger.icon_size *= 2;
        let enlarged = grid(&tokens, &bigger, 512);

        assert!(enlarged.image > plain.image);
        assert!(enlarged.cell_height > plain.cell_height);
    }

    #[test]
    fn the_action_height_never_falls_below_the_interaction_target() {
        let tokens = tokens();
        let metrics = Metrics::from_tokens(&tokens);

        assert_eq!(
            action_height(&tokens),
            metrics.space_xl,
            "the default type is shorter than the target, so the target wins",
        );

        // A theme that shrinks the type still gets a control a pointer can hit.
        let mut small = tokens.clone();
        small.typography.scale = 0.5;
        let small_metrics = Metrics::from_tokens(&small);
        assert_eq!(action_height(&small), small_metrics.space_xl);
        assert!(small_metrics.space_xl >= 32.0);
    }

    #[test]
    fn the_action_height_grows_with_the_font_scale() {
        let tokens = tokens();
        let mut doubled = tokens.clone();
        doubled.typography.scale *= 2.0;
        let metrics = Metrics::from_tokens(&doubled);

        let normal = action_height(&tokens);
        let scaled = action_height(&doubled);

        assert!(
            scaled > normal,
            "200 % text needs a taller control: {scaled} vs {normal}",
        );
        assert_eq!(
            scaled,
            line_height(&doubled, TextRole::Body) + metrics.space_sm * 2.0,
            "past the floor the height is the label's line box plus the control padding",
        );
        assert!(
            scaled > metrics.space_xl,
            "the scaled control is taller than the target it had to clear",
        );
    }

    #[test]
    fn the_content_width_spends_the_window_padding_on_both_sides() {
        let tokens = tokens();
        let metrics = Metrics::from_tokens(&tokens);
        let surface = 512;

        assert_eq!(
            metrics.content_width(surface),
            surface as f32 - metrics.space_xl * 2.0,
            "the cards get what is left of the surface after its own padding",
        );
        assert_eq!(
            metrics.content_width(0),
            MINIMUM_CONTENT_WIDTH,
            "a column of zero width draws nothing at all",
        );
    }
}
