//! The panel's surface, projected as the Qt-free JSON the QML renderer draws.
//!   that could hold on to focus.
//! Everything here is layout and drawing data, and nothing here is Qt: the
//! projection is a `serde_json::Value` of rectangles and text on absolute
//! coordinates, handed to the offscreen QML renderer one frame at a time
//!. That is what keeps QML a pure renderer: it contains
//! no colour, no size and no sentence — all three arrive from Rust, which is
//! also where the pointer is matched against the layout ([`Scene::entry_at`]).
//!
//! Which entries exist, what they say, and why one of them is disabled all
//! arrive already decided in a [`crate::view_model::PanelViewModel`]; this
//! module turns that into a scene and takes every colour, size and radius from
//! the tokens through [`crate::theme`].
//!
//! [`scene`] is the entry point the protocol leg calls: it picks the scene
//! `[panel] surface` asks for and hands each one the section that describes it.
//! The taskbar is [`panel`]; the menu bar is [`menubar::menubar`] and the dock
//! is [`dock::dock`], both of which reuse the entry painters below so that
//! three surfaces can never draw "disabled" or "hovered" two different ways.
//!
//! The taskbar strip is drawn as **groups separated by a gap wider than the
//! entries inside them** (`docs/quality/ui-guidelines.md` §3), so a user can see
//! at a glance what belongs together:
//!
//! ```text
//! [ notice + retry ]   [ task ][ task ][ task ]        │  [tray]   [clock]
//! └──── group gap ────┘└── entry spacing ──┘            └ divider
//! ```
//!
//! Four layout decisions are worth naming, because they are what acceptance D01
//! and D05 are about:
//!
//! - **The clock and the tray keep their space.** The task buttons sit in a
//!   flexible area that is laid out last, so an application with a very long
//!   title is clipped out of it instead of pushing a key action off the end of
//!   the strip.
//! - **The panel is one row of the configured height.** A taller panel gets
//!   taller entries, not a second row; the entries are inset vertically so a
//!   rounded, pressed or focused entry stays inside the strip.
//! - **Nothing on the strip takes keyboard focus by itself.** The task buttons
//!   are click targets; the clock and the tray items are plain text, and the
//!   panel's own navigation focus is drawn as a colour, never as a widget that
//!   could hold on to focus.
//! - **A disabled entry says why.** Its hover text is the reason the projection
//!   produced, not the application name the label already shows (D05).

pub mod dock;
pub mod menubar;

use inox_common::i18n::Localizer;
use inox_config::{PanelConfig, PanelPosition, PanelSurface, ShellConfig, Theme as Tokens};
use serde_json::{Value, json};

use crate::layer::Button;
use crate::tasks::{WindowKey, WindowMode, WorkspaceId};
use crate::theme::{self, EntryStyle, InteractionStatus, PaneStyle, TaskEmphasis};
use crate::view_model::{
    PanelStatus, PanelViewModel, TaskButton, TrayItemView, WorkspaceIndicator,
};

/// How many extra-large spacing steps a task label may be wide.
///
/// The token set has no width for a control, so the bound is expressed in the
/// spacing scale: six extra-large steps is 192 logical pixels at the default
/// tokens, which fits a dozen buttons on a 1280-wide panel and still leaves the
/// clock its space. It is a *maximum*, drawn from the tokens rather than written
/// as a number, and the layout clips anything longer (D05).
const LABEL_WIDTH_STEPS: f32 = 6.0;

/// How many extra-large spacing steps the panel's own sentence may be wide.
///
/// The sentence about the panel's state is more important than any single
/// entry, so it is allowed to be wider than one: eight steps is 256 logical
/// pixels, and everything after it is still reachable.
const NOTICE_WIDTH_STEPS: f32 = 8.0;

/// The ratio of an average glyph's advance to the font size.
///
/// The renderer measures text, but the *layout* has to place the clock and the
/// tray before a frame exists, so widths are estimated exactly as
/// [`crate::dock::DockGeometry::text_width`] estimates them and as the launcher
/// budgets a result row's label: about half an em per glyph. It is a lower bound
/// for the two locales the panel ships, and it is deliberately the same number
/// in both places — an estimate that disagreed with itself would make the pane
/// a different width from the row it holds.
pub const AVERAGE_ADVANCE_RATIO: f32 = 0.5;

/// Where a tooltip would open, so that it is not drawn past a screen edge.
///
/// iced carried this type; the panel projects its own so that no toolkit type
/// survives in the crate's pure data (`src/dock.rs`, `src/menubar.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooltipPosition {
    /// Away from an edge the panel occupies at the bottom.
    Above,
    /// Away from an edge the panel occupies at the top.
    Below,
}

