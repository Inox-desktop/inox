//! The Qt-free projection QML renders.
//!
//! This file is what `view.rs` was: it makes no decisions of its own, it
//! *resolves* the ones [`crate::view_model`] already made into the concrete
//! values a QML tree binds to — colours as `#AARRGGBB` strings, distances and
//! font sizes as numbers, and every user-visible sentence as the text the
//! catalogs produced. QML then has nothing left to decide: it walks `rows` and
//! draws them.
//!
//! The shape is one JSON document (docs/qml-migration-contract.md §3). The whole
//! document is re-sent on every change; that is the accepted trade of this
//! migration — correctness and a clean boundary first, incremental updates once
//! there is data saying they are needed.
//!
//! ```text
//! { "title", "query", "placeholder", "hint", "heading", "prompt", "notice",
//!   "rows": [ { …, "style": { … } } ],
//!   "banner": { "kind", …, "action": { "label", "type" } } | null,
//!   "status": { "line", "reason", "color", "retry" } | null,
//!   "flags": { "can_retry_index", "can_retry_launch" },
//!   "focus": { "field", "serial" },
//!   "window": { "width", "height" },
//!   "theme": { "name", "colors", "font", "metrics", "styles" } }
//! ```
//!
//! The module is pure: no Qt, no I/O, no clock, so every state above is a unit
//! test and the QML load test only has to prove the tree still reads the values.

use inox_config::LauncherConfig;
use inox_config::Theme;
use serde_json::{Value, json};

use crate::app::{SEARCH_INPUT_ID, State};
use crate::placement::{LogicalSize, Placement};
use crate::theme::{self, Metrics, TextRole};
use crate::view_model::{Banner, BannerKind, Row, Status, ViewModel};

/// How many items in the QML tree must carry an accessibility role.
///
/// The launcher's fixed chrome: the content pane, the search field, the list
/// heading and the key hint. Rows come and go with the query and the banner, so
/// the count is a floor, not an equality. The QML tree counts the same thing and
/// `reportReady` refuses the report when it is lower — a control that lost its
/// `Accessible.role` (root AGENTS.md §7.4) fails the headless test instead of
/// passing silently.
pub const MINIMUM_ACCESSIBLE_ITEMS: i32 = 4;

/// The message a banner's action sends, spelled the way `dispatch` parses it.
fn banner_action_kind(kind: BannerKind) -> &'static str {
    match kind {
        BannerKind::Empty | BannerKind::Unavailable => "retry-index",
        BannerKind::NoMatch => "clear-query",
        BannerKind::Loading => "none",
    }
}

/// The kind's name in the projection, for the QML tree to draw the states apart.
fn banner_kind_name(kind: BannerKind) -> &'static str {
    match kind {
        BannerKind::Loading => "loading",
        BannerKind::Empty => "empty",
        BannerKind::NoMatch => "no-match",
        BannerKind::Unavailable => "unavailable",
    }
}

/// The colour QML draws "no background" with.
fn transparent() -> String {
    theme::qml_color(inox_config::theme::Rgba {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    })
}

/// The projection of `state`, as the document QML parses.
pub fn project(state: &State) -> Value {
    project_model(
        &state.tokens,
        &state.config,
        &state.placement,
        &state.model,
        state.focus_serial,
    )
}

/// The projection, serialised for the bridge.
pub fn to_json(state: &State) -> String {
    project(state).to_string()
}

/// A projection with no state behind it, for a bridge object constructed before
/// the binary installed one.
///
/// The QML tree must not meet `undefined` (every binding would report a
/// `TypeError`), and a fallback projection cannot read a catalog, so its strings
/// are empty — a blank, structurally complete launcher window rather than a
/// window full of errors.
pub fn empty_projection() -> Value {
    let tokens = Theme::builtin_dark();
    let config = LauncherConfig::default();
    let placement = Placement::overlay(
        &config,
        &tokens,
        LogicalSize::new(crate::NOMINAL_OUTPUT_WIDTH, crate::NOMINAL_OUTPUT_HEIGHT),
        None,
    );
    project_model(&tokens, &config, &placement, &crate::app::empty_model(), 0)
}

/// The projection of one view model under one theme.
fn project_model(
    tokens: &Theme,
    config: &LauncherConfig,
    placement: &Placement,
    model: &ViewModel,
    focus_serial: u64,
) -> Value {
    let metrics = Metrics::from_tokens(tokens);

    json!({
        "title": model.title,
        "query": model.query,
        "placeholder": model.placeholder,
        "hint": model.hint,
        "heading": model.title,
        "prompt": model.prompt,
        "notice": model.notice,
        "rows": rows(model, tokens),
        "banner": banner(model.banner.as_ref()),
        "status": status(model.status.as_ref(), tokens),
        "flags": {
            "can_retry_index": model.can_retry_index(),
            "can_retry_launch": model.can_retry_launch(),
        },
        "focus": {
            "field": SEARCH_INPUT_ID,
            "serial": focus_serial,
        },
        "window": {
            "width": placement.size.width,
            "height": placement.size.height,
        },
        "theme": theme_json(tokens, config, placement, &metrics),
    })
}

