//! The dock: the panel drawn as a floating pane above the bottom edge.
//!
//! The dock is a *presentation* of the same data the taskbar draws: one square
//! tile per running application group, the status notifier items in the same
//! pane, and the panel's own state when it has one. It owns no state of its own
//! ([`crate::tasks`], [`crate::tray`] and [`crate::clock`] are untouched), so
//! the two surfaces cannot disagree about what is running.
//!
//! ```text
//! ┌ highlight ──────────────────────────────────────────┐
//! │ notice  ▢ ▢ ▢ ▢ │ tray            │  ← the glass pane
//! │         ▪   ▪                        ← the active marks
//! └ shade ──────────────────────────────────────────────┘
//!            ↑ margin above the bottom edge
//! ```
//!
//! Four decisions are worth naming:
//!
//! - **A tile is a badge, not an icon.** The panel has no icon-theme decoder and
//!   must not grow one, so a tile carries the first character of the
//!   application's name and the catalog's placeholder when it has none — the
//!   same first-letter/placeholder approach the launcher uses for its icon
//!   column ([`crate::view_model`], `inox-launcher`'s `row_widget`).
//! - **The active mark has its own slot on every tile.** A focused group draws a
//!   small accent mark below its tile; an unfocused one draws nothing, but the
//!   slot is still laid out, so the tiles never move when focus changes.
//! - **The tiles are bounded and the pane clips.** `panel.max_task_buttons`
//!   bounds how many tiles exist (the shared projection applied it, and
//!   [`DockView::project`] applies it again), and the layout clips what does not
//!   fit instead of scrolling or growing an overflow indicator.
//! - **The pane is as wide as its content.** There is no stretching a dock
//!   across the output: the geometry here is what the placement asks for and
//!   what the view lays out.
//!
//! This module is the *pure* half — the geometry and the projection. The widgets
//! are in [`crate::view::dock`].

use crate::view::TooltipPosition;
use inox_common::i18n::Localizer;
use inox_config::{DockConfig, Theme as Tokens};

use crate::tasks::WindowKey;
use crate::theme;
use crate::view_model::{PanelViewModel, TaskButton, TrayItemView};

/// How wide the active mark under a tile is, as a fraction of the tile.
const INDICATOR_WIDTH_SCALE: f32 = 0.16;

/// How tall the active mark is, in `spacing.xs` steps.
const INDICATOR_HEIGHT_STEPS: f32 = 1.0;

/// How much of a tile the badge's glyph takes.
const GLYPH_SIZE_SCALE: f32 = 0.5;

/// How much of a tile the separator before the tray group is tall.
const SEPARATOR_HEIGHT_SCALE: f32 = 0.75;

/// How many extra-large spacing steps a dock label may be wide.
const LABEL_WIDTH_STEPS: f32 = 6.0;

/// How many extra-large spacing steps the narrowest readable dock is wide.
///
/// The dock's sentence about the panel's own state is the only thing it has to
/// say when nothing is running (D05), and a pane narrower than that sentence
/// draws a clipped fragment instead of the sentence — which is the defect this
/// number exists to prevent. Sixteen extra-large steps is 512 logical pixels at
/// the default tokens: enough for the longest sentence the catalogs hold ("The
/// window service is not available", thirty-five characters) *plus* its retry
/// action at the dock's own caption size under the documented
/// [`AVERAGE_ADVANCE_RATIO`] estimate, with room for the wider glyphs of a
/// full-width (CJK) locale on top. The unit tests assert that for **both**
/// catalogs and for the sentence and action of each state the dock can show.
const MINIMUM_WIDTH_STEPS: f32 = 16.0;

/// The ratio of an average glyph's advance to the font size.
///
/// A proportional sans font's average advance is about half an em, and a
/// full-width (CJK) glyph about one, so this is a *lower* bound for the two
/// locales the panel ships. It is a dimensionless ratio rather than a size, and
/// it is the estimate the launcher already uses to budget a result row's label
/// (`inox-launcher`'s `placement::label_budget`): a pane that has to be sized
/// before any widget exists cannot measure the font, so it estimates and the
/// layout — which owns the last word and clips what does not fit — is never
/// second-guessed.
const AVERAGE_ADVANCE_RATIO: f32 = 0.5;

/// How many [`DockGeometry::label_max_width`]s the notice sentence may be wide.
///
/// The sentence about the panel's own state is what a user has to read first, so
/// it is allowed to be wider than one tile's label. This is the same bound the
/// layout clamps it with ([`DockGeometry::notice_max_width`]), so the estimate
/// and the drawing agree on the widest the sentence can ever be.
const NOTICE_WIDTH_STEPS: f32 = 8.0;