/// The geometry of the panel strip, derived from the tokens and the configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelGeometry {
    /// Height of the strip, in logical pixels.
    pub height: f32,
    /// Padding at the left and right end of the strip.
    pub padding: f32,
    /// Vertical inset of an entry inside the strip, so an entry's rounded
    /// corners and its focus ring are drawn *inside* the strip.
    pub inset: f32,
    /// Gap between two entries of the same group.
    pub spacing: f32,
    /// Gap between two groups: the notice, the tasks, the tray and the clock.
    pub group_gap: f32,
    /// Padding inside one entry.
    pub entry_padding: f32,
    /// Widest a label may be before the layout clips it.
    pub label_max_width: f32,
    /// Text size of a task label, already scaled by the user's font preference
    /// and clamped so scaled text never grows past the strip.
    pub font_size: f32,
    /// Text size of secondary information: the clock, a tray item, the ordinary
    /// states of the panel's own sentence.
    pub caption_size: f32,
    /// Thickness of the divider between the task area and the tray.
    pub divider_width: f32,
    /// Height of that divider: as tall as a line of secondary information.
    pub divider_height: f32,
    /// Which way a tooltip opens, so that it is not drawn past the screen edge.
    pub tooltip_position: TooltipPosition,
}

impl PanelGeometry {
    /// Derives the strip's geometry.
    pub fn new(tokens: &Tokens, config: &PanelConfig) -> Self {
        let height = f32::from(config.height);
        let padding = f32::from(tokens.spacing.md);
        let inset = f32::from(tokens.spacing.xs);
        let caption = theme::caption_size(tokens, config.height);
        // Text that is taller than the strip would overflow it, so the scaled
        // size is clamped to what is left after the inset.
        let available = (height - inset * 2.0).max(1.0);

        Self {
            height,
            padding,
            inset,
            spacing: f32::from(tokens.spacing.sm),
            group_gap: f32::from(tokens.spacing.lg),
            entry_padding: padding,
            label_max_width: f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS,
            font_size: theme::font_size(tokens, config.height).min(available),
            caption_size: caption.min(available),
            divider_width: theme::hairline(tokens),
            divider_height: caption.min(available),
            tooltip_position: match config.position {
                PanelPosition::Bottom => TooltipPosition::Above,
                PanelPosition::Top => TooltipPosition::Below,
            },
        }
    }

    /// The height one entry has inside the strip.
    ///
    /// This is the room scaled text has to fit in, which is what the unit tests
    /// assert the font size against.
    pub fn content_height(&self) -> f32 {
        (self.height - self.inset * 2.0).max(1.0)
    }

    /// The sizes the strip's own notice is drawn at.
    fn notice(&self) -> NoticeMetrics {
        NoticeMetrics {
            font_size: self.font_size,
            caption_size: self.caption_size,
            entry_padding: self.entry_padding,
            group_gap: self.group_gap,
            max_width: self.label_max_width * NOTICE_WIDTH_STEPS,
        }
    }
}

/// The sizes a group of display-only entries (the tray, the clock) is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct EntryMetrics {
    /// Text size of a secondary line.
    pub caption_size: f32,
    /// Widest a label may be before the layout clips it.
    pub label_max_width: f32,
    /// Gap between two entries of the group.
    pub spacing: f32,
    /// Which way a tooltip opens.
    pub tooltip_position: TooltipPosition,
}

/// The sizes the panel's own notice is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct NoticeMetrics {
    /// Text size of the action the notice offers.
    pub font_size: f32,
    /// Text size of the ordinary states of the sentence.
    pub caption_size: f32,
    /// Padding inside the retry action.
    pub entry_padding: f32,
    /// Gap between the sentence and its action.
    pub group_gap: f32,
    /// Widest the sentence may be before the layout clips it.
    pub max_width: f32,
}

/// What a click on an entry asks the panel to do.
///
/// The projection decides it, [`crate::layer`] carries it out. Left clicks keep
/// the semantics the taskbar has always had — [`EntryAction::Activate`] on a
/// window that is not focused, [`EntryAction::Toggle`] on the one that is (D03)
/// — and the two new verbs are the other mouse buttons'
/// ([`Entry::action_for`]): a middle click asks the application to close,
/// a right click asks for the next window mode. The workspace indicator asks for
/// the next workspace, and the retry action asks the window service again (D05).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryAction {
    /// Bring this window forward.
    Activate(WindowKey),
    /// Put this window away.
    Toggle(WindowKey),
    /// Ask the application to close this window.
    Close(WindowKey),
    /// Ask for `mode` on this window: one step through the cycle.
    CycleMode(WindowKey, WindowMode),
    /// Show this workspace.
    ///
    /// Its own variant rather than a window's: a workspace switch names no
    /// window, and reusing a window variant would switch a workspace when the
    /// user clicked a task.
    SwitchWorkspace(WorkspaceId),
    /// Reach the window service again.
    Retry,
}

/// What the pointer is doing, as the projection needs to know it.
///
/// The pointer is owned by [`crate::layer`] — the only thing that sees
/// `wl_pointer` — and this is the whole of what it hands to a projection: which
/// entry it rests on and which one it is pressing. The projection turns that
/// into an [`InteractionStatus`], so a hovered or pressed entry is drawn by the
/// same code, under the same state table, that draws a resting one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Interaction {
    /// The entry the pointer rests on, if any.
    pub hovered: Option<EntryAction>,
    /// The entry a button is currently pressed on, if any.
    pub pressed: Option<EntryAction>,
}