/// The theme, as the values every binding reads.
fn theme_json(
    tokens: &Theme,
    config: &LauncherConfig,
    placement: &Placement,
    metrics: &Metrics,
) -> Value {
    let colors = &tokens.colors;
    let control = metrics.control_padding();
    let card = metrics.card_padding();

    // One row is as tall as the icon it shows, but never shorter than the type's
    // own line box; the placement policy sizes the surface with the same number
    // (`crate::placement::chrome`), which is what keeps the listed rows inside
    // the window.
    let row = (f32::from(config.icon_size)).max(metrics.space_xl) + metrics.space_sm * 2.0;
    let field = theme::line_height(tokens, TextRole::Title) + metrics.space_sm * 2.0;

    json!({
        "name": tokens.name,
        "colors": {
            "background": theme::qml_color(colors.background),
            "surface": theme::qml_color(colors.surface),
            "surface_variant": theme::qml_color(colors.surface_variant),
            "border": theme::qml_color(colors.border),
            "text": theme::qml_color(colors.text),
            "text_muted": theme::qml_color(colors.text_muted),
            "accent": theme::qml_color(colors.accent),
            "on_accent": theme::qml_color(colors.on_accent),
            "danger": theme::qml_color(colors.danger),
            "warning": theme::qml_color(colors.warning),
            "success": theme::qml_color(colors.success),
        },
        "font": {
            "family": tokens.typography.font_family,
            "mono_family": tokens.typography.font_mono_family,
            "title": px(theme::role_size(tokens, TextRole::Title)),
            "heading": px(theme::role_size(tokens, TextRole::Heading)),
            "body": px(theme::role_size(tokens, TextRole::Body)),
            "caption": px(theme::role_size(tokens, TextRole::Caption)),
        },
        "metrics": {
            "space_xs": px(metrics.space_xs),
            "space_sm": px(metrics.space_sm),
            "space_md": px(metrics.space_md),
            "space_lg": px(metrics.space_lg),
            "space_xl": px(metrics.space_xl),
            "radius_sm": px(metrics.radius_sm),
            "radius_md": px(metrics.radius_md),
            "radius_lg": px(metrics.radius_lg),
            "hairline": px(metrics.hairline),
            "focus_ring": px(metrics.focus_ring),
            "icon_size": px(f32::from(config.icon_size)),
            "row_height": px(row),
            "field_height": px(field),
            "window_padding": px(metrics.space_xl),
            "group_spacing": px(metrics.space_lg),
            "list_spacing": px(metrics.space_sm),
            "heading_gap": px(metrics.space_md),
            "card_padding": px(card.top),
            "control_padding_vertical": px(control.top),
            "control_padding_horizontal": px(control.left),
            "content_width": px(metrics.content_width(placement.size.width)),
        },
        "styles": styles(tokens, metrics),
    })
}

/// The resolved colours of each control, per interaction state.
///
/// QML chooses *which* state a control is in; it never chooses a colour. That is
/// the whole point of putting these here: the guidelines' §5 five states stay one
/// decision, made once, in Rust.
fn styles(tokens: &Theme, metrics: &Metrics) -> Value {
    let colors = &tokens.colors;

    json!({
        "window": {
            "background": theme::qml_color(colors.background),
            "text": theme::qml_color(colors.text),
        },
        "card": {
            "background": theme::qml_color(colors.surface),
            "border": theme::qml_color(colors.border),
            "border_width": px(metrics.hairline),
            "radius": px(metrics.radius_md),
            "text": theme::qml_color(colors.text),
        },
        "field": {
            "background": theme::qml_color(colors.surface),
            "background_hover": theme::qml_color(colors.surface_variant),
            "background_disabled": theme::qml_color(colors.background),
            "border": theme::qml_color(colors.border),
            "border_focused": theme::qml_color(colors.accent),
            "border_width": px(metrics.hairline),
            "focus_width": px(metrics.focus_ring),
            "text": theme::qml_color(colors.text),
            "placeholder": theme::qml_color(colors.text_muted),
            "selection": theme::qml_color(colors.accent),
            "selected_text": theme::qml_color(colors.on_accent),
            "radius": px(metrics.radius_sm),
        },
        // A secondary action: an outline at rest, the alternate surface under the
        // pointer, the accent while pressed (§5). The label colour follows, which
        // is what keeps it readable on all three.
        "action": {
            "background": transparent(),
            "background_hover": theme::qml_color(colors.surface_variant),
            "background_pressed": theme::qml_color(colors.accent),
            "text": theme::qml_color(colors.text),
            "text_pressed": theme::qml_color(colors.on_accent),
            "border": theme::qml_color(colors.border),
            "border_width": px(metrics.hairline),
            "radius": px(metrics.radius_sm),
        },
    })
}