/// What the dock will draw, as far as the width it asks for is concerned.
///
/// A pane has to be sized before iced lays anything out, so the width comes from
/// these three numbers — how many tiles there are, how wide the tray group's
/// text is estimated to be, and how wide the panel's own sentence is estimated
/// to be — rather than from a measurement. [`DockGeometry::content`] derives
/// them from the projection the dock will draw.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DockContent {
    /// How many task tiles the dock holds.
    pub tiles: usize,
    /// How wide the tray group is drawn, from [`DockGeometry::tray_width`];
    /// `0.0` when the dock shows no tray group.
    pub tray_width: f32,
    /// How wide the panel's own sentence is laid out, from
    /// [`DockGeometry::notice_width`]; `0.0` when the dock has no notice.
    ///
    /// A sentence is never narrower than the slot the pane reserves for one, so
    /// this is `0.0` or at least that slot — never a sliver between the two.
    pub notice_width: f32,
}

/// The geometry of the dock, derived from the tokens and the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct DockGeometry {
    /// Size of one tile, in logical pixels.
    pub icon_size: f32,
    /// Padding between the pane's edge and its content.
    pub padding: f32,
    /// Gap between two tiles.
    pub spacing: f32,
    /// Gap between the pane and the bottom edge.
    pub margin: f32,
    /// Thickness of the highlight and shade hairlines along the pane's edges.
    pub edge_height: f32,
    /// Width of the active mark under a tile.
    pub indicator_width: f32,
    /// Height of the active mark under a tile.
    pub indicator_height: f32,
    /// Gap between a tile and the mark below it.
    pub indicator_gap: f32,
    /// Thickness of the hairline before the tray group.
    pub separator_width: f32,
    /// Height of the hairline before the tray group.
    pub separator_height: f32,
    /// Text size of a tile's badge glyph.
    pub glyph_size: f32,
    /// Text size of the tray group's labels.
    ///
    /// A dock has no configured height, so its tile is the unit the type scale
    /// is derived from, exactly as the taskbar derives its own from the panel's
    /// height (ui-guidelines §2).
    pub tray_size: f32,
    /// Widest a tile's label may be before the layout clips it.
    pub label_max_width: f32,
    /// The width the pane asks for when its only entry is its own sentence.
    ///
    /// `MINIMUM_WIDTH_STEPS` extra-large spacing steps: the floor that keeps a
    /// dock with nothing but its own sentence (no tiles, no tray) wide enough to
    /// draw that sentence instead of a clipped fragment of it.
    pub minimum_width: f32,
    /// Which way a tooltip opens. The dock floats above the bottom edge, so its
    /// tooltips open upwards.
    pub tooltip_position: TooltipPosition,
}

impl DockGeometry {
    /// Derives the dock's geometry.
    pub fn new(tokens: &Tokens, config: &DockConfig) -> Self {
        let icon_size = f32::from(config.icon_size);
        let content_height = f32::from(tokens.spacing.md) * 2.0
            + icon_size
            + f32::from(tokens.spacing.xs) * INDICATOR_HEIGHT_STEPS
            + f32::from(tokens.spacing.xs);

        Self {
            icon_size,
            padding: f32::from(tokens.spacing.md),
            spacing: f32::from(tokens.spacing.sm),
            margin: f32::from(config.margin),
            edge_height: theme::hairline(tokens),
            indicator_width: icon_size * INDICATOR_WIDTH_SCALE,
            indicator_height: f32::from(tokens.spacing.xs) * INDICATOR_HEIGHT_STEPS,
            indicator_gap: f32::from(tokens.spacing.xs),
            separator_width: theme::hairline(tokens),
            separator_height: icon_size * SEPARATOR_HEIGHT_SCALE,
            glyph_size: icon_size * GLYPH_SIZE_SCALE,
            tray_size: theme::caption_size(tokens, config.icon_size).min(content_height),
            label_max_width: f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS,
            minimum_width: f32::from(tokens.spacing.xl) * MINIMUM_WIDTH_STEPS,
            tooltip_position: TooltipPosition::Above,
        }
    }

    /// The height of the pane's content: one row of tiles plus the pane's
    /// padding, without the two edge hairlines.
    ///
    /// The active mark's slot is part of every tile — it is laid out even when
    /// the mark draws nothing — so the pane's height does not depend on which
    /// applications are running.
    pub fn content_height(&self) -> f32 {
        self.padding * 2.0 + self.icon_size + self.indicator_gap + self.indicator_height
    }

    /// The height of the pane itself: its content plus the highlight and the
    /// shade along its two edges.
    pub fn pane_height(&self) -> f32 {
        self.content_height() + self.edge_height * 2.0
    }