impl Interaction {
    /// How `action` is drawn under this interaction.
    ///
    /// A disabled entry is `Disabled` whether or not the pointer is on it: the
    /// state table says it has no fill and no outline, and clicking it does
    /// nothing (D05).
    pub fn status(&self, action: EntryAction, enabled: bool) -> InteractionStatus {
        if !enabled {
            InteractionStatus::Disabled
        } else if self.pressed == Some(action) {
            InteractionStatus::Pressed
        } else if self.hovered == Some(action) {
            InteractionStatus::Hovered
        } else {
            InteractionStatus::Active
        }
    }
}

/// One clickable area of the scene, in logical surface pixels.
///
/// A plain value: routing a pointer event only needs the geometry and the action,
/// so the input half copies entries out of a scene instead of borrowing the whole
/// projection for as long as it holds one.
///
/// An entry carries one action per mouse button, because the panel's controls do
/// more than one thing: a left click on a task activates or minimizes it, a
/// middle click asks the application to close, and a right click asks for the
/// next mode. The three are decided *here*, by the projection that knows the
/// window's state, so [`crate::layer`] never has to look a window up to know what
/// a click meant — and a button the entry has no meaning for is `None` rather
/// than an invented behaviour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    /// What a left click means.
    pub action: EntryAction,
    /// What a middle click means, when it means anything.
    pub middle: Option<EntryAction>,
    /// What a right click means, when it means anything.
    pub right: Option<EntryAction>,
    /// Whether the entry accepts a click at all.
    pub enabled: bool,
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Entry {
    /// An entry whose only meaning is its left click.
    pub const fn new(
        action: EntryAction,
        enabled: bool,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Self {
        Self {
            action,
            middle: None,
            right: None,
            enabled,
            x,
            y,
            width,
            height,
        }
    }

    /// What a press of `button` on this entry means.
    ///
    /// A side button, or a button this entry has no meaning for, is `None`: the
    /// panel has no use for "back" or "forward" on a task strip, and ignoring the
    /// press is not an error.
    pub fn action_for(self, button: Button) -> Option<EntryAction> {
        match button {
            Button::Left => Some(self.action),
            Button::Middle => self.middle,
            Button::Right => self.right,
            Button::Back | Button::Forward => None,
        }
    }

    /// Whether `(x, y)` is inside this entry.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// One frame's worth of drawing data: the JSON the QML renderer draws and the
/// entries a pointer can be matched against.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// Which surface this scene draws.
    pub surface: PanelSurface,
    /// The projection: `{ font_family, background, items: [ … ] }`.
    pub json: Value,
    /// The clickable areas, in the order they are drawn.
    pub entries: Vec<Entry>,
}

impl Scene {
    /// The entry under `(x, y)`, topmost last — what a pointer press means.
    ///
    /// Only enabled entries answer: a disabled entry's reason is its tooltip, and
    /// clicking it must do nothing at all (D05).
    pub fn entry_at(&self, x: f32, y: f32) -> Option<&Entry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.enabled && entry.contains(x, y))
    }
}

/// Projects the surface `shell.panel.surface` asks for at `width` × `height`.
///
/// The three surfaces present the same model; what changes is the scene, the
/// section of the configuration that describes it and, for the glass surfaces,
/// the material behind it. The localizer comes along because the menu bar names
/// the focused application and the dock badges it, and both fall back to a
/// catalog message for an application that has no name.
#[allow(clippy::too_many_arguments)]
pub fn scene(
    model: &PanelViewModel,
    tokens: &Tokens,
    shell: &ShellConfig,
    localizer: &Localizer,
    width: f32,
    height: f32,
    interaction: &Interaction,
) -> Scene {
    match shell.panel.surface {
        PanelSurface::Taskbar => panel(model, tokens, &shell.panel, width, height, interaction),
        PanelSurface::Menubar => {
            menubar::menubar(model, tokens, &shell.menubar, localizer, width, interaction)
        }
        PanelSurface::Dock => dock::dock(
            model,
            tokens,
            &shell.panel,
            &shell.dock,
            localizer,
            width,
            interaction,
        ),
    }
}