/// The result rows, each with the colours its state resolves to.
fn rows(model: &ViewModel, tokens: &Theme) -> Vec<Value> {
    let metrics = Metrics::from_tokens(tokens);
    model
        .rows
        .iter()
        .map(|row| {
            let mut value = row_json(row);
            if let Value::Object(object) = &mut value {
                object.insert("style".to_owned(), row_style(row, tokens, &metrics));
            }
            value
        })
        .collect()
}

/// One row's text, icon and flags.
fn row_json(row: &Row) -> Value {
    json!({
        "id": row.id,
        "icon": row.icon,
        "label": row.label,
        "truncated": row.truncated,
        "detail": row.detail,
        "selected": row.selected,
        "enabled": row.enabled,
        "disabled_reason": row.disabled_reason,
    })
}

/// The colours one row is drawn with.
///
/// | state | drawn as |
/// |-------|----------|
/// | resting | no background, `text` |
/// | hovered | `surface_variant` background |
/// | pressed | `accent` background, `on_accent` text |
/// | selected | `accent` background, `on_accent` text — the background *is* the selection |
/// | disabled | no background, `text_muted`, with the reason next to it |
///
/// A row that cannot be launched is muted rather than coloured as an error,
/// because it is not one. A row that is both selected and unusable keeps the
/// muted text and no background: the selection can be moved onto it (so that
/// Enter can explain why it will not run), but it must not look launchable.
fn row_style(row: &Row, tokens: &Theme, metrics: &Metrics) -> Value {
    let colors = &tokens.colors;

    let background = if !row.enabled {
        transparent()
    } else if row.selected {
        theme::qml_color(colors.accent)
    } else {
        transparent()
    };
    let hover = if row.enabled && !row.selected {
        theme::qml_color(colors.surface_variant)
    } else {
        background.clone()
    };
    let pressed = if row.enabled {
        theme::qml_color(colors.accent)
    } else {
        transparent()
    };
    let text = if !row.enabled {
        theme::qml_color(colors.text_muted)
    } else if row.selected {
        theme::qml_color(colors.on_accent)
    } else {
        theme::qml_color(colors.text)
    };
    let text_pressed = if row.enabled {
        theme::qml_color(colors.on_accent)
    } else {
        theme::qml_color(colors.text_muted)
    };

    // A row's secondary line: muted at rest, `on_accent` on the selection, because
    // `text_muted` is not readable on the accent and the size difference already
    // marks it as secondary.
    let detail = if row.enabled && row.selected {
        theme::qml_color(colors.on_accent)
    } else {
        theme::qml_color(colors.text_muted)
    };

    // The icon column: the letter of the name on the accent when the entry names
    // an icon, and the catalog's placeholder muted on the alternate surface when
    // it names none. Keeping the two apart is what makes a missing icon visible
    // rather than silent (acceptance D05).
    let (badge_background, badge_color) = if row.icon_placeholder {
        (
            theme::qml_color(colors.surface_variant),
            theme::qml_color(colors.text_muted),
        )
    } else {
        (
            theme::qml_color(colors.accent),
            theme::qml_color(colors.on_accent),
        )
    };

    json!({
        "background": background,
        "background_hover": hover,
        "background_pressed": pressed,
        "text": text,
        "text_pressed": text_pressed,
        "detail": detail,
        "disabled_reason": theme::qml_color(colors.warning),
        "badge_background": badge_background,
        "badge_color": badge_color,
        "radius": px(metrics.radius_sm),
    })
}

/// The state that stands in for the rows, when one does.
fn banner(banner: Option<&Banner>) -> Value {
    let Some(banner) = banner else {
        return Value::Null;
    };

    let action = match (&banner.action, banner.kind.action_message()) {
        (Some(label), Some(_)) => json!({
            "label": label,
            "type": banner_action_kind(banner.kind),
        }),
        _ => Value::Null,
    };

    json!({
        "kind": banner_kind_name(banner.kind),
        "title": banner.title,
        "body": banner.body,
        "hint": banner.hint,
        "action": action,
    })
}

