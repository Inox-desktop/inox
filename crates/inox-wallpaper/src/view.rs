//! The QML projection: one JSON document, decided in Rust.
//!
//! The QML tree is only allowed to *render*. Which strings are used, whether the
//! retry control is enabled and why not when it is not, which phase the
//! component is in, which colour and type role each line takes, and the geometry
//! of the wallpaper were all decided in [`crate::view_model`]; the tokens and
//! their QML conversion live in [`crate::theme`]. This module flattens those two
//! projections into the single document the bridge publishes as `modelJson`
//! (`docs/qml-migration-contract.md` §3).
//!
//! # Why one document and not a property per field
//!
//! A property per field would make the QML↔Rust boundary a second UI
//! definition, updated by hand every time a line is added. Instead QML parses
//! one value and binds to its paths; Rust republishes the whole document
//! whenever the state changes. The trade-off is accepted deliberately — a bigger
//! message per change in exchange for one place that decides everything.
//!
//! # Why the geometry is not in QML
//!
//! A copy of the wallpaper is as large as the rectangle [`crate::scaling`]
//! computed, because that rectangle *is* the answer this component exists to
//! give. The projection hands QML one rectangle and a `tiled` flag: Qt tiles the
//! image for `tile` and stretches it to the rectangle for every other mode, so
//! the QML side never scales an image to a shape the geometry did not choose.
//!
//! # The image
//!
//! QML cannot be handed a pixel buffer without a `QQuickImageProvider` or a file
//! on disk, so a decoded frame carries a PNG `data:` URL
//! ([`crate::decode::FrameHandle`]). The encode happens once per committed frame
//! on the worker that decoded it; the cost is recorded as a known gap in the
//! crate `README.md`.

use inox_config::Theme;
use serde_json::{Value, json};

use crate::app::{Message, PREVIEW_SIZE, State};
use crate::scaling::Rect;
use crate::theme::{self, ButtonRole, ButtonState};
use crate::view_model::{
    self, Action, LineRole, Notice, NoticeKind, PhaseKind, ProjectedLine, Wallpaper,
    WallpaperViewModel,
};

/// One projection of the state.
///
/// Besides the document QML parses, a projection carries the two values the
/// readiness report is checked against, and whether it draws a wallpaper at all
/// (a report that claims a loaded image when the projection has none is not
/// evidence of anything).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Projection {
    json: String,
    title: String,
    accessible_items: i32,
    has_wallpaper: bool,
}

impl Projection {
    /// The projection before any state exists: no title, no wallpaper.
    ///
    /// Used only by the bridge's fallback, which must be total when the binary
    /// has not installed a runtime.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            json: String::from("{}"),
            title: String::new(),
            accessible_items: 0,
            has_wallpaper: false,
        }
    }

    /// The JSON document QML parses.
    #[must_use]
    pub fn json(&self) -> &str {
        &self.json
    }

    /// The window title the projection carries.
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    /// How many accessible items the QML tree must expose for this model.
    #[must_use]
    pub fn accessible_items(&self) -> i32 {
        self.accessible_items
    }

    /// Whether the model draws an image.
    #[must_use]
    pub fn has_wallpaper(&self) -> bool {
        self.has_wallpaper
    }
}

/// Projects the state into the document QML renders.
#[must_use]
pub fn project(state: &State) -> Projection {
    let model = view_model::project(state);
    let document = document(&model, state.theme());

    Projection {
        json: document.to_string(),
        title: model.title.clone(),
        accessible_items: model.accessible_items(),
        has_wallpaper: model.wallpaper.is_some(),
    }
}

/// The document for a view model and a token set.
///
/// Separate from [`project`] so a test can project any model — loaded, empty,
/// degraded, at 200% font scale — without a running application.
#[must_use]
pub fn document(model: &WallpaperViewModel, tokens: &Theme) -> Value {
    json!({
        "title": model.title,
        "footer": model.footer.as_deref().map_or(Value::Null, |footer| line(
            footer,
            text_px(theme::caption_size(tokens)),
            theme::NORMAL_WEIGHT,
            theme::muted(tokens),
        )),
        "tokens": token_values(tokens),
        "card": {
            "lines": model
                .card
                .lines()
                .map(|line| line_value(line, tokens))
                .collect::<Vec<Value>>(),
        },
        "notice": model
            .notice
            .as_ref()
            .map_or(Value::Null, |notice| notice_values(notice, tokens)),
        "wallpaper": model
            .wallpaper
            .as_ref()
            .map_or(Value::Null, wallpaper_values),
        "accessibleItems": model.accessible_items(),
    })
}

/// Reads one JSON action out of the bridge's `dispatch` channel.
///
/// One parser for every control: a control reports `{"type":"<id>"}` and this
/// maps the id back onto the state machine. An unknown id is `None`, which the
/// caller only logs — a QML tree that asks for something the binary does not do
/// is a programming error, not a reason to stop drawing the desktop.
#[must_use]
pub fn parse_action(action: &str) -> Option<Message> {
    let parsed: Value = serde_json::from_str(action).ok()?;

    match parsed.get("type")?.as_str()? {
        "reload" => Some(Message::Reload),
        _ => None,
    }
}