/// Draws the panel's taskbar strip.
pub fn panel(
    model: &PanelViewModel,
    tokens: &Tokens,
    config: &PanelConfig,
    width: f32,
    height: f32,
    interaction: &Interaction,
) -> Scene {
    let geometry = PanelGeometry::new(tokens, config);
    let mut painter = Painter::new(PanelSurface::Taskbar, tokens);

    painter.rect(0.0, 0.0, width, height, &theme::panel_fill(tokens), 0.0);

    let top = geometry.inset;
    let content_height = (height - geometry.inset * 2.0).max(1.0);

    // The notice opens the strip, and it is laid out first because it is what a
    // user has to read before anything else (D05).
    let mut left = geometry.padding;

    if let Some(notice) = &model.notice {
        let slot = painter.notice(
            notice,
            model.retry.as_deref(),
            model.status,
            tokens,
            geometry.notice(),
            left,
            top,
            content_height,
            interaction,
        );

        left += slot + geometry.group_gap;
    }

    // The clock and the tray are laid out from the right edge inwards, which is
    // what keeps them where they belong whatever the task area does (D05).
    let mut right = width - geometry.padding;

    if let Some(clock) = &model.clock {
        let text_width =
            estimate_width(&clock.text, geometry.caption_size).min(geometry.label_max_width);

        painter.text(
            right - text_width,
            top,
            text_width,
            content_height,
            &clock.text,
            &theme::clock_text(tokens),
            geometry.caption_size,
            false,
            Align::Left,
        );

        right -= text_width + geometry.group_gap;
    }

    if !model.tray.is_empty() {
        let mut cursor = right;

        for item in model.tray.iter().rev() {
            let text_width =
                estimate_width(&item.label, geometry.caption_size).min(geometry.label_max_width);

            cursor -= text_width;

            painter.text(
                cursor,
                top,
                text_width,
                content_height,
                &item.label,
                &theme::tray_text(tokens, item.attention),
                geometry.caption_size,
                false,
                Align::Left,
            );

            cursor -= geometry.entry_padding;
        }

        right = cursor;

        // The divider separates what the user acts on from what the panel
        // reports, and it opens the group it belongs to.
        right -= geometry.divider_width;

        painter.rect(
            right,
            top + (content_height - geometry.divider_height) / 2.0,
            geometry.divider_width,
            geometry.divider_height,
            &theme::divider_color(tokens),
            0.0,
        );

        right -= geometry.group_gap;
    }

    // The workspace indicator opens the task area: it says which workspace the
    // windows below belong to, so it is read before them (ADR 0045 §4).
    if let Some(indicator) = &model.workspace {
        let slot = painter.workspace_indicator(
            indicator,
            tokens,
            &geometry,
            left,
            top,
            content_height,
            interaction,
        );

        left += slot + geometry.group_gap;
    }

    // The task area is what is left: fluid, clipped, laid out last.
    let area_width = (right - left).max(0.0);
    let mut cursor = left;

    for task in &model.tasks {
        let label_width =
            estimate_width(&task.label, geometry.font_size).min(geometry.label_max_width);
        let entry_width = label_width + geometry.entry_padding * 2.0;

        if cursor + entry_width > left + area_width {
            // The strip is full: the rest is clipped, exactly as the old widget
            // tree's `clip` did, so a long list never pushes the clock away.
            break;
        }

        task_view(
            &mut painter,
            task,
            model.focused == Some(task.key),
            tokens,
            &geometry,
            cursor,
            top,
            entry_width,
            content_height,
            interaction,
        );

        cursor += entry_width + geometry.spacing;
    }

    painter.finish(width, height)
}

/// One task button.
///
/// A click on the application the user is already looking at is the request to
/// put it away, which is what a taskbar's active button does (D03).
#[allow(clippy::too_many_arguments)]
fn task_view(
    painter: &mut Painter,
    task: &TaskButton,
    focused: bool,
    tokens: &Tokens,
    geometry: &PanelGeometry,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    interaction: &Interaction,
) {
    let emphasis = if task.active {
        TaskEmphasis::Active
    } else if task.minimized {
        TaskEmphasis::Minimized
    } else {
        TaskEmphasis::Idle
    };
    let action = if task.active {
        EntryAction::Toggle(task.key)
    } else {
        EntryAction::Activate(task.key)
    };
    let style = theme::task_button(
        tokens,
        emphasis,
        focused,
        interaction.status(action, task.enabled),
    );

    painter.entry(&style, x, y, width, height);
    painter.text(
        x + geometry.entry_padding,
        y,
        (width - geometry.entry_padding * 2.0).max(0.0),
        height,
        &task.label,
        &style.text_color,
        geometry.font_size,
        false,
        Align::Left,
    );

    // The three buttons a task entry answers: left keeps the taskbar's
    // activate/minimize semantics, middle asks the application to close, right
    // asks for the next mode. All three are decided here, from the state the
    // button was drawn from, so the input path never has to look a window up.
    painter.entries.push(Entry {
        action,
        middle: Some(EntryAction::Close(task.key)),
        right: Some(EntryAction::CycleMode(task.key, task.mode.cycled())),
        enabled: task.enabled,
        x,
        y,
        width,
        height,
    });
}

/// The width a label is laid out at, before the layout clips it.
pub fn estimate_width(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * size * AVERAGE_ADVANCE_RATIO
}

/// Where a line of text sits inside its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Align {
    /// Against the box's left edge.
    Left,
    /// Centred in the box.
    Center,
}

impl Align {
    /// The value the QML renderer reads.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
        }
    }
}