/// The launch line, if there is one, with the colour its outcome resolves to.
fn status(status: Option<&Status>, tokens: &Theme) -> Value {
    let Some(status) = status else {
        return Value::Null;
    };
    let failed = status.reason.is_some();

    json!({
        "line": status.line,
        "reason": status.reason,
        "color": theme::qml_color(if failed {
            tokens.colors.danger
        } else {
            tokens.colors.text
        }),
        "retry": status.retry.as_ref().map(|label| json!({
            "label": label,
            "type": "retry-launch",
        })),
    })
}

/// A distance or a font size, rounded so the document stays readable.
///
/// Two decimal places is far below anything a screen can show, and rounding
/// keeps a 110 % font scale from filling the JSON with `17.600000381469727`.
fn px(value: f32) -> f64 {
    (f64::from(value) * 100.0).round() / 100.0
}

/// The projection's theme block, for the tests that compare it against tokens.
#[cfg(test)]
fn theme_block(value: &Value) -> &Value {
    value
        .get("theme")
        .expect("the projection always carries a theme")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{Failure, FailureKind, ServiceState};
    use crate::index::AppIndex;
    use crate::view_model;
    use inox_common::i18n::{Locale, Localizer};

    fn localizer(tag: &str) -> Localizer {
        crate::i18n::localizer(&Locale::parse(tag).expect("a tag")).expect("the catalogs load")
    }

    struct Fixture {
        tokens: Theme,
        config: LauncherConfig,
        placement: Placement,
        localizer: Localizer,
        index: AppIndex,
        service: ServiceState,
        query: String,
        selected: usize,
        activation: Option<crate::activation::Activation>,
    }

    impl Fixture {
        fn new(tag: &str, entries: Vec<crate::index::IndexedEntry>) -> Self {
            let tokens = Theme::builtin_dark();
            let config = LauncherConfig::default();
            let placement = Placement::overlay(&config, &tokens, LogicalSize::new(1280, 720), None);
            let index = AppIndex::from_entries(entries, 1);
            Self {
                tokens,
                config,
                placement,
                localizer: localizer(tag),
                service: index.state(),
                index,
                query: String::new(),
                selected: 0,
                activation: None,
            }
        }

        /// The projection over an explicit search result.
        fn project_with(&self, hits: &[(&str, &str)]) -> Value {
            let mut engine = crate::search::SearchEngine::new();
            let request = engine.request(&self.query, 128, 8);
            engine.apply(crate::search::SearchResult {
                generation: request.generation,
                query: self.query.clone(),
                truncated: false,
                hits: hits
                    .iter()
                    .map(|(id, name)| crate::search::Hit {
                        id: (*id).to_owned(),
                        name: (*name).to_owned(),
                        generic_name: None,
                        kind: crate::search::MatchKind::Prefix,
                    })
                    .collect(),
            });

            let model = view_model::project(&view_model::Projection {
                localizer: &self.localizer,
                config: &self.config,
                index: Some(&self.index),
                service: &self.service,
                result: engine.current(),
                query: &self.query,
                selected: self.selected,
                activation: self.activation.as_ref(),
                label_budget: self.placement.label_chars,
            });

            project_model(&self.tokens, &self.config, &self.placement, &model, 1)
        }
    }

    fn fixture(tag: &str) -> Fixture {
        Fixture::new(
            tag,
            vec![
                AppIndex::test_entry("firefox.desktop", "Firefox", "firefox"),
                AppIndex::test_entry("editor.desktop", "Text Editor", "editor"),
            ],
        )
    }

    #[test]
    fn every_user_visible_string_comes_from_the_catalogs() {
        let mut fixture = fixture("en-US");
        fixture.query = "fire".to_owned();
        let value = fixture.project_with(&[("firefox.desktop", "Firefox")]);

        assert_eq!(
            value["title"].as_str(),
            Some(fixture.localizer.message("launcher-window-title").as_str())
        );
        assert_eq!(
            value["placeholder"].as_str(),
            Some(
                fixture
                    .localizer
                    .message("launcher-search-placeholder")
                    .as_str()
            )
        );
        assert_eq!(
            value["hint"].as_str(),
            Some(fixture.localizer.message("launcher-search-hint").as_str())
        );
        assert!(
            !value["title"].as_str().unwrap_or_default().is_empty(),
            "a message id leaked instead of text",
        );
    }

    #[test]
    fn the_projection_follows_the_negotiated_language() {
        let en = fixture("en-US");
        let zh = fixture("zh-CN");

        let en_value = en.project_with(&[("firefox.desktop", "Firefox")]);
        let zh_value = zh.project_with(&[("firefox.desktop", "Firefox")]);

        assert_eq!(
            en_value["title"].as_str(),
            Some(en.localizer.message("launcher-window-title").as_str())
        );
        assert_eq!(
            zh_value["title"].as_str(),
            Some(zh.localizer.message("launcher-window-title").as_str())
        );
        assert_ne!(
            en_value["title"], zh_value["title"],
            "the zh-CN catalog must reach the projection, not fall back to English",
        );
        for key in ["placeholder", "hint"] {
            assert!(
                zh_value[key].as_str().is_some_and(|text| !text.is_empty()),
                "{key} is empty in zh-CN",
            );
        }
    }

    #[test]
    fn the_rows_follow_the_search_result_and_mark_the_selection() {
        let mut fixture = fixture("en-US");
        fixture.query = "fi".to_owned();
        fixture.selected = 0;
        let value = fixture.project_with(&[("firefox.desktop", "Firefox")]);

        let rows = value["rows"].as_array().expect("a row list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"].as_str(), Some("firefox.desktop"));
        assert_eq!(rows[0]["label"].as_str(), Some("Firefox"));
        assert_eq!(rows[0]["selected"].as_bool(), Some(true));
        assert_eq!(rows[0]["enabled"].as_bool(), Some(true));
    }

    #[test]
    fn the_theme_colours_are_the_token_colours_as_qt_argb_strings() {
        let fixture = fixture("en-US");
        let value = fixture.project_with(&[]);
        let colors = fixture.tokens.colors;

        for (name, colour) in [
            ("background", colors.background),
            ("surface", colors.surface),
            ("border", colors.border),
            ("text", colors.text),
            ("text_muted", colors.text_muted),
            ("accent", colors.accent),
            ("on_accent", colors.on_accent),
            ("danger", colors.danger),
            ("warning", colors.warning),
        ] {
            assert_eq!(
                value["theme"]["colors"][name].as_str(),
                Some(theme::qml_color(colour).as_str()),
                "{name} is not the token colour",
            );
        }

        assert_eq!(
            value["theme"]["styles"]["card"]["background"].as_str(),
            Some(theme::qml_color(colors.surface).as_str())
        );
        assert_eq!(
            value["theme"]["styles"]["field"]["border_focused"].as_str(),
            Some(theme::qml_color(colors.accent).as_str()),
            "the focus ring is the accent token",
        );
        assert_ne!(
            value["theme"]["styles"]["window"]["background"],
            value["theme"]["styles"]["card"]["background"],
            "a card that is the same colour as the window has no boundary",
        );
    }

    #[test]
    fn the_sizes_are_the_token_derivations() {
        let fixture = fixture("en-US");
        let value = fixture.project_with(&[]);
        let metrics = Metrics::from_tokens(&fixture.tokens);

        assert_eq!(
            value["theme"]["font"]["title"].as_f64(),
            Some(px(theme::role_size(&fixture.tokens, TextRole::Title)))
        );
        assert_eq!(
            value["theme"]["font"]["body"].as_f64(),
            Some(px(theme::text_size(&fixture.tokens)))
        );
        assert_eq!(
            value["theme"]["font"]["caption"].as_f64(),
            Some(px(theme::role_size(&fixture.tokens, TextRole::Caption)))
        );
        assert_eq!(
            value["theme"]["metrics"]["space_lg"].as_f64(),
            Some(px(metrics.space_lg))
        );
        assert_eq!(
            value["theme"]["metrics"]["hairline"].as_f64(),
            Some(px(metrics.hairline))
        );
        assert_eq!(
            value["theme"]["metrics"]["focus_ring"].as_f64(),
            Some(px(metrics.focus_ring))
        );
        assert_eq!(
            value["theme"]["metrics"]["radius_md"].as_f64(),
            Some(px(metrics.radius_md))
        );
        assert_eq!(
            value["theme"]["metrics"]["icon_size"].as_f64(),
            Some(px(f32::from(fixture.config.icon_size)))
        );
        assert_eq!(
            value["theme"]["metrics"]["window_padding"].as_f64(),
            Some(px(metrics.space_xl))
        );
        assert_eq!(
            value["theme"]["metrics"]["content_width"].as_f64(),
            Some(px(metrics.content_width(fixture.placement.size.width)))
        );
        assert_eq!(
            value["theme"]["font"]["family"].as_str(),
            Some(fixture.tokens.typography.font_family.as_str())
        );
    }

    #[test]
    fn a_two_hundred_percent_font_scale_doubles_the_projected_type() {
        let mut fixture = fixture("en-US");
        let normal = fixture.project_with(&[]);
        fixture.tokens.typography.scale *= 2.0;
        let doubled = fixture.project_with(&[]);

        let normal_body = normal["theme"]["font"]["body"].as_f64().expect("a size");
        let doubled_body = doubled["theme"]["font"]["body"].as_f64().expect("a size");
        assert_eq!(doubled_body, normal_body * 2.0);

        let normal_row = normal["theme"]["metrics"]["row_height"]
            .as_f64()
            .expect("a height");
        let doubled_field = doubled["theme"]["metrics"]["field_height"]
            .as_f64()
            .expect("a height");
        let normal_field = normal["theme"]["metrics"]["field_height"]
            .as_f64()
            .expect("a height");
        assert!(
            doubled_field > normal_field,
            "the search field grows with the type: {doubled_field} vs {normal_field}",
        );
        assert!(normal_row > 0.0);
    }

    #[test]
    fn the_selected_row_is_drawn_in_the_accent_colour() {
        let mut fixture = fixture("en-US");
        fixture.selected = 0;
        let value = fixture.project_with(&[("firefox.desktop", "Firefox")]);
        let style = &value["rows"][0]["style"];

        assert_eq!(
            style["background"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.accent).as_str())
        );
        assert_eq!(
            style["text"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.on_accent).as_str())
        );
    }

    #[test]
    fn the_rows_states_are_drawn_apart() {
        // The guidelines' §5 states, as this surface draws a row: resting,
        // hovered, pressed, selected and unusable. `Fixture` selects the first
        // row by default, so "resting" is the fixture that selects nothing.
        let mut plain = fixture("en-US");
        plain.selected = usize::MAX;
        let resting = plain.project_with(&[("editor.desktop", "Text Editor")]);
        let mut selected = fixture("en-US");
        selected.selected = 0;
        let highlighted = selected.project_with(&[("editor.desktop", "Text Editor")]);

        // A row that cannot be launched stays muted and says why.
        let unusable = {
            let mut entry = AppIndex::test_entry("htop.desktop", "htop", "htop");
            entry.terminal = true;
            Fixture::new("en-US", vec![entry]).project_with(&[("htop.desktop", "htop")])
        };

        // A row that names an icon draws the icon tile; one that names none draws
        // the placeholder. `AppIndex::test_entry` names no icon, so the icon row
        // is spelled out explicitly.
        let placeholder_row = {
            let mut bare = fixture("en-US");
            bare.index = AppIndex::from_entries(
                vec![AppIndex::test_entry("bare.desktop", "Bare", "bare")],
                1,
            );
            bare.service = bare.index.state();
            bare.project_with(&[("bare.desktop", "Bare")])
        };
        let icon_row = {
            let mut entry = AppIndex::test_entry("editor.desktop", "Text Editor", "editor");
            entry.icon = Some("accessories-text-editor".to_owned());
            let mut named = Fixture::new("en-US", vec![entry]);
            named.selected = usize::MAX;
            named.project_with(&[("editor.desktop", "Text Editor")])
        };

        let resting_style = &resting["rows"][0]["style"];
        let selected_style = &highlighted["rows"][0]["style"];
        let unusable_style = &unusable["rows"][0]["style"];
        let bare_style = &placeholder_row["rows"][0]["style"];
        let named_style = &icon_row["rows"][0]["style"];

        let looks: Vec<(&str, Value, Value)> = vec![
            (
                "resting",
                resting_style["background"].clone(),
                resting_style["text"].clone(),
            ),
            (
                "hovered",
                resting_style["background_hover"].clone(),
                resting_style["text"].clone(),
            ),
            (
                "selected",
                selected_style["background"].clone(),
                selected_style["text"].clone(),
            ),
            (
                "unusable",
                unusable_style["background"].clone(),
                unusable_style["text"].clone(),
            ),
        ];
        for (index, (name, background, text)) in looks.iter().enumerate() {
            for (other_name, other_background, other_text) in looks.iter().skip(index + 1) {
                assert!(
                    background != other_background || text != other_text,
                    "{name} and {other_name} are drawn the same way",
                );
            }
        }

        // Pressing an unselected row deliberately shows the selection it is about
        // to make (`row_style`'s table): the press is the preview, so it is the
        // accent token rather than a fifth colour nobody derived from anything.
        assert_eq!(
            resting_style["background_pressed"].as_str(),
            Some(theme::qml_color(plain.tokens.colors.accent).as_str()),
        );
        assert_eq!(
            resting_style["text_pressed"].as_str(),
            Some(theme::qml_color(plain.tokens.colors.on_accent).as_str()),
        );

        assert_ne!(
            named_style["badge_background"], bare_style["badge_background"],
            "an entry that names an icon and one that does not are told apart",
        );
        assert_ne!(named_style["badge_color"], bare_style["badge_color"]);
    }

    #[test]
    fn an_unusable_row_is_muted_and_carries_its_reason() {
        let mut entry = AppIndex::test_entry("htop.desktop", "htop", "htop");
        entry.terminal = true;
        entry.icon = Some("utilities-terminal".to_owned());
        let mut fixture = Fixture::new("en-US", vec![entry]);
        let value = fixture.project_with(&[("htop.desktop", "htop")]);

        let row = &value["rows"][0];
        assert_eq!(row["enabled"].as_bool(), Some(false));
        assert!(
            row["disabled_reason"]
                .as_str()
                .is_some_and(|reason| reason.contains(
                    &fixture
                        .localizer
                        .message("launcher-failure-missing-terminal")
                )),
            "{:?}",
            row["disabled_reason"],
        );
        assert_eq!(
            row["style"]["text"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.text_muted).as_str()),
            "a row that cannot be launched must not look like the selection",
        );
        assert_ne!(
            row["style"]["background"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.accent).as_str())
        );

        // The same row, selected: still muted and still not filled.
        fixture.selected = 0;
        let selected = fixture.project_with(&[("htop.desktop", "htop")]);
        assert_eq!(
            selected["rows"][0]["style"]["text"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.text_muted).as_str())
        );
        assert_ne!(
            selected["rows"][0]["style"]["background"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.accent).as_str())
        );
    }

    #[test]
    fn an_empty_query_lists_every_application_and_says_what_typing_does() {
        let fixture = fixture("en-US");
        let value = fixture.project_with(&[
            ("editor.desktop", "Text Editor"),
            ("firefox.desktop", "Firefox"),
        ]);

        assert_eq!(value["rows"].as_array().map(Vec::len), Some(2));
        assert_eq!(value["banner"], Value::Null);
        assert_eq!(
            value["prompt"].as_str(),
            Some(fixture.localizer.message("launcher-list-prompt").as_str())
        );
    }

    #[test]
    fn a_query_that_matches_nothing_offers_the_clear_action() {
        let mut fixture = fixture("en-US");
        fixture.query = "zzzz".to_owned();
        fixture.service = ServiceState::Ready {
            generation: 1,
            entries: 2,
            skipped: 0,
            directories: Vec::new(),
        };
        let value = fixture.project_with(&[]);

        assert_eq!(value["rows"].as_array().map(Vec::len), Some(0));
        assert_eq!(value["banner"]["kind"].as_str(), Some("no-match"));
        assert_eq!(
            value["banner"]["title"].as_str(),
            Some(
                fixture
                    .localizer
                    .message("launcher-state-nomatch-title")
                    .as_str()
            )
        );
        assert_eq!(
            value["banner"]["action"]["type"].as_str(),
            Some("clear-query"),
            "a query that matched nothing is cleared, not re-read",
        );
        assert_eq!(value["flags"]["can_retry_index"].as_bool(), Some(false));
    }

    #[test]
    fn an_unreadable_index_offers_the_retry_action() {
        let mut fixture = fixture("en-US");
        fixture.service = ServiceState::Unavailable {
            generation: 1,
            failure: Failure::service(FailureKind::IndexUnavailable {
                detail: "no HOME".to_owned(),
            }),
        };
        let value = fixture.project_with(&[]);

        assert_eq!(value["banner"]["kind"].as_str(), Some("unavailable"));
        assert_eq!(
            value["banner"]["action"]["type"].as_str(),
            Some("retry-index")
        );
        assert_eq!(
            value["banner"]["action"]["label"].as_str(),
            Some(
                fixture
                    .localizer
                    .message("launcher-state-action-retry")
                    .as_str()
            )
        );
        assert_eq!(value["flags"]["can_retry_index"].as_bool(), Some(true));
    }

    #[test]
    fn a_loading_index_offers_no_action() {
        let mut fixture = fixture("en-US");
        fixture.service = ServiceState::Loading { generation: 2 };
        let value = fixture.project_with(&[]);

        assert_eq!(value["banner"]["kind"].as_str(), Some("loading"));
        assert_eq!(value["banner"]["action"], Value::Null);
        assert_eq!(value["flags"]["can_retry_index"].as_bool(), Some(false));
    }

    #[test]
    fn an_empty_index_names_the_directories_it_looked_in() {
        let mut fixture = Fixture::new("en-US", Vec::new());
        fixture.service = ServiceState::Ready {
            generation: 1,
            entries: 0,
            skipped: 0,
            directories: vec![std::path::PathBuf::from("/usr/share")],
        };
        let value = fixture.project_with(&[]);

        assert_eq!(value["banner"]["kind"].as_str(), Some("empty"));
        assert!(
            value["banner"]["body"]
                .as_str()
                .unwrap_or_default()
                .contains("/usr/share")
        );
        assert_eq!(
            value["banner"]["action"]["type"].as_str(),
            Some("retry-index")
        );
    }

    #[test]
    fn a_failed_launch_is_drawn_in_the_danger_colour_with_a_retry() {
        let mut fixture = fixture("en-US");
        fixture.activation = Some(crate::activation::Activation {
            selection: 1,
            entry: "firefox.desktop".to_owned(),
            name: "Firefox".to_owned(),
            outcome: crate::activation::LaunchOutcome::Failed {
                failure: Failure::for_entry(
                    FailureKind::SpawnFailed {
                        detail: "Permission denied".to_owned(),
                    },
                    "Firefox",
                ),
            },
        });
        let value = fixture.project_with(&[("firefox.desktop", "Firefox")]);

        let status = &value["status"];
        assert!(
            status["reason"]
                .as_str()
                .unwrap_or_default()
                .contains("Permission denied"),
            "{status}",
        );
        assert_eq!(
            status["color"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.danger).as_str())
        );
        assert_eq!(status["retry"]["type"].as_str(), Some("retry-launch"));
        assert_eq!(value["flags"]["can_retry_launch"].as_bool(), Some(true));
    }

    #[test]
    fn a_launch_in_progress_is_drawn_in_the_text_colour_without_a_retry() {
        let mut fixture = fixture("en-US");
        fixture.activation = Some(crate::activation::Activation {
            selection: 1,
            entry: "firefox.desktop".to_owned(),
            name: "Firefox".to_owned(),
            outcome: crate::activation::LaunchOutcome::Requested,
        });
        let value = fixture.project_with(&[("firefox.desktop", "Firefox")]);

        assert_eq!(
            value["status"]["color"].as_str(),
            Some(theme::qml_color(fixture.tokens.colors.text).as_str())
        );
        assert_eq!(value["status"]["retry"], Value::Null);
        assert_eq!(value["flags"]["can_retry_launch"].as_bool(), Some(false));
    }

    #[test]
    fn a_skipped_entry_is_reported_as_a_notice() {
        let mut fixture = fixture("en-US");
        fixture.service = ServiceState::Ready {
            generation: 1,
            entries: 2,
            skipped: 5,
            directories: Vec::new(),
        };
        let value = fixture.project_with(&[]);

        assert!(
            value["notice"]
                .as_str()
                .is_some_and(|notice| notice.contains('5')),
            "{:?}",
            value["notice"],
        );
    }

    #[test]
    fn the_focus_target_and_the_window_size_are_projected() {
        let fixture = fixture("en-US");
        let value = fixture.project_with(&[]);

        assert_eq!(value["focus"]["field"].as_str(), Some(SEARCH_INPUT_ID));
        assert_eq!(value["focus"]["serial"].as_u64(), Some(1));
        assert_eq!(
            value["window"]["width"].as_u64(),
            Some(u64::from(fixture.placement.size.width))
        );
        assert_eq!(
            value["window"]["height"].as_u64(),
            Some(u64::from(fixture.placement.size.height))
        );
    }

    #[test]
    fn the_document_is_valid_json_with_every_key_the_tree_binds_to() {
        let fixture = fixture("en-US");
        let value = fixture.project_with(&[]);
        let serialised = value.to_string();
        let parsed: Value = serde_json::from_str(&serialised).expect("the projection is JSON");

        for key in [
            "title",
            "query",
            "placeholder",
            "hint",
            "heading",
            "prompt",
            "notice",
            "rows",
            "banner",
            "status",
            "flags",
            "focus",
            "window",
            "theme",
        ] {
            assert!(parsed.get(key).is_some(), "the projection lacks {key}");
        }

        let theme = theme_block(&parsed);
        for key in ["name", "colors", "font", "metrics", "styles"] {
            assert!(theme.get(key).is_some(), "the theme lacks {key}");
        }
        for style in ["window", "card", "field", "action"] {
            assert!(
                theme["styles"].get(style).is_some(),
                "the styles lack {style}"
            );
        }
    }

    #[test]
    fn the_fallback_projection_is_total_and_empty() {
        let value = empty_projection();

        assert_eq!(value["rows"].as_array().map(Vec::len), Some(0));
        assert_eq!(value["banner"], Value::Null);
        assert_eq!(value["status"], Value::Null);
        assert_eq!(value["title"].as_str(), Some(""));
        assert_eq!(value["focus"]["serial"].as_u64(), Some(0));
        assert!(
            value["theme"]["styles"]["window"]["background"]
                .as_str()
                .is_some_and(|colour| colour.len() == 9),
            "even the fallback carries a real colour",
        );
    }

    #[test]
    fn every_role_name_is_distinct() {
        let roles = [
            TextRole::Title,
            TextRole::Heading,
            TextRole::Body,
            TextRole::Caption,
        ];
        for (index, role) in roles.iter().enumerate() {
            for other in roles.iter().skip(index + 1) {
                assert_ne!(role.name(), other.name());
            }
        }
    }
}