/// The token values QML binds to.
fn token_values(tokens: &Theme) -> Value {
    let padding = theme::control_padding(tokens);
    let card = theme::card(tokens);

    json!({
        "window": {
            "width": PREVIEW_SIZE.width,
            "height": PREVIEW_SIZE.height,
        },
        "colors": {
            "background": theme::qml_color(tokens.colors.background),
            "surface": theme::qml_color(tokens.colors.surface),
            "surfaceVariant": theme::qml_color(tokens.colors.surface_variant),
            "border": theme::qml_color(tokens.colors.border),
            "text": theme::text(tokens),
            "textMuted": theme::muted(tokens),
            "accent": theme::qml_color(tokens.colors.accent),
            "onAccent": theme::qml_color(tokens.colors.on_accent),
            "warning": theme::warning(tokens),
            "success": theme::success(tokens),
            "danger": theme::qml_color(tokens.colors.danger),
        },
        "font": {
            "family": theme::font_family(tokens),
            "weight": {
                "normal": theme::NORMAL_WEIGHT,
                "emphasis": theme::EMPHASIS_WEIGHT,
            },
            "size": {
                "title": text_px(theme::title_size(tokens)),
                "heading": text_px(theme::heading_size(tokens)),
                "body": text_px(theme::body_size(tokens)),
                "caption": text_px(theme::caption_size(tokens)),
            },
        },
        "spacing": {
            "sm": theme::units(theme::gap(tokens)),
            "md": theme::units(theme::strip_padding(tokens)),
            "lg": theme::units(theme::section_gap(tokens)),
            "xl": theme::units(theme::window_padding(tokens)),
        },
        "radius": {
            "sm": theme::units(theme::radius_sm(tokens)),
            "md": theme::units(theme::radius_md(tokens)),
            "lg": theme::units(theme::radius_lg(tokens)),
        },
        "sizes": {
            "hairline": theme::units(theme::hairline(tokens)),
            "focusRing": theme::units(theme::focus_ring(tokens)),
            "controlHeight": theme::units(theme::control_height(tokens)),
            "controlPadding": {
                "top": theme::units(padding.top),
                "right": theme::units(padding.right),
                "bottom": theme::units(padding.bottom),
                "left": theme::units(padding.left),
            },
            "cardWidth": theme::units(theme::card_width(tokens)),
            "cardPadding": theme::units(theme::card_padding(tokens)),
            "stripPadding": theme::units(theme::strip_padding(tokens)),
            "windowPadding": theme::units(theme::window_padding(tokens)),
        },
        "card": {
            "background": card.background,
            "border": card.border,
            "borderWidth": card.border_width,
            "radius": card.radius,
        },
    })
}

/// One projected line, with everything needed to draw it.
///
/// The type role becomes a size and a weight, and the phase becomes a colour and a
/// name, here — never in QML, which binds the four values and decides nothing
/// (`docs/qml-migration-contract.md` §4). `kind` is the phase a line reports, and
/// `null` for every line that is not a phase: the document says which state a
/// sentence is about without the tree having to match on its text.
fn line_value(projected: ProjectedLine<'_>, tokens: &Theme) -> Value {
    let (size, weight, color) = match projected.role {
        LineRole::Title => (
            text_px(theme::title_size(tokens)),
            theme::EMPHASIS_WEIGHT,
            theme::text(tokens),
        ),
        LineRole::Heading => (
            text_px(theme::heading_size(tokens)),
            theme::EMPHASIS_WEIGHT,
            theme::text(tokens),
        ),
        LineRole::WarningHeading => (
            text_px(theme::heading_size(tokens)),
            theme::EMPHASIS_WEIGHT,
            theme::warning(tokens),
        ),
        LineRole::Body => (
            text_px(theme::body_size(tokens)),
            theme::NORMAL_WEIGHT,
            theme::text(tokens),
        ),
        LineRole::Caption => (
            text_px(theme::caption_size(tokens)),
            theme::NORMAL_WEIGHT,
            theme::muted(tokens),
        ),
        LineRole::Phase(kind) => (
            text_px(theme::caption_size(tokens)),
            theme::NORMAL_WEIGHT,
            phase_color(kind, tokens),
        ),
    };

    let kind = match projected.role {
        LineRole::Phase(kind) => Value::from(phase_name(kind)),
        _ => Value::Null,
    };

    json!({
        "text": projected.text,
        "size": size,
        "weight": weight,
        "color": color,
        "kind": kind,
    })
}

/// The name a phase is spelled with in the document.
fn phase_name(kind: PhaseKind) -> &'static str {
    match kind {
        PhaseKind::Loading => "loading",
        PhaseKind::Ready => "ready",
        PhaseKind::Degraded => "degraded",
        PhaseKind::Idle => "idle",
    }
}