/// One drawing command, in **every** field the QML delegate binds on it.
///
/// [`Painter::rect`] and [`Painter::text`] both go through here, and that is the
/// point: the delegate in `qml/Main.qml` reads the same property names off both
/// kinds of item — `radius`, `border` and `border_color` off a line of text, and
/// `text`, `size`, `bold` and `align` off a rectangle — because a renderer that
/// branched on `kind` would be making a decision (contract §4). So a field an
/// item's kind does not draw carries the structural neutral (`0`, no outline
/// colour, an empty sentence, the left edge) rather than being *absent*. Absent
/// is what made Qt log `Unable to assign [undefined] to
/// double/QColor/QString/int/bool` once per binding, per item, per frame.
///
/// The complete set is pinned by this module's own test,
/// `every_drawing_command_carries_every_field_the_renderer_binds`.
#[allow(clippy::too_many_arguments)]
fn rectangle(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: &str,
    radius: f32,
    border: f32,
    border_color: &str,
) -> Value {
    json!({
        "kind": "rect",
        "x": x,
        "y": y,
        "w": width.max(0.0),
        "h": height.max(0.0),
        "color": color,
        "radius": radius,
        "border": border,
        "border_color": border_color,
        // A rectangle draws no glass edge: the pane fields of [`Painter::pane`],
        // at their neutral values.
        "highlight": theme::TRANSPARENT,
        "shade": theme::TRANSPARENT,
        "hairline": 0.0,
        "highlighted": false,
        "shaded": false,
        // A rectangle draws no sentence: the text fields of [`line`], at their
        // neutral values.
        "text": "",
        "size": 0.0,
        "bold": false,
        "align": Align::Left.as_str(),
    })
}

/// One line of text, in **every** field the QML delegate binds on it.
///
/// The counterpart of [`rectangle`]: a line of text draws no fill and no outline
/// of its own, so those fields hold the neutral values the delegate expects.
#[allow(clippy::too_many_arguments)]
fn line(
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    text: &str,
    color: &str,
    size: f32,
    bold: bool,
    align: Align,
) -> Value {
    json!({
        "kind": "text",
        "x": x,
        "y": y,
        "w": width.max(0.0),
        "h": height.max(0.0),
        "color": color,
        "text": text,
        "size": size,
        "bold": bold,
        "align": align.as_str(),
        // A line of text has no fill and no outline: the rectangle fields of
        // [`rectangle`], at their neutral values.
        "radius": 0.0,
        "border": 0.0,
        "border_color": theme::TRANSPARENT,
        // Nor does it draw a glass edge: the pane fields, likewise neutral.
        "highlight": theme::TRANSPARENT,
        "shade": theme::TRANSPARENT,
        "hairline": 0.0,
        "highlighted": false,
        "shaded": false,
    })
}

/// Turns the model into JSON drawing commands and clickable areas.
pub(super) struct Painter {
    surface: PanelSurface,
    font_family: String,
    items: Vec<Value>,
    entries: Vec<Entry>,
}

impl Painter {
    fn new(surface: PanelSurface, tokens: &Tokens) -> Self {
        Self {
            surface,
            font_family: theme::font_family(tokens),
            items: Vec::new(),
            entries: Vec::new(),
        }
    }