    /// The width the pane asks for, in logical pixels.
    ///
    /// `tray_width` is how wide the tray group is drawn; the tray is text, and
    /// how wide text is depends on the font, so the caller measures it and the
    /// layout — not this function — is what has the last word. `0.0` means the
    /// pane has no tray group. This is the same row as
    /// [`DockGeometry::surface_width`] without a sentence in it: the
    /// tiles-and-tray subtotal.
    pub fn content_width(&self, tiles: usize, tray_width: f32) -> f32 {
        self.row_width(DockContent {
            tiles,
            tray_width,
            notice_width: 0.0,
        })
    }

    /// The width the pane is drawn at, from everything the dock will draw.
    ///
    /// The pane is one row — the notice, the tiles, the hairline separator and
    /// the tray group — inset by [`DockGeometry::padding`] at both ends and
    /// separated by [`DockGeometry::spacing`], which is exactly the tree
    /// [`crate::view::dock`] lays out. A dock whose only entry is its own
    /// sentence therefore asks for exactly [`DockGeometry::minimum_width`],
    /// which is the room that sentence needs — [`DockGeometry::notice_width`]
    /// never gives it less than its slot.
    ///
    /// This is the number the *surface* asks the compositor for and the width
    /// the *pane* is laid out at, so the two agree: the compositor frosts the
    /// pane's own area (see [`crate::frost`]), and a surface wider than the pane
    /// would frost a band of wallpaper around the pill.
    pub fn surface_width(&self, content: DockContent) -> f32 {
        self.row_width(content)
    }

    /// How wide a run of `chars` characters is drawn at `size`.
    ///
    /// An estimate rather than a measurement, for the reason `AVERAGE_ADVANCE_RATIO`
    /// gives: no font is available to a pure geometry. It is the same estimate the
    /// launcher makes for a result row's label, and it follows `typography.scale`
    /// because `size` does.
    pub fn text_width(&self, chars: usize, size: f32) -> f32 {
        chars as f32 * size * AVERAGE_ADVANCE_RATIO
    }

    /// How wide the tray group is drawn, from the labels it will show.
    ///
    /// Every label is bounded by [`DockGeometry::label_max_width`] — the same
    /// bound the layout clamps it with — so one very long title cannot make the
    /// group wider than the pane will draw it. The items are separated by the
    /// group's own spacing.
    pub fn tray_width<'a>(&self, labels: impl IntoIterator<Item = &'a str>) -> f32 {
        let mut width = 0.0;
        let mut items = 0usize;

        for label in labels {
            width += self
                .text_width(label.chars().count(), self.tray_size)
                .min(self.label_max_width);
            items += 1;
        }

        if items > 1 {
            width += self.spacing * (items - 1) as f32;
        }

        width
    }

    /// The widest the notice sentence may be before the layout clips it.
    ///
    /// The one number [`crate::view::dock`] clamps the sentence with and the
    /// estimate here is clamped by, so the two can never disagree about how wide
    /// the sentence is allowed to be.
    pub fn notice_max_width(&self) -> f32 {
        self.label_max_width * NOTICE_WIDTH_STEPS
    }

    /// How wide the panel's own sentence is laid out, retry action included.
    ///
    /// The sentence is drawn at the dock's caption size
    /// ([`DockGeometry::tray_size`]) and bounded by
    /// [`DockGeometry::notice_max_width`]. When a notice offers an action, the
    /// action is a button at the same text size with [`DockGeometry::padding`]
    /// on both sides, separated from the sentence by the group gap the view
    /// uses.
    ///
    /// The result is never below the slot the pane reserves for a sentence
    /// ([`DockGeometry::minimum_width`] less the pane's own padding), so the
    /// dock's width does not jump when its state changes from one sentence to
    /// another, and a sentence is never clipped to a sliver of itself.
    pub fn notice_width(&self, notice: &str, retry: Option<&str>) -> f32 {
        let sentence = self
            .text_width(notice.chars().count(), self.tray_size)
            .min(self.notice_max_width());

        let width = match retry {
            None => sentence,
            Some(retry) => {
                sentence
                    + self.spacing
                    + self.padding * 2.0
                    + self.text_width(retry.chars().count(), self.tray_size)
            }
        };

        width.max(self.notice_slot_width())
    }

    /// The room the pane keeps for a sentence, whatever that sentence says.
    ///
    /// The difference between what a dock with nothing but its own sentence asks
    /// for ([`DockGeometry::minimum_width`]) and the pane's own padding: the
    /// width the longest catalog sentence needs at the dock's caption size under
    /// the documented estimate, with the margin a wider font or a full-width
    /// (CJK) locale needs on top.
    fn notice_slot_width(&self) -> f32 {
        (self.minimum_width - self.padding * 2.0).max(0.0)
    }

    /// The content of the dock `view` will draw, as far as its width is
    /// concerned.
    ///
    /// This is the one place the projection's tiles, tray labels and sentence
    /// become the numbers a placement is sized from, so the leg never has to
    /// spell out which part of the projection matters.
    pub fn content(&self, view: &DockView) -> DockContent {
        DockContent {
            tiles: view.tiles.len(),
            tray_width: self.tray_width(view.tray.iter().map(|item| item.label.as_str())),
            notice_width: match &view.notice {
                Some(notice) => self.notice_width(notice, view.retry.as_deref()),
                None => 0.0,
            },
        }
    }

    /// The entries of the pane's row, summed with the gaps between them and the
    /// pane's padding: the tree [`crate::view::dock`] lays out, as arithmetic.
    fn row_width(&self, content: DockContent) -> f32 {
        let mut entries: Vec<f32> = Vec::new();

        if content.notice_width > 0.0 {
            entries.push(content.notice_width);
        }
        if content.tiles > 0 {
            let tiles = content.tiles as f32;
            entries.push(tiles * self.icon_size + (tiles - 1.0) * self.spacing);
        }
        if content.tray_width > 0.0 {
            entries.push(self.separator_width);
            entries.push(content.tray_width);
        }

        let gaps = entries.len().saturating_sub(1) as f32;

        self.padding * 2.0 + entries.iter().sum::<f32>() + gaps * self.spacing
    }

    /// The vertical extent the dock's surface draws itself in: the pane plus the
    /// margin between it and the bottom edge. This is the number the placement's
    /// `height` is derived from and the size the offscreen render harness lays
    /// the widget tree out at.
    pub fn window_height(&self) -> f32 {
        self.pane_height() + self.margin
    }
}

