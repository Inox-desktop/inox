//! The popup stack as a Qt-free JSON projection.
//!
//! This module is the only place that decides what the surface draws. Which
//! notifications are shown, in what order, what each one says, how urgent it says
//! it is, how long ago it arrived, which actions are offered and **why one of them
//! is disabled** all arrive decided in a [`crate::view_model::PopupStack`]; this
//! module turns that into a `serde_json::Value` and takes every colour, distance
//! and radius from the tokens through [`crate::theme`]. `qml/Main.qml` renders
//! that value and nothing else: it contains no colour, no size and no
//! user-visible string of its own (root AGENTS.md §4 and §7.3).
//!
//! The projection is a whole model on every change rather than a delta. That is
//! the trade this migration makes (docs/qml-migration-contract.md §3): one
//! property, recomputed in Rust, is a boundary that can be reasoned about and
//! tested without a display, and the popup stack is small enough that a
//! diff-based model would buy nothing measurable.
//!
//! Six layout decisions are worth naming, because they are what the acceptance
//! criteria are about and they are all made here rather than in QML:
//!
//! - **The stack is in the configured corner**, inset by the configured margin:
//!   the alignment is the same place the layer-surface anchors would put it
//!   ([`crate::placement`]).
//! - **A card is a card**: the `surface` token, a hairline edge, the `md` radius
//!   and the `lg` inset, with `lg` between two cards.
//! - **The hierarchy is a heading over body text over captions**: the summary is
//!   the card's one `heading`, the message is `body`, and the application name,
//!   the urgency sentence, the timestamp and the reason an action is unavailable
//!   are all `caption`.
//! - **A popup's body crosses the bridge as rich text this module wrote.** The
//!   view model caps the text; here every character of every span is escaped and
//!   wrapped in the handful of tags the sanitiser can produce, so QML hands the
//!   string to a text renderer and never to a markup parser of its own. Escaping
//!   is what makes wrapping and `<u>` work without trusting the client: a body
//!   that says `<script>` stays the eight characters the user sent.
//! - **Controls never leave the card.** The actions carry the row size they are
//!   laid out in ([`crate::view_model::MAX_ACTIONS_PER_ROW`]), so even the
//!   sixteen actions a client may legally send wrap instead of overflowing
//!   (ui-guidelines §7).
//! - **A link is clickable and nothing else is.** A span that the sanitiser
//!   produced from an `<a href>` carries its target; QML reports a click on that
//!   span and nothing else produces one.
//!
//! Three presentation facts are decided here for the same reason: QML only draws
//! them.
//!
//! - **The surface exists only when it has something to show**
//!   ([`surface_is_visible`]): an empty stack projects `window.visible: false`,
//!   and a daemon with nothing to say puts no window on screen.
//! - **The drawn height follows the stack, not a budget.** A card's height
//!   depends on how its body wraps, which only the text layout knows, so
//!   `window.height` stays the *placement* budget
//!   ([`StackGeometry::window_size`]) that a layer surface would ask the
//!   compositor for, and the surface sizes itself to the column it drew.
//! - **Entrance motion is timed by the tokens and ends where the card rests.**
//!   `motion` carries the three durations and the rise; QML animates *to* the
//!   resting values, so a card is fully readable even if the animation never runs
//!   (ui-guidelines §8).

use inox_config::{DisplayConfig, NotificationPosition, Theme as Tokens};
use serde_json::{Value, json};

use crate::markup::Span as MarkupSpan;
use crate::theme::{self, NoticeTone, SpanStyle, TextRole};
use crate::view_model::{MAX_ACTIONS_PER_ROW, Popup, PopupStack};

/// Width of the popup stack's surface, in logical pixels, before configuration.
///
/// The configured `display.width` is what a popup is drawn at; this is only the
/// fallback the bridge uses before the daemon has installed a projection.
pub const DEFAULT_WIDTH: f32 = 360.0;

/// The geometry of the popup stack, derived from the tokens and the configuration.
///
/// Every field is either a configuration value or a design token; nothing here is
/// a literal (ui-guidelines §1). It is computed in Rust rather than in QML so the
/// surface's size is a fact a test can assert without rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct StackGeometry {
    /// Width of one popup.
    pub width: f32,
    /// Distance between the stack and the output edges it sits against.
    pub margin: f32,
    /// Inset inside one card (ui-guidelines §4: `lg`).
    pub card_padding: f32,
    /// Inset inside one of the daemon's own lines (ui-guidelines §3: `md`).
    pub notice_padding: f32,
    /// Gap between two cards, and between a card and the overflow line (§3: `lg`).
    pub card_gap: f32,
    /// Gap between the parts of one card (§3: the default control step, `md`).
    pub section_gap: f32,
    /// Gap between two controls in one row (§3: `md`).
    pub control_gap: f32,
    /// Gap between items of one line — an icon and its text (§3: `sm`).
    pub inline_gap: f32,
    /// Size of the space an application icon occupies.
    pub icon_size: f32,
    /// Padding inside a control, as [`theme::control_inset`] derives it.
    pub control_inset: f32,
    /// Size of a card's heading.
    pub heading_size: f32,
    /// Size of body text and control labels.
    pub body_size: f32,
    /// Size of captions: the application name, status sentences, timestamps.
    pub caption_size: f32,
    /// The height a control needs to be a target a pointer can hit.
    pub control_height: f32,
}

impl StackGeometry {
    /// Derives the stack's geometry.
    pub fn new(tokens: &Tokens, display: &DisplayConfig) -> Self {
        Self {
            width: f32::from(display.width),
            margin: f32::from(display.margin),
            card_padding: theme::card_inset(tokens),
            notice_padding: theme::notice_inset(tokens),
            card_gap: theme::block_gap(tokens),
            section_gap: theme::section_gap(tokens),
            control_gap: theme::control_gap(tokens),
            inline_gap: theme::inline_gap(tokens),
            icon_size: f32::from(display.icon_size),
            control_inset: theme::control_inset(tokens),
            heading_size: theme::text_size(tokens, TextRole::Heading),
            body_size: theme::text_size(tokens, TextRole::Body),
            caption_size: theme::text_size(tokens, TextRole::Caption),
            control_height: theme::control_height(tokens),
        }
    }