    /// A filled rectangle with an optional radius.
    fn rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: &str, radius: f32) {
        self.items.push(rectangle(
            x,
            y,
            width,
            height,
            color,
            radius,
            0.0,
            theme::TRANSPARENT,
        ));
    }

    /// An entry's fill and outline, in the state the theme decided.
    fn entry(&mut self, style: &EntryStyle, x: f32, y: f32, width: f32, height: f32) {
        let background = style
            .background
            .clone()
            .unwrap_or_else(|| theme::TRANSPARENT.to_owned());

        self.items.push(rectangle(
            x,
            y,
            width,
            height,
            &background,
            style.radius,
            style.border_width,
            &style.border_color,
        ));
    }

    /// A frosted pane, drawn by the shared `GlassPane` component.
    ///
    /// The panel used to lay the pane out itself as a stack of solid bands, one
    /// per colour stop, because a renderer that branched on a gradient would be
    /// making a decision. `GlassPane` — one of `inox-glass`'s stateless
    /// components, compiled into this crate's own QML module by `build.rs` —
    /// draws the same material (the tint, one hairline highlight along the top
    /// edge, one hairline shade along the bottom) from the same tokens, so the
    /// bands became a component call without the panel gaining a drawing
    /// decision. It is the same component the dock, the menu bar and every later
    /// consumer draws with (ADR 0040 §3).
    ///
    /// The kind is `pane` rather than a rectangle with a colour: the renderer has
    /// to instantiate a component, and the geometry it needs (radius, hairline,
    /// both edge flags) is what [`crate::theme::PaneStyle`] decided.
    fn pane(&mut self, x: f32, y: f32, width: f32, height: f32, style: &PaneStyle) {
        self.items.push(json!({
            "kind": "pane",
            "x": x,
            "y": y,
            "w": width.max(0.0),
            "h": height.max(0.0),
            // The pane's fill is the tint; `color` carries it so the shared
            // helpers (`rects`, `color`) keep working on every filled item.
            "color": style.tint,
            "radius": style.radius,
            "border": 0.0,
            "border_color": theme::TRANSPARENT,
            // The two edge hairlines, which `GlassPane` draws along the pane's
            // own top and bottom edges rather than as extra rows of widgets.
            "highlight": style.highlight,
            "shade": style.shade,
            "hairline": style.hairline,
            "highlighted": style.highlighted,
            "shaded": style.shaded,
            // A pane draws no sentence: the text fields of [`line`], at their
            // neutral values.
            "text": "",
            "size": 0.0,
            "bold": false,
            "align": Align::Left.as_str(),
        }));
    }

    /// A line of text in a box it may be clipped to.
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        text: &str,
        color: &str,
        size: f32,
        bold: bool,
        align: Align,
    ) {
        self.items
            .push(line(x, y, width, height, text, color, size, bold, align));
    }

    /// The sentence the panel shows about itself, and its retry action.
    ///
    /// The panel's own state is what a user has to read first, so the state that
    /// needs action — the window service is not available — is written at the
    /// body size in the warning colour, while the ordinary states (starting,
    /// empty) are secondary information at the caption size in the muted colour.
    /// The sentence never claims more than the projection knows, and a retry is
    /// always offered when there is something to retry (D05: never an endless
    /// spinner).
    ///
    /// Returns the width the group took, so the caller can place the next group.
    ///
    /// The taskbar and the menu bar both draw this, so the panel's own state can
    /// never be worded, coloured or laid out two different ways.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn notice(
        &mut self,
        notice: &str,
        retry: Option<&str>,
        status: PanelStatus,
        tokens: &Tokens,
        metrics: NoticeMetrics,
        x: f32,
        y: f32,
        height: f32,
        interaction: &Interaction,
    ) -> f32 {
        let colour = theme::notice_text(tokens, status);
        let size = match status {
            PanelStatus::Unavailable => metrics.font_size,
            PanelStatus::Starting | PanelStatus::Empty | PanelStatus::Ready => metrics.caption_size,
        };
        let sentence_width = estimate_width(notice, size).min(metrics.max_width);

        self.text(
            x,
            y,
            sentence_width,
            height,
            notice,
            &colour,
            size,
            false,
            Align::Left,
        );

        let Some(retry) = retry else {
            return sentence_width;
        };

        let action_x = x + sentence_width + metrics.group_gap;
        let action_width = estimate_width(retry, metrics.font_size) + metrics.entry_padding * 2.0;
        let style = theme::notice_action(tokens, interaction.status(EntryAction::Retry, true));

        self.entry(&style, action_x, y, action_width, height);
        self.text(
            action_x + metrics.entry_padding,
            y,
            (action_width - metrics.entry_padding * 2.0).max(0.0),
            height,
            retry,
            &style.text_color,
            metrics.font_size,
            false,
            Align::Left,
        );
        self.entries.push(Entry::new(
            EntryAction::Retry,
            true,
            action_x,
            y,
            action_width,
            height,
        ));

        sentence_width + metrics.group_gap + action_width
    }

    /// The workspace indicator: the active workspace of the count, and the click
    /// that asks for the next one.
    ///
    /// It is drawn as an entry and a line of text — the same two primitives every
    /// other control uses — so the renderer needs no new kind and the panel makes
    /// no drawing decision of its own: the fill, the label colour, the outline and
    /// the radius all come from [`theme::workspace_indicator`].
    ///
    /// Returns the width the group took, so the caller can place the task area
    /// after it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn workspace_indicator(
        &mut self,
        indicator: &WorkspaceIndicator,
        tokens: &Tokens,
        geometry: &PanelGeometry,
        x: f32,
        y: f32,
        height: f32,
        interaction: &Interaction,
    ) -> f32 {
        let action = EntryAction::SwitchWorkspace(indicator.next);
        let style =
            theme::workspace_indicator(tokens, interaction.status(action, indicator.enabled));
        let width = estimate_width(&indicator.label, geometry.caption_size)
            .min(geometry.label_max_width)
            + geometry.entry_padding * 2.0;

        self.entry(&style, x, y, width, height);
        self.text(
            x + geometry.entry_padding,
            y,
            (width - geometry.entry_padding * 2.0).max(0.0),
            height,
            &indicator.label,
            &style.text_color,
            geometry.caption_size,
            false,
            Align::Center,
        );

        // The indicator answers the left button only: closing or re-moding a
        // workspace is not a verb, so the two other buttons mean nothing here.
        self.entries
            .push(Entry::new(action, indicator.enabled, x, y, width, height));

        width
    }

    /// The tray items, as one group: display-only, so they show a title and take
    /// no pointer or keyboard input.
    pub(super) fn tray_group(
        &mut self,
        items: &[TrayItemView],
        tokens: &Tokens,
        metrics: EntryMetrics,
        x: f32,
        y: f32,
        height: f32,
    ) -> f32 {
        let mut cursor = x;

        for item in items {
            let width =
                estimate_width(&item.label, metrics.caption_size).min(metrics.label_max_width);

            self.text(
                cursor,
                y,
                width,
                height,
                &item.label,
                &theme::tray_text(tokens, item.attention),
                metrics.caption_size,
                false,
                Align::Left,
            );

            cursor += width + metrics.spacing;
        }

        (cursor - metrics.spacing - x).max(0.0)
    }

    /// The width the same tray group will take, for a layout that has to reserve
    /// it before drawing the task area.
    pub(super) fn tray_width(items: &[TrayItemView], metrics: EntryMetrics) -> f32 {
        let widths: f32 = items
            .iter()
            .map(|item| {
                estimate_width(&item.label, metrics.caption_size).min(metrics.label_max_width)
            })
            .sum();

        if items.len() > 1 {
            widths + metrics.spacing * (items.len() - 1) as f32
        } else {
            widths
        }
    }

    /// Finishes the frame: the root's background and the projection QML reads.
    fn finish(mut self, width: f32, height: f32) -> Scene {
        // The root is *structurally* transparent on all three surfaces: the
        // taskbar paints its opaque strip as a rectangle, and the menu bar and
        // the dock are glass panes whose backdrop has to show through. That is
        // the "no design colour" sentinel `theme::TRANSPARENT`, not a token a
        // theme could set.
        let json = json!({
            "surface": surface_name(self.surface),
            "width": width,
            "height": height,
            "font_family": self.font_family,
            "background": theme::TRANSPARENT,
            "items": std::mem::take(&mut self.items),
        });

        Scene {
            surface: self.surface,
            json,
            entries: std::mem::take(&mut self.entries),
        }
    }
}