/// One tile of the dock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockTile {
    /// The window an activation of this tile applies to, exactly as a task
    /// button's key is.
    pub key: WindowKey,
    /// The badge: the first character of the application's name, or the
    /// catalog's placeholder.
    pub glyph: String,
    /// Whether the glyph is the placeholder rather than a name's first
    /// character, so a missing name is visible rather than silent.
    pub placeholder: bool,
    /// What a hover shows: the application and its windows, or the reason the
    /// tile cannot be pressed (D05).
    pub tooltip: String,
    /// Whether the tile can be pressed.
    pub enabled: bool,
    /// Whether the compositor reports one of the group's windows as focused.
    pub active: bool,
    /// Whether the panel's own navigation focus is on this tile.
    pub focused: bool,
}

/// What the dock draws, derived from what the panel knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockView {
    /// The sentence about the panel's own state, when there is one (D05).
    pub notice: Option<String>,
    /// The label of the retry action, when the notice offers one.
    pub retry: Option<String>,
    /// One tile per task group, in the order the taskbar draws them.
    pub tiles: Vec<DockTile>,
    /// The status notifier items, empty when `show_tray` is off.
    pub tray: Vec<TrayItemView>,
}

impl DockView {
    /// Projects the shared panel model onto the dock's own arrangement.
    ///
    /// `max_tiles` is `panel.max_task_buttons`. The shared projection has
    /// already bounded the task list by it, and the dock applies the same bound
    /// again: the cap is a property of the surface, so no caller has to
    /// remember to keep it.
    pub fn project(
        model: &PanelViewModel,
        config: &DockConfig,
        max_tiles: u16,
        localizer: &Localizer,
    ) -> Self {
        let tiles = model
            .tasks
            .iter()
            .take(usize::from(max_tiles))
            .map(|task| tile(task, model.focused, localizer))
            .collect();

        Self {
            notice: model.notice.clone(),
            retry: model.retry.clone(),
            tiles,
            tray: if config.show_tray {
                model.tray.clone()
            } else {
                Vec::new()
            },
        }
    }

    /// Whether the dock has anything to draw beyond its own pane.
    pub fn is_empty(&self) -> bool {
        self.notice.is_none() && self.tiles.is_empty() && self.tray.is_empty()
    }

    /// Whether the pane carries a separator before its tray group.
    ///
    /// A separator with nothing after it would be a line that separates nothing,
    /// so it exists exactly when there is a tray group to separate.
    pub fn has_separator(&self) -> bool {
        !self.tray.is_empty()
    }
}

/// One task group, as a tile.
fn tile(task: &TaskButton, focused: Option<WindowKey>, localizer: &Localizer) -> DockTile {
    let (glyph, placeholder) = glyph(&task.app_id, localizer);

    // A disabled tile says why, exactly as a disabled task button does: the
    // reason is a sentence the user can act on, and the label already carries
    // the application's name (D05).
    let tooltip = if task.enabled {
        task.tooltip.clone()
    } else {
        task.disabled_reason
            .clone()
            .unwrap_or_else(|| task.tooltip.clone())
    };

    DockTile {
        key: task.key,
        glyph,
        placeholder,
        tooltip,
        enabled: task.enabled,
        active: task.active,
        focused: focused == Some(task.key),
    }
}