    /// The size a popup stack reserves on its output, before its content is laid
    /// out.
    ///
    /// This is the **placement** budget: the room a layer surface would ask the
    /// compositor to reserve for [`DisplayConfig::max_visible`] popups, computed
    /// from the tokens alone. It is not the height the window is drawn at — a
    /// card grows with the lines its body wraps to, which is a fact only the text
    /// layout has, so the surface is sized to the stack it actually drew (see
    /// `qml/Main.qml`). Keeping both numbers is deliberate: the placement budget
    /// is what a host needs before anything is rendered, and it is what
    /// [`crate::placement`] is written against.
    pub fn window_size(&self, display: &DisplayConfig) -> (f32, f32) {
        let visible = f32::from(display.max_visible);
        let unit = 2.0 * self.card_padding + self.icon_size.max(self.control_height);
        let stack = unit * visible + self.card_gap * (visible - 1.0).max(0.0);

        (self.width + self.margin * 2.0, self.margin * 2.0 + stack)
    }

    /// Which way the popups are aligned inside the surface.
    pub fn alignment(display: &DisplayConfig) -> (&'static str, &'static str) {
        let horizontal = match display.position {
            NotificationPosition::TopRight | NotificationPosition::BottomRight => "right",
            NotificationPosition::TopLeft | NotificationPosition::BottomLeft => "left",
        };
        let vertical = match display.position {
            NotificationPosition::TopRight | NotificationPosition::TopLeft => "top",
            NotificationPosition::BottomRight | NotificationPosition::BottomLeft => "bottom",
        };

        (horizontal, vertical)
    }

    /// The geometry as the JSON object QML reads.
    fn to_json(&self, tokens: &Tokens) -> Value {
        json!({
            "width": self.width,
            "margin": self.margin,
            "cardInset": self.card_padding,
            "noticeInset": self.notice_padding,
            "cardGap": self.card_gap,
            "sectionGap": self.section_gap,
            "controlGap": self.control_gap,
            "inlineGap": self.inline_gap,
            "iconSize": self.icon_size,
            "controlInset": self.control_inset,
            "headingSize": self.heading_size,
            "bodySize": self.body_size,
            "captionSize": self.caption_size,
            "controlHeight": self.control_height,
            "borderWidth": theme::BORDER_WIDTH,
            "cardRadius": theme::card_radius(tokens),
            "controlRadius": theme::control_radius(tokens),
            "focusRingWidth": theme::focus_ring_width(tokens),
            "actionsPerRow": MAX_ACTIONS_PER_ROW,
        })
    }
}

/// The projection: state, tokens and catalogues as the value QML renders.
///
/// `title` is the surface's own title, which the caller resolves from the
/// catalogues (`app::title`); everything else is derived here.
pub fn project(model: &PopupStack, tokens: &Tokens, display: &DisplayConfig, title: &str) -> Value {
    let geometry = StackGeometry::new(tokens, display);
    let (width, height) = geometry.window_size(display);
    let (horizontal, vertical) = StackGeometry::alignment(display);
    let colors = theme::Colors::resolve(tokens);
    let motion = tokens.motion;

    json!({
        "window": {
            "title": title,
            // The surface's width is fixed by the configuration; its height is
            // the placement budget (see `StackGeometry::window_size`) rather
            // than the height it is drawn at, which follows the stack.
            "width": width,
            "height": height,
            // A notification daemon with nothing to show is not a window.
            "visible": surface_is_visible(model),
        },
        "palette": {
            "background": colors.background,
            "surface": colors.surface,
            "surfaceVariant": colors.surface_variant,
            "border": colors.border,
            "text": colors.text,
            "textMuted": colors.text_muted,
            "accent": colors.accent,
            "onAccent": colors.on_accent,
            "danger": colors.danger,
            "warning": colors.warning,
            "success": colors.success,
        },
        "metrics": geometry.to_json(tokens),
        // The durations and the distance the entrance motion is drawn with
        // (ui-guidelines §8): QML animates *to* the resting state, so a card
        // that never animated is still fully visible.
        "motion": {
            "fast": motion.fast_ms,
            "normal": motion.normal_ms,
            "slow": motion.slow_ms,
            "rise": theme::entry_rise(tokens),
        },
        "alignment": {
            "horizontal": horizontal,
            "vertical": vertical,
        },
        "stack": {
            "status": status_name(model),
            "popups": model
                .popups
                .iter()
                .map(|popup| popup_value(popup, tokens, &geometry))
                .collect::<Vec<Value>>(),
            "overflow": model.overflow.as_ref().map(|overflow| {
                notice_value(
                    &overflow.message,
                    NoticeTone::Caution,
                    tokens,
                    &geometry,
                )
            }),
            "notice": model.notice.as_ref().map(|notice| {
                notice_value(notice, NoticeTone::Quiet, tokens, &geometry)
            }),
        },
    })
}

/// Whether the surface has anything to put on screen.
///
/// A daemon holding nothing must not leave a window behind: the empty state is
/// the absence of a surface, not a rectangle saying "No notifications". Three
/// things are worth a window:
///
/// - a popup to draw;
/// - the overflow sentence, which says notifications are waiting behind the ones
///   on screen (D05) — it is never projected without popups, but it is listed so
///   that the rule is about what is drawn rather than about a count;
/// - a notice explaining a state the user chose, which in this stack is the
///   muted line while do not disturb is on: silence with no explanation is
///   indistinguishable from a daemon that has stopped working.
///
/// The plain empty line is deliberately not on that list. It exists in the model
/// so that the catalogue and the projection tests can read it, and it is not a
/// reason to draw.
fn surface_is_visible(model: &PopupStack) -> bool {
    !model.popups.is_empty()
        || model.overflow.is_some()
        || (model.status == crate::view_model::StackStatus::Suppressed && model.notice.is_some())
}

/// The projection of a daemon holding nothing, used before the host installs one.
///
/// The QML engine may construct the element before the binary has installed the
/// real projection (or in a unit test that drives the backing struct directly),
/// and a surface whose `modelJson` is not valid JSON cannot render at all. This is
/// the value that keeps that path total instead of panicking in a constructor.
pub fn empty(tokens: &Tokens, display: &DisplayConfig, title: &str) -> Value {
    let model = PopupStack {
        status: crate::view_model::StackStatus::Empty,
        popups: Vec::new(),
        overflow: None,
        notice: None,
    };

    project(&model, tokens, display, title)
}