/// The name the renderer sees for a surface, so a QML-side diagnostic can say
/// which one it drew. It is a diagnostic label, not a translated string.
const fn surface_name(surface: PanelSurface) -> &'static str {
    match surface {
        PanelSurface::Taskbar => "taskbar",
        PanelSurface::Menubar => "menubar",
        PanelSurface::Dock => "dock",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(config: &PanelConfig) -> PanelGeometry {
        PanelGeometry::new(&Tokens::builtin_dark(), config)
    }

    fn model() -> PanelViewModel {
        PanelViewModel {
            status: PanelStatus::Ready,
            tasks: Vec::new(),
            workspace: None,
            tray: Vec::new(),
            clock: None,
            notice: None,
            retry: None,
            focus_order: Vec::new(),
            focused: None,
        }
    }

    #[test]
    fn the_strip_is_as_tall_as_the_configuration_says() {
        let geometry = geometry(&PanelConfig {
            height: 48,
            ..PanelConfig::default()
        });

        assert!((geometry.height - 48.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_label_bound_comes_from_the_spacing_tokens() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&PanelConfig::default());

        assert!(
            (geometry.label_max_width - f32::from(tokens.spacing.xl) * LABEL_WIDTH_STEPS).abs()
                < f32::EPSILON
        );
        assert!(geometry.label_max_width > f32::from(tokens.spacing.lg));
    }

    #[test]
    fn scaled_text_never_grows_past_the_strip() {
        let mut tokens = Tokens::builtin_dark();
        tokens.typography.scale = 4.0;
        let config = PanelConfig {
            height: 24,
            ..PanelConfig::default()
        };

        let geometry = PanelGeometry::new(&tokens, &config);

        assert!(
            geometry.font_size <= geometry.content_height(),
            "the text has to fit inside the strip"
        );
        assert!(geometry.font_size > 0.0);
    }

    #[test]
    fn the_entries_are_inset_so_their_corners_stay_inside_the_strip() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&PanelConfig::default());

        assert!((geometry.inset - f32::from(tokens.spacing.xs)).abs() < f32::EPSILON);
        assert!(geometry.inset > 0.0);
        assert!(
            geometry.content_height() < geometry.height,
            "an entry is shorter than the strip it sits in"
        );
    }

    #[test]
    fn every_size_comes_from_a_token() {
        let tokens = Tokens::builtin_dark();
        let geometry = geometry(&PanelConfig::default());

        assert!((geometry.padding - f32::from(tokens.spacing.md)).abs() < f32::EPSILON);
        assert!((geometry.entry_padding - geometry.padding).abs() < f32::EPSILON);
        assert!((geometry.spacing - f32::from(tokens.spacing.sm)).abs() < f32::EPSILON);
        assert!((geometry.group_gap - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON);
        assert!((geometry.divider_width - theme::hairline(&tokens)).abs() < f32::EPSILON);
    }

    #[test]
    fn groups_are_further_apart_than_the_entries_inside_them() {
        let geometry = geometry(&PanelConfig::default());

        assert!(
            geometry.group_gap > geometry.spacing,
            "grouping is what tells a user what belongs together (ui-guidelines §3)"
        );
    }

    #[test]
    fn the_secondary_information_is_smaller_than_a_task_label() {
        let geometry = geometry(&PanelConfig::default());

        assert!(geometry.caption_size < geometry.font_size);
        assert!(
            (geometry.divider_height - geometry.caption_size).abs() < f32::EPSILON,
            "the divider is as tall as a line of secondary information"
        );
    }

    #[test]
    fn a_tooltip_opens_away_from_the_edge_the_panel_sits_on() {
        let bottom = geometry(&PanelConfig {
            position: PanelPosition::Bottom,
            ..PanelConfig::default()
        });
        let top = geometry(&PanelConfig {
            position: PanelPosition::Top,
            ..PanelConfig::default()
        });

        assert_eq!(bottom.tooltip_position, TooltipPosition::Above);
        assert_eq!(top.tooltip_position, TooltipPosition::Below);
    }

    #[test]
    fn an_empty_model_still_lays_out() {
        let tokens = Tokens::builtin_dark();
        let config = PanelConfig {
            height: 34,
            ..PanelConfig::default()
        };

        // Projecting the scene is the assertion: a model with nothing in it
        // must still produce a strip of the configured height.
        let scene = panel(
            &model(),
            &tokens,
            &config,
            1280.0,
            34.0,
            &Interaction::default(),
        );

        assert_eq!(scene.surface, PanelSurface::Taskbar);
        assert!(
            scene.entries.is_empty(),
            "an empty panel has nothing to click"
        );
        assert!(
            scene.json["items"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "the strip itself is still painted"
        );
        assert!((geometry(&config).height - 34.0).abs() < f32::EPSILON);
    }

    #[test]
    fn every_drawing_command_carries_every_field_the_renderer_binds() {
        // `qml/Main.qml` binds the same property names on both kinds of item, so
        // the projection has to send all of them on every item: a field it left
        // out is the `Unable to assign [undefined]` Qt logged once per binding,
        // per item, per frame — the renderer then drew the *default* rather than
        // what the tokens said. The list is the delegate's, not this module's.
        //
        // `highlight`, `shade`, `hairline`, `highlighted` and `shaded` are on it
        // because the pane is now the shared `GlassPane` component, which binds
        // those five alongside the geometry every item carries. A rectangle or a
        // line of text carries them at their structural neutral values, exactly
        // as it carries the other kind's fields.
        const BOUND: [&str; 18] = [
            "kind",
            "x",
            "y",
            "w",
            "h",
            "color",
            "radius",
            "border",
            "border_color",
            "text",
            "size",
            "bold",
            "align",
            "highlight",
            "shade",
            "hairline",
            "highlighted",
            "shaded",
        ];

        let tokens = Tokens::builtin_dark();
        let mut painter = Painter::new(PanelSurface::Taskbar, &tokens);
        let style = theme::task_button(
            &tokens,
            TaskEmphasis::Idle,
            false,
            InteractionStatus::Active,
        );

        painter.rect(0.0, 0.0, 16.0, 16.0, &theme::panel_fill(&tokens), 0.0);
        painter.entry(&style, 0.0, 0.0, 16.0, 16.0);
        painter.text(
            0.0,
            0.0,
            16.0,
            16.0,
            "Inox",
            &theme::text(&tokens),
            12.0,
            false,
            Align::Left,
        );
        painter.pane(
            0.0,
            0.0,
            16.0,
            16.0,
            &theme::pane_style(&tokens, false, theme::radius_md(&tokens), true),
        );

        let scene = painter.finish(16.0, 16.0);
        let items = scene.json["items"]
            .as_array()
            .expect("the scene holds its drawing commands");

        assert_eq!(
            items.len(),
            4,
            "every command is in the scene: a rectangle, an entry, a line of \
             text and one pane (the pane is a single component call now, not a \
             body plus two edge bands): {items:?}"
        );
        assert!(
            items
                .iter()
                .any(|item| item["kind"].as_str() == Some("pane")),
            "the frosted pane is projected as a pane, which is what makes the \
             renderer instantiate the shared `GlassPane`: {items:?}"
        );

        for item in items {
            for field in BOUND {
                assert!(
                    item.get(field).is_some(),
                    "every item carries `{field}`, because the renderer binds it: {item}"
                );
            }
        }
    }

    #[test]
    fn the_scene_picker_draws_the_section_the_configuration_names() {
        let tokens = Tokens::builtin_dark();
        let locale = inox_common::i18n::Locale::default_locale();
        let localizer = crate::i18n::load(&locale).expect("the catalogs parse");

        for mode in [
            PanelSurface::Taskbar,
            PanelSurface::Menubar,
            PanelSurface::Dock,
        ] {
            let shell = ShellConfig {
                panel: PanelConfig {
                    surface: mode,
                    ..PanelConfig::default()
                },
                ..ShellConfig::default()
            };

            let scene = scene(
                &model(),
                &tokens,
                &shell,
                &localizer,
                1280.0,
                96.0,
                &Interaction::default(),
            );

            assert_eq!(scene.surface, mode);
            assert_eq!(scene.json["surface"], surface_name(mode));
        }
    }
}