/// The badge a tile shows: the first character of the application's name, or the
/// catalog's placeholder when it has none.
///
/// The panel has no icon theme decoder, so
/// this is the same degradation the launcher's icon column uses, and the
/// placeholder keeps a missing name visible rather than silent.
fn glyph(app_id: &str, localizer: &Localizer) -> (String, bool) {
    match app_id.chars().next() {
        Some(first) => (first.to_uppercase().collect(), false),
        None => (localizer.message("panel-dock-icon-placeholder"), true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inox_common::i18n::Locale;

    use crate::i18n;
    use crate::tasks::{SessionGeneration, WindowId};
    use crate::view_model::PanelStatus;

    fn localizer(tag: &str) -> Localizer {
        let locale = Locale::parse(tag).expect("a well-formed tag");

        i18n::load(&locale).expect("the catalogs parse")
    }

    fn english() -> Localizer {
        localizer("en-US")
    }

    fn key(window: u64) -> WindowKey {
        WindowKey::new(WindowId::new(window), SessionGeneration::new(1))
    }

    fn task(app_id: &str, active: bool) -> TaskButton {
        TaskButton {
            key: key(1),
            app_id: app_id.to_owned(),
            windows: 1,
            label: "A window".to_owned(),
            active,
            minimized: false,
            enabled: true,
            disabled_reason: None,
            tooltip: format!("{app_id}: A window"),
        }
    }

    fn model(tasks: Vec<TaskButton>) -> PanelViewModel {
        PanelViewModel {
            status: PanelStatus::Ready,
            tasks,
            tray: Vec::new(),
            clock: None,
            notice: None,
            retry: None,
            focus_order: Vec::new(),
            focused: None,
        }
    }

    fn geometry(config: &DockConfig) -> DockGeometry {
        DockGeometry::new(&Tokens::builtin_dark(), config)
    }

    #[test]
    fn the_tile_is_the_configured_size() {
        let geometry = geometry(&DockConfig {
            icon_size: 64,
            ..DockConfig::default()
        });

        assert!((geometry.icon_size - 64.0).abs() < f32::EPSILON);
        assert_eq!(
            DockGeometry::new(&Tokens::builtin_dark(), &DockConfig::default()).icon_size,
            48.0,
            "the default tile is the 48 logical pixels the schema documents"
        );
    }

    #[test]
    fn the_pane_grows_by_one_tile_and_one_gap_per_tile() {
        let geometry = geometry(&DockConfig::default());
        let one = geometry.content_width(1, 0.0);
        let two = geometry.content_width(2, 0.0);

        assert!((one - (geometry.padding * 2.0 + geometry.icon_size)).abs() < f32::EPSILON);
        assert!(
            (two - one - geometry.icon_size - geometry.spacing).abs() < f32::EPSILON,
            "a second tile adds a tile and the gap before it"
        );
        assert!(
            geometry.content_width(0, 0.0) < one,
            "an empty pane is narrower than one with a tile in it"
        );
    }

    #[test]
    fn a_tray_group_widens_the_pane_by_its_own_width_and_the_separator() {
        let geometry = geometry(&DockConfig::default());
        let without = geometry.content_width(2, 0.0);
        let with = geometry.content_width(2, 120.0);

        assert!(
            (with - without - (geometry.spacing * 2.0 + geometry.separator_width + 120.0)).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn the_pane_reserves_a_slot_for_the_active_mark_on_every_tile() {
        let geometry = geometry(&DockConfig::default());

        assert!(geometry.indicator_height > 0.0);
        assert!(geometry.indicator_width > 0.0);
        assert!(
            geometry.indicator_width < geometry.icon_size,
            "the mark is smaller than the tile it belongs to"
        );
        assert!(
            geometry.indicator_gap > 0.0,
            "the mark is not drawn on top of the tile"
        );
        // The pane's height is one row of tiles plus the pane's own padding; the
        // mark's slot is inside that, which is what keeps the pane's height the
        // same whether anything is running or not.
        let expected = geometry.padding * 2.0
            + geometry.icon_size
            + geometry.indicator_gap
            + geometry.indicator_height;

        assert!((geometry.content_height() - expected).abs() < f32::EPSILON);
        assert!(
            (geometry.pane_height() - expected - geometry.edge_height * 2.0).abs() < f32::EPSILON,
            "the highlight and the shade are part of the pane, not of its content"
        );
    }

    #[test]
    fn the_tray_labels_are_derived_from_the_tile_and_fit_inside_the_pane() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&DockConfig::default());

        assert!(
            (geometry.tray_size - theme::caption_size(&tokens, 48)).abs() < f32::EPSILON,
            "the dock's unit is its tile, so the caption role comes from it"
        );
        assert!(geometry.tray_size > 0.0);
        assert!(geometry.tray_size <= geometry.content_height());
        assert!(
            geometry.glyph_size <= geometry.icon_size,
            "a badge glyph is drawn inside its tile"
        );
    }

    #[test]
    fn the_window_is_the_pane_plus_the_margin_below_it() {
        let geometry = geometry(&DockConfig {
            margin: 16,
            ..DockConfig::default()
        });

        assert!((geometry.window_height() - geometry.pane_height() - 16.0).abs() < f32::EPSILON);
    }

    /// Every sentence about the panel's own state, with the action it offers.
    fn notices(localizer: &Localizer) -> Vec<(String, Option<String>)> {
        vec![
            (localizer.message("panel-state-starting"), None),
            (localizer.message("panel-state-empty"), None),
            (
                localizer.message("panel-service-unavailable"),
                Some(localizer.message("panel-service-retry")),
            ),
        ]
    }

    #[test]
    fn the_minimum_width_is_a_documented_number_of_spacing_steps() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&DockConfig::default());

        assert!(
            (geometry.minimum_width - f32::from(tokens.spacing.xl) * MINIMUM_WIDTH_STEPS).abs()
                < f32::EPSILON,
            "the floor is a token-derived number of steps, not a pixel size"
        );
        assert!(
            geometry.minimum_width > geometry.content_width(0, 0.0),
            "the empty pane's padding alone is not a readable width"
        );
    }

    #[test]
    fn the_minimum_width_holds_every_sentence_the_dock_can_show_in_both_catalogs() {
        let geometry = geometry(&DockConfig::default());

        for (tag, advance) in [("en-US", AVERAGE_ADVANCE_RATIO), ("zh-CN", 1.0)] {
            for (notice, retry) in notices(&localizer(tag)) {
                // The worst case for a full-width (CJK) catalog is one em per
                // character, which is twice the documented estimate; the floor
                // has to hold that too, or a Chinese sentence would be clipped
                // where the English one is not.
                let sentence = notice.chars().count() as f32 * geometry.tray_size * advance;
                let action = retry.as_ref().map_or(0.0, |retry| {
                    retry.chars().count() as f32 * geometry.tray_size * advance
                });

                let needed = sentence.min(geometry.notice_max_width())
                    + if retry.is_some() {
                        geometry.spacing + geometry.padding * 2.0 + action
                    } else {
                        0.0
                    };

                assert!(
                    needed <= geometry.minimum_width,
                    "{tag}: {notice:?} needs {needed} logical pixels, the floor is {}",
                    geometry.minimum_width
                );
            }
        }
    }

    #[test]
    fn a_sentence_is_never_laid_out_narrower_than_its_slot_and_grows_past_it() {
        let geometry = geometry(&DockConfig::default());
        let slot = geometry.notice_slot_width();
        let short = geometry.notice_width("No windows open", None);
        let ordinary = geometry.notice_width("The window service is not available", None);
        // Far longer than any catalog sentence, so the estimate is what shows.
        let long = geometry.notice_width(&"an unusually long sentence ".repeat(8), None);

        assert!(slot > 0.0);
        assert!(
            (short - slot).abs() < f32::EPSILON,
            "a short sentence still gets the whole slot, so the dock does not resize between states"
        );
        assert!(
            (ordinary - slot).abs() < f32::EPSILON,
            "and neither does the longest catalog sentence"
        );
        assert!(
            long > slot,
            "but a sentence wider than the slot is given the room it needs"
        );
        assert!(
            geometry.notice_width("The window service is not available", Some("Retry")) >= ordinary,
            "an offered action is part of what the sentence needs"
        );
        assert!(
            geometry.notice_width("", None) >= slot,
            "a sentence is never given less than the slot, even an empty one"
        );
    }

    #[test]
    fn the_sentence_estimate_follows_the_font_scale() {
        let mut tokens = Tokens::builtin_dark();
        let plain = DockGeometry::new(&tokens, &DockConfig::default());
        tokens.typography.scale *= 2.0;
        let scaled = DockGeometry::new(&tokens, &DockConfig::default());
        let sentence = "The window service is not available";

        assert!(
            scaled.tray_size > plain.tray_size,
            "the dock's type scale comes from the tile and the font scale"
        );
        assert!(
            scaled.notice_width(sentence, None) > plain.notice_width(sentence, None),
            "at 200 % text the sentence needs more room, not the same room"
        );
        assert!(
            scaled.surface_width(DockContent {
                tiles: 0,
                tray_width: 0.0,
                notice_width: scaled.notice_width(sentence, None),
            }) > plain.surface_width(DockContent {
                tiles: 0,
                tray_width: 0.0,
                notice_width: plain.notice_width(sentence, None),
            }),
            "and the pane a sentence asks for follows it, so the reservation scales with the text \
             rather than clipping it"
        );
    }

    #[test]
    fn a_tray_label_is_estimated_at_most_as_wide_as_the_layout_draws_it() {
        let geometry = geometry(&DockConfig::default());
        let long = "An application name that is far longer than the pane can show";

        assert!(
            geometry.text_width(long.chars().count(), geometry.tray_size)
                > geometry.label_max_width,
            "the estimate would exceed the bound the layout clamps with"
        );
        assert!(
            (geometry.tray_width([long]) - geometry.label_max_width).abs() < f32::EPSILON,
            "so the group is estimated at the bound, exactly as it is drawn"
        );
    }

    #[test]
    fn more_and_longer_tray_labels_make_the_group_wider() {
        let geometry = geometry(&DockConfig::default());
        let one = geometry.tray_width(["Mail"]);
        let two = geometry.tray_width(["Mail", "Music"]);
        let longer = geometry.tray_width(["Mail", "Music and podcasts"]);

        assert!(one > 0.0);
        assert!(two > one, "a second item adds its own width and a gap");
        assert!(longer > two, "and a longer label adds the room it needs");
        assert!(
            (geometry.tray_width(Vec::<&str>::new()) - 0.0).abs() < f32::EPSILON,
            "no items is no group"
        );
    }

    #[test]
    fn the_surface_asks_for_the_row_it_draws() {
        let geometry = geometry(&DockConfig::default());
        let sentence = geometry.notice_width("The window service is not available", Some("Retry"));
        let alone = DockContent {
            tiles: 0,
            tray_width: 0.0,
            notice_width: sentence,
        };
        let twelve = DockContent {
            tiles: 12,
            tray_width: 0.0,
            notice_width: 0.0,
        };

        assert!(
            (geometry.surface_width(alone) - geometry.minimum_width).abs() < f32::EPSILON,
            "a dock with nothing but its own sentence asks for exactly the documented minimum"
        );
        assert!(
            (geometry.surface_width(twelve) - geometry.content_width(12, 0.0)).abs() < f32::EPSILON,
            "and otherwise the row is exactly what the tiles and the tray ask for"
        );
        assert!(
            geometry.surface_width(alone) >= sentence,
            "the pane is at least as wide as the sentence it has to show"
        );
        assert!(
            geometry.surface_width(DockContent {
                tiles: 12,
                notice_width: sentence,
                ..twelve
            }) > geometry.surface_width(twelve),
            "a sentence beside the tiles widens the row it is part of"
        );
        assert!(
            (geometry.surface_width(DockContent::default()) - geometry.padding * 2.0).abs()
                < f32::EPSILON,
            "a dock with nothing at all draws nothing but its padding, and asks for no more"
        );
    }

    #[test]
    fn the_content_of_a_view_is_the_projection_the_dock_draws() {
        let geometry = geometry(&DockConfig::default());
        let mut showing = model(vec![task("firefox", true), task("terminal", false)]);
        showing.notice = Some("Waiting for the window service".to_owned());
        showing.tray = vec![TrayItemView {
            id: crate::tray::TrayItemId::new(":1.1", "/StatusNotifierItem"),
            label: "Mail".to_owned(),
            status: "active".to_owned(),
            attention: false,
            tooltip: "Mail (active)".to_owned(),
        }];

        let view = DockView::project(&showing, &DockConfig::default(), 12, &english());
        let content = geometry.content(&view);

        assert_eq!(content.tiles, 2, "one entry per tile the dock draws");
        assert!(
            (content.tray_width - geometry.tray_width(["Mail"])).abs() < f32::EPSILON,
            "the tray estimate is the labels' estimate"
        );
        assert!(
            (content.notice_width - geometry.notice_width("Waiting for the window service", None))
                .abs()
                < f32::EPSILON,
            "the sentence estimate is the notice's"
        );

        let empty = DockView::project(&model(Vec::new()), &DockConfig::default(), 12, &english());
        assert_eq!(geometry.content(&empty), DockContent::default());
    }

    #[test]
    fn the_separator_is_a_hairline_that_is_shorter_than_a_tile() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&DockConfig::default());

        assert!((geometry.separator_width - theme::hairline(&tokens)).abs() < f32::EPSILON);
        assert!(geometry.separator_height > 0.0);
        assert!(geometry.separator_height <= geometry.icon_size);
    }

    #[test]
    fn the_tooltips_open_upwards_because_the_dock_is_at_the_bottom() {
        assert_eq!(
            geometry(&DockConfig::default()).tooltip_position,
            TooltipPosition::Above
        );
    }

    #[test]
    fn one_tile_per_task_group() {
        let view = DockView::project(
            &model(vec![task("firefox", false), task("terminal", true)]),
            &DockConfig::default(),
            12,
            &english(),
        );

        assert_eq!(view.tiles.len(), 2);
        assert_eq!(view.tiles[0].glyph, "F");
        assert!(!view.tiles[0].active);
        assert_eq!(view.tiles[1].glyph, "T");
        assert!(view.tiles[1].active);
        assert!(!view.is_empty());
    }

    #[test]
    fn the_configured_cap_bounds_the_tiles_that_are_drawn() {
        let view = DockView::project(
            &model(vec![
                task("firefox", false),
                task("terminal", false),
                task("editor", false),
            ]),
            &DockConfig::default(),
            2,
            &english(),
        );

        assert_eq!(
            view.tiles.len(),
            2,
            "the dock shows what `panel.max_task_buttons` allows and clips the rest"
        );
        assert_eq!(view.tiles[0].glyph, "F");
        assert_eq!(view.tiles[1].glyph, "T");
    }

    #[test]
    fn an_application_without_a_name_is_drawn_with_the_placeholder() {
        let view = DockView::project(
            &model(vec![task("", false)]),
            &DockConfig::default(),
            12,
            &english(),
        );

        assert_eq!(view.tiles[0].glyph, "?");
        assert!(view.tiles[0].placeholder);

        let chinese = DockView::project(
            &model(vec![task("", false)]),
            &DockConfig::default(),
            12,
            &localizer("zh-CN"),
        );
        assert_eq!(chinese.tiles[0].glyph, "？");

        let named = DockView::project(
            &model(vec![task("firefox", false)]),
            &DockConfig::default(),
            12,
            &english(),
        );
        assert!(!named.tiles[0].placeholder);
    }

    #[test]
    fn an_empty_session_draws_nothing_but_its_own_state() {
        let empty = model(Vec::new());
        let view = DockView::project(&empty, &DockConfig::default(), 12, &english());

        assert!(view.tiles.is_empty());
        assert!(view.tray.is_empty());
        assert!(view.is_empty());
        assert!(!view.has_separator());
    }

    #[test]
    fn a_disabled_tile_carries_the_reason_as_its_tooltip() {
        let mut disabled = task("firefox", false);
        disabled.enabled = false;
        disabled.disabled_reason = Some("The window service is not available".to_owned());

        let view = DockView::project(
            &model(vec![disabled]),
            &DockConfig::default(),
            12,
            &english(),
        );

        assert!(!view.tiles[0].enabled);
        assert_eq!(
            view.tiles[0].tooltip, "The window service is not available",
            "a disabled tile says why (D05)"
        );
    }

    #[test]
    fn the_tray_group_is_shown_unless_the_configuration_hides_it() {
        let mut model = model(vec![task("firefox", true)]);
        model.tray = vec![TrayItemView {
            id: crate::tray::TrayItemId::new(":1.1", "/StatusNotifierItem"),
            label: "Mail".to_owned(),
            status: "active".to_owned(),
            attention: false,
            tooltip: "Mail (active)".to_owned(),
        }];

        let showing = DockView::project(&model, &DockConfig::default(), 12, &english());
        assert_eq!(showing.tray.len(), 1);
        assert!(showing.has_separator());

        let hidden = DockView::project(
            &model,
            &DockConfig {
                show_tray: false,
                ..DockConfig::default()
            },
            12,
            &english(),
        );
        assert!(hidden.tray.is_empty());
        assert!(
            !hidden.has_separator(),
            "a separator with nothing after it separates nothing"
        );
    }

    #[test]
    fn the_notice_and_its_retry_come_from_the_shared_projection() {
        let mut model = model(Vec::new());
        model.notice = Some("Waiting for the window service".to_owned());
        model.retry = Some("Retry".to_owned());

        let view = DockView::project(&model, &DockConfig::default(), 12, &english());

        assert_eq!(
            view.notice.as_deref(),
            Some("Waiting for the window service")
        );
        assert_eq!(view.retry.as_deref(), Some("Retry"));
        assert!(!view.is_empty());
    }

    #[test]
    fn the_panel_navigation_focus_marks_the_tile_it_is_on() {
        let mut model = model(vec![task("firefox", false)]);
        model.focused = Some(key(1));

        let focused = DockView::project(&model, &DockConfig::default(), 12, &english());
        assert!(focused.tiles[0].focused);

        model.focused = None;
        let unfocused = DockView::project(&model, &DockConfig::default(), 12, &english());
        assert!(!unfocused.tiles[0].focused);
    }
}