/// The projection a bridge uses before the daemon has installed one.
///
/// It is the empty projection of the built-in dark theme and the default display
/// configuration; [`crate::cxxqt_object`] hands it over when QML constructs the
/// element first, so that the surface always has JSON it can parse.
pub fn fallback_json() -> String {
    let model = PopupStack {
        status: crate::view_model::StackStatus::Empty,
        popups: Vec::new(),
        overflow: None,
        notice: None,
    };

    project(
        &model,
        &Tokens::builtin_dark(),
        &DisplayConfig::default(),
        "",
    )
    .to_string()
}

/// The stack's state as a name QML can compare.
fn status_name(model: &PopupStack) -> &'static str {
    match model.status {
        crate::view_model::StackStatus::Empty => "empty",
        crate::view_model::StackStatus::Showing => "showing",
        crate::view_model::StackStatus::Suppressed => "suppressed",
    }
}

/// One card: its header, its heading, its body, its actions and its status line.
fn popup_value(popup: &Popup, tokens: &Tokens, geometry: &StackGeometry) -> Value {
    let colors = theme::Colors::resolve(tokens);

    json!({
        "id": popup.id.get(),
        "urgency": urgency_name(popup.urgency),
        "urgencyNote": popup.urgency_note.as_deref(),
        "urgencyColor": theme::urgency_color(tokens, popup.urgency),
        "age": popup.age.as_str(),
        "appName": popup.content.app_name(),
        "icon": {
            "present": popup.content.icon().is_some(),
            "size": geometry.icon_size,
            "fill": colors.surface_variant,
            "radius": theme::control_radius(tokens),
        },
        "summary": popup.content.summary(),
        // The plain sentence, for the semantic tree: a screen reader is given
        // the text, never the tags (root AGENTS.md §7.4).
        "bodyText": popup
            .content
            .body()
            .map(crate::markup::plain_text)
            .unwrap_or_default(),
        // The same sentence as escaped rich text, for the text layout: this is
        // what makes the body wrap inside the card and what makes `<u>` visible.
        "bodyHtml": popup
            .content
            .body()
            .map(|spans| body_html(spans, tokens))
            .unwrap_or_default(),
        "close": {
            "label": popup.close.as_str(),
            "control": control_value(&theme::close_control(tokens)),
        },
        "card": {
            "background": colors.surface,
            "edge": theme::card_edge(tokens, popup.urgency),
            "radius": theme::card_radius(tokens),
            "inset": geometry.card_padding,
            // The urgency rail: a mark on the leading edge of a critical card,
            // in the same colour as its urgency sentence. The colour is never the
            // only cue — the sentence says "Critical" in words (ui-guidelines
            // §5, §6).
            "rail": {
                "visible": popup.urgency == crate::protocol::Urgency::Critical,
                "width": theme::urgency_rail_width(tokens),
                "color": theme::urgency_color(tokens, popup.urgency),
                "radius": theme::control_radius(tokens),
                "inset": theme::card_radius(tokens),
            },
        },
        "actions": popup
            .actions
            .iter()
            .map(|action| action_value(action, tokens))
            .collect::<Vec<Value>>(),
    })
}

/// The name of an urgency, for QML to compare rather than to draw.
fn urgency_name(urgency: crate::protocol::Urgency) -> &'static str {
    match urgency {
        crate::protocol::Urgency::Low => "low",
        crate::protocol::Urgency::Normal => "normal",
        crate::protocol::Urgency::Critical => "critical",
    }
}

/// One popup's body as the escaped rich text QML lays out.
///
/// The body is untrusted input from any application on the session. The
/// sanitiser ([`crate::markup`]) already turned it into spans, and a span cannot
/// carry markup: its text is whatever was not accepted as a tag. This function
/// is the second half of that promise — it may write the tags the sanitiser
/// can produce (`<b>`, `<i>`, `<u>`, `<font color>`, `<a href>`) and nothing
/// else, and every character that came from the notification is
/// [`escape_text`]ed, so a body that says `<script>` is the eight characters a
/// text renderer draws.
///
/// It is HTML because that is what a Qt text layout understands, and it is what
/// makes the body wrap inside the card: a `Text` in rich-text mode lays out with
/// the card's width, where a row of separate text items would run past the edge.
fn body_html(spans: &[MarkupSpan], tokens: &Tokens) -> String {
    let mut html = String::new();

    for span in spans {
        let style = SpanStyle::from_emphasis(span.emphasis());
        let (href, link) = match span {
            MarkupSpan::Link { href, .. } => (Some(href), true),
            MarkupSpan::Text { .. } | MarkupSpan::Image { .. } => (None, false),
        };
        let color = if link {
            theme::link(tokens)
        } else {
            theme::qml_color(tokens.colors.text)
        };

        // Open the wrappers outside-in and remember what has to be closed, so
        // the tags nest even though the emphasis arrives as flags.
        let mut closing: Vec<&str> = Vec::new();

        if let Some(href) = href {
            html.push_str("<a href=\"");
            html.push_str(&escape_text(href));
            html.push_str("\">");
            closing.push("</a>");
        }

        html.push_str("<font color=\"");
        html.push_str(&color);
        html.push_str("\">");
        closing.push("</font>");

        if style.bold {
            html.push_str("<b>");
            closing.push("</b>");
        }

        if style.italic {
            html.push_str("<i>");
            closing.push("</i>");
        }

        // A link is underlined as well as coloured: the colour alone is not a
        // signal a reader who cannot tell it apart can use (ui-guidelines §5).
        if style.underline || link {
            html.push_str("<u>");
            closing.push("</u>");
        }

        html.push_str(&escape_text(span.text()));

        for tag in closing.iter().rev() {
            html.push_str(tag);
        }
    }

    html
}

/// Escapes every character a rich-text renderer would otherwise read as markup.
///
/// `&`, `<`, `>` and `"` are the four that can change the structure of the
/// document: the first starts an entity, the next two open and close a tag, and
/// the fourth ends an attribute. They are escaped wherever they appear, in text
/// and in an `href`, which is what keeps a body from ever becoming markup. A
/// character that is not one of the four is passed through unchanged — this is
/// an escape, not an encoding, so the text a user reads is the text that was
/// sent.
fn escape_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());

    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }

    escaped
}

/// One action button and the four states QML draws it in.
fn action_value(action: &crate::view_model::ActionButton, tokens: &Tokens) -> Value {
    json!({
        "id": action.id.as_str(),
        "label": action.label.as_str(),
        "emphasis": emphasis_name(action.emphasis),
        "enabled": action.enabled,
        "disabledReason": action.disabled_reason.as_deref(),
        "control": control_value(&theme::action_control(tokens, action.emphasis)),
    })
}