/// The colour of a phase sentence.
///
/// The sentence is the state; the colour only agrees with it (UI spec §6: colour
/// is never the only thing that carries the state).
fn phase_color(kind: PhaseKind, tokens: &Theme) -> String {
    match kind {
        PhaseKind::Loading | PhaseKind::Idle => theme::text(tokens),
        PhaseKind::Ready => theme::success(tokens),
        PhaseKind::Degraded => theme::warning(tokens),
    }
}

/// The notice card: what happened, why, and the control that answers it.
fn notice_values(notice: &Notice, tokens: &Theme) -> Value {
    json!({
        "kind": match notice.kind {
            NoticeKind::Problem => "problem",
            NoticeKind::Loading => "loading",
        },
        "lines": notice
            .lines()
            .map(|line| line_value(line, tokens))
            .collect::<Vec<Value>>(),
        "action": action_values(&notice.action, tokens),
    })
}

/// The one control a notice offers, and why it cannot be pressed when it cannot.
///
/// All four states are projected rather than only the current one: QML needs the
/// hovered and pressed drawings to answer the pointer, and they are token
/// arithmetic rather than colours a QML file could spell.
fn action_values(action: &Action, tokens: &Theme) -> Value {
    let styles =
        |state: ButtonState| style_value(theme::button_style(tokens, ButtonRole::Primary, state));
    let current = if action.enabled {
        styles(ButtonState::Active)
    } else {
        styles(ButtonState::Disabled)
    };

    json!({
        "id": "reload",
        "label": action.label,
        "enabled": action.enabled,
        "reason": action.reason.as_deref().map_or(Value::Null, |reason| caption(reason, tokens)),
        "style": current,
        "styles": {
            "active": styles(ButtonState::Active),
            "hovered": styles(ButtonState::Hovered),
            "pressed": styles(ButtonState::Pressed),
            "disabled": styles(ButtonState::Disabled),
        },
        "size": text_px(theme::body_size(tokens)),
        "weight": theme::NORMAL_WEIGHT,
    })
}

/// One button drawing, as the fields QML binds.
fn style_value(style: theme::ButtonStyle) -> Value {
    json!({
        "background": style.background,
        "text": style.text,
        "border": style.border,
        "borderWidth": style.border_width,
        "radius": style.radius,
    })
}

/// The image, the rectangle it occupies, and whether Qt should tile it.
fn wallpaper_values(wallpaper: &Wallpaper) -> Value {
    // Tiling is the one mode whose copies are not one rectangle: Qt's own tiling
    // draws the same grid from the origin, so QML gets the output as the area to
    // cover instead of up to `scaling::MAX_TILES` rectangles.
    let rect = if wallpaper.placement.is_tiled() {
        Rect::new(0, 0, PREVIEW_SIZE.width, PREVIEW_SIZE.height)
    } else {
        wallpaper.placement.first()
    };

    json!({
        "dataUrl": wallpaper.image.data_url(),
        "sourceWidth": wallpaper.image.width(),
        "sourceHeight": wallpaper.image.height(),
        "tiled": wallpaper.placement.is_tiled(),
        "rect": rect_value(rect),
    })
}

/// A rectangle, as the four numbers QML positions an item with.
fn rect_value(rect: Rect) -> Value {
    json!({
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height,
    })
}

/// One line of text with everything needed to draw it.
fn line(text: &str, size: i32, weight: i32, color: String) -> Value {
    json!({
        "text": text,
        "size": size,
        "weight": weight,
        "color": color,
    })
}

/// A secondary line: a path, a mode, a state sentence, an explanation.
fn caption(text: &str, tokens: &Theme) -> Value {
    line(
        text,
        text_px(theme::caption_size(tokens)),
        theme::NORMAL_WEIGHT,
        theme::muted(tokens),
    )
}

/// A text size in whole logical pixels, never below one.
///
/// [`theme::px`] rounds a token length to whole pixels and is allowed to answer
/// zero for a nonsensical font scale; text is not, because a zero-pixel label is
/// an invisible one.
fn text_px(size: f64) -> i32 {
    theme::px(size).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_action_is_parsed_and_an_unknown_one_is_not() {
        assert!(matches!(
            parse_action(r#"{"type":"reload"}"#),
            Some(Message::Reload)
        ));
        assert!(parse_action(r#"{"type":"quit"}"#).is_none());
        assert!(parse_action(r#"{"type":7}"#).is_none());
        assert!(parse_action("not json").is_none());
        assert!(parse_action(r#"{"action":"reload"}"#).is_none());
    }

    #[test]
    fn an_empty_projection_is_not_a_document() {
        let projection = Projection::empty();

        assert_eq!(projection.json(), "{}");
        assert!(projection.title().is_empty());
        assert_eq!(projection.accessible_items(), 0);
        assert!(!projection.has_wallpaper());
    }
}