/// The name of an action tier, for QML to compare rather than to draw.
fn emphasis_name(emphasis: crate::view_model::ActionEmphasis) -> &'static str {
    match emphasis {
        crate::view_model::ActionEmphasis::Primary => "primary",
        crate::view_model::ActionEmphasis::Secondary => "secondary",
        crate::view_model::ActionEmphasis::Danger => "danger",
    }
}

/// The four states of one control.
fn control_value(style: &theme::ControlStyle) -> Value {
    json!({
        "rest": control_state_value(&style.rest),
        "hover": control_state_value(&style.hover),
        "press": control_state_value(&style.press),
        "disabled": control_state_value(&style.disabled),
    })
}

/// One state of one control.
///
/// A state without a fill is projected as the colour name `transparent` rather
/// than as `null`: QML assigns these strings to `color` properties directly, and
/// keeping "there is no fill" in the projection is what lets `qml/Main.qml`
/// contain no colour at all (root AGENTS.md §4).
fn control_state_value(state: &theme::ControlState) -> Value {
    json!({
        "fill": state.fill.as_deref().unwrap_or("transparent"),
        "text": state.text,
        "border": state.border,
    })
}

/// The daemon's own line: the empty and muted states, and the overflow sentence.
fn notice_value(text: &str, tone: NoticeTone, tokens: &Tokens, geometry: &StackGeometry) -> Value {
    let colors = theme::Colors::resolve(tokens);

    json!({
        "text": text,
        "tone": match tone {
            NoticeTone::Quiet => "quiet",
            NoticeTone::Caution => "caution",
        },
        "textColor": theme::notice_text(tokens, tone),
        "edge": theme::notice_edge(tokens, tone),
        "background": colors.surface_variant,
        "radius": theme::card_radius(tokens),
        "inset": geometry.notice_padding,
        "width": geometry.width,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup::sanitize;
    use crate::policy::{Policy, SessionState};
    use crate::protocol::Urgency;
    use crate::queue::{Draft, NotificationId, Queue};
    use crate::view_model::{StackInput, StackStatus};
    use inox_common::i18n::{Locale, Localizer};
    use inox_config::{QueueConfig, TimeoutConfig};
    use std::time::Instant;

    fn localizer(tag: &str) -> Localizer {
        let locale = Locale::parse(tag).expect("a well-formed tag");
        crate::i18n::load(&locale).expect("the catalogs parse")
    }

    fn action_list(flat: &[&str]) -> crate::actions::Actions {
        let flat: Vec<String> = flat.iter().map(|action| (*action).to_owned()).collect();
        crate::actions::Actions::from_flat(&flat).expect("a well-formed action list")
    }

    fn queue_with(notifications: &[(&str, &str)]) -> Queue {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);

        for (app, summary) in notifications {
            queue.notify(
                Draft {
                    app_name: (*app).to_owned(),
                    summary: (*summary).to_owned(),
                    ..Draft::default()
                },
                Instant::now(),
            );
        }

        queue
    }

    fn model(
        queue: &Queue,
        display: &DisplayConfig,
        policy: &Policy,
        session: SessionState,
    ) -> PopupStack {
        StackInput {
            queue,
            display,
            policy,
            session,
            now: Instant::now(),
        }
        .project(&localizer("en-US"))
    }

    /// The projection of a daemon holding `queue`'s notifications, in en-US.
    fn projected(queue: &Queue) -> Value {
        let display = DisplayConfig::default();
        let policy = Policy::default();
        let model = model(queue, &display, &policy, SessionState::Unlocked);

        project(
            &model,
            &Tokens::builtin_dark(),
            &display,
            "Inox Notifications",
        )
    }

    fn metric(value: &Value, name: &str) -> f64 {
        value["metrics"][name]
            .as_f64()
            .unwrap_or_else(|| panic!("`{name}` is a number: {value}"))
    }

    // --- the projection as a whole ------------------------------------------

    #[test]
    fn the_projection_carries_the_window_the_palette_and_the_metrics() {
        let value = projected(&queue_with(&[("terminal", "a build finished")]));
        let tokens = Tokens::builtin_dark();
        let display = DisplayConfig::default();
        let geometry = StackGeometry::new(&tokens, &display);
        let (width, height) = geometry.window_size(&display);

        assert_eq!(value["window"]["title"], "Inox Notifications");
        assert_eq!(value["window"]["width"].as_f64(), Some(f64::from(width)));
        assert_eq!(value["window"]["height"].as_f64(), Some(f64::from(height)));
        assert_eq!(
            value["palette"]["background"],
            theme::qml_color(tokens.colors.background)
        );
        assert_eq!(
            value["palette"]["surface"],
            theme::qml_color(tokens.colors.surface)
        );
        assert_eq!(
            value["palette"]["text"],
            theme::qml_color(tokens.colors.text)
        );
        assert_eq!(
            value["metrics"]["width"].as_f64(),
            Some(f64::from(display.width))
        );
        assert_eq!(
            value["metrics"]["cardInset"].as_f64(),
            Some(f64::from(theme::card_inset(&tokens)))
        );
        assert_eq!(
            value["metrics"]["bodySize"].as_f64(),
            Some(f64::from(theme::text_size(&tokens, TextRole::Body)))
        );
        assert_eq!(
            metric(&value, "borderWidth"),
            f64::from(theme::BORDER_WIDTH)
        );
        assert_eq!(metric(&value, "actionsPerRow"), MAX_ACTIONS_PER_ROW as f64);
    }

    #[test]
    fn every_colour_is_a_qml_colour_and_never_a_literal() {
        let value = projected(&queue_with(&[("app", "hello")]));
        let colours = [
            &value["palette"]["background"],
            &value["palette"]["surface"],
            &value["palette"]["text"],
            &value["stack"]["popups"][0]["card"]["background"],
            &value["stack"]["popups"][0]["card"]["edge"],
            &value["stack"]["popups"][0]["close"]["control"]["rest"]["text"],
        ];

        for colour in colours {
            let colour = colour.as_str().expect("a colour is a string");
            assert_eq!(colour.len(), 9, "`{colour}` is #AARRGGBB");
            assert!(colour.starts_with('#'));
            assert!(
                colour[1..].chars().all(|c| c.is_ascii_hexdigit()),
                "`{colour}` is not hexadecimal"
            );
        }
    }

    #[test]
    fn the_light_and_dark_themes_project_different_palettes() {
        let queue = queue_with(&[("app", "hello")]);
        let display = DisplayConfig::default();
        let policy = Policy::default();
        let model = model(&queue, &display, &policy, SessionState::Unlocked);

        let dark = project(&model, &Tokens::builtin_dark(), &display, "Dark");
        let light = project(&model, &Tokens::builtin_light(), &display, "Light");

        assert_ne!(
            dark["palette"]["background"],
            light["palette"]["background"]
        );
        assert_ne!(dark["palette"]["surface"], light["palette"]["surface"]);
        assert_ne!(
            dark["stack"]["popups"][0]["card"]["background"],
            light["stack"]["popups"][0]["card"]["background"]
        );
    }

    #[test]
    fn the_chinese_catalogues_reach_the_projection_and_the_english_ones_differ() {
        let queue = queue_with(&[("terminal", "a build finished")]);
        let display = DisplayConfig::default();
        let policy = Policy::default();
        let tokens = Tokens::builtin_dark();

        let en_model = model(&queue, &display, &policy, SessionState::Unlocked);
        let en = project(&en_model, &tokens, &display, "Inox Notifications");

        let zh_model = StackInput {
            queue: &queue,
            display: &display,
            policy: &policy,
            session: SessionState::Unlocked,
            now: Instant::now(),
        }
        .project(&localizer("zh-CN"));
        let zh = project(&zh_model, &tokens, &display, "Inox 通知");

        assert_eq!(en["window"]["title"], "Inox Notifications");
        assert_eq!(zh["window"]["title"], "Inox 通知");
        assert_eq!(en["stack"]["popups"][0]["close"]["label"], "Close");
        assert_eq!(zh["stack"]["popups"][0]["close"]["label"], "关闭");
        assert_ne!(
            en["stack"]["popups"][0]["close"]["label"], zh["stack"]["popups"][0]["close"]["label"],
            "the second locale is a translation, not the fallback"
        );
        // Application names and summaries are data: they are not translated.
        assert_eq!(en["stack"]["popups"][0]["summary"], "a build finished");
        assert_eq!(zh["stack"]["popups"][0]["summary"], "a build finished");
    }

    #[test]
    fn the_empty_state_says_so_instead_of_projecting_nothing() {
        let value = projected(&queue_with(&[]));

        assert_eq!(value["stack"]["status"], "empty");
        assert_eq!(value["stack"]["popups"].as_array().map(Vec::len), Some(0));
        assert_eq!(value["stack"]["notice"]["text"], "No notifications");
        assert_eq!(value["stack"]["notice"]["tone"], "quiet");
        assert_eq!(value["stack"]["overflow"], Value::Null);
    }

    #[test]
    fn the_empty_projection_is_the_state_a_bridge_can_always_parse() {
        let tokens = Tokens::builtin_dark();
        let display = DisplayConfig::default();
        let value = empty(&tokens, &display, "Inox Notifications");

        assert_eq!(value["stack"]["status"], "empty");
        assert!(
            value["stack"]["popups"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
        assert!(value["window"]["width"].as_f64().is_some_and(|w| w > 0.0));
    }

    // --- the window: visibility, height, motion ----------------------------

    #[test]
    fn the_surface_is_hidden_while_the_stack_is_empty() {
        let value = projected(&queue_with(&[]));

        assert_eq!(
            value["window"]["visible"], false,
            "a daemon with nothing to show puts no window on screen: {value}"
        );
    }

    #[test]
    fn the_surface_is_visible_while_a_popup_is_drawn() {
        let value = projected(&queue_with(&[("app", "hello")]));

        assert_eq!(value["window"]["visible"], true);
    }

    #[test]
    fn the_muted_surface_stays_visible_because_it_explains_the_silence() {
        let queue = queue_with(&[("app", "one")]);
        let display = DisplayConfig::default();
        let mut policy = Policy::default();
        policy.set_do_not_disturb(true);
        let value = project(
            &model(&queue, &display, &policy, SessionState::Unlocked),
            &Tokens::builtin_dark(),
            &display,
            "Inox Notifications",
        );

        assert_eq!(value["stack"]["status"], "suppressed");
        assert_eq!(
            value["window"]["visible"], true,
            "do not disturb silences popups; the line that says why is still worth drawing"
        );
    }

    #[test]
    fn the_overflow_line_is_a_reason_to_draw_on_its_own() {
        let model = PopupStack {
            status: StackStatus::Showing,
            popups: Vec::new(),
            overflow: Some(crate::view_model::OverflowNotice {
                hidden: 2,
                message: "2 more waiting".to_owned(),
            }),
            notice: None,
        };
        let value = project(
            &model,
            &Tokens::builtin_dark(),
            &DisplayConfig::default(),
            "Inox Notifications",
        );

        assert_eq!(
            value["window"]["visible"], true,
            "a sentence about notifications the user cannot see is something to show (D05)"
        );
    }

    #[test]
    fn the_window_height_is_the_placement_budget_while_the_surface_follows_its_content() {
        // Three ordinary notifications do not fit the height the projection
        // budgets for them: each card is at least a header row, a heading, one
        // line of body and a control row, separated by the section gaps, and
        // three of those plus the gaps already exceed `window_size`. That is
        // why the surface is sized to the column it drew and `window.height`
        // stays what a layer surface would reserve (the placement budget);
        // binding the window to this number is what clipped the second and
        // third cards.
        let tokens = Tokens::builtin_dark();
        let display = DisplayConfig::default();
        let geometry = StackGeometry::new(&tokens, &display);
        let (_, budget) = geometry.window_size(&display);

        let header = geometry.icon_size.max(geometry.control_height);
        let card = 2.0 * geometry.card_padding
            + header
            + geometry.section_gap
            + geometry.heading_size
            + geometry.section_gap
            + geometry.body_size
            + geometry.section_gap
            + geometry.control_height;
        let popups = f32::from(display.max_visible);
        let drawn = 2.0 * geometry.margin + card * popups + geometry.card_gap * (popups - 1.0);

        assert!(
            drawn > budget,
            "the least three cards can occupy is {drawn} px, more than the {budget} px budget that used to be the window"
        );

        let value = projected(&queue_with(&[("a", "one"), ("b", "two"), ("c", "three")]));

        assert_eq!(value["stack"]["popups"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            value["window"]["height"].as_f64(),
            Some(f64::from(budget)),
            "the placement budget is still projected, so a host can reserve it"
        );
        assert_eq!(
            value["window"]["width"].as_f64(),
            Some(f64::from(geometry.width + 2.0 * geometry.margin)),
            "the width still comes from the projection"
        );
    }

    #[test]
    fn the_motion_durations_are_the_tokens_and_the_rise_is_a_token_step() {
        let value = projected(&queue_with(&[("app", "hello")]));
        let tokens = Tokens::builtin_dark();
        let motion = &value["motion"];

        assert_eq!(
            motion["fast"].as_u64(),
            Some(u64::from(tokens.motion.fast_ms))
        );
        assert_eq!(
            motion["normal"].as_u64(),
            Some(u64::from(tokens.motion.normal_ms))
        );
        assert_eq!(
            motion["slow"].as_u64(),
            Some(u64::from(tokens.motion.slow_ms))
        );
        assert_eq!(
            motion["rise"].as_f64(),
            Some(f64::from(theme::entry_rise(&tokens)))
        );
        assert!(
            motion["fast"].as_u64().unwrap_or_default() <= u64::from(tokens.motion.normal_ms)
                && motion["fast"].as_u64().unwrap_or_default() > 0,
            "the entrance is neither instant nor slower than a normal transition: {motion}"
        );
    }

    // --- do not disturb, the lock, the overflow ----------------------------

    #[test]
    fn do_not_disturb_projects_no_popup_and_the_muted_line() {
        let queue = queue_with(&[("app", "one"), ("app", "two")]);
        let display = DisplayConfig::default();
        let mut policy = Policy::default();
        policy.set_do_not_disturb(true);
        let model = model(&queue, &display, &policy, SessionState::Unlocked);
        let value = project(
            &model,
            &Tokens::builtin_dark(),
            &display,
            "Inox Notifications",
        );

        assert_eq!(value["stack"]["status"], "suppressed");
        assert!(
            value["stack"]["popups"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
        assert_eq!(
            value["stack"]["notice"]["text"],
            "Notifications are muted while do not disturb is on"
        );
    }

    #[test]
    fn a_locked_session_projects_no_field_a_body_could_be_read_from() {
        let queue = queue_with(&[("secret-app", "a secret summary")]);
        let display = DisplayConfig::default();
        let policy = Policy::default();
        let model = model(&queue, &display, &policy, SessionState::Locked);
        let value = project(
            &model,
            &Tokens::builtin_dark(),
            &display,
            "Inox Notifications",
        );

        let popup = &value["stack"]["popups"][0];

        assert_eq!(popup["summary"], "A notification arrived");
        assert_eq!(popup["appName"], Value::Null);
        assert_eq!(popup["icon"]["present"], false);
        assert_eq!(
            popup["bodyText"], "",
            "the body is not in the projection at all: {popup}"
        );
        assert_eq!(
            popup["bodyHtml"], "",
            "no rich text is drawn for a withheld body: {popup}"
        );
        assert!(
            !value.to_string().contains("a secret summary"),
            "nothing about the notification may leak through the projection: {value}"
        );
        assert!(!value.to_string().contains("secret-app"));
    }

    #[test]
    fn more_waiting_than_can_be_shown_is_projected_as_a_caution() {
        let queue = queue_with(&[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4"), ("e", "5")]);
        let value = projected(&queue);
        let tokens = Tokens::builtin_dark();

        assert_eq!(value["stack"]["overflow"]["text"], "2 more waiting");
        assert_eq!(value["stack"]["overflow"]["tone"], "caution");
        assert_eq!(
            value["stack"]["overflow"]["textColor"],
            theme::qml_color(tokens.colors.warning)
        );
        assert_eq!(value["stack"]["popups"].as_array().map(Vec::len), Some(3));
    }

    // --- cards --------------------------------------------------------------

    #[test]
    fn a_card_takes_its_surface_edge_radius_and_inset_from_the_tokens() {
        let tokens = Tokens::builtin_dark();
        let value = projected(&queue_with(&[("terminal", "a build finished")]));
        let card = &value["stack"]["popups"][0]["card"];

        assert_eq!(card["background"], theme::qml_color(tokens.colors.surface));
        assert_eq!(card["edge"], theme::qml_color(tokens.colors.border));
        assert_eq!(
            card["radius"].as_f64(),
            Some(f64::from(theme::card_radius(&tokens)))
        );
        assert_eq!(
            card["inset"].as_f64(),
            Some(f64::from(theme::card_inset(&tokens)))
        );
    }

    #[test]
    fn a_critical_notification_is_edged_and_says_so_rather_than_only_being_red() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                summary: "urgent".to_owned(),
                hints: crate::protocol::Hints {
                    urgency: Urgency::Critical,
                    ..crate::protocol::Hints::default()
                },
                ..Draft::default()
            },
            Instant::now(),
        );
        let tokens = Tokens::builtin_dark();
        let value = projected(&queue);
        let popup = &value["stack"]["popups"][0];

        assert_eq!(popup["urgency"], "critical");
        assert_eq!(popup["urgencyNote"], "Critical");
        assert_eq!(
            popup["card"]["edge"],
            theme::qml_color(tokens.colors.danger)
        );
        assert_ne!(
            popup["card"]["edge"],
            theme::qml_color(tokens.colors.border)
        );
    }

    #[test]
    fn a_critical_card_carries_an_urgency_rail_in_the_token_colour() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                summary: "urgent".to_owned(),
                hints: crate::protocol::Hints {
                    urgency: Urgency::Critical,
                    ..crate::protocol::Hints::default()
                },
                ..Draft::default()
            },
            Instant::now(),
        );
        let tokens = Tokens::builtin_dark();
        let value = projected(&queue);
        let popup = &value["stack"]["popups"][0];
        let rail = &popup["card"]["rail"];

        assert_eq!(rail["visible"], true);
        assert_eq!(
            rail["color"],
            theme::urgency_color(&tokens, Urgency::Critical)
        );
        assert_eq!(
            rail["width"].as_f64(),
            Some(f64::from(theme::urgency_rail_width(&tokens)))
        );
        assert!(
            rail["inset"].as_f64().unwrap_or_default() > rail["width"].as_f64().unwrap_or_default(),
            "the rail is inset past the card's own corner: {rail}"
        );
        assert_eq!(
            popup["urgencyNote"], "Critical",
            "the rail is a second cue, never the only one: the card still says it in words"
        );

        let ordinary = projected(&queue_with(&[("app", "hello")]));
        assert_eq!(
            ordinary["stack"]["popups"][0]["card"]["rail"]["visible"],
            false
        );
    }

    #[test]
    fn a_low_priority_notification_is_said_in_a_sentence_and_not_in_the_danger_colour() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                summary: "meh".to_owned(),
                hints: crate::protocol::Hints {
                    urgency: Urgency::Low,
                    ..crate::protocol::Hints::default()
                },
                ..Draft::default()
            },
            Instant::now(),
        );
        let tokens = Tokens::builtin_dark();
        let value = projected(&queue);
        let popup = &value["stack"]["popups"][0];

        assert_eq!(popup["urgency"], "low");
        assert_eq!(popup["urgencyNote"], "Low priority");
        assert_eq!(
            popup["urgencyColor"],
            theme::qml_color(tokens.colors.text_muted)
        );
        assert_ne!(
            popup["card"]["edge"],
            theme::qml_color(tokens.colors.danger)
        );
    }

    #[test]
    fn an_ordinary_notification_carries_no_urgency_sentence() {
        let value = projected(&queue_with(&[("app", "hello")]));

        assert_eq!(value["stack"]["popups"][0]["urgency"], "normal");
        assert_eq!(value["stack"]["popups"][0]["urgencyNote"], Value::Null);
    }

    // --- the body as escaped rich text --------------------------------------

    /// The rich text and the plain sentence projected for one body, in en-US.
    fn rich_body(body: &str) -> (String, String) {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                summary: "marked up".to_owned(),
                body: sanitize(body),
                ..Draft::default()
            },
            Instant::now(),
        );
        let value = projected(&queue);
        let popup = &value["stack"]["popups"][0];

        (
            popup["bodyHtml"].as_str().unwrap_or_default().to_owned(),
            popup["bodyText"].as_str().unwrap_or_default().to_owned(),
        )
    }

    /// Asserts that no `&` in `html` can start a character reference.
    fn every_ampersand_is_an_entity(html: &str) {
        for (index, _) in html.match_indices('&') {
            let rest = &html[index..];

            assert!(
                ["&amp;", "&lt;", "&gt;", "&quot;"]
                    .iter()
                    .any(|entity| rest.starts_with(entity)),
                "an unescaped `&` can start a character reference: {html}"
            );
        }
    }

    #[test]
    fn a_body_keeps_its_emphasis_and_its_link_as_rich_text() {
        let (html, text) = rich_body(
            "plain <b>bold</b> <i>italic</i> <u>under</u> <a href=\"https://example.com\">a link</a>",
        );
        let tokens = Tokens::builtin_dark();

        assert!(html.contains("<b>bold</b>"), "bold survives: {html}");
        assert!(html.contains("<i>italic</i>"), "italic survives: {html}");
        assert!(
            html.contains("<u>under</u>"),
            "the underline the model kept is now drawn: {html}"
        );
        assert!(
            html.contains("<a href=\"https://example.com\">"),
            "a link is an anchor: {html}"
        );
        assert!(
            html.contains(&format!("color=\"{}\"", theme::link(&tokens))),
            "a link is drawn in the accent colour: {html}"
        );
        assert!(
            html.contains("<u>a link</u>"),
            "a link is underlined as well as coloured: {html}"
        );
        assert_eq!(
            text, "plain bold italic under a link",
            "the plain sentence is the text a reader is told about"
        );
        every_ampersand_is_an_entity(&html);
    }

    #[test]
    fn a_body_that_looks_like_markup_cannot_become_markup() {
        // The sanitiser keeps an unknown tag as text rather than dropping it, so
        // `<script>` reaches this module as characters. Escaping is what keeps
        // those characters from opening an element in the renderer.
        let (html, text) = rich_body("Hello <script>alert(1)</script> & <b>bold</b>");

        assert!(
            !html.contains("<script"),
            "an escaped body has no tag the sanitiser did not mint: {html}"
        );
        assert!(
            html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
            "the tag is text, character for character: {html}"
        );
        assert!(html.contains("&amp;"), "an ampersand is escaped: {html}");
        assert!(
            html.contains("<b>bold</b>"),
            "the emphasis the sanitiser accepted is still drawn: {html}"
        );
        assert_eq!(
            text, "Hello <script>alert(1)</script> & bold",
            "the semantic name is the literal text, tag included"
        );
        every_ampersand_is_an_entity(&html);
    }

    #[test]
    fn a_body_with_a_script_url_is_projected_as_text_and_not_as_a_link() {
        let (html, _) = rich_body("<a href=\"javascript:alert(1)\">click</a>");

        assert!(
            !html.contains("<a href="),
            "the projection never mints an anchor the sanitiser refused: {html}"
        );
        assert!(
            html.contains("javascript"),
            "the refused target is kept as the text it was: {html}"
        );
        every_ampersand_is_an_entity(&html);
    }

    #[test]
    fn escaping_only_touches_the_four_structural_characters() {
        assert_eq!(
            escape_text("<a href=\"x\">&"),
            "&lt;a href=&quot;x&quot;&gt;&amp;"
        );
        assert_eq!(
            escape_text("plain — Unicode stays"),
            "plain — Unicode stays",
            "an escape is not an encoding: the text a user reads is the text sent"
        );
    }

    // --- actions ------------------------------------------------------------

    fn with_actions(actions: &[&str]) -> Value {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                summary: "with actions".to_owned(),
                actions: action_list(actions),
                ..Draft::default()
            },
            Instant::now(),
        );

        projected(&queue)
    }

    #[test]
    fn three_action_tiers_are_projected_in_three_colours() {
        let tokens = Tokens::builtin_dark();
        let value = with_actions(&["open", "Open", "delete", "Delete", "later", "Later"]);
        let actions = value["stack"]["popups"][0]["actions"]
            .as_array()
            .expect("actions are an array");

        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0]["emphasis"], "primary");
        assert_eq!(actions[1]["emphasis"], "danger");
        assert_eq!(actions[2]["emphasis"], "secondary");
        assert_eq!(
            actions[0]["control"]["rest"]["text"],
            theme::qml_color(tokens.colors.accent)
        );
        assert_eq!(
            actions[1]["control"]["rest"]["text"],
            theme::qml_color(tokens.colors.danger)
        );
        assert_eq!(
            actions[2]["control"]["rest"]["text"],
            theme::qml_color(tokens.colors.text)
        );
        assert_eq!(actions[0]["enabled"], true);
        assert_eq!(actions[0]["disabledReason"], Value::Null);
    }

    #[test]
    fn an_action_the_policy_turns_off_is_projected_with_its_reason() {
        let display = DisplayConfig::default();
        let policy = Policy::from_config(&inox_config::PolicyConfig {
            show_actions: false,
            ..inox_config::PolicyConfig::default()
        });
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                summary: "off".to_owned(),
                actions: action_list(&["open", "Open"]),
                ..Draft::default()
            },
            Instant::now(),
        );
        let model = model(&queue, &display, &policy, SessionState::Unlocked);
        let value = project(
            &model,
            &Tokens::builtin_dark(),
            &display,
            "Inox Notifications",
        );
        let action = &value["stack"]["popups"][0]["actions"][0];

        assert_eq!(action["enabled"], false);
        assert_eq!(
            action["disabledReason"], "Actions are turned off in notifications.toml",
            "a disabled action is explained on the card, not only on hover (D05)"
        );
        assert_eq!(
            action["control"]["disabled"]["text"],
            theme::qml_color(Tokens::builtin_dark().colors.text_muted)
        );
    }

    // --- geometry and placement --------------------------------------------

    #[test]
    fn the_geometry_comes_from_the_tokens_and_the_configuration() {
        let display = DisplayConfig {
            width: 400,
            margin: 20,
            icon_size: 48,
            ..DisplayConfig::default()
        };
        let tokens = Tokens::builtin_dark();
        let geometry = StackGeometry::new(&tokens, &display);

        assert!((geometry.width - 400.0).abs() < f32::EPSILON);
        assert!((geometry.margin - 20.0).abs() < f32::EPSILON);
        assert!((geometry.icon_size - 48.0).abs() < f32::EPSILON);
        assert!(
            (geometry.card_padding - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON,
            "a card is inset by the large step (ui-guidelines §4)"
        );
        assert!(
            (geometry.notice_padding - f32::from(tokens.spacing.md)).abs() < f32::EPSILON,
            "a status line is inset by the medium step (ui-guidelines §3)"
        );
        assert!((geometry.card_gap - f32::from(tokens.spacing.lg)).abs() < f32::EPSILON);
        assert!((geometry.section_gap - f32::from(tokens.spacing.md)).abs() < f32::EPSILON);
        assert!((geometry.control_gap - f32::from(tokens.spacing.md)).abs() < f32::EPSILON);
        assert!((geometry.inline_gap - f32::from(tokens.spacing.sm)).abs() < f32::EPSILON);
    }

    #[test]
    fn the_card_carries_the_three_text_roles_it_draws() {
        let tokens = Tokens::builtin_dark();
        let geometry = StackGeometry::new(&tokens, &DisplayConfig::default());

        assert!(
            (geometry.heading_size - theme::text_size(&tokens, TextRole::Heading)).abs()
                < f32::EPSILON
        );
        assert!(
            (geometry.body_size - theme::text_size(&tokens, TextRole::Body)).abs() < f32::EPSILON
        );
        assert!(
            (geometry.caption_size - theme::text_size(&tokens, TextRole::Caption)).abs()
                < f32::EPSILON
        );
        assert!(
            geometry.caption_size < geometry.body_size
                && geometry.body_size < geometry.heading_size,
            "the hierarchy has to be a hierarchy: {} / {} / {}",
            geometry.caption_size,
            geometry.body_size,
            geometry.heading_size
        );
    }

    #[test]
    fn the_window_is_sized_for_the_popups_it_may_show() {
        let display = DisplayConfig::default();
        let geometry = StackGeometry::new(&Tokens::builtin_dark(), &display);
        let (width, height) = geometry.window_size(&display);

        assert!(width > geometry.width, "the margin is included");
        assert!(height >= geometry.icon_size * f32::from(display.max_visible));

        let taller = DisplayConfig {
            max_visible: 8,
            ..DisplayConfig::default()
        };
        let (_, more) = geometry.window_size(&taller);

        assert!(more > height, "a stack that shows more needs more room");
    }

    #[test]
    fn the_stack_is_aligned_to_the_configured_corner() {
        let cases = [
            (NotificationPosition::TopRight, ("right", "top")),
            (NotificationPosition::TopLeft, ("left", "top")),
            (NotificationPosition::BottomRight, ("right", "bottom")),
            (NotificationPosition::BottomLeft, ("left", "bottom")),
        ];

        for (position, expected) in cases {
            let display = DisplayConfig {
                position,
                ..DisplayConfig::default()
            };
            let value = project(
                &model(
                    &queue_with(&[]),
                    &display,
                    &Policy::default(),
                    SessionState::Unlocked,
                ),
                &Tokens::builtin_dark(),
                &display,
                "Inox Notifications",
            );

            assert_eq!(
                (
                    value["alignment"]["horizontal"].as_str(),
                    value["alignment"]["vertical"].as_str()
                ),
                (Some(expected.0), Some(expected.1))
            );
        }
    }

    #[test]
    fn every_stack_state_projects_and_parses_as_json() {
        for status in [
            StackStatus::Empty,
            StackStatus::Suppressed,
            StackStatus::Showing,
        ] {
            let model = PopupStack {
                status,
                popups: Vec::new(),
                overflow: None,
                notice: Some("a line".to_owned()),
            };
            let value = project(
                &model,
                &Tokens::builtin_dark(),
                &DisplayConfig::default(),
                "Inox Notifications",
            );

            let text = serde_json::to_string(&value).expect("a projection serialises");
            let parsed: Value = serde_json::from_str(&text).expect("and parses back");
            assert_eq!(parsed["stack"]["status"], status_name(&model));
        }
    }

    #[test]
    fn an_icon_is_projected_as_the_space_it_occupies() {
        let mut queue = Queue::new(QueueConfig::default(), TimeoutConfig::default(), 3);
        queue.notify(
            Draft {
                app_icon: Some("firefox".to_owned()),
                summary: "with an icon".to_owned(),
                ..Draft::default()
            },
            Instant::now(),
        );
        let tokens = Tokens::builtin_dark();
        let value = projected(&queue);
        let icon = &value["stack"]["popups"][0]["icon"];

        assert_eq!(icon["present"], true);
        assert_eq!(
            icon["size"].as_f64(),
            Some(f64::from(DisplayConfig::default().icon_size))
        );
        assert_eq!(
            icon["fill"],
            theme::qml_color(tokens.colors.surface_variant)
        );
    }

    #[test]
    fn the_notification_identifier_reaches_qml_as_a_number() {
        let value = projected(&queue_with(&[("app", "hello")]));

        assert_eq!(
            value["stack"]["popups"][0]["id"].as_u64(),
            Some(u64::from(NotificationId::new(1).get()))
        );
    }
}
